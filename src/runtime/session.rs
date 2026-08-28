use std::{
    collections::BTreeSet,
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

use fs4::{FileExt, TryLockError};
use futures_util::{StreamExt, stream};
use rustix::process::{Pid, Signal, kill_process};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    mihomo::{ApiClient, ApiError, OperatingMode, ProxiesResponse},
    probe::ProbeTarget,
    profile::ProfileStore,
    state::{DesiredStateSnapshot, DesiredStateStore, unix_seconds_now},
};

use super::{
    RuntimeError,
    config::build_managed_config,
    process::{
        ManagedRuntime, prepare_runtime_root, random_hex, read_private_profile,
        remove_runtime_directory, sync_directory,
    },
};

const LEGACY_SESSION_SCHEMA_VERSION: u8 = 1;
const SESSION_SCHEMA_VERSION: u8 = 2;
const SESSION_FILE: &str = "session.json";
const SESSION_LOCK: &str = ".session.lock";
const MAX_SESSION_BYTES: u64 = 64 * 1024;
const STOP_GRACE_PERIOD: Duration = Duration::from_secs(2);
const STOP_KILL_PERIOD: Duration = Duration::from_secs(1);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const RESTART_READY_TIMEOUT: Duration = Duration::from_secs(15);
const HEALTHY_UPTIME: Duration = Duration::from_secs(60);
const MAX_CONSECUTIVE_RESTARTS: usize = 6;
const RESTART_BACKOFF_SECONDS: [u64; MAX_CONSECUTIVE_RESTARTS] = [1, 2, 4, 8, 16, 30];
const HEALTH_INTERVAL: Duration = Duration::from_secs(5 * 60);
const HEALTH_RECHECK_DELAY: Duration = Duration::from_secs(15);
const RECOVERY_COOLDOWN_SECONDS: u64 = 10 * 60;
const MAX_RECOVERY_CANDIDATES: usize = 4;
const NO_PROXY: &str = "localhost,127.0.0.1,::1,.local";

#[derive(Clone)]
pub struct SessionManager {
    root: PathBuf,
    desired: DesiredStateStore,
    state_root: PathBuf,
}

pub struct ManagedSession {
    record: StoredSession,
}

pub struct ProxyEnvironment {
    session_id: String,
    http_proxy: SecretString,
    all_proxy: SecretString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    NotGlobal,
    ControllerUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthReport {
    pub status: HealthStatus,
    pub successful_probes: usize,
    pub total_probes: usize,
    pub codex_reachable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryOutcome {
    AlreadyHealthy,
    NotApplicable,
    Cooldown,
    RestartRequested,
    RecoveredByRefresh,
    RecoveredByKnownGood,
    Degraded,
}

#[derive(Clone, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supervisor_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supervisor_start_ticks: Option<u64>,
}

impl SessionManager {
    pub fn with_state(runtime_root: &Path, state_root: &Path) -> Result<Self, RuntimeError> {
        Ok(Self {
            root: prepare_runtime_root(runtime_root)?,
            desired: DesiredStateStore::new(state_root.to_owned())
                .map_err(|_| RuntimeError::PersistentState)?,
            state_root: state_root.to_owned(),
        })
    }

    pub fn active(&self) -> Result<Option<ManagedSession>, RuntimeError> {
        let _lock = try_acquire_lock(&self.root)?;
        self.active_locked()
    }

