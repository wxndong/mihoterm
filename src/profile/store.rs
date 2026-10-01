use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use fs4::{FileExt, TryLockError};
use reqwest::{Client, redirect::Policy};

use super::{
    ProfileError, ProfileSource, ProfileSourceSummary, source::MAX_PROFILE_BYTES,
    validate::validate_profile,
};

const PROFILE_FILE: &str = "profile.yaml";
const BACKUP_FILE: &str = "profile.previous.yaml";
const SOURCE_FILE: &str = "source.toml";
const BACKUP_SOURCE_FILE: &str = "source.previous.toml";
const LOCK_FILE: &str = ".lock";
const MAX_DESCRIPTOR_BYTES: u64 = 64 * 1024;
// Many subscription services use this de facto client identifier to return
// Mihomo-compatible YAML instead of an encoded generic URI list.
const SUBSCRIPTION_USER_AGENT: &str = "clash.meta";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct ProfileStore {
    root: PathBuf,
    client: Client,
    fallback_client: Option<Client>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSummary {
    pub id: String,
    pub has_backup: bool,
    pub source: ProfileSourceSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceReplacement {
    pub inherited_fallback_removed: bool,
}

impl SourceReplacement {
    #[must_use]
    pub fn notice(self) -> Option<&'static str> {
        self.inherited_fallback_removed.then_some(
            "Previous custom fallback does not match this subscription; using the subscription's own groups.",
        )
    }
}

struct SourceSwap {
    current: Vec<u8>,
    backup: Vec<u8>,
}

impl ProfileStore {
    pub fn new(root: PathBuf) -> Result<Self, ProfileError> {
        Ok(Self {
            root,
            client: subscription_client(None)?,
            fallback_client: None,
        })
    }

    /// Try the verified managed loopback proxy only if the direct HTTPS request fails.
    pub fn with_download_fallback(mut self, proxy: reqwest::Proxy) -> Result<Self, ProfileError> {
        self.fallback_client = Some(subscription_client(Some(proxy))?);
        Ok(self)
    }

    async fn load_source(&self, source: &ProfileSource) -> Result<Vec<u8>, ProfileError> {
        let contents = self.load_raw_source(source).await?;
        source.apply_fallback(&contents)
    }

    async fn load_raw_source(&self, source: &ProfileSource) -> Result<Vec<u8>, ProfileError> {
        match source.load_raw(&self.client).await {
            Err(ProfileError::DownloadRequest | ProfileError::DownloadStatus(_))
                if self.fallback_client.is_some() =>
            {
                source
                    .load_raw(
                        self.fallback_client
                            .as_ref()
                            .expect("fallback client checked"),
                    )
                    .await
            }
            result => result,
        }
    }

    pub async fn add(&self, id: &str, source: ProfileSource) -> Result<(), ProfileError> {
        validate_id(id)?;
        let contents = self.load_source(&source).await?;
        validate_profile(&contents)?;
        let _lock = acquire_lock(&self.root)?;
        let directory = self.profile_dir(id);
        if directory.exists() {
            return Err(ProfileError::AlreadyExists);
        }

        fs::create_dir(&directory).map_err(|_| ProfileError::StorageInitialization)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| ProfileError::StorageInitialization)?;

        let result = (|| {
            write_new_private(
                &directory.join(SOURCE_FILE),
                source.descriptor()?.as_bytes(),
            )?;
            write_new_private(&directory.join(PROFILE_FILE), &contents)?;
            sync_directory(&directory)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&directory);
        }
        result
    }

    pub fn source(&self, id: &str) -> Result<ProfileSource, ProfileError> {
        validate_id(id)?;
        let _lock = acquire_lock(&self.root)?;
        read_source(&self.existing_profile_dir(id)?)
    }

    pub async fn configure_policy(
        &self,
        id: &str,
        group: String,
        codex_log_db: Option<PathBuf>,
    ) -> Result<(), ProfileError> {
        validate_id(id)?;
        let _lock = acquire_lock(&self.root)?;
        let directory = self.existing_profile_dir(id)?;
        let source = read_source(&directory)?.with_managed_policy(group, codex_log_db)?;
        let contents = self.load_source(&source).await?;
        validate_profile(&contents)?;
        replace_source_with_backup(&directory, &source, &contents)
    }

    pub async fn disable_policy(&self, id: &str) -> Result<(), ProfileError> {
        validate_id(id)?;
        let _lock = acquire_lock(&self.root)?;
        let directory = self.existing_profile_dir(id)?;
        let mut source = read_source(&directory)?;
        source.clear_fallback();
        let contents = self.load_raw_source(&source).await?;
        validate_profile(&contents)?;
        replace_source_with_backup(&directory, &source, &contents)
    }

    pub async fn update(&self, id: &str) -> Result<(), ProfileError> {
        validate_id(id)?;
        let _lock = acquire_lock(&self.root)?;
        let directory = self.existing_profile_dir(id)?;
        let source = read_source(&directory)?;
        let contents = self.load_source(&source).await?;
        validate_profile(&contents)?;
        replace_with_backup(&directory, &contents)
    }

    pub async fn replace_source(
        &self,
        id: &str,
        mut source: ProfileSource,
    ) -> Result<SourceReplacement, ProfileError> {
        validate_id(id)?;
        let _lock = acquire_lock(&self.root)?;
        let directory = self.existing_profile_dir(id)?;
        let inherited = source.retain_policy_from(&read_source(&directory)?);
        let raw = self.load_raw_source(&source).await?;
        validate_profile(&raw)?;
        // Download once: an incompatible inherited overlay must not force a
        // second request or replace the provider's own groups with guessed nodes.
        let (contents, inherited_fallback_removed) = match source.apply_fallback(&raw) {
            Err(ProfileError::InvalidFallbackPolicy) if inherited => {
                source.clear_fallback();
                (raw, true)
            }
            result => (result?, false),
        };
        validate_profile(&contents)?;
        replace_source_with_backup(&directory, &source, &contents)?;
        Ok(SourceReplacement {
            inherited_fallback_removed,
        })
    }

    pub fn rollback(&self, id: &str) -> Result<(), ProfileError> {
        validate_id(id)?;
        let _lock = acquire_lock(&self.root)?;
        let directory = self.existing_profile_dir(id)?;
        let current_path = directory.join(PROFILE_FILE);
        let backup_path = directory.join(BACKUP_FILE);
        if !backup_path.is_file() {
            return Err(ProfileError::NoBackup);
        }

        let current = read_bounded(&current_path, MAX_PROFILE_BYTES as u64)?;
        let backup = read_bounded(&backup_path, MAX_PROFILE_BYTES as u64)?;
        validate_profile(&current)?;
        validate_profile(&backup)?;
        let source_swap = read_source_swap(&directory)?;
        swap_files(&directory, &current, &backup)?;
        if let Some(source) = source_swap {
            swap_source_files(&directory, &source.current, &source.backup)?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<ProfileSummary>, ProfileError> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut profiles = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|_| ProfileError::Storage)? {
            let Ok(entry) = entry else {
                continue;
            };
            let Ok(id) = entry.file_name().into_string() else {
                continue;
            };
            let path = entry.path();
            if validate_id(&id).is_err()
                || !path.is_dir()
                || !path.join(PROFILE_FILE).is_file()
                || !path.join(SOURCE_FILE).is_file()
            {
                continue;
            }
            profiles.push(ProfileSummary {
                id,
                has_backup: path.join(BACKUP_FILE).is_file(),
                source: read_source(&path)?.summary(),
            });
        }
        profiles.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(profiles)
    }

    pub fn profile_path(&self, id: &str) -> Result<PathBuf, ProfileError> {
        let directory = self.existing_profile_dir(id)?;
        let path = directory.join(PROFILE_FILE);
        let contents = read_bounded(&path, MAX_PROFILE_BYTES as u64)?;
        validate_profile(&contents)?;
        Ok(path)
    }

    fn profile_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn existing_profile_dir(&self, id: &str) -> Result<PathBuf, ProfileError> {
        validate_id(id)?;
        let directory = self.profile_dir(id);
        if directory.is_dir() {
            Ok(directory)
        } else {
            Err(ProfileError::NotFound)
        }
    }
}

