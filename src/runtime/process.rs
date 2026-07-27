use std::{
    env,
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io,
    net::TcpListener,
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Component, Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use rustix::{
    fs::Mode,
    process::{Pid, Signal, getppid, kill_process, set_parent_process_death_signal, setsid, umask},
};
use secrecy::{ExposeSecret, SecretString};

use crate::mihomo::ApiClient;

use super::{RuntimeError, config::build_managed_config};

const MAX_PROFILE_BYTES: u64 = 16 * 1024 * 1024;
const VALIDATION_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(2);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const RUNTIME_CONFIG: &str = "runtime.yaml";
const RUNTIME_LOG: &str = "mihomo.log";
const MAX_BUNDLED_DATA_BYTES: u64 = 64 * 1024 * 1024;
const BUNDLED_DATA_FILES: [&str; 3] = ["geoip.metadb", "geoip.dat", "geosite.dat"];

pub struct ManagedRuntime {
    child: Option<Child>,
    runtime_root: PathBuf,
    runtime_dir: PathBuf,
    controller_url: String,
    mixed_port: u16,
    secret: SecretString,
    proxy_username: SecretString,
    proxy_password: SecretString,
    cleanup_on_drop: bool,
}

impl ManagedRuntime {
    pub async fn start(
        mihomo: &Path,
        profile: &Path,
        runtime_root: &Path,
    ) -> Result<Self, RuntimeError> {
        Self::start_with_lifetime(mihomo, profile, runtime_root, false).await
    }

    pub async fn start_persistent(
        mihomo: &Path,
        profile: &Path,
        runtime_root: &Path,
    ) -> Result<Self, RuntimeError> {
        Self::start_with_lifetime(mihomo, profile, runtime_root, true).await
    }

    async fn start_with_lifetime(
        mihomo: &Path,
        profile: &Path,
        runtime_root: &Path,
        persistent: bool,
    ) -> Result<Self, RuntimeError> {
        let executable = resolve_executable(mihomo)?;
        let runtime_root = prepare_runtime_root(runtime_root)?;
        let runtime_dir = create_runtime_directory(&runtime_root)?;

        let result = Self::start_in(
            executable,
            profile,
            runtime_root.clone(),
            runtime_dir.clone(),
            persistent,
        )
        .await;
        if result.is_err() {
            cleanup_runtime_directory(&runtime_root, &runtime_dir);
        }
        result
    }

    async fn start_in(
        executable: PathBuf,
        profile: &Path,
        runtime_root: PathBuf,
        runtime_dir: PathBuf,
        persistent: bool,
    ) -> Result<Self, RuntimeError> {
        let home = runtime_dir.join("home");
        let temporary = runtime_dir.join("tmp");
        create_private_directory(&home)?;
        create_private_directory(&temporary)?;
        seed_bundled_data(&executable, &home)?;

        let ports = PortReservations::new()?;
        let controller_port = ports.controller_port();
        let mixed_port = ports.mixed_port();
        let controller_url = format!("http://127.0.0.1:{controller_port}");
        let secret = generate_secret()?;
        let proxy_username = SecretString::from(format!("mihoterm-{}", random_hex::<8>()?));
        let proxy_password = generate_secret()?;
        let profile = read_private_profile(profile)?;
        let configuration = build_managed_config(
            &profile,
            controller_port,
            mixed_port,
            secret.expose_secret(),
            proxy_username.expose_secret(),
            proxy_password.expose_secret(),
        )?;
        let configuration_path = runtime_dir.join(RUNTIME_CONFIG);
        write_private(&configuration_path, &configuration)?;
        sync_directory(&runtime_dir)?;

        let log_path = runtime_dir.join(RUNTIME_LOG);
        let launcher = env::current_exe().map_err(|_| RuntimeError::Launch)?;
        validate_configuration(
            &launcher,
            &executable,
            &home,
            &temporary,
            &configuration_path,
            &log_path,
        )
        .await?;

        drop(ports);
        let child = spawn_managed_child(
            &launcher,
            &executable,
            &home,
            &temporary,
            &configuration_path,
            &log_path,
            persistent,
        )?;

        Ok(Self {
            child: Some(child),
            runtime_root,
            runtime_dir,
            controller_url,
            mixed_port,
            secret,
            proxy_username,
            proxy_password,
            cleanup_on_drop: true,
        })
    }

    pub fn api_client(&self, timeout: Duration) -> Result<ApiClient, RuntimeError> {
        ApiClient::with_timeout(
            &self.controller_url,
            Some(self.secret.expose_secret().to_owned()),
            timeout,
        )
        .map_err(|_| RuntimeError::ApiInitialization)
    }

    #[must_use]
    pub fn controller_url(&self) -> &str {
        &self.controller_url
    }

    #[must_use]
    pub const fn mixed_port(&self) -> u16 {
        self.mixed_port
    }

    #[must_use]
    pub fn proxy_username(&self) -> &SecretString {
        &self.proxy_username
    }

    #[must_use]
    pub fn proxy_password(&self) -> &SecretString {
        &self.proxy_password
    }

    #[must_use]
    pub fn controller_secret(&self) -> &SecretString {
        &self.secret
    }

    #[must_use]
    pub fn runtime_directory(&self) -> &Path {
        &self.runtime_dir
    }

    #[must_use]
    pub fn child_id(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    pub fn detach(mut self) {
        self.cleanup_on_drop = false;
        self.child.take();
    }

    pub async fn wait_ready(
        &mut self,
        client: &ApiClient,
        timeout: Duration,
    ) -> Result<(), RuntimeError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.try_wait()? {
                return Err(RuntimeError::UnexpectedExit {
                    code: status.code(),
                });
            }
            if client.version().await.is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(RuntimeError::ControllerUnavailable);
            }
            tokio::time::sleep(PROCESS_POLL_INTERVAL).await;
        }
    }

    pub async fn wait(&mut self) -> Result<ExitStatus, RuntimeError> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            tokio::time::sleep(PROCESS_POLL_INTERVAL).await;
        }
    }

    pub fn stop(&mut self) -> Result<(), RuntimeError> {
        self.stop_child(true)?;
        remove_runtime_directory(&self.runtime_root, &self.runtime_dir)
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>, RuntimeError> {
        let Some(child) = &mut self.child else {
            return Err(RuntimeError::ProcessStatus);
        };
        let status = child.try_wait().map_err(|_| RuntimeError::ProcessStatus)?;
        if status.is_some() {
            self.child.take();
        }
        Ok(status)
    }

    fn stop_child(&mut self, graceful: bool) -> Result<(), RuntimeError> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {}
            Err(_) => {
                self.child = Some(child);
                return Err(RuntimeError::ProcessStatus);
            }
        }

        if graceful {
            if let Some(pid) = i32::try_from(child.id()).ok().and_then(Pid::from_raw) {
                let _ = kill_process(pid, Signal::TERM);
            }
            let deadline = Instant::now() + SHUTDOWN_GRACE_PERIOD;
            while Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(_)) => return Ok(()),
                    Ok(None) => std::thread::sleep(PROCESS_POLL_INTERVAL),
                    Err(_) => {
                        self.child = Some(child);
                        return Err(RuntimeError::ProcessStatus);
                    }
                }
            }
        }

        if child.kill().is_err() && child.try_wait().ok().flatten().is_none() {
            self.child = Some(child);
            return Err(RuntimeError::Stop);
        }
        match child.wait() {
            Ok(_) => Ok(()),
            Err(_) => {
                self.child = Some(child);
                Err(RuntimeError::Stop)
            }
        }
    }
}

