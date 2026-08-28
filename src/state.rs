//! Durable user intent used to rebuild a managed session after a crash or reboot.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use fs4::{FileExt, TryLockError};
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::mihomo::OperatingMode;

const STATE_SCHEMA_VERSION: u8 = 1;
const STATE_FILE: &str = "desired.json";
const STATE_LOCK: &str = ".desired.lock";
const MAX_STATE_BYTES: u64 = 256 * 1024;
const MAX_SELECTIONS: usize = 128;
const MAX_KNOWN_GOOD_PER_GROUP: usize = 4;
const MAX_NAME_BYTES: usize = 512;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum StateError {
    #[error("cannot initialize the persistent state directory")]
    Initialization,
    #[error("cannot lock the persistent state")]
    Lock,
    #[error("the persistent state is busy")]
    Busy,
    #[error("cannot read the persistent state")]
    Read,
    #[error("the persistent state is invalid")]
    Invalid,
    #[error("cannot write the persistent state")]
    Write,
}

#[derive(Clone)]
pub struct DesiredStateStore {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredStateSnapshot {
    pub active_profile: Option<String>,
    pub profile_sha256: Option<String>,
    pub mode: OperatingMode,
    pub selections: BTreeMap<String, String>,
    pub known_good: BTreeMap<String, Vec<String>>,
    pub last_recovery_unix_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DesiredState {
    schema_version: u8,
    active_profile: Option<String>,
    profile_sha256: Option<String>,
    mode: String,
    selections: BTreeMap<String, String>,
    known_good: BTreeMap<String, Vec<KnownGoodSelection>>,
    last_recovery_unix_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KnownGoodSelection {
    name: String,
    verified_unix_seconds: u64,
}

impl Default for DesiredState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            active_profile: None,
            profile_sha256: None,
            mode: OperatingMode::Global.as_str().to_owned(),
            selections: BTreeMap::new(),
            known_good: BTreeMap::new(),
            last_recovery_unix_seconds: None,
        }
    }
}

impl DesiredStateStore {
    pub fn new(root: PathBuf) -> Result<Self, StateError> {
        ensure_private_directory(&root)?;
        Ok(Self { root })
    }

    pub fn load(&self) -> Result<DesiredStateSnapshot, StateError> {
        let _lock = acquire_lock(&self.root)?;
        let state = read_state(&self.root)?.unwrap_or_default();
        validate_state(&state)?;
        Ok(snapshot(state))
    }

    pub fn record_profile(&self, profile: &str, contents: &[u8]) -> Result<(), StateError> {
        validate_profile_id(profile)?;
        let digest = profile_digest(contents);
        self.mutate(|state| {
            if state.active_profile.as_deref() != Some(profile) {
                state.selections.clear();
                state.known_good.clear();
            }
            state.active_profile = Some(profile.to_owned());
            state.profile_sha256 = Some(digest);
        })
    }

    pub fn record_runtime(
        &self,
        profile: &str,
        contents: &[u8],
        mode: OperatingMode,
    ) -> Result<(), StateError> {
        validate_profile_id(profile)?;
        let digest = profile_digest(contents);
        self.mutate(|state| {
            if state.active_profile.as_deref() != Some(profile) {
                state.selections.clear();
                state.known_good.clear();
            }
            state.active_profile = Some(profile.to_owned());
            state.profile_sha256 = Some(digest);
            state.mode = mode.as_str().to_owned();
        })
    }

    pub fn record_mode(&self, mode: OperatingMode) -> Result<(), StateError> {
        self.mutate(|state| state.mode = mode.as_str().to_owned())
    }

    pub fn record_selection(&self, group: &str, selection: &str) -> Result<(), StateError> {
        validate_name(group)?;
        validate_name(selection)?;
        self.mutate(|state| {
            state
                .selections
                .insert(group.to_owned(), selection.to_owned());
        })
    }

