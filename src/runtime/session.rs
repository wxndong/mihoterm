use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use fs4::FileExt;
use rustix::process::{Pid, Signal, kill_process};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::mihomo::ApiClient;

use super::{
    RuntimeError,
    config::build_managed_config,
    process::{
        ManagedRuntime, prepare_runtime_root, random_hex, read_private_profile,
        remove_runtime_directory, sync_directory,
    },
};

const SESSION_SCHEMA_VERSION: u8 = 1;
const SESSION_FILE: &str = "session.json";
const SESSION_LOCK: &str = ".session.lock";
const MAX_SESSION_BYTES: u64 = 64 * 1024;
const STOP_GRACE_PERIOD: Duration = Duration::from_secs(2);
const STOP_KILL_PERIOD: Duration = Duration::from_secs(1);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const NO_PROXY: &str = "localhost,127.0.0.1,::1,.local";

pub struct SessionManager {
    root: PathBuf,
}

pub struct ManagedSession {
    record: StoredSession,
}

pub struct ProxyEnvironment {
    session_id: String,
    http_proxy: SecretString,
    all_proxy: SecretString,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSession {
    schema_version: u8,
    session_id: String,
    pid: u32,
    process_start_ticks: u64,
    profile: String,
    runtime_dir: PathBuf,
    controller_url: String,
    controller_secret: String,
    mixed_port: u16,
    proxy_username: String,
    proxy_password: String,
}

impl SessionManager {
    pub fn new(runtime_root: &Path) -> Result<Self, RuntimeError> {
        Ok(Self {
            root: prepare_runtime_root(runtime_root)?,
        })
    }

    pub fn active(&self) -> Result<Option<ManagedSession>, RuntimeError> {
        let _lock = acquire_lock(&self.root)?;
        self.active_locked()
    }

    pub async fn start(
        &self,
        profile: &str,
        profile_path: &Path,
        mihomo: &Path,
    ) -> Result<ManagedSession, RuntimeError> {
        let _lock = acquire_lock(&self.root)?;
        if let Some(session) = self.active_locked()? {
            return if session.profile() == profile {
                Ok(session)
            } else {
                Err(RuntimeError::SessionProfileConflict)
            };
        }

        let mut runtime =
            ManagedRuntime::start_persistent(mihomo, profile_path, &self.root).await?;
        let readiness_client = runtime.api_client(Duration::from_millis(500))?;
        runtime
            .wait_ready(&readiness_client, Duration::from_secs(15))
            .await?;

        let pid = runtime.child_id().ok_or(RuntimeError::ProcessStatus)?;
        let process_start_ticks = process_start_ticks(pid).ok_or(RuntimeError::ProcessStatus)?;
        let record = StoredSession {
            schema_version: SESSION_SCHEMA_VERSION,
            session_id: random_hex::<16>()?,
            pid,
            process_start_ticks,
            profile: profile.to_owned(),
            runtime_dir: runtime.runtime_directory().to_owned(),
            controller_url: runtime.controller_url().to_owned(),
            controller_secret: runtime.controller_secret().expose_secret().to_owned(),
            mixed_port: runtime.mixed_port(),
            proxy_username: runtime.proxy_username().expose_secret().to_owned(),
            proxy_password: runtime.proxy_password().expose_secret().to_owned(),
        };
        validate_record(&self.root, &record)?;
        write_record(&self.root, &record)?;
        runtime.detach();
        Ok(ManagedSession { record })
    }

    pub async fn stop(&self) -> Result<bool, RuntimeError> {
        let _lock = acquire_lock(&self.root)?;
        let Some(record) = read_record(&self.root)? else {
            return Ok(false);
        };

        if process_matches(&record) {
            stop_process(&record).await?;
        }
        remove_session_files(&self.root, &record)?;
        Ok(true)
    }