fn subscription_client(proxy: Option<reqwest::Proxy>) -> Result<Client, ProfileError> {
    crate::tls::install_default_provider();
    let mut builder = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.stop()
            } else if attempt.url().scheme() == "https" {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .user_agent(SUBSCRIPTION_USER_AGENT);
    if let Some(proxy) = proxy {
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|_| ProfileError::DownloadInitialization)
}

fn validate_id(id: &str) -> Result<(), ProfileError> {
    let mut characters = id.chars();
    let Some(first) = characters.next() else {
        return Err(ProfileError::InvalidId);
    };
    if !first.is_ascii_alphanumeric()
        || id.len() > 40
        || !characters
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(ProfileError::InvalidId);
    }
    Ok(())
}

fn ensure_private_directory(path: &Path) -> Result<(), ProfileError> {
    if path.exists() {
        let metadata =
            fs::symlink_metadata(path).map_err(|_| ProfileError::StorageInitialization)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ProfileError::StorageInitialization);
        }
    } else {
        fs::create_dir_all(path).map_err(|_| ProfileError::StorageInitialization)?;
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| ProfileError::StorageInitialization)
}

fn acquire_lock(root: &Path) -> Result<File, ProfileError> {
    ensure_private_directory(root)?;
    let path = root.join(LOCK_FILE);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|_| ProfileError::StorageInitialization)?;
    fs::set_permissions(root.join(LOCK_FILE), fs::Permissions::from_mode(0o600))
        .map_err(|_| ProfileError::StorageInitialization)?;
    match FileExt::try_lock(&file) {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(ProfileError::Busy),
        Err(TryLockError::Error(_)) => Err(ProfileError::Storage),
    }
}

