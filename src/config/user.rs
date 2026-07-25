use std::{collections::BTreeSet, fs, os::unix::fs::PermissionsExt, path::Path};

use serde::Deserialize;
use thiserror::Error;

use crate::probe::{ProbeError, ProbeTarget};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_CUSTOM_PROBES: usize = 32;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UserConfig {
    #[serde(default)]
    probes: Vec<ProbeConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeConfig {
    name: String,
    url: String,
    #[serde(default = "default_expected")]
    expected: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u32,
}

pub fn load_probe_targets(
    path: &Path,
    require_file: bool,
) -> Result<Vec<ProbeTarget>, UserConfigError> {
    let mut targets = ProbeTarget::built_in();
    if !path.exists() {
        if require_file {
            return Err(UserConfigError::Missing);
        }
        return Ok(targets);
    }

    let metadata = fs::metadata(path).map_err(|_| UserConfigError::Metadata)?;
    if !metadata.is_file() {
        return Err(UserConfigError::NotFile);
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(UserConfigError::InsecurePermissions);
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(UserConfigError::TooLarge);
    }

    let contents = fs::read_to_string(path).map_err(|_| UserConfigError::Read)?;
    let custom = parse_custom_probes(&contents)?;
    targets.extend(custom);
    Ok(targets)
}

fn parse_custom_probes(contents: &str) -> Result<Vec<ProbeTarget>, UserConfigError> {
    let config: UserConfig = toml::from_str(contents).map_err(|_| UserConfigError::Parse)?;
    if config.probes.len() > MAX_CUSTOM_PROBES {
        return Err(UserConfigError::TooManyProbes);
    }

    let mut names = ProbeTarget::built_in()
        .into_iter()
        .map(|target| target.name().to_lowercase())
        .collect::<BTreeSet<_>>();
    config
        .probes
        .into_iter()
        .map(|probe| {
            if !names.insert(probe.name.to_lowercase()) {
                return Err(UserConfigError::DuplicateProbe);
            }
            ProbeTarget::new(probe.name, &probe.url, &probe.expected, probe.timeout_ms)
                .map_err(UserConfigError::InvalidProbe)
        })
        .collect()
}

fn default_expected() -> String {
    "200-299".into()
}

const fn default_timeout_ms() -> u32 {
    5_000
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UserConfigError {
    #[error("the requested configuration file does not exist")]
    Missing,

    #[error("failed to inspect the configuration file")]
    Metadata,

    #[error("the configuration path is not a regular file")]
    NotFile,

    #[error("the configuration file must not be accessible by group or other users")]
    InsecurePermissions,

    #[error("the configuration file exceeds 1 MiB")]
    TooLarge,

    #[error("failed to read the configuration file")]
    Read,

    #[error("the configuration file is invalid TOML")]
    Parse,

    #[error("the configuration contains more than 32 custom probes")]
    TooManyProbes,

    #[error("probe names must be unique, including built-in names")]
    DuplicateProbe,

    #[error("the configuration contains an invalid probe: {0}")]
    InvalidProbe(ProbeError),
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{UserConfigError, load_probe_targets, parse_custom_probes};

    #[test]
    fn parses_a_custom_probe_with_defaults() {
        let probes = parse_custom_probes(
            r#"
                [[probes]]
                name = "Example"
                url = "https://example.com/health"
            "#,
        )
        .expect("config should parse");

        assert_eq!(probes[0].name(), "Example");
        assert_eq!(probes[0].expected(), "200-299");
        assert_eq!(probes[0].timeout_ms(), 5_000);
    }

    #[test]
    fn rejects_a_duplicate_builtin_name() {
        let result = parse_custom_probes(
            r#"
                [[probes]]
                name = "Google"
                url = "https://example.com/health"
            "#,
        );

        assert_eq!(
            result.expect_err("duplicate should fail"),
            UserConfigError::DuplicateProbe
        );
    }

    #[test]
    fn rejects_unknown_configuration_fields() {
        let result = parse_custom_probes("unexpected = true");

        assert_eq!(
            result.expect_err("unknown field should fail"),
            UserConfigError::Parse
        );
    }

    #[test]
    fn loads_an_owner_only_configuration_file() {
        let path = temporary_path();
        fs::write(
            &path,
            r#"
                [[probes]]
                name = "Example"
                url = "https://example.com/health"
                expected = "204"
            "#,
        )
        .expect("fixture should be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("permissions should be set");

        let targets = load_probe_targets(&path, true).expect("configuration should load");
        fs::remove_file(path).expect("fixture should be removed");

        assert_eq!(targets.len(), 4);
        assert_eq!(targets[3].name(), "Example");
    }

    #[test]
    fn rejects_a_group_readable_configuration_file() {
        let path = temporary_path();
        fs::write(&path, "").expect("fixture should be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
            .expect("permissions should be set");

        let result = load_probe_targets(&path, true);
        fs::remove_file(path).expect("fixture should be removed");

        assert_eq!(
            result.expect_err("permissions should fail"),
            UserConfigError::InsecurePermissions
        );
    }

    fn temporary_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mihoterm-config-test-{}-{nonce}",
            std::process::id()
        ))
    }
}