    pub async fn switch_profile(
        &self,
        profile: &str,
        profile_path: &Path,
    ) -> Result<ManagedSession, RuntimeError> {
        let _lock = acquire_lock(&self.root)?;
        let mut session = self
            .active_locked()?
            .ok_or(RuntimeError::SessionNotRunning)?;
        if session.profile() == profile {
            return Ok(session);
        }

        let client = session.api_client(Duration::from_secs(15))?;
        let record = &mut session.record;
        let controller_port = Url::parse(&record.controller_url)
            .ok()
            .and_then(|url| url.port())
            .ok_or(RuntimeError::InvalidSession)?;
        let source = read_private_profile(profile_path)?;
        let next = build_managed_config(
            &source,
            controller_port,
            record.mixed_port,
            &record.controller_secret,
            &record.proxy_username,
            &record.proxy_password,
        )?;
        let next = String::from_utf8(next).map_err(|_| RuntimeError::ConfigurationSerialization)?;
        let runtime_config = record.runtime_dir.join("runtime.yaml");
        let previous = read_private_profile(&runtime_config)?;
        let previous_text =
            std::str::from_utf8(&previous).map_err(|_| RuntimeError::InvalidSession)?;
        client
            .reload_configuration(&next)
            .await
            .map_err(|_| RuntimeError::SessionReload)?;
        if replace_private(&runtime_config, next.as_bytes()).is_err() {
            let file_rolled_back = replace_private(&runtime_config, &previous).is_ok();
            let core_rolled_back = client.reload_configuration(previous_text).await.is_ok();
            return if file_rolled_back && core_rolled_back {
                Err(RuntimeError::ConfigurationWrite)
            } else {
                Err(RuntimeError::SessionSwitchRollback)
            };
        }

        let previous_profile = std::mem::replace(&mut record.profile, profile.to_owned());
        if write_record(&self.root, record).is_err() {
            record.profile = previous_profile;
            let file_rolled_back = replace_private(&runtime_config, &previous).is_ok();
            let core_rolled_back = client.reload_configuration(previous_text).await.is_ok();
            let descriptor_rolled_back = write_record(&self.root, record).is_ok();
            return if file_rolled_back && core_rolled_back && descriptor_rolled_back {
                Err(RuntimeError::SessionWrite)
            } else {
                Err(RuntimeError::SessionSwitchRollback)
            };
        }

        Ok(session)
    }

    fn active_locked(&self) -> Result<Option<ManagedSession>, RuntimeError> {
        let Some(record) = read_record(&self.root)? else {
            return Ok(None);
        };
        if process_matches(&record) {
            Ok(Some(ManagedSession { record }))
        } else {
            remove_session_files(&self.root, &record)?;
            Ok(None)
        }
    }
}

impl ManagedSession {
    pub fn api_client(&self, timeout: Duration) -> Result<ApiClient, RuntimeError> {
        ApiClient::with_timeout(
            &self.record.controller_url,
            Some(self.record.controller_secret.clone()),
            timeout,
        )
        .map_err(|_| RuntimeError::ApiInitialization)
    }

    #[must_use]
    pub fn profile(&self) -> &str {
        &self.record.profile
    }

    #[must_use]
    pub const fn pid(&self) -> u32 {
        self.record.pid
    }

    #[must_use]
    pub const fn mixed_port(&self) -> u16 {
        self.record.mixed_port
    }

    #[must_use]
    pub fn proxy_environment(&self) -> ProxyEnvironment {
        let authority = format!(
            "{}:{}@127.0.0.1:{}",
            self.record.proxy_username, self.record.proxy_password, self.record.mixed_port
        );
        ProxyEnvironment {
            session_id: self.record.session_id.clone(),
            http_proxy: SecretString::from(format!("http://{authority}")),
            all_proxy: SecretString::from(format!("socks5h://{authority}")),
        }
    }
}

impl ProxyEnvironment {
    pub fn apply(&self, command: &mut Command) {
        let http_proxy = self.http_proxy.expose_secret();
        let all_proxy = self.all_proxy.expose_secret();
        command
            .env("HTTP_PROXY", http_proxy)
            .env("HTTPS_PROXY", http_proxy)
            .env("http_proxy", http_proxy)
            .env("https_proxy", http_proxy)
            .env("ALL_PROXY", all_proxy)
            .env("all_proxy", all_proxy)
            .env("NO_PROXY", NO_PROXY)
            .env("no_proxy", NO_PROXY)
            .env("MIHOTERM_PROXY_SESSION", &self.session_id);
    }