#[must_use]
pub fn default_mihomo_executable() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|current| bundled_mihomo_for(&current))
        .unwrap_or_else(|| PathBuf::from("mihomo"))
}

fn bundled_mihomo_for(current_executable: &Path) -> Option<PathBuf> {
    current_executable
        .parent()
        .map(|parent| parent.join("mihomo"))
        .filter(|candidate| is_executable(candidate))
}

impl Drop for ManagedRuntime {
    fn drop(&mut self) {
        if self.cleanup_on_drop && self.stop_child(false).is_ok() {
            cleanup_runtime_directory(&self.runtime_root, &self.runtime_dir);
        }
    }
}

pub fn exec_managed_child(
    parent_pid: u32,
    executable: &Path,
    home: &Path,
    configuration: &Path,
    test: bool,
    detached: bool,
) -> Result<(), RuntimeError> {
    let parent_pid = i32::try_from(parent_pid)
        .ok()
        .and_then(Pid::from_raw)
        .ok_or(RuntimeError::ChildInitialization)?;
    if detached {
        setsid().map_err(|_| RuntimeError::ChildInitialization)?;
    } else {
        set_parent_process_death_signal(Some(Signal::TERM))
            .map_err(|_| RuntimeError::ChildInitialization)?;
        if getppid() != Some(parent_pid) {
            return Err(RuntimeError::ParentExited);
        }
    }
    umask(Mode::RWXG | Mode::RWXO);

    let mut command = Command::new(executable);
    if test {
        command.arg("-t");
    }
    let error = command
        .arg("-d")
        .arg(home)
        .arg("-f")
        .arg(configuration)
        .exec();
    let _ = error;
    Err(RuntimeError::ChildExec)
}

