mod paths;
mod user;

use std::{env, fs, os::unix::fs::PermissionsExt, path::Path};

use thiserror::Error;

pub use paths::{AppPaths, PathError};
pub use user::{UserConfigError, load_probe_targets};

const MAX_SECRET_BYTES: u64 = 8 * 1024;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to inspect the controller secret file")]
    SecretMetadata,

    #[error("the controller secret path is not a regular file")]
    SecretNotFile,

    #[error("the controller secret file must not be accessible by group or other users")]
    InsecureSecretPermissions,

    #[error("the controller secret file exceeds {MAX_SECRET_BYTES} bytes")]
    SecretTooLarge,

    #[error("failed to read the controller secret file")]
    SecretRead,

    #[error("the controller secret is not valid UTF-8")]
    SecretEncoding,

    #[error("MIHOTERM_SECRET is not valid UTF-8")]
    SecretEnvironmentEncoding,
}

pub fn load_controller_secret(path: Option<&Path>) -> Result<Option<String>, ConfigError> {
    match path {
        Some(path) => load_secret_file(path),
        None => load_secret_environment(),
    }
}

fn load_secret_file(path: &Path) -> Result<Option<String>, ConfigError> {
    let metadata = fs::metadata(path).map_err(|_| ConfigError::SecretMetadata)?;
    if !metadata.is_file() {
        return Err(ConfigError::SecretNotFile);
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(ConfigError::InsecureSecretPermissions);
    }
    if metadata.len() > MAX_SECRET_BYTES {
        return Err(ConfigError::SecretTooLarge);
    }

    let bytes = fs::read(path).map_err(|_| ConfigError::SecretRead)?;
    let secret = String::from_utf8(bytes).map_err(|_| ConfigError::SecretEncoding)?;
    Ok(normalize_secret(secret))
}

fn load_secret_environment() -> Result<Option<String>, ConfigError> {
    match env::var_os("MIHOTERM_SECRET") {
        Some(value) => value
            .into_string()
            .map(normalize_secret)
            .map_err(|_| ConfigError::SecretEnvironmentEncoding),
        None => Ok(None),
    }
}

fn normalize_secret(mut secret: String) -> Option<String> {
    let trimmed_length = secret.trim_end_matches(['\r', '\n']).len();
    secret.truncate(trimmed_length);
    (!secret.is_empty()).then_some(secret)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{ConfigError, load_secret_file, normalize_secret};

    #[test]
    fn removes_only_line_endings() {
        assert_eq!(
            normalize_secret(" secret value \r\n".into()).as_deref(),
            Some(" secret value ")
        );
    }

    #[test]
    fn treats_an_empty_secret_as_missing() {
        assert_eq!(normalize_secret("\n".into()), None);
    }

    #[test]
    fn rejects_a_group_readable_secret_file() {
        let path = temporary_path();
        fs::write(&path, "test-secret\n").expect("fixture should be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
            .expect("fixture permissions should be set");

        let result = load_secret_file(&path);
        fs::remove_file(path).expect("fixture should be removed");

        assert!(matches!(
            result,
            Err(ConfigError::InsecureSecretPermissions)
        ));
    }

    #[test]
    fn loads_an_owner_only_secret_file() {
        let path = temporary_path();
        fs::write(&path, "test-secret\n").expect("fixture should be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("fixture permissions should be set");

        let secret = load_secret_file(&path).expect("secure fixture should load");
        fs::remove_file(path).expect("fixture should be removed");

        assert_eq!(secret.as_deref(), Some("test-secret"));
    }

    fn temporary_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mihoterm-secret-test-{}-{nonce}",
            std::process::id()
        ))
    }
}
