use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    config_file: PathBuf,
    state_dir: PathBuf,
    runtime_dir: PathBuf,
}

impl AppPaths {
    pub fn discover(
        config_file: Option<&Path>,
        state_dir: Option<&Path>,
        runtime_dir: Option<&Path>,
    ) -> Result<Self, PathError> {
        let project =
            ProjectDirs::from("", "", "mihoterm").ok_or(PathError::HomeDirectoryUnavailable)?;
        let config_file = match config_file {
            Some(path) => absolute_path(path)?,
            None => project.config_dir().join("config.toml"),
        };
        let state_dir = match state_dir {
            Some(path) => absolute_path(path)?,
            None => project
                .state_dir()
                .unwrap_or_else(|| project.data_local_dir())
                .to_owned(),
        };
        let runtime_dir = match runtime_dir {
            Some(path) => absolute_path(path)?,
            None => project
                .runtime_dir()
                .map_or_else(|| state_dir.join("runtime"), Path::to_owned),
        };

        Ok(Self {
            config_file,
            state_dir,
            runtime_dir,
        })
    }

    #[must_use]
    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    #[must_use]
    pub fn profiles_dir(&self) -> PathBuf {
        self.state_dir.join("profiles")
    }

    #[must_use]
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    pub fn prepare_private_state(&self) -> Result<(), PathError> {
        if self.state_dir.exists() {
            let metadata = fs::symlink_metadata(&self.state_dir)
                .map_err(|_| PathError::StateInitialization)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PathError::StateInitialization);
            }
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(PathError::InsecureStateDirectory);
            }
        } else {
            fs::create_dir_all(&self.state_dir).map_err(|_| PathError::StateInitialization)?;
            fs::set_permissions(&self.state_dir, fs::Permissions::from_mode(0o700))
                .map_err(|_| PathError::StateInitialization)?;
        }
        Ok(())
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf, PathError> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|_| PathError::CurrentDirectoryUnavailable)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathError {
    #[error("the user configuration directory is unavailable")]
    HomeDirectoryUnavailable,

    #[error("the current directory is unavailable")]
    CurrentDirectoryUnavailable,

    #[error("failed to initialize the private state directory")]
    StateInitialization,

    #[error("the state directory must not be accessible by group or other users")]
    InsecureStateDirectory,
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::AppPaths;

    #[test]
    fn explicit_state_directory_controls_profile_storage() {
        let paths = AppPaths::discover(
            Some(Path::new("/tmp/mihoterm-test-config.toml")),
            Some(Path::new("/tmp/mihoterm-test-state")),
            Some(Path::new("/tmp/mihoterm-test-runtime")),
        )
        .expect("explicit paths should resolve");

        assert_eq!(
            paths.profiles_dir(),
            Path::new("/tmp/mihoterm-test-state/profiles")
        );
        assert_eq!(paths.runtime_dir(), Path::new("/tmp/mihoterm-test-runtime"));
    }

    #[test]
    fn state_directory_is_owner_only() {
        let state = temporary_directory();
        let paths = AppPaths::discover(None, Some(&state), None).expect("paths should resolve");

        paths
            .prepare_private_state()
            .expect("state should be initialized");

        assert_eq!(
            fs::metadata(&state)
                .expect("state should exist")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        fs::remove_dir_all(state).expect("state should be removed");
    }

    #[test]
    fn existing_shared_state_directory_is_rejected_without_chmod() {
        let state = temporary_directory();
        fs::create_dir(&state).expect("state should be created");
        fs::set_permissions(&state, fs::Permissions::from_mode(0o755))
            .expect("fixture permissions should be set");
        let paths = AppPaths::discover(None, Some(&state), None).expect("paths should resolve");

        assert_eq!(
            paths.prepare_private_state(),
            Err(super::PathError::InsecureStateDirectory)
        );
        assert_eq!(
            fs::metadata(&state)
                .expect("state should exist")
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        fs::remove_dir_all(state).expect("state should be removed");
    }

    fn temporary_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mihoterm-state-test-{}-{nonce}",
            std::process::id()
        ))
    }
}
