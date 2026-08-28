use std::{ffi::OsString, path::PathBuf};

use clap::{Parser, Subcommand};

pub const DEFAULT_CONTROLLER: &str = "http://127.0.0.1:9090";

#[derive(Debug, Clone, Parser)]
#[command(
    name = "mihoterm",
    version,
    about = "A tiny, fast, keyboard-first TUI for Mihomo on Linux."
)]
pub struct Cli {
    /// User configuration file.
    #[arg(long, env = "MIHOTERM_CONFIG", value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Directory for managed profiles and other persistent state.
    #[arg(long, env = "MIHOTERM_STATE_DIR", value_name = "PATH")]
    pub state_dir: Option<PathBuf>,

    /// Directory for isolated, transient managed-runtime files.
    #[arg(long, env = "MIHOTERM_RUNTIME_DIR", value_name = "PATH")]
    pub runtime_dir: Option<PathBuf>,

    /// Mihomo external-controller URL.
    #[arg(long, env = "MIHOTERM_CONTROLLER", value_name = "URL")]
    pub controller: Option<String>,

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
    /// Attach the TUI to an existing Mihomo external controller.
    Attach,

    /// Print a sanitized one-line controller status without opening the TUI.
    Status,

    /// Diagnose managed-session, profile, connectivity, and inherited-client state.
    Doctor {
        /// Start/recover the managed session and bypass the automatic recovery cooldown.
        #[arg(long)]
        repair: bool,
    },

    /// Probe configured HTTPS targets through one Mihomo proxy.
    Probe {
        /// Mihomo proxy or policy-group name to probe without changing selection.
        #[arg(long, value_name = "NAME")]
        proxy: String,

        /// Probe target name; repeat to select several (defaults to all).
        #[arg(long = "target", value_name = "NAME")]
        targets: Vec<String>,
    },

    /// Start or reuse the background managed proxy without opening the TUI.
    Start {
        /// Managed profile identifier; auto-select or guide setup when omitted.
        profile: Option<String>,

        /// Mihomo executable name or path.
        #[arg(long, env = "MIHOTERM_MIHOMO", value_name = "PATH")]
        mihomo: Option<PathBuf>,
    },

    /// Stop only the background managed proxy owned by MihoTerm.
    Stop,

    /// Run the managed proxy supervisor in the foreground (for systemd).
    Supervise {
        /// Managed profile identifier; restores the last active profile when omitted.
        profile: Option<String>,

        /// Mihomo executable name or path.
        #[arg(long, env = "MIHOTERM_MIHOMO", value_name = "PATH")]
        mihomo: Option<PathBuf>,
    },

    /// Print shell commands for the active managed proxy environment.
    Env {
        /// Return a safe cleanup script instead of an error when no session is active.
        #[arg(long)]
        if_running: bool,
    },

    /// Open an interactive shell whose child processes use the managed proxy.
    Shell {
        /// Managed profile identifier; auto-select or guide setup when omitted.
        #[arg(long)]
        profile: Option<String>,

        /// Mihomo executable name or path.
        #[arg(long, env = "MIHOTERM_MIHOMO", value_name = "PATH")]
        mihomo: Option<PathBuf>,
    },

    /// Run one command with the managed proxy environment.
    Exec {
        /// Managed profile identifier; auto-select or guide setup when omitted.
        #[arg(long)]
        profile: Option<String>,

        /// Mihomo executable name or path.
        #[arg(long, env = "MIHOTERM_MIHOMO", value_name = "PATH")]
        mihomo: Option<PathBuf>,

        /// Command and arguments, specified after `--`.
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<OsString>,
    },

    /// Remove the user-local installation and shell integration.
    Uninstall {
        /// Also remove user configuration, profiles, and runtime state.
        #[arg(long)]
        purge: bool,
    },

    /// Manage validated Mihomo profiles without changing a running instance.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },

    /// Start one isolated Mihomo child from a managed profile and open the TUI.
    Run {
        /// Managed profile identifier; auto-select or guide setup when omitted.
        profile: Option<String>,

        /// Mihomo executable name or path.
        #[arg(long, env = "MIHOTERM_MIHOMO", value_name = "PATH")]
        mihomo: Option<PathBuf>,
    },

    #[command(name = "__runtime-child", hide = true)]
    RuntimeChild {
        #[arg(long, hide = true)]
        parent_pid: u32,

        #[arg(long, hide = true)]
        binary: PathBuf,

        #[arg(long, hide = true)]
        home: PathBuf,

        #[arg(long, hide = true)]
        config: PathBuf,

        #[arg(long, hide = true)]
        test: bool,
    },

    #[command(name = "__session-start", hide = true)]
    SessionStart {
        #[arg(long, hide = true)]
        profile: String,

        #[arg(long, hide = true)]
        profile_path: PathBuf,

        #[arg(long, hide = true)]
        mihomo: PathBuf,

        #[arg(long, hide = true)]
        runtime_root: PathBuf,

        #[arg(long, hide = true)]
        state_root: PathBuf,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ProfileCommand {
    /// Add a named profile from a protected URL file or local YAML file.
    Add {
        /// Stable profile identifier.
        id: String,

        /// Owner-only file containing one HTTPS subscription URL.
        #[arg(
            long,
            value_name = "PATH",
            required_unless_present = "file",
            conflicts_with = "file"
        )]
        url_file: Option<PathBuf>,

        /// Local Mihomo YAML file.
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
    },

    /// Refresh a profile from its stored source.
    Update {
        /// Stable profile identifier.
        id: String,

        /// Apply the validated revision to the managed session without recreating listeners.
        #[arg(long)]
        apply: bool,
    },