fn write_new_private(path: &Path, contents: &[u8]) -> Result<(), ProfileError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| ProfileError::Storage)?;
    file.write_all(contents)
        .map_err(|_| ProfileError::Storage)?;
    file.sync_all().map_err(|_| ProfileError::Storage)
}

fn write_temporary(
    directory: &Path,
    label: &str,
    contents: &[u8],
) -> Result<PathBuf, ProfileError> {
    for _ in 0..16 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(".{label}.tmp-{}-{sequence}", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut file) => {
                if file.write_all(contents).is_err() || file.sync_all().is_err() {
                    let _ = fs::remove_file(&path);
                    return Err(ProfileError::Storage);
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(ProfileError::Storage),
        }
    }
    Err(ProfileError::Storage)
}

fn replace_with_backup(directory: &Path, contents: &[u8]) -> Result<(), ProfileError> {
    let current_path = directory.join(PROFILE_FILE);
    let backup_path = directory.join(BACKUP_FILE);
    let current = read_bounded(&current_path, MAX_PROFILE_BYTES as u64)?;
    let current_source = read_bounded(&directory.join(SOURCE_FILE), MAX_DESCRIPTOR_BYTES)?;
    let next_temp = write_temporary(directory, "profile", contents)?;
    let backup_temp = write_temporary(directory, "backup", &current)?;
    let source_backup_temp = write_temporary(directory, "source-backup", &current_source)?;

    if fs::rename(&source_backup_temp, directory.join(BACKUP_SOURCE_FILE)).is_err()
        || fs::rename(&backup_temp, &backup_path).is_err()
    {
        let _ = fs::remove_file(&next_temp);
        let _ = fs::remove_file(&backup_temp);
        let _ = fs::remove_file(&source_backup_temp);
        return Err(ProfileError::Storage);
    }
    if fs::rename(&next_temp, &current_path).is_err() {
        let _ = fs::remove_file(&next_temp);
        return Err(ProfileError::Storage);
    }
    sync_directory(directory)
}

