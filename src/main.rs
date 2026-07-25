#![forbid(unsafe_code)]

use std::{process::ExitCode, time::Duration};

use clap::Parser;
use mihoterm::{
    app::{App, fetch_snapshot},
    cli::{Cli, Command},
    config::load_controller_secret,
    mihomo::ApiClient,
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
    let secret = load_controller_secret(cli.secret_file.as_deref())?;
    let client = ApiClient::with_timeout(
        &cli.controller,
        secret,
        Duration::from_millis(cli.timeout_ms),
    )?;

    match cli.command {
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
            let app = App::new(controller);
            tui::run(client, app, Duration::from_millis(cli.refresh_ms)).await?;
            Ok(())
        }
    }
}
