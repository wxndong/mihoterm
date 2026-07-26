#![forbid(unsafe_code)]

use std::{path::Path, process::ExitCode, time::Duration};

use clap::Parser;
use mihoterm::{
    app::{App, fetch_snapshot},
    cli::{Cli, Command, ProfileCommand},
    config::{AppPaths, load_controller_secret, load_probe_targets},
    mihomo::ApiClient,
    profile::{ProfileSource, ProfileStore},
    runtime::{ManagedRuntime, RuntimeError, exec_managed_child},
    tui,
};

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mihoterm: {error}");
            ExitCode::FAILURE
        }
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
        command => command,
    };

    let config_is_explicit = config.is_some();
    let paths = AppPaths::discover(
        config.as_deref(),
        state_dir.as_deref(),
        runtime_dir.as_deref(),
    )?;

    let command = match command {
        Some(Command::Profile { command }) => {
            return run_profile(command, paths.profiles_dir()).await;
        }
        Some(Command::Run { profile, mihomo }) => {
            let probes = load_probe_targets(paths.config_file(), config_is_explicit)?;
            return run_managed(
                &profile,
                &mihomo,
                &paths,
                probes,
                Duration::from_millis(timeout_ms),
                Duration::from_millis(refresh_ms),
            )
            .await;
        }
        command => command,
    };

    let secret = load_controller_secret(secret_file.as_deref())?;
    let client = ApiClient::with_timeout(&controller, secret, Duration::from_millis(timeout_ms))?;

    match command {
        Some(Command::Status) => {
            let snapshot = fetch_snapshot(&client).await?;
            println!(
                "Mihomo {} | mode {} | {} policy groups",
                snapshot.version,
                snapshot.mode,
                snapshot.groups.len()
            );
            Ok(())
        }
        None => {
            let controller = client.controller_url().origin().ascii_serialization();
            let probes = load_probe_targets(paths.config_file(), config_is_explicit)?;
            let app = App::with_probes(controller, probes);
            tui::run(client, app, Duration::from_millis(refresh_ms)).await?;
            Ok(())
        }
        Some(Command::Profile { .. } | Command::Run { .. } | Command::RuntimeChild { .. }) => {
            unreachable!("non-attach commands return before API setup")
        }
    }
}

async fn run_managed(
    profile: &str,
    mihomo: &Path,
    paths: &AppPaths,
    probes: Vec<mihoterm::probe::ProbeTarget>,
    request_timeout: Duration,
    refresh_interval: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let store = ProfileStore::new(paths.profiles_dir())?;
    let profile_path = store.profile_path(profile)?;
    let mut runtime = ManagedRuntime::start(mihomo, &profile_path, paths.runtime_dir()).await?;
    let readiness_client = runtime.api_client(Duration::from_millis(500))?;
    runtime
        .wait_ready(&readiness_client, Duration::from_secs(15))
        .await?;

    let client = runtime.api_client(request_timeout)?;
    let display = format!("managed  mixed 127.0.0.1:{}", runtime.mixed_port());
    let app = App::with_probes(display, probes);

    enum Outcome {
        Tui(Result<(), tui::TuiError>),
        Child(Result<std::process::ExitStatus, RuntimeError>),
    }

    let outcome = tokio::select! {
        result = tui::run(client, app, refresh_interval) => Outcome::Tui(result),
        status = runtime.wait() => Outcome::Child(status),
    };

    match outcome {
        Outcome::Tui(result) => {
            let stop = runtime.stop();
            result?;
            stop?;
            Ok(())
        }
        Outcome::Child(status) => {
            let status = status?;
            Err(RuntimeError::UnexpectedExit {
                code: status.code(),
            }
            .into())
        }
    }
}

async fn run_profile(
    command: ProfileCommand,
    profiles_dir: std::path::PathBuf,
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