fn replace_source_with_backup(
    directory: &Path,
    source: &ProfileSource,
    contents: &[u8],
) -> Result<(), ProfileError> {
    let source_path = directory.join(SOURCE_FILE);
    let current_path = directory.join(PROFILE_FILE);
    let backup_path = directory.join(BACKUP_FILE);
    let current = read_bounded(&current_path, MAX_PROFILE_BYTES as u64)?;
    let current_source = read_bounded(&source_path, MAX_DESCRIPTOR_BYTES)?;
    let source_temp = write_temporary(directory, "source", source.descriptor()?.as_bytes())?;
    let next_temp = write_temporary(directory, "profile", contents)?;
    let backup_temp = write_temporary(directory, "backup", &current)?;
    let source_backup_temp = write_temporary(directory, "source-backup", &current_source)?;

    if fs::rename(&source_backup_temp, directory.join(BACKUP_SOURCE_FILE)).is_err()
        || fs::rename(&backup_temp, &backup_path).is_err()
    {
        let _ = fs::remove_file(&source_temp);
        let _ = fs::remove_file(&next_temp);
        let _ = fs::remove_file(&backup_temp);
        let _ = fs::remove_file(&source_backup_temp);
        return Err(ProfileError::Storage);
    }
    if fs::rename(&source_temp, &source_path).is_err() {
        let _ = fs::remove_file(&source_temp);
        let _ = fs::remove_file(&next_temp);
        return Err(ProfileError::Storage);
    }
    if fs::rename(&next_temp, &current_path).is_err() {
        let _ = fs::remove_file(&next_temp);
        return Err(ProfileError::Storage);
    }
    sync_directory(directory)
}

fn swap_files(directory: &Path, current: &[u8], backup: &[u8]) -> Result<(), ProfileError> {
    let current_path = directory.join(PROFILE_FILE);
    let backup_path = directory.join(BACKUP_FILE);
    let current_temp = write_temporary(directory, "rollback-current", backup)?;
    let backup_temp = write_temporary(directory, "rollback-backup", current)?;

    if fs::rename(&backup_temp, &backup_path).is_err() {
        let _ = fs::remove_file(&current_temp);
        let _ = fs::remove_file(&backup_temp);
        return Err(ProfileError::Storage);
    }
    if fs::rename(&current_temp, &current_path).is_err() {
        let _ = fs::rename(&current_temp, &backup_path);
        return Err(ProfileError::Storage);
    }
    sync_directory(directory)
}

fn swap_source_files(directory: &Path, current: &[u8], backup: &[u8]) -> Result<(), ProfileError> {
    let current_path = directory.join(SOURCE_FILE);
    let backup_path = directory.join(BACKUP_SOURCE_FILE);
    let current_temp = write_temporary(directory, "rollback-source-current", backup)?;
    let backup_temp = write_temporary(directory, "rollback-source-backup", current)?;

    if fs::rename(&backup_temp, &backup_path).is_err() {
        let _ = fs::remove_file(&current_temp);
        let _ = fs::remove_file(&backup_temp);
        return Err(ProfileError::Storage);
    }
    if fs::rename(&current_temp, &current_path).is_err() {
        let _ = fs::rename(&current_temp, &backup_path);
        return Err(ProfileError::Storage);
    }
    sync_directory(directory)
}

fn read_source_swap(directory: &Path) -> Result<Option<SourceSwap>, ProfileError> {
    let backup_path = directory.join(BACKUP_SOURCE_FILE);
    if !backup_path.exists() {
        return Ok(None);
    }
    let current = read_bounded(&directory.join(SOURCE_FILE), MAX_DESCRIPTOR_BYTES)?;
    let backup = read_bounded(&backup_path, MAX_DESCRIPTOR_BYTES)?;
    let current_text =
        std::str::from_utf8(&current).map_err(|_| ProfileError::InvalidSourceDescriptor)?;
    let backup_text =
        std::str::from_utf8(&backup).map_err(|_| ProfileError::InvalidSourceDescriptor)?;
    ProfileSource::from_descriptor(current_text)?;
    ProfileSource::from_descriptor(backup_text)?;
    Ok(Some(SourceSwap { current, backup }))
}