    #[must_use]
    pub fn shell_exports(&self) -> String {
        let http_proxy = shell_quote(self.http_proxy.expose_secret());
        let all_proxy = shell_quote(self.all_proxy.expose_secret());
        let no_proxy = shell_quote(NO_PROXY);
        let session_id = shell_quote(&self.session_id);
        format!(
            "export HTTP_PROXY={http_proxy}\n\
             export HTTPS_PROXY={http_proxy}\n\
             export http_proxy={http_proxy}\n\
             export https_proxy={http_proxy}\n\
             export ALL_PROXY={all_proxy}\n\
             export all_proxy={all_proxy}\n\
             export NO_PROXY={no_proxy}\n\
             export no_proxy={no_proxy}\n\
             export MIHOTERM_PROXY_SESSION={session_id}\n"
        )
    }
}

#[must_use]
pub fn shell_clear_owned() -> &'static str {
    "if [ -n \"${MIHOTERM_PROXY_SESSION-}\" ]; then\n\
     unset HTTP_PROXY HTTPS_PROXY http_proxy https_proxy ALL_PROXY all_proxy\n\
     unset NO_PROXY no_proxy MIHOTERM_PROXY_SESSION\n\
     fi\n"
}

fn acquire_lock(root: &Path) -> Result<File, RuntimeError> {
    let path = root.join(SESSION_LOCK);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|_| RuntimeError::SessionLock)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| RuntimeError::SessionLock)?;
    FileExt::lock(&file).map_err(|_| RuntimeError::SessionLock)?;
    Ok(file)
}

fn read_record(root: &Path) -> Result<Option<StoredSession>, RuntimeError> {
    let path = root.join(SESSION_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(RuntimeError::SessionRead),
    };
    if !metadata.file_type().is_file()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.len() > MAX_SESSION_BYTES
    {
        return Err(RuntimeError::InvalidSession);
    }
    let contents = fs::read(&path).map_err(|_| RuntimeError::SessionRead)?;
    let record = serde_json::from_slice::<StoredSession>(&contents)
        .map_err(|_| RuntimeError::InvalidSession)?;
    validate_record(root, &record)?;
    Ok(Some(record))
}

fn write_record(root: &Path, record: &StoredSession) -> Result<(), RuntimeError> {
    let contents = serde_json::to_vec(record).map_err(|_| RuntimeError::SessionWrite)?;
    let temporary = root.join(format!(".session-{}.tmp", record.session_id));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| RuntimeError::SessionWrite)?;
        file.write_all(&contents)
            .and_then(|()| file.sync_all())
            .map_err(|_| RuntimeError::SessionWrite)?;
        fs::rename(&temporary, root.join(SESSION_FILE)).map_err(|_| RuntimeError::SessionWrite)?;
        sync_directory(root).map_err(|_| RuntimeError::SessionWrite)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn replace_private(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    let parent = path.parent().ok_or(RuntimeError::ConfigurationWrite)?;
    let temporary = parent.join(format!(".runtime-{}.tmp", random_hex::<8>()?));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| RuntimeError::ConfigurationWrite)?;
        file.write_all(contents)
            .and_then(|()| file.sync_all())
            .map_err(|_| RuntimeError::ConfigurationWrite)?;
        fs::rename(&temporary, path).map_err(|_| RuntimeError::ConfigurationWrite)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn validate_record(root: &Path, record: &StoredSession) -> Result<(), RuntimeError> {
    let controller =
        Url::parse(&record.controller_url).map_err(|_| RuntimeError::InvalidSession)?;
    let valid_controller = controller.scheme() == "http"
        && controller.host_str() == Some("127.0.0.1")
        && controller.port().is_some()
        && controller.path() == "/"
        && controller.query().is_none()
        && controller.fragment().is_none();
    let valid_runtime = record.runtime_dir.parent() == Some(root)
        && record
            .runtime_dir
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("run-"));
    let valid_profile = record
        .profile
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && record.profile.len() <= 40
        && record
            .profile
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'));
    if record.schema_version != SESSION_SCHEMA_VERSION
        || record.pid <= 1
        || record.mixed_port == 0
        || !valid_hex(&record.session_id, 32)
        || !valid_hex(&record.controller_secret, 64)
        || !record.proxy_username.starts_with("mihoterm-")
        || !valid_hex(&record.proxy_username["mihoterm-".len()..], 16)
        || !valid_hex(&record.proxy_password, 64)
        || !valid_controller
        || !valid_runtime
        || !valid_profile
    {
        return Err(RuntimeError::InvalidSession);
    }
    Ok(())
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn process_start_ticks(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let command_end = stat.rfind(')')?;
    stat.get(command_end + 1..)?
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

fn process_matches(record: &StoredSession) -> bool {
    if process_start_ticks(record.pid) != Some(record.process_start_ticks) {
        return false;
    }
    let expected = record.runtime_dir.join("runtime.yaml");
    fs::read(format!("/proc/{}/cmdline", record.pid))
        .ok()
        .is_some_and(|command_line| {
            command_line
                .split(|byte| *byte == 0)
                .any(|argument| argument == expected.as_os_str().as_bytes())
        })
}

async fn stop_process(record: &StoredSession) -> Result<(), RuntimeError> {
    let pid = i32::try_from(record.pid)
        .ok()
        .and_then(Pid::from_raw)
        .ok_or(RuntimeError::SessionStop)?;
    if kill_process(pid, Signal::TERM).is_err() {
        return if process_matches(record) {
            Err(RuntimeError::SessionStop)
        } else {
            Ok(())
        };
    }
    if wait_for_exit(record, STOP_GRACE_PERIOD).await {
        return Ok(());
    }
    if kill_process(pid, Signal::KILL).is_err() {
        return if process_matches(record) {
            Err(RuntimeError::SessionStop)
        } else {
            Ok(())
        };
    }
    if wait_for_exit(record, STOP_KILL_PERIOD).await {
        Ok(())
    } else {
        Err(RuntimeError::SessionStop)
    }
}

async fn wait_for_exit(record: &StoredSession, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_matches(record) {
            return true;
        }
        tokio::time::sleep(PROCESS_POLL_INTERVAL).await;
    }
    !process_matches(record)
}