struct PortReservations {
    controller: TcpListener,
    mixed: TcpListener,
}

impl PortReservations {
    fn new() -> Result<Self, RuntimeError> {
        let controller =
            TcpListener::bind(("127.0.0.1", 0)).map_err(|_| RuntimeError::PortReservation)?;
        let mixed =
            TcpListener::bind(("127.0.0.1", 0)).map_err(|_| RuntimeError::PortReservation)?;
        Ok(Self { controller, mixed })
    }

    fn controller_port(&self) -> u16 {
        self.controller
            .local_addr()
            .expect("a bound listener must have a local address")
            .port()
    }

    fn mixed_port(&self) -> u16 {
        self.mixed
            .local_addr()
            .expect("a bound listener must have a local address")
            .port()
    }
}

async fn validate_configuration(
    launcher: &Path,
    executable: &Path,
    home: &Path,
    temporary: &Path,
    configuration: &Path,
    log_path: &Path,
) -> Result<(), RuntimeError> {
    let (stdout, stderr) = private_log(log_path, false)?;
    let mut command = child_wrapper_command(
        launcher,
        executable,
        home,
        temporary,
        configuration,
        true,
        false,
    );
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|_| RuntimeError::ValidationLaunch)?;
    let deadline = Instant::now() + VALIDATION_TIMEOUT;

    loop {
        if let Some(status) = child.try_wait().map_err(|_| RuntimeError::ProcessStatus)? {
            return if status.success() {
                Ok(())
            } else {
                Err(RuntimeError::ValidationFailed {
                    code: status.code(),
                })
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RuntimeError::ValidationTimeout);
        }
        tokio::time::sleep(PROCESS_POLL_INTERVAL).await;
    }
}

fn spawn_managed_child(
    launcher: &Path,
    executable: &Path,
    home: &Path,
    temporary: &Path,
    configuration: &Path,
    log_path: &Path,
    detached: bool,
) -> Result<Child, RuntimeError> {
    let (stdout, stderr) = private_log(log_path, true)?;
    let mut command = child_wrapper_command(
        launcher,
        executable,
        home,
        temporary,
        configuration,
        false,
        detached,
    );
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|_| RuntimeError::Launch)
}

fn child_wrapper_command(
    launcher: &Path,
    executable: &Path,
    home: &Path,
    temporary: &Path,
    configuration: &Path,
    test: bool,
    detached: bool,
) -> Command {
    let mut command = isolated_command(launcher, home, temporary);
    command
        .arg("__runtime-child")
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .arg("--binary")
        .arg(executable)
        .arg("--home")
        .arg(home)
        .arg("--config")
        .arg(configuration);
    if test {
        command.arg("--test");
    }
    if detached {
        command.arg("--detached");
    }
    command
}

fn isolated_command(program: &Path, home: &Path, temporary: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("HOME", home)
        .env("TMPDIR", temporary)
        .env("XDG_CACHE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_DATA_HOME", home)
        .env("XDG_STATE_HOME", home)
        .current_dir(home);
    command
}

fn resolve_executable(value: &Path) -> Result<PathBuf, RuntimeError> {
    let candidate = if value.is_absolute()
        || value
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || value.components().count() > 1
    {
        value.to_owned()
    } else {
        let path = env::var_os("PATH").ok_or(RuntimeError::ExecutableNotFound)?;
        env::split_paths(&path)
            .map(|directory| directory.join(value))
            .find(|candidate| is_executable(candidate))
            .ok_or(RuntimeError::ExecutableNotFound)?
    };
    let candidate = candidate
        .canonicalize()
        .map_err(|_| RuntimeError::ExecutableNotFound)?;
    if is_executable(&candidate) {
        Ok(candidate)
    } else {
        Err(RuntimeError::ExecutableNotExecutable)
    }
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

pub(super) fn prepare_runtime_root(path: &Path) -> Result<PathBuf, RuntimeError> {
    if path.exists() {
        let metadata =
            fs::symlink_metadata(path).map_err(|_| RuntimeError::RuntimeInitialization)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(RuntimeError::RuntimeInitialization);
        }
    } else {
        fs::create_dir_all(path).map_err(|_| RuntimeError::RuntimeInitialization)?;
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| RuntimeError::RuntimeInitialization)?;
    path.canonicalize()
        .map_err(|_| RuntimeError::RuntimeInitialization)
}