    pub fn record_healthy_selection(
        &self,
        group: &str,
        selection: &str,
        verified_unix_seconds: u64,
    ) -> Result<(), StateError> {
        validate_name(group)?;
        validate_name(selection)?;
        self.mutate(|state| {
            let entries = state.known_good.entry(group.to_owned()).or_default();
            entries.retain(|entry| entry.name != selection);
            entries.insert(
                0,
                KnownGoodSelection {
                    name: selection.to_owned(),
                    verified_unix_seconds,
                },
            );
            entries.truncate(MAX_KNOWN_GOOD_PER_GROUP);
        })
    }

    pub fn mark_recovery_attempt(&self, unix_seconds: u64) -> Result<(), StateError> {
        self.mutate(|state| state.last_recovery_unix_seconds = Some(unix_seconds))
    }

    fn mutate(&self, update: impl FnOnce(&mut DesiredState)) -> Result<(), StateError> {
        let _lock = acquire_lock(&self.root)?;
        let mut state = read_state(&self.root)?.unwrap_or_default();
        validate_state(&state)?;
        update(&mut state);
        validate_state(&state)?;
        write_state(&self.root, &state)
    }
}

#[must_use]
pub fn profile_digest(contents: &[u8]) -> String {
    let digest = digest(&SHA256, contents);
    let mut output = String::with_capacity(digest.as_ref().len() * 2);
    for byte in digest.as_ref() {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

#[must_use]
pub fn unix_seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn snapshot(state: DesiredState) -> DesiredStateSnapshot {
    DesiredStateSnapshot {
        active_profile: state.active_profile,
        profile_sha256: state.profile_sha256,
        mode: OperatingMode::from_api(&state.mode).unwrap_or(OperatingMode::Global),
        selections: state.selections,
        known_good: state
            .known_good
            .into_iter()
            .map(|(group, entries)| (group, entries.into_iter().map(|entry| entry.name).collect()))
            .collect(),
        last_recovery_unix_seconds: state.last_recovery_unix_seconds,
    }
}

fn ensure_private_directory(path: &Path) -> Result<(), StateError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(StateError::Initialization);
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|_| StateError::Initialization)?;
        }
        Err(_) => return Err(StateError::Initialization),
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| StateError::Initialization)
}

fn acquire_lock(root: &Path) -> Result<File, StateError> {
    let path = root.join(STATE_LOCK);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .map_err(|_| StateError::Lock)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|_| StateError::Lock)?;
    match FileExt::try_lock(&file) {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(StateError::Busy),
        Err(TryLockError::Error(_)) => Err(StateError::Lock),
    }
}

fn read_state(root: &Path) -> Result<Option<DesiredState>, StateError> {
    let path = root.join(STATE_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(StateError::Read),
    };
    if !metadata.file_type().is_file()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.len() > MAX_STATE_BYTES
    {
        return Err(StateError::Invalid);
    }
    let contents = fs::read(path).map_err(|_| StateError::Read)?;
    serde_json::from_slice(&contents)
        .map(Some)
        .map_err(|_| StateError::Invalid)
}

fn write_state(root: &Path, state: &DesiredState) -> Result<(), StateError> {
    let contents = serde_json::to_vec(state).map_err(|_| StateError::Write)?;
    if contents.len() as u64 > MAX_STATE_BYTES {
        return Err(StateError::Invalid);
    }

    for _ in 0..16 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = root.join(format!(".desired.tmp-{}-{sequence}", std::process::id()));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(StateError::Write),
        };
        let result = file
            .write_all(&contents)
            .and_then(|()| file.sync_all())
            .and_then(|()| fs::rename(&temporary, root.join(STATE_FILE)))
            .and_then(|()| File::open(root)?.sync_all());
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
            return Err(StateError::Write);
        }
        return Ok(());
    }
    Err(StateError::Write)
}

fn validate_state(state: &DesiredState) -> Result<(), StateError> {
    if state.schema_version != STATE_SCHEMA_VERSION
        || OperatingMode::from_api(&state.mode).is_none()
        || state.selections.len() > MAX_SELECTIONS
        || state.known_good.len() > MAX_SELECTIONS
    {
        return Err(StateError::Invalid);
    }
    if let Some(profile) = state.active_profile.as_deref() {
        validate_profile_id(profile)?;
    }
    match (&state.active_profile, &state.profile_sha256) {
        (None, None) => {}
        (Some(_), Some(digest)) if valid_digest(digest) => {}
        _ => return Err(StateError::Invalid),
    }
    for (group, selection) in &state.selections {
        validate_name(group)?;
        validate_name(selection)?;
    }
    for (group, entries) in &state.known_good {
        validate_name(group)?;
        if entries.len() > MAX_KNOWN_GOOD_PER_GROUP {
            return Err(StateError::Invalid);
        }
        for entry in entries {
            validate_name(&entry.name)?;
        }
    }
    Ok(())
}