fn remove_session_files(root: &Path, record: &StoredSession) -> Result<(), RuntimeError> {
    remove_runtime_directory(root, &record.runtime_dir)?;
    match fs::remove_file(root.join(SESSION_FILE)) {
        Ok(()) => sync_directory(root).map_err(|_| RuntimeError::SessionWrite),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(RuntimeError::SessionWrite),
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        SESSION_SCHEMA_VERSION, StoredSession, process_start_ticks, replace_private,
        shell_clear_owned, shell_quote, validate_record, write_record,
    };

    #[test]
    fn shell_values_are_single_quote_safe() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn clear_script_only_unsets_owned_proxy_variables() {
        let script = shell_clear_owned();

        assert!(script.contains("MIHOTERM_PROXY_SESSION"));
        assert!(script.contains("unset HTTP_PROXY"));
    }

    #[test]
    fn session_descriptor_is_private_and_does_not_accept_other_paths() {
        let root = temporary_directory();
        fs::create_dir(&root).expect("root should be created");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .expect("root permissions should be set");
        let runtime = root.join("run-test");
        fs::create_dir(&runtime).expect("runtime should be created");
        let record = StoredSession {
            schema_version: SESSION_SCHEMA_VERSION,
            session_id: "11".repeat(16),
            pid: std::process::id(),
            process_start_ticks: process_start_ticks(std::process::id())
                .expect("current process should have proc metadata"),
            profile: "default".into(),
            runtime_dir: runtime,
            controller_url: "http://127.0.0.1:41001".into(),
            controller_secret: "22".repeat(32),
            mixed_port: 41002,
            proxy_username: format!("mihoterm-{}", "33".repeat(8)),
            proxy_password: "44".repeat(32),
        };

        validate_record(&root, &record).expect("record should validate");
        write_record(&root, &record).expect("record should be written");
        assert_eq!(
            fs::metadata(root.join("session.json"))
                .expect("session metadata should exist")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let mut escaped = record;
        escaped.runtime_dir = root.join("../outside");
        assert!(validate_record(&root, &escaped).is_err());
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn runtime_configuration_replacement_is_atomic_and_private() {
        let root = temporary_directory();
        fs::create_dir(&root).expect("root should be created");
        let path = root.join("runtime.yaml");
        fs::write(&path, b"old").expect("fixture should be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("fixture permissions should be set");

        replace_private(&path, b"new").expect("configuration should be replaced");

        assert_eq!(fs::read(&path).expect("configuration should read"), b"new");
        assert_eq!(
            fs::metadata(&path)
                .expect("configuration metadata should exist")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_dir(&root)
                .expect("root should read")
                .collect::<Result<Vec<_>, _>>()
                .expect("entries should read")
                .len(),
            1
        );
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    fn temporary_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mihoterm-session-test-{}-{nonce}",
            std::process::id()
        ))
    }
}