fn create_runtime_directory(root: &Path) -> Result<PathBuf, RuntimeError> {
    for _ in 0..16 {
        let suffix = random_hex::<8>()?;
        let path = root.join(format!("run-{}-{suffix}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                    .map_err(|_| RuntimeError::RuntimeInitialization)?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(RuntimeError::RuntimeInitialization),
        }
    }
    Err(RuntimeError::RuntimeInitialization)
}

fn create_private_directory(path: &Path) -> Result<(), RuntimeError> {
    fs::create_dir(path).map_err(|_| RuntimeError::RuntimeInitialization)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| RuntimeError::RuntimeInitialization)
}

fn seed_bundled_data(executable: &Path, home: &Path) -> Result<(), RuntimeError> {
    let Some(bundle_dir) = executable.parent() else {
        return Ok(());
    };

    for name in BUNDLED_DATA_FILES {
        let source = bundle_dir.join(name);
        let metadata = match fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return Err(RuntimeError::BundledData),
        };
        if !metadata.file_type().is_file() || metadata.len() > MAX_BUNDLED_DATA_BYTES {
            return Err(RuntimeError::BundledData);
        }

        let destination = home.join(name);
        fs::copy(&source, &destination).map_err(|_| RuntimeError::BundledData)?;
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o600))
            .map_err(|_| RuntimeError::BundledData)?;
    }

    Ok(())
}

fn read_private_profile(path: &Path) -> Result<Vec<u8>, RuntimeError> {
    let metadata = fs::metadata(path).map_err(|_| RuntimeError::ProfileRead)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(RuntimeError::ProfileRead);
    }
    if metadata.len() > MAX_PROFILE_BYTES {
        return Err(RuntimeError::ProfileTooLarge);
    }
    fs::read(path).map_err(|_| RuntimeError::ProfileRead)
}

fn write_private(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    use std::io::Write as _;

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| RuntimeError::ConfigurationWrite)?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|_| RuntimeError::ConfigurationWrite)
}

fn private_log(path: &Path, append: bool) -> Result<(File, File), RuntimeError> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(!append)
        .append(append)
        .mode(0o600)
        .open(path)
        .map_err(|_| RuntimeError::ConfigurationWrite)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| RuntimeError::ConfigurationWrite)?;
    let copy = file
        .try_clone()
        .map_err(|_| RuntimeError::ConfigurationWrite)?;
    Ok((file, copy))
}

fn generate_secret() -> Result<SecretString, RuntimeError> {
    random_hex::<32>().map(SecretString::from)
}

pub(super) fn random_hex<const N: usize>() -> Result<String, RuntimeError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|_| RuntimeError::Random)?;
    let mut output = String::with_capacity(N * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(output)
}

pub(super) fn sync_directory(path: &Path) -> Result<(), RuntimeError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| RuntimeError::ConfigurationWrite)
}

pub(super) fn remove_runtime_directory(root: &Path, path: &Path) -> Result<(), RuntimeError> {
    if path.parent() != Some(root)
        || !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("run-"))
    {
        return Err(RuntimeError::RuntimeCleanup);
    }
    if path.exists() {
        fs::remove_dir_all(path).map_err(|_| RuntimeError::RuntimeCleanup)?;
    }
    Ok(())
}