    pub async fn supervise(
        &self,
        profile: &str,
        profile_path: &Path,
        mihomo: &Path,
    ) -> Result<(), RuntimeError> {
        let desired = self
            .desired
            .load()
            .map_err(|_| RuntimeError::PersistentState)?;
        let mode = desired.mode;
        let profile_contents = read_private_profile(profile_path)?;

        let lock = try_acquire_lock(&self.root)?;
        if self.active_locked()?.is_some() {
            return Err(RuntimeError::SessionAlreadyRunning);
        }

        let mut runtime =
            ManagedRuntime::start_with_mode(mihomo, profile_path, &self.root, mode).await?;
        let readiness_client = runtime.api_client(Duration::from_millis(500))?;
        runtime
            .wait_ready(&readiness_client, RESTART_READY_TIMEOUT)
            .await?;

        if desired.active_profile.as_deref() == Some(profile) {
            let selection_client = runtime.api_client(Duration::from_secs(5))?;
            restore_selections_reliably(&selection_client, &desired).await?;
        }
        self.desired
            .record_runtime(profile, &profile_contents, mode)
            .map_err(|_| RuntimeError::PersistentState)?;

        let pid = runtime.child_id().ok_or(RuntimeError::ProcessStatus)?;
        let supervisor_pid = std::process::id();
        let mut record = StoredSession {
            schema_version: SESSION_SCHEMA_VERSION,
            session_id: random_hex::<16>()?,
            pid,
            process_start_ticks: process_start_ticks(pid).ok_or(RuntimeError::ProcessStatus)?,
            profile: profile.to_owned(),
            runtime_dir: runtime.runtime_directory().to_owned(),
            controller_url: runtime.controller_url().to_owned(),
            controller_secret: runtime.controller_secret().expose_secret().to_owned(),
            mixed_port: runtime.mixed_port(),
            proxy_username: runtime.proxy_username().expose_secret().to_owned(),
            proxy_password: runtime.proxy_password().expose_secret().to_owned(),
            supervisor_pid: Some(supervisor_pid),
            supervisor_start_ticks: Some(
                process_start_ticks(supervisor_pid).ok_or(RuntimeError::ProcessStatus)?,
            ),
        };
        validate_record(&self.root, &record)?;
        write_record(&self.root, &record)?;
        drop(lock);

        let supervision_result = self.supervision_loop(&mut runtime, &mut record).await;
        let stop_result = runtime.stop();
        let cleanup_result = self.remove_owned_session(&record);
        match supervision_result {
            Err(error) => Err(error),
            Ok(()) => stop_result.and(cleanup_result),
        }
    }

    async fn supervision_loop(
        &self,
        runtime: &mut ManagedRuntime,
        record: &mut StoredSession,
    ) -> Result<(), RuntimeError> {
        use tokio::signal::unix::{SignalKind, signal};

        let mut interrupt =
            signal(SignalKind::interrupt()).map_err(|_| RuntimeError::SupervisorSignal)?;
        let mut terminate =
            signal(SignalKind::terminate()).map_err(|_| RuntimeError::SupervisorSignal)?;
        let mut started_at = Instant::now();
        let mut consecutive_restarts = 0;
        let health_monitor = tokio::spawn(self.clone().health_loop());

        let result = async {
            loop {
                let status = tokio::select! {
                    _ = interrupt.recv() => return Ok(()),
                    _ = terminate.recv() => return Ok(()),
                    status = runtime.wait() => status?,
                };
                if started_at.elapsed() >= HEALTHY_UPTIME {
                    consecutive_restarts = 0;
                }
                let exit_code = status.code();

                loop {
                    if consecutive_restarts >= MAX_CONSECUTIVE_RESTARTS {
                        return Err(RuntimeError::SupervisorRestartLimit { code: exit_code });
                    }
                    let delay = Duration::from_secs(RESTART_BACKOFF_SECONDS[consecutive_restarts]);
                    consecutive_restarts += 1;
                    tokio::select! {
                        _ = interrupt.recv() => return Ok(()),
                        _ = terminate.recv() => return Ok(()),
                        () = tokio::time::sleep(delay) => {}
                    }

                    if runtime.restart(RESTART_READY_TIMEOUT).await.is_ok() {
                        self.restore_runtime_selections(runtime).await?;
                        self.update_child_record(record, runtime)?;
                        started_at = Instant::now();
                        break;
                    }
                }
            }
        }
        .await;
        health_monitor.abort();
        let _ = health_monitor.await;
        result
    }

