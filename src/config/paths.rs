use std::{
    env, fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    config_file: PathBuf,
    state_dir: PathBuf,
    runtime_dir: PathBuf,
    runtime_fallback: bool,
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
        let conventional_runtime = PathBuf::from(format!(
            "/run/user/{}/mihoterm",
            rustix::process::geteuid().as_raw()
        ));
        let implicit_runtime = project
            .runtime_dir()
            .map(Path::to_owned)
            .unwrap_or(conventional_runtime);
        let (runtime_dir, runtime_fallback) = match runtime_dir {
            Some(path) => (absolute_path(path)?, false),
            None => choose_implicit_runtime(Some(&implicit_runtime), &state_dir),
        };

        Ok(Self {
            config_file,
            state_dir,
            runtime_dir,
            runtime_fallback,
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

    #[must_use]
    pub const fn runtime_uses_state_fallback(&self) -> bool {
        self.runtime_fallback
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

fn choose_implicit_runtime(candidate: Option<&Path>, state_dir: &Path) -> (PathBuf, bool) {
    let state_runtime = state_dir.join("runtime");
    if let Some(candidate) = candidate.filter(|path| private_session_descriptor_exists(path)) {
        return (candidate.to_owned(), false);
    }
    if private_session_descriptor_exists(&state_runtime) {
        return (state_runtime, true);
    }
    candidate
        .filter(|path| runtime_path_is_usable(path))
        .map_or_else(|| (state_runtime, true), |path| (path.to_owned(), false))
}

fn private_session_descriptor_exists(runtime: &Path) -> bool {
    if !private_owned_writable_directory(runtime) {
        return false;
    }
    fs::symlink_metadata(runtime.join("session.json")).is_ok_and(|metadata| {
        metadata.file_type().is_file()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.permissions().mode() & 0o077 == 0
    })
}

fn runtime_path_is_usable(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if !private_owned_writable_directory(parent) {
        return false;
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => private_owned_writable_metadata(&metadata),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

fn private_owned_writable_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| private_owned_writable_metadata(&metadata))
}

fn private_owned_writable_metadata(metadata: &fs::Metadata) -> bool {
    !metadata.file_type().is_symlink()
        && metadata.is_dir()
        && metadata.uid() == rustix::process::geteuid().as_raw()
        && metadata.permissions().mode() & 0o077 == 0
        && metadata.permissions().mode() & 0o300 == 0o300
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

    use super::{AppPaths, choose_implicit_runtime};

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
        assert!(!paths.runtime_uses_state_fallback());
    }

    #[test]
    fn missing_or_insecure_ephemeral_runtime_falls_back_to_state() {
        let root = temporary_directory();
        let state = root.join("state");
        let missing = root.join("missing/mihoterm");
        assert_eq!(
            choose_implicit_runtime(Some(&missing), &state),
            (state.join("runtime"), true)
        );

        let insecure_parent = root.join("shared-runtime");
        fs::create_dir_all(&insecure_parent).expect("fixture should be created");
        fs::set_permissions(&insecure_parent, fs::Permissions::from_mode(0o755))
            .expect("fixture permissions should be set");
        assert_eq!(
            choose_implicit_runtime(Some(&insecure_parent.join("mihoterm")), &state),
            (state.join("runtime"), true)
        );
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn secure_ephemeral_runtime_is_preferred() {
        let root = temporary_directory();
        fs::create_dir(&root).expect("fixture should be created");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .expect("fixture permissions should be set");
        let candidate = root.join("mihoterm");
        let state = root.join("state");

        assert_eq!(
            choose_implicit_runtime(Some(&candidate), &state),
            (candidate, false)
        );
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn an_existing_durable_session_wins_when_ephemeral_runtime_appears_later() {
        let root = temporary_directory();
        let state = root.join("state");
        let durable = state.join("runtime");
        let ephemeral = root.join("ephemeral");
        fs::create_dir_all(&durable).expect("durable runtime should be created");
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700))
            .expect("state permissions should be set");
        fs::set_permissions(&durable, fs::Permissions::from_mode(0o700))
            .expect("durable permissions should be set");
        fs::create_dir(&ephemeral).expect("ephemeral runtime should be created");
        fs::set_permissions(&ephemeral, fs::Permissions::from_mode(0o700))
            .expect("ephemeral permissions should be set");
        fs::write(durable.join("session.json"), b"fixture")
            .expect("session marker should be written");
        fs::set_permissions(
            durable.join("session.json"),
            fs::Permissions::from_mode(0o600),
        )
        .expect("session permissions should be set");

        assert_eq!(
            choose_implicit_runtime(Some(&ephemeral.join("mihoterm")), &state),
            (durable, true)
        );
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn existing_descriptor_does_not_override_runtime_security_checks() {
        let root = temporary_directory();
        let state = root.join("state");
        let candidate = root.join("candidate");
        fs::create_dir_all(&candidate).expect("candidate should be created");
        fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755))
            .expect("candidate permissions should be set");
        fs::write(candidate.join("session.json"), b"fixture")
            .expect("session marker should be written");
        fs::set_permissions(
            candidate.join("session.json"),
            fs::Permissions::from_mode(0o600),
        )
        .expect("session permissions should be set");

        assert_eq!(
            choose_implicit_runtime(Some(&candidate), &state),
            (state.join("runtime"), true)
        );
        fs::remove_dir_all(root).expect("fixture should be removed");
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