    /// Swap the current profile with its previous validated version.
    Rollback {
        /// Stable profile identifier.
        id: String,
    },

    /// List managed profiles without revealing their sources.
    List,

    /// Print the validated YAML path for integration with Mihomo.
    Path {
        /// Stable profile identifier.
        id: String,
    },
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command, ProfileCommand};

    #[test]
    fn no_subcommand_selects_guided_managed_mode() {
        let cli = Cli::try_parse_from(["mihoterm"]).expect("defaults should parse");

        assert!(cli.controller.is_none());
        assert!(cli.command.is_none());
    }

    #[test]
    fn systemd_unit_runs_the_foreground_supervisor_with_restart_policy() {
        let unit = include_str!("../packaging/systemd/mihoterm.service");

        assert!(unit.contains("Type=simple"));
        assert!(unit.contains("ExecStart=%h/.local/bin/mihoterm supervise"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(!unit.contains("RemainAfterExit"));
    }

    #[test]
    fn parses_doctor_repair_and_transactional_profile_apply() {
        let doctor = Cli::try_parse_from(["mihoterm", "doctor", "--repair"])
            .expect("doctor repair should parse");
        assert!(matches!(
            doctor.command,
            Some(Command::Doctor { repair: true })
        ));

        let update = Cli::try_parse_from(["mihoterm", "profile", "update", "biu", "--apply"])
            .expect("profile apply should parse");
        assert!(matches!(
            update.command,
            Some(Command::Profile {
                command: ProfileCommand::Update {
                    id,
                    apply: true
                }
            }) if id == "biu"
        ));
    }

    #[test]
    fn parses_explicit_attach_mode() {
        let cli = Cli::try_parse_from(["mihoterm", "attach"]).expect("attach should parse");

        assert!(matches!(cli.command, Some(Command::Attach)));
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
        assert_eq!(cli.controller.as_deref(), Some("http://127.0.0.1:19090"));
        assert_eq!(cli.timeout_ms, 5_000);
    }

    #[test]
    fn parses_the_headless_probe_command() {
        let cli = Cli::try_parse_from([
            "mihoterm",
            "--controller",
            "http://127.0.0.1:19090",
            "probe",
            "--proxy",
            "Proxy A",
            "--target",
            "Google",
            "--target",
            "openai",
        ])
        .expect("probe should parse");

        assert!(matches!(
            cli.command,
            Some(Command::Probe {
                proxy,
                targets
            }) if proxy == "Proxy A" && targets == ["Google", "openai"]
        ));
    }

    #[test]
    fn probe_requires_an_explicit_proxy() {
        let result = Cli::try_parse_from(["mihoterm", "probe"]);

        assert!(result.is_err());
    }

    #[test]
    fn rejects_an_excessive_refresh_rate() {
        let result = Cli::try_parse_from(["mihoterm", "--refresh-ms", "10"]);

        assert!(result.is_err());
    }

    #[test]
    fn parses_a_profile_url_file_without_exposing_a_url_argument() {
        let cli = Cli::try_parse_from([
            "mihoterm",
            "--state-dir",
            "/tmp/mihoterm-state",
            "profile",
            "add",
            "primary",
            "--url-file",
            "/tmp/subscription.url",
        ])
        .expect("profile add should parse");

        assert!(matches!(
            cli.command,
            Some(Command::Profile {
                command: ProfileCommand::Add {
                    id,
                    url_file: Some(_),
                    file: None,
                },
            }) if id == "primary"
        ));
    }

    #[test]
    fn profile_add_requires_exactly_one_source() {
        let missing = Cli::try_parse_from(["mihoterm", "profile", "add", "primary"]);
        let conflicting = Cli::try_parse_from([
            "mihoterm",
            "profile",
            "add",
            "primary",
            "--url-file",
            "/tmp/subscription.url",
            "--file",
            "/tmp/profile.yaml",
        ]);

        assert!(missing.is_err());
        assert!(conflicting.is_err());
    }

    #[test]
    fn parses_an_explicit_managed_runtime() {
        let cli = Cli::try_parse_from([
            "mihoterm",
            "--runtime-dir",
            "/tmp/mihoterm-runtime",
            "run",
            "primary",
            "--mihomo",
            "/usr/local/bin/mihomo",
        ])
        .expect("managed runtime should parse");

        assert!(matches!(
            cli.command,
            Some(Command::Run { profile: Some(profile), mihomo: Some(mihomo) })
                if profile == "primary"
                    && mihomo.as_path() == std::path::Path::new("/usr/local/bin/mihomo")
        ));
    }

    #[test]
    fn managed_runtime_can_guide_first_run() {
        let cli = Cli::try_parse_from(["mihoterm", "run"]).expect("guided run should parse");

        assert!(matches!(
            cli.command,
            Some(Command::Run {
                profile: None,
                mihomo: None
            })
        ));
    }

    #[test]
    fn parses_background_lifecycle_and_proxied_command() {
        let start =
            Cli::try_parse_from(["mihoterm", "start", "primary"]).expect("start should parse");
        let command = Cli::try_parse_from([
            "mihoterm",
            "exec",
            "--profile",
            "primary",
            "--",
            "curl",
            "https://example.com",
        ])
        .expect("exec should parse");

        assert!(matches!(
            start.command,
            Some(Command::Start {
                profile: Some(profile),
                ..
            }) if profile == "primary"
        ));
        assert!(matches!(
            command.command,
            Some(Command::Exec { command, .. }) if command.len() == 2
        ));
    }
}