    async fn restore_runtime_selections(
        &self,
        runtime: &ManagedRuntime,
    ) -> Result<(), RuntimeError> {
        let snapshot = self
            .desired
            .load()
            .map_err(|_| RuntimeError::PersistentState)?;
        let client = runtime.api_client(Duration::from_secs(5))?;
        restore_selections_reliably(&client, &snapshot).await
    }

    fn update_child_record(
        &self,
        record: &mut StoredSession,
        runtime: &ManagedRuntime,
    ) -> Result<(), RuntimeError> {
        let pid = runtime.child_id().ok_or(RuntimeError::ProcessStatus)?;
        let start_ticks = process_start_ticks(pid).ok_or(RuntimeError::ProcessStatus)?;
        let _lock = acquire_lock(&self.root)?;
        let current = read_record(&self.root)?.ok_or(RuntimeError::SessionNotRunning)?;
        if current.session_id != record.session_id {
            return Err(RuntimeError::InvalidSession);
        }
        record.pid = pid;
        record.process_start_ticks = start_ticks;
        write_record(&self.root, record)
    }

    fn remove_owned_session(&self, record: &StoredSession) -> Result<(), RuntimeError> {
        let _lock = acquire_lock(&self.root)?;
        let Some(current) = read_record(&self.root)? else {
            return Ok(());
        };
        if current.session_id != record.session_id {
            return Ok(());
        }
        remove_session_files(&self.root, &current)
    }

    pub async fn stop(&self) -> Result<bool, RuntimeError> {
        let record = {
            let _lock = try_acquire_lock(&self.root)?;
            let Some(record) = read_record(&self.root)? else {
                return Ok(false);
            };
            record
        };

        if session_owner_matches(&record) {
            stop_session_owner(&record).await?;
        }
        if record.supervisor_pid.is_some() && child_matches(&record) {
            stop_session_child(&record).await?;
        }

        let _lock = try_acquire_lock(&self.root)?;
        let Some(current) = read_record(&self.root)? else {
            return Ok(true);
        };
        if current.session_id == record.session_id {
            remove_session_files(&self.root, &current)?;
        }
        Ok(true)
    }

    pub async fn health_report(&self) -> Result<HealthReport, RuntimeError> {
        let session = self.active()?.ok_or(RuntimeError::SessionNotRunning)?;
        Ok(observe_session_health(&session).await.report)
    }

    pub async fn repair_once(
        &self,
        ignore_cooldown: bool,
    ) -> Result<RecoveryOutcome, RuntimeError> {
        let session = self.active()?.ok_or(RuntimeError::SessionNotRunning)?;
        let observation = observe_session_health(&session).await;
        match observation.report.status {
            HealthStatus::Healthy => return Ok(RecoveryOutcome::AlreadyHealthy),
            HealthStatus::NotGlobal => return Ok(RecoveryOutcome::NotApplicable),
            HealthStatus::Unhealthy | HealthStatus::ControllerUnavailable => {}
        }

        let snapshot = self
            .desired
            .load()
            .map_err(|_| RuntimeError::PersistentState)?;
        let now = unix_seconds_now();
        if !ignore_cooldown && recovery_cooldown_active(now, snapshot.last_recovery_unix_seconds) {
            return Ok(RecoveryOutcome::Cooldown);
        }
        self.desired
            .mark_recovery_attempt(now)
            .map_err(|_| RuntimeError::PersistentState)?;

        if observation.report.status == HealthStatus::ControllerUnavailable {
            if child_matches(&session.record) {
                stop_session_child(&session.record).await?;
            }
            return Ok(RecoveryOutcome::RestartRequested);
        }
        self.attempt_network_recovery(&session, observation.selection.as_deref())
            .await
    }

