#![forbid(unsafe_code)]

use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, ExitCode, Stdio},
    time::{Duration, Instant},
};

use clap::Parser;
use futures_util::{StreamExt, stream};
use mihoterm::{
    app::{App, fetch_snapshot},
    cli::{Cli, Command, DEFAULT_CONTROLLER, ProfileCommand},
    config::{AppPaths, load_controller_secret, load_probe_targets},
    doctor::stale_clients,
    mihomo::ApiClient,
    onboarding,
    probe::{ProbeTarget, select_probe_targets},
    profile::{ProfileSource, ProfileStore},
    runtime::{
        HealthStatus, ManagedSession, RecoveryOutcome, RuntimeError, SessionManager,
        default_mihomo_executable, detach_supervisor, exec_managed_child, shell_clear_owned,
    },
    state::{DesiredStateStore, profile_digest},
    tui,
};
use tokio::signal::unix::{SignalKind, signal};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let signal_passthrough = matches!(
        &cli.command,
        Some(
            Command::Shell { .. }
                | Command::Exec { .. }
                | Command::Uninstall { .. }
                | Command::RuntimeChild { .. }
                | Command::SessionStart { .. }
                | Command::Supervise { .. }
        )
    );
    let result = if signal_passthrough {
        run(cli).await
    } else {
        run_until_signal(cli).await
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mihoterm: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run_until_signal(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;

    tokio::select! {
        result = run(cli) => result,
        _ = interrupt.recv() => Ok(()),
        _ = terminate.recv() => Ok(()),
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let Cli {
        config,
        state_dir,
        runtime_dir,
        controller,
        secret_file,
        timeout_ms,
        refresh_ms,
        command,
    } = cli;

    let command = match command {
        Some(Command::RuntimeChild {
            parent_pid,
            binary,
            home,
            config,
            test,
        }) => {
            exec_managed_child(parent_pid, &binary, &home, &config, test)?;
            unreachable!("a successful child exec does not return");
        }
        Some(Command::SessionStart {
            profile,
            profile_path,
            mihomo,
            runtime_root,
            state_root,
        }) => {
            detach_supervisor()?;
            let manager = SessionManager::with_state(&runtime_root, &state_root)?;
            manager.supervise(&profile, &profile_path, &mihomo).await?;
            return Ok(());
        }
        Some(Command::Uninstall { purge }) => return run_uninstaller(purge),
        command => command,
    };

    let config_is_explicit = config.is_some();
    let paths = AppPaths::discover(
        config.as_deref(),
        state_dir.as_deref(),
        runtime_dir.as_deref(),
    )?;
    paths.prepare_private_state()?;

    match command {
        Some(Command::Profile { command }) => run_profile(command, &paths).await,
        Some(Command::Start { profile, mihomo }) => {
            let session =
                start_session_direct(profile.as_deref(), mihomo.as_deref(), &paths).await?;
            println!(
                "Managed proxy running | profile {} | mixed 127.0.0.1:{} | pid {}",
                session.profile(),
                session.mixed_port(),
                session.pid()
            );
            Ok(())
        }
        Some(Command::Supervise { profile, mihomo }) => {
            let store = ProfileStore::new(paths.profiles_dir())?;
            let profile =
                resolve_managed_profile(&store, profile.as_deref(), paths.state_dir()).await?;
            let profile_path = store.profile_path(&profile)?;
            let mihomo = mihomo
                .as_deref()
                .map(Path::to_owned)
                .unwrap_or_else(default_mihomo_executable);
            managed_session_manager(&paths)?
                .supervise(&profile, &profile_path, &mihomo)
                .await?;
            Ok(())
        }
        Some(Command::Stop) => {
            let manager = managed_session_manager(&paths)?;
            if manager.stop().await? {
                println!("Stopped the managed proxy.");
            } else {
                println!("The managed proxy is not running.");
            }
            Ok(())
        }
        Some(Command::Env { if_running }) => {
            let manager = managed_session_manager(&paths)?;
            match manager.active()? {
                Some(session) => print!("{}", session.proxy_environment().shell_exports()),
                None if if_running => print!("{}", shell_clear_owned()),
                None => return Err(RuntimeError::SessionNotRunning.into()),
            }
            Ok(())
        }
        Some(Command::Shell { profile, mihomo }) => {
            let session = ensure_session(profile.as_deref(), mihomo.as_deref(), &paths).await?;
            run_proxy_shell(&session)
        }
        Some(Command::Exec {
            profile,
            mihomo,
            command,
        }) => {
            let session = ensure_session(profile.as_deref(), mihomo.as_deref(), &paths).await?;
            run_proxy_command(&session, &command)
        }
        Some(Command::Run { profile, mihomo }) => {
            let probes = load_probe_targets(paths.config_file(), config_is_explicit)?;
            let session = ensure_session(profile.as_deref(), mihomo.as_deref(), &paths).await?;
            run_session_tui(
                session,
                probes,
                Duration::from_millis(timeout_ms),
                Duration::from_millis(refresh_ms),
                paths.profiles_dir(),
                paths.runtime_dir().to_owned(),
                paths.state_dir().to_owned(),
            )
            .await
        }
        None if controller.is_none() => {
            let probes = load_probe_targets(paths.config_file(), config_is_explicit)?;
            let session = ensure_session(None, None, &paths).await?;
            run_session_tui(
                session,
                probes,
                Duration::from_millis(timeout_ms),
                Duration::from_millis(refresh_ms),
                paths.profiles_dir(),
                paths.runtime_dir().to_owned(),
                paths.state_dir().to_owned(),
            )
            .await
        }
        None => {
            let secret = load_controller_secret(secret_file.as_deref())?;
            let client = ApiClient::with_timeout(
                controller.as_deref().unwrap_or(DEFAULT_CONTROLLER),
                secret,
                Duration::from_millis(timeout_ms),
            )?;
            let controller = client.controller_url().origin().ascii_serialization();
            let probes = load_probe_targets(paths.config_file(), config_is_explicit)?;
            let app = App::with_probes(controller, probes);
            tui::run(client, app, Duration::from_millis(refresh_ms), None).await?;
            Ok(())
        }
        Some(Command::Status) if controller.is_none() => {
            let manager = managed_session_manager(&paths)?;
            let session = manager.active()?.ok_or(RuntimeError::SessionNotRunning)?;
            let snapshot =
                fetch_snapshot(&session.api_client(Duration::from_millis(timeout_ms))?).await?;
            println!(
                "MihoTerm proxy running | Mihomo {} | mode {} | profile {} | mixed 127.0.0.1:{} | pid {}",
                snapshot.version,
                snapshot.mode,
                session.profile(),
                session.mixed_port(),
                session.pid()
            );
            Ok(())
        }
        Some(Command::Status) => {
            let secret = load_controller_secret(secret_file.as_deref())?;
            let client = ApiClient::with_timeout(
                controller.as_deref().unwrap_or(DEFAULT_CONTROLLER),
                secret,
                Duration::from_millis(timeout_ms),
            )?;
            let snapshot = fetch_snapshot(&client).await?;
            println!(
                "Mihomo {} | mode {} | {} policy groups",
                snapshot.version,
                snapshot.mode,
                snapshot.groups.len()
            );
            Ok(())
        }
        Some(Command::Doctor { repair }) => run_doctor(&paths, repair).await,
        Some(Command::Probe { proxy, targets }) => {
            let probes = load_probe_targets(paths.config_file(), config_is_explicit)?;
            let probes = select_probe_targets(&probes, &targets)?;
            let request_timeout = Duration::from_millis(timeout_ms);
            let client = if let Some(controller) = controller.as_deref() {
                let secret = load_controller_secret(secret_file.as_deref())?;
                ApiClient::with_timeout(controller, secret, request_timeout)?
            } else {
                let manager = managed_session_manager(&paths)?;
                let session = manager.active()?.ok_or(RuntimeError::SessionNotRunning)?;
                session.api_client(request_timeout)?
            };

            run_probe_command(&client, &proxy, probes).await
        }
        Some(Command::Attach) => {
            let secret = load_controller_secret(secret_file.as_deref())?;
            let client = ApiClient::with_timeout(
                controller.as_deref().unwrap_or(DEFAULT_CONTROLLER),
                secret,
                Duration::from_millis(timeout_ms),
            )?;
            let controller = client.controller_url().origin().ascii_serialization();
            let probes = load_probe_targets(paths.config_file(), config_is_explicit)?;
            let app = App::with_probes(controller, probes);
            tui::run(client, app, Duration::from_millis(refresh_ms), None).await?;
            Ok(())
        }
        Some(
            Command::Uninstall { .. } | Command::RuntimeChild { .. } | Command::SessionStart { .. },
        ) => unreachable!("internal commands return before path discovery"),
    }
}

async fn start_session_direct(
    profile: Option<&str>,
    mihomo: Option<&Path>,
    paths: &AppPaths,
) -> Result<ManagedSession, Box<dyn std::error::Error>> {
    ensure_session(profile, mihomo, paths).await
}

async fn ensure_session(
    profile: Option<&str>,
    mihomo: Option<&Path>,
    paths: &AppPaths,
) -> Result<ManagedSession, Box<dyn std::error::Error>> {
    let manager = managed_session_manager(paths)?;
    let lock_deadline = Instant::now() + Duration::from_secs(50);
    loop {
        match manager.active() {
            Ok(Some(session)) => {
                if profile.is_none_or(|requested| requested == session.profile()) {
                    return Ok(session);
                }
                return Err(RuntimeError::SessionProfileConflict.into());
            }
            Ok(None) => break,
            Err(RuntimeError::SessionBusy) if Instant::now() < lock_deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }

    let store = ProfileStore::new(paths.profiles_dir())?;
    let profile = resolve_managed_profile(&store, profile, paths.state_dir()).await?;
    let profile_path = store.profile_path(&profile)?;
    let mihomo = mihomo
        .map(Path::to_owned)
        .unwrap_or_else(default_mihomo_executable);
    let mut child = ProcessCommand::new(env::current_exe()?)
        .arg("__session-start")
        .arg("--profile")
        .arg(&profile)
        .arg("--profile-path")
        .arg(profile_path)
        .arg("--mihomo")
        .arg(mihomo)
        .arg("--runtime-root")
        .arg(paths.runtime_dir())
        .arg("--state-root")
        .arg(paths.state_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(50);
    let mut launcher_status = None;
    loop {
        if launcher_status.is_none() {
            launcher_status = child.try_wait()?;
        }
        match manager.active() {
            Ok(Some(session)) => {
                drop(child);
                return Ok(session);
            }
            Ok(None) => {
                if let Some(status) = launcher_status.as_ref() {
                    return Err(RuntimeError::UnexpectedExit {
                        code: status.code(),
                    }
                    .into());
                }
            }
            Err(RuntimeError::SessionBusy) => {}
            Err(error) => return Err(error.into()),
        }
        if Instant::now() >= deadline {
            if launcher_status.is_none() {
                let _ = child.kill();
                let _ = child.try_wait();
            }
            return Err(RuntimeError::ControllerUnavailable.into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn run_session_tui(
    session: ManagedSession,
    probes: Vec<mihoterm::probe::ProbeTarget>,
    request_timeout: Duration,
    refresh_interval: Duration,
    profiles_dir: PathBuf,
    runtime_dir: PathBuf,
    state_dir: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = session.api_client(request_timeout)?;
    let display = format!("managed  mixed 127.0.0.1:{}", session.mixed_port());
    let active_profile = session.profile().to_owned();
    let store = ProfileStore::new(profiles_dir)?;
    let profiles = store.list()?;
    let app = App::with_managed_profiles(display, probes, active_profile, profiles);
    let session_manager = SessionManager::with_state(&runtime_dir, &state_dir)?;
    let desired = DesiredStateStore::new(state_dir)?;
    let managed_profiles = tui::ManagedProfiles::new(store, session_manager, desired);
    tui::run(client, app, refresh_interval, Some(managed_profiles)).await?;
    Ok(())
}

fn managed_session_manager(paths: &AppPaths) -> Result<SessionManager, RuntimeError> {
    SessionManager::with_state(paths.runtime_dir(), paths.state_dir())
}

async fn resolve_managed_profile(
    store: &ProfileStore,
    requested: Option<&str>,
    state_root: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(profile) = requested {
        return Ok(profile.to_owned());
    }
    let desired = DesiredStateStore::new(state_root.to_owned())?.load()?;
    if let Some(profile) = desired.active_profile
        && store.profile_path(&profile).is_ok()
    {
        return Ok(profile);
    }
    onboarding::resolve_profile(store, None)
        .await
        .map_err(Into::into)
}

async fn run_doctor(paths: &AppPaths, repair: bool) -> Result<(), Box<dyn std::error::Error>> {
    let manager = managed_session_manager(paths)?;
    let mut repair_outcome = None;
    if repair {
        let _ = ensure_session(None, None, paths).await?;
        let outcome = manager.repair_once(true).await?;
        if outcome == RecoveryOutcome::RestartRequested {
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                if manager
                    .health_report()
                    .await
                    .is_ok_and(|report| report.status != HealthStatus::ControllerUnavailable)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        repair_outcome = Some(outcome);
    }

    println!(
        "runtime storage: {}{}",
        paths.runtime_dir().display(),
        if paths.runtime_uses_state_fallback() {
            " (durable state fallback)"
        } else {
            ""
        }
    );
    if let Some(outcome) = repair_outcome {
        println!("repair: {}", recovery_label(outcome));
    }

    let mut critical_issue = false;
    let session = match manager.active() {
        Ok(Some(session)) => {
            println!(
                "managed session: running | profile {} | mixed 127.0.0.1:{} | pid {}",
                session.profile(),
                session.mixed_port(),
                session.pid()
            );
            Some(session)
        }
        Ok(None) => {
            println!("managed session: stopped");
            critical_issue = true;
            None
        }
        Err(error) => {
            println!("managed session: invalid ({error})");
            critical_issue = true;
            None
        }
    };

    match DesiredStateStore::new(paths.state_dir().to_owned()).and_then(|store| store.load()) {
        Ok(desired) => {
            let revision_matches = desired
                .active_profile
                .as_deref()
                .zip(desired.profile_sha256.as_deref())
                .and_then(|(profile, expected)| {
                    let store = ProfileStore::new(paths.profiles_dir()).ok()?;
                    let path = store.profile_path(profile).ok()?;
                    let contents = fs::read(path).ok()?;
                    Some(profile_digest(&contents) == expected)
                })
                .unwrap_or(false);
            let active_matches = session.as_ref().is_some_and(|session| {
                desired.active_profile.as_deref() == Some(session.profile())
            });
            if revision_matches && active_matches {
                println!("desired state: profile revision and active session match");
            } else {
                println!("desired state: profile revision or active session mismatch");
                critical_issue = true;
            }
        }
        Err(error) => {
            println!("desired state: invalid ({error})");
            critical_issue = true;
        }
    }

    if session.is_some() {
        match manager.health_report().await {
            Ok(report) => {
                println!(
                    "connectivity: {} | probes {}/{} | Codex {}",
                    health_label(report.status),
                    report.successful_probes,
                    report.total_probes,
                    if report.codex_reachable {
                        "reachable"
                    } else {
                        "unreachable"
                    }
                );
                if matches!(
                    report.status,
                    HealthStatus::Unhealthy | HealthStatus::ControllerUnavailable
                ) {
                    critical_issue = true;
                }
            }
            Err(error) => {
                println!("connectivity: unavailable ({error})");
                critical_issue = true;
            }
        }
    }

    let clients = stale_clients(session.as_ref().map(ManagedSession::session_id));
    if clients.is_empty() {
        println!("inherited clients: no stale MihoTerm session markers found");
    } else {
        println!(
            "inherited clients: {} process(es) use a different MihoTerm session marker",
            clients.len()
        );
        for client in clients.iter().take(8) {
            println!("  pid {}  {}", client.pid, client.command);
        }
        println!("  these may be stale clients or another explicitly isolated MihoTerm instance");
        println!("  restart stale terminals/processes or re-source the MihoTerm shell integration");
    }

    if critical_issue {
        Err(io::Error::other("doctor found unresolved managed proxy issues").into())
    } else {
        Ok(())
    }
}

const fn health_label(status: HealthStatus) -> &'static str {
    match status {
        HealthStatus::Healthy => "healthy",
        HealthStatus::Unhealthy => "unhealthy",
        HealthStatus::NotGlobal => "skipped outside global mode",
        HealthStatus::ControllerUnavailable => "controller unavailable",
    }
}

const fn recovery_label(outcome: RecoveryOutcome) -> &'static str {
    match outcome {
        RecoveryOutcome::AlreadyHealthy => "already healthy",
        RecoveryOutcome::NotApplicable => "not applicable outside global mode",
        RecoveryOutcome::Cooldown => "automatic recovery is cooling down",
        RecoveryOutcome::RestartRequested => "requested an exact-child restart",
        RecoveryOutcome::RecoveredByRefresh => "recovered after a validated profile refresh",
        RecoveryOutcome::RecoveredByKnownGood => "recovered with a bounded known-good fallback",
        RecoveryOutcome::Degraded => "recovery exhausted; session remains degraded",
    }
}

async fn run_probe_command(
    client: &ApiClient,
    proxy: &str,
    probes: Vec<ProbeTarget>,
) -> Result<(), Box<dyn std::error::Error>> {
    const MAX_CONCURRENT_PROBES: usize = 4;

    let mut results = stream::iter(probes.into_iter().enumerate().map(|(index, target)| {
        let client = client.clone();
        async move {
            let result = client.probe_delay(proxy, &target).await;
            (index, target, result)
        }
    }))
    .buffer_unordered(MAX_CONCURRENT_PROBES)
    .collect::<Vec<_>>()
    .await;
    results.sort_by_key(|(index, _, _)| *index);

    let total = results.len();
    let mut failures = 0;
    for (_, target, result) in results {
        match result {
            Ok(response) => {
                println!(
                    "{}: {proxy} responded in {} ms",
                    target.name(),
                    response.delay
                );
            }
            Err(error) => {
                failures += 1;
                println!("{}: {proxy} failed: {error}", target.name());
            }
        }
    }

    if failures == 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!("{failures} of {total} probes failed")).into())
    }
}

fn run_proxy_shell(session: &ManagedSession) -> Result<(), Box<dyn std::error::Error>> {
    let shell = env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
    let mut command = ProcessCommand::new(shell);
    session.proxy_environment().apply(&mut command);
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("proxy shell exited with {status}")).into())
    }
}

fn run_proxy_command(
    session: &ManagedSession,
    arguments: &[OsString],
) -> Result<(), Box<dyn std::error::Error>> {
    let (program, arguments) = arguments
        .split_first()
        .ok_or_else(|| io::Error::other("a command is required"))?;
    let mut command = ProcessCommand::new(program);
    command.args(arguments);
    session.proxy_environment().apply(&mut command);
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("proxied command exited with {status}")).into())
    }
}