fn read_source(directory: &Path) -> Result<ProfileSource, ProfileError> {
    let path = directory.join(SOURCE_FILE);
    let metadata = fs::metadata(&path).map_err(|_| ProfileError::InvalidSourceDescriptor)?;
    if !metadata.is_file()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.len() > MAX_DESCRIPTOR_BYTES
    {
        return Err(ProfileError::InvalidSourceDescriptor);
    }
    let descriptor = fs::read_to_string(path).map_err(|_| ProfileError::InvalidSourceDescriptor)?;
    ProfileSource::from_descriptor(&descriptor)
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, ProfileError> {
    let metadata = fs::metadata(path).map_err(|_| ProfileError::Storage)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 || metadata.len() > limit {
        return Err(ProfileError::Storage);
    }
    fs::read(path).map_err(|_| ProfileError::Storage)
}

fn sync_directory(path: &Path) -> Result<(), ProfileError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| ProfileError::Storage)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{ProfileStore, SUBSCRIPTION_USER_AGENT};
    use crate::profile::{ProfileError, ProfileSource};

    #[test]
    fn subscription_user_agent_requests_mihomo_yaml() {
        assert_eq!(SUBSCRIPTION_USER_AGENT, "clash.meta");
    }

    #[tokio::test]
    async fn add_update_and_rollback_are_atomic_and_private() {
        let base = temporary_directory();
        let source_path = base.join("source.yaml");
        fs::create_dir(&base).expect("base should be created");
        fs::write(&source_path, fixture("Proxy A")).expect("source should be written");
        let store =
            ProfileStore::new(base.join("state/profiles")).expect("store should initialize");
        let source =
            ProfileSource::from_local_file(&source_path).expect("source should initialize");

        store
            .add("primary", source)
            .await
            .expect("profile should be added");
        let profile_path = store.profile_path("primary").expect("profile should exist");
        assert_eq!(
            fs::metadata(&profile_path)
                .expect("metadata should exist")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        fs::write(&source_path, fixture("Proxy B")).expect("source should update");
        store
            .update("primary")
            .await
            .expect("profile should update");
        assert!(
            fs::read_to_string(&profile_path)
                .expect("profile should read")
                .contains("Proxy B")
        );

        store.rollback("primary").expect("profile should roll back");
        assert!(
            fs::read_to_string(&profile_path)
                .expect("profile should read")
                .contains("Proxy A")
        );
        assert!(store.list().expect("list should work")[0].has_backup);

        fs::remove_dir_all(base).expect("test directory should be removed");
    }

    #[tokio::test]
    async fn invalid_content_does_not_create_a_profile() {
        let base = temporary_directory();
        let source_path = base.join("invalid.yaml");
        fs::create_dir(&base).expect("base should be created");
        fs::write(&source_path, "<html>invalid</html>").expect("source should be written");
        let store =
            ProfileStore::new(base.join("state/profiles")).expect("store should initialize");
        let source =
            ProfileSource::from_local_file(&source_path).expect("source should initialize");

        let result = store.add("invalid", source).await;

        assert_eq!(result, Err(ProfileError::InvalidYamlRoot));
        assert!(!base.join("state/profiles/invalid").exists());
        fs::remove_dir_all(base).expect("test directory should be removed");
    }

    #[tokio::test]
    async fn replacing_a_source_validates_before_changing_stored_files() {
        let base = temporary_directory();
        fs::create_dir(&base).expect("base should be created");
        let original_path = base.join("original.yaml");
        let invalid_path = base.join("invalid.yaml");
        let replacement_path = base.join("replacement.yaml");
        fs::write(&original_path, fixture("Proxy A")).expect("original should be written");
        fs::write(&invalid_path, "<html>invalid</html>").expect("invalid source should be written");
        fs::write(&replacement_path, fixture("Proxy B")).expect("replacement should be written");
        let store =
            ProfileStore::new(base.join("state/profiles")).expect("store should initialize");
        store
            .add(
                "primary",
                ProfileSource::from_local_file(&original_path)
                    .expect("original source should initialize"),
            )
            .await
            .expect("profile should be added");
        let profile_path = store.profile_path("primary").expect("profile should exist");

        let invalid = ProfileSource::from_local_file(&invalid_path)
            .expect("invalid source path should initialize");
        assert_eq!(
            store.replace_source("primary", invalid).await,
            Err(ProfileError::InvalidYamlRoot)
        );
        assert!(
            fs::read_to_string(&profile_path)
                .expect("profile should remain readable")
                .contains("Proxy A")
        );

        let replacement = ProfileSource::from_local_file(&replacement_path)
            .expect("replacement source should initialize");
        store
            .replace_source("primary", replacement)
            .await
            .expect("replacement should succeed");
        assert!(
            fs::read_to_string(&profile_path)
                .expect("profile should be readable")
                .contains("Proxy B")
        );
        assert_eq!(
            store.list().expect("list should work")[0].source.display,
            replacement_path.to_string_lossy()
        );

        store
            .rollback("primary")
            .expect("profile and source should roll back together");
        assert!(
            fs::read_to_string(&profile_path)
                .expect("rolled-back profile should be readable")
                .contains("Proxy A")
        );
        assert_eq!(
            store.list().expect("list should work")[0].source.display,
            original_path.to_string_lossy()
        );

        fs::remove_dir_all(base).expect("test directory should be removed");
    }

    #[test]
    fn rejects_path_like_profile_ids() {
        let store = ProfileStore::new(temporary_directory()).expect("store should initialize");

        assert_eq!(
            store.profile_path("../escape"),
            Err(ProfileError::InvalidId)
        );
    }

    #[tokio::test]
    async fn refresh_and_source_replacement_retain_policy_and_invalid_updates_retain_files() {
        let base = temporary_directory();
        fs::create_dir(&base).unwrap();
        let first = base.join("first.yaml");
        let second = base.join("second.yaml");
        let document = |name: &str| {
            format!(
                "proxies:\n- {{name: {name}, type: http, server: 127.0.0.1, port: 8080}}\nproxy-groups:\n- {{name: AI, type: select, proxies: [{name}]}}\n"
            )
        };
        fs::write(&first, document("A")).unwrap();
        let store = ProfileStore::new(base.join("profiles")).unwrap();
        let source = ProfileSource::from_local_file(&first)
            .unwrap()
            .with_fallback("AI".into(), Some("A".into()))
            .unwrap();
        store.add("primary", source).await.unwrap();
        fs::write(&first, document("B")).unwrap();
        store.update("primary").await.unwrap();
        let path = store.profile_path("primary").unwrap();
        let value: yaml_serde::Value = yaml_serde::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["proxy-groups"][1]["proxies"][0].as_str(), Some("B"));
        fs::write(&second, document("C")).unwrap();
        store
            .replace_source("primary", ProfileSource::from_local_file(&second).unwrap())
            .await
            .unwrap();
        let profile = fs::read(&path).unwrap();
        let value: yaml_serde::Value = yaml_serde::from_slice(&profile).unwrap();
        assert_eq!(value["proxy-groups"][1]["proxies"][0].as_str(), Some("C"));
        let descriptor = fs::read(path.parent().unwrap().join("source.toml")).unwrap();
        fs::write(
            &second,
            document("D").replace("name: AI,", "name: Missing,"),
        )
        .unwrap();
        assert_eq!(
            store.update("primary").await,
            Err(ProfileError::InvalidFallbackPolicy)
        );
        assert_eq!(fs::read(&path).unwrap(), profile);
        assert_eq!(
            fs::read(path.parent().unwrap().join("source.toml")).unwrap(),
            descriptor
        );
        store.rollback("primary").unwrap();
        let value: yaml_serde::Value = yaml_serde::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["proxy-groups"][1]["proxies"][0].as_str(), Some("B"));
        assert_eq!(
            store.list().unwrap()[0].source.display,
            first.to_string_lossy()
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn incompatible_inherited_policy_is_removed_on_replace_and_rollback_restores_it() {
        let base = temporary_directory();
        fs::create_dir(&base).unwrap();
        let first = base.join("first.yaml");
        let second = base.join("second.yaml");
        let document = "proxies:\n- {name: A, type: http, server: 127.0.0.1, port: 8080}\nproxy-groups:\n- {name: Custom, type: select, proxies: [A]}\n";
        fs::write(&first, document).unwrap();
        let store = ProfileStore::new(base.join("profiles")).unwrap();
        store
            .add(
                "primary",
                ProfileSource::from_local_file(&first)
                    .unwrap()
                    .with_fallback("Custom".into(), Some("A".into()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let path = store.profile_path("primary").unwrap();
        let descriptor = path.with_file_name("source.toml");
        let original = fs::read(&path).unwrap();
        let original_source = fs::read(&descriptor).unwrap();
        let variants = [
            document.replace("name: Custom,", "name: BiuBiu,"),
            document.replace("type: select", "type: fallback"),
            document.replace("proxies: [A]", "proxies: [DIRECT]"),
            format!("{document}- {{name: Custom Auto, type: select, proxies: [A]}}\n"),
            "proxy-providers: {}\nproxy-groups: []\n".into(),
        ];
        for contents in variants {
            fs::write(&second, &contents).unwrap();
            let replacement = store
                .replace_source("primary", ProfileSource::from_local_file(&second).unwrap())
                .await
                .unwrap();
            assert!(replacement.inherited_fallback_removed);
            assert!(replacement.notice().is_some());
            assert_eq!(fs::read(&path).unwrap(), contents.as_bytes());
            assert!(
                !fs::read_to_string(&descriptor)
                    .unwrap()
                    .contains("[fallback]")
            );
            store.update("primary").await.unwrap();
            assert_eq!(fs::read(&path).unwrap(), contents.as_bytes());
            // Refresh creates its own rollback revision; replace again after restoring
            // the original pair to check that a source replacement keeps that pair.
            fs::write(&path, &original).unwrap();
            fs::write(&descriptor, &original_source).unwrap();
            store
                .replace_source("primary", ProfileSource::from_local_file(&second).unwrap())
                .await
                .unwrap();
            store.rollback("primary").unwrap();
            assert_eq!(fs::read(&path).unwrap(), original);
            assert_eq!(fs::read(&descriptor).unwrap(), original_source);
        }
        fs::write(&second, document.replace("name: Custom,", "name: Other,")).unwrap();
        let explicit = ProfileSource::from_local_file(&second)
            .unwrap()
            .with_fallback("Missing".into(), None)
            .unwrap();
        assert_eq!(
            store.replace_source("primary", explicit).await,
            Err(ProfileError::InvalidFallbackPolicy)
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read(&descriptor).unwrap(), original_source);
        fs::write(&second, "<html>subscription unavailable</html>").unwrap();
        assert_eq!(
            store
                .replace_source("primary", ProfileSource::from_local_file(&second).unwrap())
                .await,
            Err(ProfileError::InvalidYamlRoot)
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read(&descriptor).unwrap(), original_source);
        fs::remove_dir_all(base).unwrap();
    }

    fn fixture(name: &str) -> String {
        format!(
            "proxies:\n  - name: {name}\n    type: ss\n    server: 192.0.2.1\n    port: 443\n    cipher: aes-128-gcm\n    password: fixture-only\n"
        )
    }

    fn temporary_directory() -> PathBuf {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mihoterm-profile-test-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