    async fn health_loop(self) {
        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + HEALTH_INTERVAL,
            HEALTH_INTERVAL,
        );
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let session = match self.active() {
                Ok(Some(session)) => session,
                Ok(None) => return,
                Err(RuntimeError::SessionBusy) => continue,
                Err(_) => return,
            };
            let first = observe_session_health(&session).await;
            if first.report.status == HealthStatus::Healthy {
                self.remember_healthy_selection(first.selection.as_deref());
                continue;
            }
            if first.report.status == HealthStatus::NotGlobal {
                continue;
            }
            tokio::time::sleep(HEALTH_RECHECK_DELAY).await;
            let session = match self.active() {
                Ok(Some(session)) => session,
                Ok(None) => return,
                Err(RuntimeError::SessionBusy) => continue,
                Err(_) => return,
            };
            let second = observe_session_health(&session).await;
            if second.report.status == HealthStatus::Healthy {
                self.remember_healthy_selection(second.selection.as_deref());
                continue;
            }
            if second.report.status != HealthStatus::NotGlobal {
                let _ = self.repair_once(false).await;
            }
        }
    }

    fn remember_healthy_selection(&self, selection: Option<&str>) {
        let Some(selection) = selection else {
            return;
        };
        let _ = self
            .desired
            .record_healthy_selection("GLOBAL", selection, unix_seconds_now());
    }

    async fn attempt_network_recovery(
        &self,
        session: &ManagedSession,
        original_selection: Option<&str>,
    ) -> Result<RecoveryOutcome, RuntimeError> {
        let profile = session.profile().to_owned();
        let store = ProfileStore::new(self.state_root.join("profiles"))
            .map_err(|_| RuntimeError::PersistentState)?;
        let mut applied_refresh = false;

        if store.update(&profile).await.is_ok() {
            let path = store
                .profile_path(&profile)
                .map_err(|_| RuntimeError::ProfileRead)?;
            if self.switch_profile(&profile, &path).await.is_ok() {
                applied_refresh = true;
                if let Some(active) = self.active()?
                    && observe_session_health(&active).await.report.status == HealthStatus::Healthy
                {
                    self.remember_healthy_selection(original_selection);
                    return Ok(RecoveryOutcome::RecoveredByRefresh);
                }
            } else {
                let _ = store.rollback(&profile);
            }
        }

        let active = self.active()?.ok_or(RuntimeError::SessionNotRunning)?;
        let recovery = self
            .attempt_selection_recovery(&active, original_selection)
            .await?;
        if recovery != RecoveryOutcome::Degraded {
            return Ok(recovery);
        }
        if applied_refresh
            && store.rollback(&profile).is_ok()
            && let Ok(path) = store.profile_path(&profile)
        {
            let _ = self.switch_profile(&profile, &path).await;
        }
        Ok(RecoveryOutcome::Degraded)
    }

    async fn attempt_selection_recovery(
        &self,
        session: &ManagedSession,
        original_selection: Option<&str>,
    ) -> Result<RecoveryOutcome, RuntimeError> {
        let snapshot = self
            .desired
            .load()
            .map_err(|_| RuntimeError::PersistentState)?;
        let client = session.api_client(Duration::from_secs(15))?;
        let proxies = client.proxies().await.ok();
        let candidates = recovery_candidates(&snapshot, proxies.as_ref(), original_selection);

        for candidate in candidates {
            if client.select_proxy("GLOBAL", &candidate).await.is_err() {
                continue;
            }
            let observation = observe_session_health(session).await;
            if observation.report.status == HealthStatus::Healthy {
                self.desired
                    .record_selection("GLOBAL", &candidate)
                    .and_then(|()| {
                        self.desired.record_healthy_selection(
                            "GLOBAL",
                            &candidate,
                            unix_seconds_now(),
                        )
                    })
                    .map_err(|_| RuntimeError::PersistentState)?;
                return Ok(RecoveryOutcome::RecoveredByKnownGood);
            }
        }

        if let Some(original) = original_selection {
            let _ = client.select_proxy("GLOBAL", original).await;
        }
        Ok(RecoveryOutcome::Degraded)
    }

    pub async fn switch_profile_with_recovery(
        &self,
        profile: &str,
        profile_path: &Path,
    ) -> Result<(ManagedSession, RecoveryOutcome), RuntimeError> {
        let session = self.switch_profile(profile, profile_path).await?;
        let observation = observe_session_health(&session).await;
        let outcome = match observation.report.status {
            HealthStatus::Healthy => {
                self.remember_healthy_selection(observation.selection.as_deref());
                RecoveryOutcome::AlreadyHealthy
            }
            HealthStatus::NotGlobal => RecoveryOutcome::NotApplicable,
            HealthStatus::Unhealthy => self
                .attempt_selection_recovery(&session, observation.selection.as_deref())
                .await
                .unwrap_or(RecoveryOutcome::Degraded),
            HealthStatus::ControllerUnavailable => RecoveryOutcome::Degraded,
        };
        Ok((session, outcome))
    }

    pub async fn switch_profile(
        &self,
        profile: &str,
        profile_path: &Path,
    ) -> Result<ManagedSession, RuntimeError> {
        let _lock = try_acquire_lock(&self.root)?;
        let mut session = self
            .active_locked()?
            .ok_or(RuntimeError::SessionNotRunning)?;

        let client = session.api_client(Duration::from_secs(15))?;
        let mode = client
            .configuration()
            .await
            .map_err(|_| RuntimeError::SessionReload)?
            .mode
            .as_deref()
            .and_then(OperatingMode::from_api)
            .unwrap_or(OperatingMode::Global);
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
            mode,
        )?;
        let next = String::from_utf8(next).map_err(|_| RuntimeError::ConfigurationSerialization)?;
        let runtime_config = record.runtime_dir.join("runtime.yaml");
        let previous = read_private_profile(&runtime_config)?;
        let previous_text =
            std::str::from_utf8(&previous).map_err(|_| RuntimeError::InvalidSession)?;
        let previous_record = record.clone();
        let desired_before = self
            .desired
            .load()
            .map_err(|_| RuntimeError::PersistentState)?;
        if client
            .reload_configuration_preserving_listeners(&next)
            .await
            .is_err()
        {
            return if client
                .reload_configuration_preserving_listeners(previous_text)
                .await
                .is_ok()
            {
                Err(RuntimeError::SessionReload)
            } else {
                Err(RuntimeError::SessionSwitchRollback)
            };
        }
        if replace_private(&runtime_config, next.as_bytes()).is_err() {
            return rollback_switch(
                &self.root,
                &client,
                &runtime_config,
                &previous,
                &previous_record,
                RuntimeError::ConfigurationWrite,
            )
            .await;
        }

        record.profile = profile.to_owned();
        if write_record(&self.root, record).is_err() {
            return rollback_switch(
                &self.root,
                &client,
                &runtime_config,
                &previous,
                &previous_record,
                RuntimeError::SessionWrite,
            )
            .await;
        }

        if self.desired.record_runtime(profile, &source, mode).is_err() {
            return rollback_switch(
                &self.root,
                &client,
                &runtime_config,
                &previous,
                &previous_record,
                RuntimeError::PersistentState,
            )
            .await;
        }
        if desired_before.active_profile.as_deref() == Some(profile) {
            let _ = restore_selections_reliably(&client, &desired_before).await;
        }

        Ok(session)
    }

    fn active_locked(&self) -> Result<Option<ManagedSession>, RuntimeError> {
        let Some(record) = read_record(&self.root)? else {
            return Ok(None);
        };
        if session_owner_matches(&record) {
            Ok(Some(ManagedSession { record }))
        } else {
            remove_session_files(&self.root, &record)?;
            Ok(None)
        }
    }
}