fn validate_profile_id(id: &str) -> Result<(), StateError> {
    let mut characters = id.chars();
    let Some(first) = characters.next() else {
        return Err(StateError::Invalid);
    };
    if !first.is_ascii_alphanumeric()
        || id.len() > 40
        || !characters
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(StateError::Invalid);
    }
    Ok(())
}

fn validate_name(value: &str) -> Result<(), StateError> {
    if value.is_empty() || value.len() > MAX_NAME_BYTES || value.chars().any(char::is_control) {
        return Err(StateError::Invalid);
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

    use super::{DesiredStateStore, StateError, acquire_lock, profile_digest};
    use crate::mihomo::OperatingMode;

    #[test]
    fn state_round_trip_is_private_and_deterministic() {
        let root = temporary_directory("round-trip");
        let store = DesiredStateStore::new(root.clone()).expect("store should initialize");

        store
            .record_profile("biu", b"profile contents")
            .expect("profile should persist");
        store
            .record_mode(OperatingMode::Rule)
            .expect("mode should persist");
        store
            .record_selection("GLOBAL", "fast-node")
            .expect("selection should persist");
        store
            .record_healthy_selection("GLOBAL", "fast-node", 42)
            .expect("known-good selection should persist");
        store
            .mark_recovery_attempt(43)
            .expect("recovery time should persist");

        let snapshot = store.load().expect("state should load");
        assert_eq!(snapshot.active_profile.as_deref(), Some("biu"));
        assert_eq!(
            snapshot.profile_sha256.as_deref(),
            Some(profile_digest(b"profile contents").as_str())
        );
        assert_eq!(snapshot.mode, OperatingMode::Rule);
        assert_eq!(snapshot.selections["GLOBAL"], "fast-node");
        assert_eq!(snapshot.known_good["GLOBAL"], ["fast-node"]);
        assert_eq!(snapshot.last_recovery_unix_seconds, Some(43));
        assert_eq!(
            fs::metadata(root.join("desired.json"))
                .expect("state metadata should exist")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn invalid_or_overgrown_state_is_rejected() {
        let root = temporary_directory("invalid");
        let store = DesiredStateStore::new(root.clone()).expect("store should initialize");
        assert!(matches!(
            store.record_selection("GLOBAL\n", "node"),
            Err(StateError::Invalid)
        ));
        assert!(matches!(
            store.record_profile("../escape", b"profile"),
            Err(StateError::Invalid)
        ));
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn contended_state_lock_fails_fast_and_recovers_after_release() {
        let root = temporary_directory("busy");
        let _store = DesiredStateStore::new(root.clone()).expect("store should initialize");
        let held = acquire_lock(&root).expect("first lock should be acquired");

        assert!(matches!(acquire_lock(&root), Err(StateError::Busy)));
        drop(held);
        assert!(acquire_lock(&root).is_ok());
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn newest_healthy_selection_wins_and_history_is_bounded() {
        let root = temporary_directory("known-good");
        let store = DesiredStateStore::new(root.clone()).expect("store should initialize");
        for index in 0..8 {
            store
                .record_healthy_selection("GLOBAL", &format!("node-{index}"), index)
                .expect("selection should persist");
        }
        store
            .record_healthy_selection("GLOBAL", "node-5", 10)
            .expect("duplicate should move to the front");

        assert_eq!(
            store.load().expect("state should load").known_good["GLOBAL"],
            ["node-5", "node-7", "node-6", "node-4"]
        );
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    fn temporary_directory(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should follow the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mihoterm-state-{label}-{}-{nonce}",
            std::process::id()
        ))
    }
}
