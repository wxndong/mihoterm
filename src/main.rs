#![forbid(unsafe_code)]

use std::{
    env,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, ExitCode, Stdio},
    time::Duration,
};

use clap::Parser;
use futures_util::{StreamExt, stream};
use mihoterm::{
    app::{App, fetch_snapshot},
    cli::{Cli, Command, DEFAULT_CONTROLLER, ProfileCommand},
    config::{AppPaths, load_controller_secret, load_probe_targets},
    mihomo::ApiClient,
    onboarding,
    probe::{ProbeTarget, select_probe_targets},
    profile::{ProfileSource, ProfileStore},
    runtime::{
        ManagedSession, RuntimeError, SessionManager, default_mihomo_executable,
        exec_managed_child, shell_clear_owned,
    },
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
            detached,
        }) => {
            exec_managed_child(parent_pid, &binary, &home, &config, test, detached)?;
            unreachable!("a successful child exec does not return");
        }
        Some(Command::SessionStart {
            profile,
            profile_path,
            mihomo,
            runtime_root,
        }) => {
            let manager = SessionManager::new(&runtime_root)?;
            manager.start(&profile, &profile_path, &mihomo).await?;
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
        Some(Command::Profile { command }) => run_profile(command, paths.profiles_dir()).await,
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
        Some(Command::Stop) => {
            let manager = SessionManager::new(paths.runtime_dir())?;
            if manager.stop().await? {
                println!("Stopped the managed proxy.");
            } else {
                println!("The managed proxy is not running.");
            }
            Ok(())
        }
        Some(Command::Env { if_running }) => {
            let manager = SessionManager::new(paths.runtime_dir())?;
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
            let manager = SessionManager::new(paths.runtime_dir())?;
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
        Some(Command::Probe { proxy, targets }) => {
            let probes = load_probe_targets(paths.config_file(), config_is_explicit)?;
            let probes = select_probe_targets(&probes, &targets)?;
            let request_timeout = Duration::from_millis(timeout_ms);
            let client = if let Some(controller) = controller.as_deref() {
                let secret = load_controller_secret(secret_file.as_deref())?;
                ApiClient::with_timeout(controller, secret, request_timeout)?
            } else {
                let manager = SessionManager::new(paths.runtime_dir())?;
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
    let manager = SessionManager::new(paths.runtime_dir())?;
    if let Some(session) = manager.active()? {
        if profile.is_none_or(|requested| requested == session.profile()) {
            return Ok(session);
        }
        return Err(RuntimeError::SessionProfileConflict.into());
    }

    let store = ProfileStore::new(paths.profiles_dir())?;
    let profile = onboarding::resolve_profile(&store, profile).await?;
    let profile_path = store.profile_path(&profile)?;
    let mihomo = mihomo
        .map(Path::to_owned)
        .unwrap_or_else(default_mihomo_executable);
    manager
        .start(&profile, &profile_path, &mihomo)
        .await
        .map_err(Into::into)
}

async fn ensure_session(
    profile: Option<&str>,
    mihomo: Option<&Path>,
    paths: &AppPaths,
) -> Result<ManagedSession, Box<dyn std::error::Error>> {
    let manager = SessionManager::new(paths.runtime_dir())?;
    if let Some(session) = manager.active()? {
        if profile.is_none_or(|requested| requested == session.profile()) {
            return Ok(session);
        }
        return Err(RuntimeError::SessionProfileConflict.into());
    }

    let store = ProfileStore::new(paths.profiles_dir())?;
    let profile = onboarding::resolve_profile(&store, profile).await?;
    let profile_path = store.profile_path(&profile)?;
    let mihomo = mihomo
        .map(Path::to_owned)
        .unwrap_or_else(default_mihomo_executable);
    let status = ProcessCommand::new(env::current_exe()?)
        .arg("__session-start")
        .arg("--profile")
        .arg(&profile)
        .arg("--profile-path")
        .arg(profile_path)
        .arg("--mihomo")
        .arg(mihomo)
        .arg("--runtime-root")
        .arg(paths.runtime_dir())
        .stdin(Stdio::null())
        .status()?;
    if !status.success() {
        return Err(RuntimeError::UnexpectedExit {
            code: status.code(),
        }
        .into());
    }
    manager
        .active()?
        .ok_or_else(|| RuntimeError::SessionNotRunning.into())
}

async fn run_session_tui(
    session: ManagedSession,
    probes: Vec<mihoterm::probe::ProbeTarget>,
    request_timeout: Duration,
    refresh_interval: Duration,
    profiles_dir: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = session.api_client(request_timeout)?;
    let display = format!(
        "managed  profile {}  mixed 127.0.0.1:{}",
        session.profile(),
        session.mixed_port()
    );
    let active_profile = session.profile().to_owned();
    let store = ProfileStore::new(profiles_dir)?;
    let profiles = store.list()?;
    let app = App::with_managed_profiles(display, probes, active_profile, profiles);
    tui::run(client, app, refresh_interval, Some(store)).await?;
    Ok(())
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
    profiles_dir: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let store = ProfileStore::new(profiles_dir)?;

    match command {
        ProfileCommand::Add { id, url_file, file } => {
            let source = profile_source(url_file.as_deref(), file.as_deref())?;
            let kind = source.kind();
            store.add(&id, source).await?;
            println!("Added profile {id} from {kind}.");
        }
        ProfileCommand::Update { id } => {
            store.update(&id).await?;
            println!("Updated profile {id}.");
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
