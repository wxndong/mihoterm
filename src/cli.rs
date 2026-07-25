use std::path::PathBuf;

use clap::{Parser, Subcommand};

const DEFAULT_CONTROLLER: &str = "http://127.0.0.1:9090";

#[derive(Debug, Clone, Parser)]
#[command(
    name = "mihoterm",
    version,
    about = "A tiny, fast, keyboard-first TUI for Mihomo on Linux."
)]
pub struct Cli {
    /// Mihomo external-controller URL.
    #[arg(
        long,
        env = "MIHOTERM_CONTROLLER",
        default_value = DEFAULT_CONTROLLER,
        value_name = "URL"
    )]
    pub controller: String,

    /// File containing the Mihomo controller secret.
    #[arg(
        long,
        env = "MIHOTERM_SECRET_FILE",
        value_name = "PATH",
        hide_env_values = true
    )]
    pub secret_file: Option<PathBuf>,

    /// Total timeout for each controller request.
    #[arg(
        long,
        default_value_t = 5_000,
        value_parser = clap::value_parser!(u64).range(100..=60_000),
        value_name = "MILLISECONDS"
    )]
    pub timeout_ms: u64,

    /// Delay between background snapshots.
    #[arg(
        long,
        default_value_t = 1_500,
        value_parser = clap::value_parser!(u64).range(250..=60_000),
        value_name = "MILLISECONDS"
    )]
    pub refresh_ms: u64,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Print a sanitized one-line controller status without opening the TUI.
    Status,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command, DEFAULT_CONTROLLER};

    #[test]
    fn defaults_to_the_local_controller_and_tui() {
        let cli = Cli::try_parse_from(["mihoterm"]).expect("defaults should parse");

        assert_eq!(cli.controller, DEFAULT_CONTROLLER);
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_the_headless_status_command() {
        let cli = Cli::try_parse_from([
            "mihoterm",
            "--controller",
            "http://127.0.0.1:19090",
            "status",
        ])
        .expect("status should parse");

        assert!(matches!(cli.command, Some(Command::Status)));
        assert_eq!(cli.timeout_ms, 5_000);
    }

    #[test]
    fn rejects_an_excessive_refresh_rate() {
        let result = Cli::try_parse_from(["mihoterm", "--refresh-ms", "10"]);

        assert!(result.is_err());
    }
}