fn cleanup_runtime_directory(root: &Path, path: &Path) {
    let _ = remove_runtime_directory(root, path);
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        process::{Child, Command},
        time::{SystemTime, UNIX_EPOCH},
    };

    use secrecy::SecretString;

    use super::{
        ManagedRuntime, PortReservations, bundled_mihomo_for, prepare_runtime_root,
        read_private_profile, resolve_executable, seed_bundled_data,
    };

    #[test]
    fn loopback_port_reservations_are_distinct() {
        let ports = PortReservations::new().expect("ports should be reserved");

        assert_ne!(ports.controller_port(), ports.mixed_port());
    }

    #[test]
    fn executable_resolution_rejects_a_non_executable_file() {
        let base = temporary_directory();
        let path = base.join("mihomo");
        fs::create_dir(&base).expect("directory should be created");
        fs::write(&path, "").expect("file should be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("permissions should be set");

        assert!(resolve_executable(&path).is_err());
        fs::remove_dir_all(base).expect("directory should be removed");
    }

    #[test]
    fn portable_bundle_prefers_an_executable_sibling() {
        let base = temporary_directory();
        let launcher = base.join("mihoterm");
        let core = base.join("mihomo");
        fs::create_dir(&base).expect("directory should be created");
        fs::write(&core, "").expect("core fixture should be written");
        fs::set_permissions(&core, fs::Permissions::from_mode(0o700))
            .expect("permissions should be set");

        assert_eq!(bundled_mihomo_for(&launcher), Some(core));
        fs::remove_dir_all(base).expect("directory should be removed");
    }

    #[test]
    fn bundled_data_is_copied_into_the_private_runtime_home() {
        let base = temporary_directory();
        let bundle = base.join("bundle");
        let home = base.join("home");
        fs::create_dir_all(&bundle).expect("bundle should be created");
        fs::create_dir(&home).expect("home should be created");
        fs::write(bundle.join("geoip.metadb"), b"fixture").expect("data should be written");

        seed_bundled_data(&bundle.join("mihomo"), &home).expect("data should be seeded");

        let destination = home.join("geoip.metadb");
        assert_eq!(
            fs::read(&destination).expect("seeded data should be readable"),
            b"fixture"
        );
        assert_eq!(
            fs::metadata(destination)
                .expect("seeded data metadata should be readable")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_dir_all(base).expect("directory should be removed");
    }

    #[test]
    fn bundled_data_rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let base = temporary_directory();
        let bundle = base.join("bundle");
        let home = base.join("home");
        fs::create_dir_all(&bundle).expect("bundle should be created");
        fs::create_dir(&home).expect("home should be created");
        symlink("/dev/null", bundle.join("geoip.metadb")).expect("link should be created");

        assert!(seed_bundled_data(&bundle.join("mihomo"), &home).is_err());
        fs::remove_dir_all(base).expect("directory should be removed");
    }

    #[test]
    fn runtime_root_rejects_a_symbolic_link() {
        use std::os::unix::fs::symlink;

        let base = temporary_directory();
        let target = base.join("target");
        let link = base.join("runtime");
        fs::create_dir(&base).expect("directory should be created");
        fs::create_dir(&target).expect("target should be created");
        symlink(&target, &link).expect("link should be created");

        assert!(prepare_runtime_root(&link).is_err());
        fs::remove_dir_all(base).expect("directory should be removed");
    }

    #[test]
    fn managed_profiles_must_remain_owner_only() {
        let base = temporary_directory();
        let path = base.join("profile.yaml");
        fs::create_dir(&base).expect("directory should be created");
        fs::write(&path, "proxies: []\n").expect("profile should be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
            .expect("permissions should be set");

        assert!(read_private_profile(&path).is_err());
        fs::remove_dir_all(base).expect("directory should be removed");
    }

    #[test]
    fn managed_stop_leaves_unrelated_process_running() {
        let base = temporary_directory();
        let root = base.join("runtime");
        let run = root.join("run-test-owned");
        fs::create_dir_all(&run).expect("runtime directory should be created");
        let owned = Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("owned fixture should start");
        let mut unrelated = ChildGuard(
            Command::new("/bin/sleep")
                .arg("30")
                .spawn()
                .expect("unrelated fixture should start"),
        );
        let mut runtime = ManagedRuntime {
            child: Some(owned),
            runtime_root: root.clone(),
            runtime_dir: run.clone(),
            controller_url: "http://127.0.0.1:1".into(),
            mixed_port: 2,
            secret: SecretString::from("fixture-only"),
            proxy_username: SecretString::from("fixture-user"),
            proxy_password: SecretString::from("fixture-password"),
            cleanup_on_drop: true,
        };

        runtime.stop().expect("managed runtime should stop");

        assert!(!run.exists());
        assert!(
            unrelated
                .0
                .try_wait()
                .expect("unrelated status should be readable")
                .is_none()
        );
        fs::remove_dir_all(base).expect("directory should be removed");
    }

    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn temporary_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mihoterm-runtime-test-{}-{nonce}",
            std::process::id()
        ))
    }
}