fn recovery_candidates(
    snapshot: &DesiredStateSnapshot,
    proxies: Option<&ProxiesResponse>,
    original_selection: Option<&str>,
) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(known_good) = snapshot.known_good.get("GLOBAL") {
        for candidate in known_good {
            if Some(candidate.as_str()) != original_selection && seen.insert(candidate.clone()) {
                candidates.push(candidate.clone());
            }
        }
    }
    if let Some(proxies) = proxies
        && let Some(global) = proxies
            .proxies
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("GLOBAL"))
            .map(|(_, info)| info)
    {
        for candidate in &global.all {
            let profile_defined_fallback = proxies.proxies.get(candidate).is_some_and(|info| {
                matches!(
                    info.kind.to_ascii_lowercase().as_str(),
                    "fallback" | "urltest"
                )
            });
            if profile_defined_fallback
                && Some(candidate.as_str()) != original_selection
                && seen.insert(candidate.clone())
            {
                candidates.push(candidate.clone());
            }
        }
    }
    candidates.truncate(MAX_RECOVERY_CANDIDATES);
    candidates
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
    pub fn session_id(&self) -> &str {
        &self.record.session_id
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

fn open_lock(root: &Path) -> Result<File, RuntimeError> {
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
    Ok(file)
}

fn acquire_lock(root: &Path) -> Result<File, RuntimeError> {
    let file = open_lock(root)?;
    FileExt::lock(&file).map_err(|_| RuntimeError::SessionLock)?;
    Ok(file)
}

fn try_acquire_lock(root: &Path) -> Result<File, RuntimeError> {
    let file = open_lock(root)?;
    match FileExt::try_lock(&file) {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(RuntimeError::SessionBusy),
        Err(TryLockError::Error(_)) => Err(RuntimeError::SessionLock),
    }
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
    let temporary = root.join(format!(".session-{}.tmp", random_hex::<8>()?));
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
    let valid_schema = match record.schema_version {
        LEGACY_SESSION_SCHEMA_VERSION => {
            record.supervisor_pid.is_none() && record.supervisor_start_ticks.is_none()
        }
        SESSION_SCHEMA_VERSION => {
            record.supervisor_pid.is_some_and(|pid| pid > 1)
                && record.supervisor_start_ticks.is_some_and(|ticks| ticks > 0)
        }
        _ => false,
    };
    if !valid_schema
        || record.pid <= 1
        || record.process_start_ticks == 0
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

fn child_matches(record: &StoredSession) -> bool {
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

fn session_owner_matches(record: &StoredSession) -> bool {
    let (Some(pid), Some(start_ticks)) = (record.supervisor_pid, record.supervisor_start_ticks)
    else {
        return child_matches(record);
    };
    if process_start_ticks(pid) != Some(start_ticks) {
        return false;
    }
    fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .is_some_and(|command_line| {
            command_line
                .split(|byte| *byte == 0)
                .any(|argument| matches!(argument, b"supervise" | b"__session-start"))
        })
}

async fn stop_session_owner(record: &StoredSession) -> Result<(), RuntimeError> {
    let pid = record.supervisor_pid.unwrap_or(record.pid);
    stop_matching_process(pid, || session_owner_matches(record)).await
}

async fn stop_session_child(record: &StoredSession) -> Result<(), RuntimeError> {
    stop_matching_process(record.pid, || child_matches(record)).await
}

async fn stop_matching_process(pid: u32, matches: impl Fn() -> bool) -> Result<(), RuntimeError> {
    let pid = i32::try_from(pid)
        .ok()
        .and_then(Pid::from_raw)
        .ok_or(RuntimeError::SessionStop)?;
    if kill_process(pid, Signal::TERM).is_err() {
        return if matches() {
            Err(RuntimeError::SessionStop)
        } else {
            Ok(())
        };
    }
    if wait_for_exit(&matches, STOP_GRACE_PERIOD).await {
        return Ok(());
    }
    if kill_process(pid, Signal::KILL).is_err() {
        return if matches() {
            Err(RuntimeError::SessionStop)
        } else {
            Ok(())
        };
    }
    if wait_for_exit(&matches, STOP_KILL_PERIOD).await {
        Ok(())
    } else {
        Err(RuntimeError::SessionStop)
    }
}

async fn wait_for_exit(matches: &impl Fn() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !matches() {
            return true;
        }
        tokio::time::sleep(PROCESS_POLL_INTERVAL).await;
    }
    !matches()
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

struct HealthObservation {
    report: HealthReport,
    selection: Option<String>,
}

async fn observe_session_health(session: &ManagedSession) -> HealthObservation {
    const REQUIRED_SUCCESSES: usize = 2;

    let total_probes = ProbeTarget::built_in().len();
    let Ok(client) = session.api_client(Duration::from_secs(6)) else {
        return controller_unavailable(total_probes);
    };
    let Ok(configuration) = client.configuration().await else {
        return controller_unavailable(total_probes);
    };
    let mode = configuration
        .mode
        .as_deref()
        .and_then(OperatingMode::from_api)
        .unwrap_or(OperatingMode::Global);
    if mode != OperatingMode::Global {
        return HealthObservation {
            report: HealthReport {
                status: HealthStatus::NotGlobal,
                successful_probes: 0,
                total_probes,
                codex_reachable: false,
            },
            selection: None,
        };
    }
    let Ok(proxies) = client.proxies().await else {
        return controller_unavailable(total_probes);
    };
    let Some((group_name, global)) = proxies
        .proxies
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("GLOBAL"))
    else {
        return HealthObservation {
            report: HealthReport {
                status: HealthStatus::Unhealthy,
                successful_probes: 0,
                total_probes,
                codex_reachable: false,
            },
            selection: None,
        };
    };
    let results = stream::iter(ProbeTarget::built_in().into_iter().map(|target| {
        let client = client.clone();
        let group_name = group_name.clone();
        async move {
            let codex = target.name() == "OpenAI / Codex";
            let success = client.probe_delay(&group_name, &target).await.is_ok();
            (codex, success)
        }
    }))
    .buffer_unordered(total_probes)
    .collect::<Vec<_>>()
    .await;
    let successful_probes = results.iter().filter(|(_, success)| *success).count();
    let codex_reachable = results.iter().any(|(codex, success)| *codex && *success);
    HealthObservation {
        report: HealthReport {
            status: if codex_reachable && successful_probes >= REQUIRED_SUCCESSES {
                HealthStatus::Healthy
            } else {
                HealthStatus::Unhealthy
            },
            successful_probes,
            total_probes,
            codex_reachable,
        },
        selection: global.now.clone(),
    }
}

fn controller_unavailable(total_probes: usize) -> HealthObservation {
    HealthObservation {
        report: HealthReport {
            status: HealthStatus::ControllerUnavailable,
            successful_probes: 0,
            total_probes,
            codex_reachable: false,
        },
        selection: None,
    }
}

fn recovery_cooldown_active(now: u64, last_attempt: Option<u64>) -> bool {
    last_attempt.is_some_and(|last| now >= last && now - last < RECOVERY_COOLDOWN_SECONDS)
}

async fn restore_selections(
    client: &ApiClient,
    desired: &DesiredStateSnapshot,
) -> Result<(), ApiError> {
    let proxies = client.proxies().await?;
    for (group, selection) in &desired.selections {
        let Some(info) = proxies.proxies.get(group) else {
            continue;
        };
        if info.kind.eq_ignore_ascii_case("selector")
            && info.all.iter().any(|candidate| candidate == selection)
        {
            client.select_proxy(group, selection).await?;
        }
    }
    Ok(())
}

async fn restore_selections_reliably(
    client: &ApiClient,
    desired: &DesiredStateSnapshot,
) -> Result<(), RuntimeError> {
    for attempt in 0..3_u64 {
        match restore_selections(client, desired).await {
            Ok(()) => return Ok(()),
            Err(_) if attempt < 2 => {
                tokio::time::sleep(Duration::from_millis(100 * (attempt + 1))).await;
            }
            Err(_) => return Err(RuntimeError::SessionReload),
        }
    }
    unreachable!("the bounded restore loop always returns")
}

async fn rollback_switch(
    root: &Path,
    client: &ApiClient,
    runtime_config: &Path,
    previous: &[u8],
    previous_record: &StoredSession,
    original: RuntimeError,
) -> Result<ManagedSession, RuntimeError> {
    let previous_text = std::str::from_utf8(previous).map_err(|_| RuntimeError::InvalidSession)?;
    let core_rolled_back = client
        .reload_configuration_preserving_listeners(previous_text)
        .await
        .is_ok();
    let file_rolled_back = replace_private(runtime_config, previous).is_ok();
    let descriptor_rolled_back = write_record(root, previous_record).is_ok();
    if core_rolled_back && file_rolled_back && descriptor_rolled_back {
        Err(original)
    } else {
        Err(RuntimeError::SessionSwitchRollback)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        RuntimeError, SESSION_SCHEMA_VERSION, StoredSession, acquire_lock, process_start_ticks,
        recovery_candidates, recovery_cooldown_active, replace_private, shell_clear_owned,
        shell_quote, try_acquire_lock, validate_record, write_record,
    };
    use crate::{
        mihomo::{OperatingMode, ProxiesResponse},
        state::DesiredStateSnapshot,
    };

    #[test]
    fn recovery_cooldown_tolerates_clock_rollback() {
        assert!(recovery_cooldown_active(1_500, Some(1_000)));
        assert!(!recovery_cooldown_active(1_600, Some(1_000)));
        assert!(!recovery_cooldown_active(900, Some(1_000)));
        assert!(!recovery_cooldown_active(1_000, None));
    }

    #[test]
    fn recovery_candidates_are_bounded_to_known_good_and_profile_groups() {
        let snapshot = DesiredStateSnapshot {
            active_profile: Some("default".into()),
            profile_sha256: Some("11".repeat(32)),
            mode: OperatingMode::Global,
            selections: BTreeMap::from([("GLOBAL".into(), "Current".into())]),
            known_good: BTreeMap::from([(
                "GLOBAL".into(),
                vec!["Known Good".into(), "Current".into(), "Auto".into()],
            )]),
            last_recovery_unix_seconds: None,
        };
        let proxies = serde_json::from_value::<ProxiesResponse>(serde_json::json!({
            "proxies": {
                "GLOBAL": {
                    "name": "GLOBAL",
                    "type": "Selector",
                    "now": "Current",
                    "all": ["Current", "Auto", "Fallback", "Leaf", "Extra Fallback"]
                },
                "Auto": {"name": "Auto", "type": "URLTest"},
                "Fallback": {"name": "Fallback", "type": "Fallback"},
                "Leaf": {"name": "Leaf", "type": "Shadowsocks"},
                "Extra Fallback": {"name": "Extra Fallback", "type": "Fallback"}
            }
        }))
        .expect("fixture should deserialize");

        assert_eq!(
            recovery_candidates(&snapshot, Some(&proxies), Some("Current")),
            ["Known Good", "Auto", "Fallback", "Extra Fallback"]
        );
    }

    #[test]
    fn contended_session_lock_fails_fast_and_recovers_after_release() {
        let root = temporary_directory();
        fs::create_dir(&root).expect("root should be created");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .expect("root permissions should be set");
        let held = acquire_lock(&root).expect("first lock should be acquired");

        assert!(matches!(
            try_acquire_lock(&root),
            Err(RuntimeError::SessionBusy)
        ));
        drop(held);
        assert!(try_acquire_lock(&root).is_ok());
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

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
            supervisor_pid: Some(std::process::id()),
            supervisor_start_ticks: Some(
                process_start_ticks(std::process::id())
                    .expect("current process should have proc metadata"),
            ),
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