fn run_uninstaller(purge: bool) -> Result<(), Box<dyn std::error::Error>> {
    let executable = env::current_exe()?;
    let script = executable
        .parent()
        .map(|directory| directory.join("install.sh"))
        .ok_or_else(|| io::Error::other("the installed bundle directory is unavailable"))?;
    if !script.is_file() {
        return Err(
            io::Error::other("install.sh was not found beside the MihoTerm executable").into(),
        );
    }

    let mut command = ProcessCommand::new("/bin/sh");
    command.arg(script).arg("uninstall");
    if purge {
        command.arg("--purge");
    }
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("uninstaller exited with {status}")).into())
    }
}

async fn run_profile(
    command: ProfileCommand,
    paths: &AppPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let store = ProfileStore::new(paths.profiles_dir())?;

    match command {
        ProfileCommand::Add { id, url_file, file } => {
            let source = profile_source(url_file.as_deref(), file.as_deref())?;
            let kind = source.kind();
            store.add(&id, source).await?;
            println!("Added profile {id} from {kind}.");
        }
        ProfileCommand::Update { id, apply } => {
            store.update(&id).await?;
            if apply {
                let profile_path = store.profile_path(&id)?;
                let manager = managed_session_manager(paths)?;
                let recovery = match manager
                    .switch_profile_with_recovery(&id, &profile_path)
                    .await
                {
                    Ok((_, recovery)) => recovery,
                    Err(error) => {
                        if store.rollback(&id).is_err() {
                            return Err(RuntimeError::SessionSwitchRollback.into());
                        }
                        return Err(error.into());
                    }
                };
                println!("Updated and applied profile {id} without recreating listeners.");
                if !matches!(
                    recovery,
                    RecoveryOutcome::AlreadyHealthy | RecoveryOutcome::NotApplicable
                ) {
                    println!("Post-apply recovery: {}.", recovery_label(recovery));
                }
            } else {
                println!("Updated profile {id}.");
            }
        }
        ProfileCommand::Rollback { id } => {
            store.rollback(&id)?;
            println!("Rolled back profile {id}.");
        }
        ProfileCommand::List => {
            for profile in store.list()? {
                let backup = if profile.has_backup {
                    " (backup available)"
                } else {
                    ""
                };
                println!("{}{backup}", profile.id);
            }
        }
        ProfileCommand::Path { id } => {
            println!("{}", store.profile_path(&id)?.display());
        }
    }
    Ok(())
}

fn profile_source(
    url_file: Option<&Path>,
    file: Option<&Path>,
) -> Result<ProfileSource, Box<dyn std::error::Error>> {
    match (url_file, file) {
        (Some(path), None) => Ok(ProfileSource::from_url_file(path)?),
        (None, Some(path)) => Ok(ProfileSource::from_local_file(path)?),
        _ => unreachable!("clap requires exactly one profile source"),
    }
}
