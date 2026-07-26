use std::{
    env,
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
}

#[cfg(test)]
mod tests {
    use std::path::Path;

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
}
