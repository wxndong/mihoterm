use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("failed to obtain cryptographically secure random data")]
    Random,

    #[error("failed to reserve isolated loopback ports")]
    PortReservation,

    #[error("the Mihomo executable could not be found")]
    ExecutableNotFound,

    #[error("the Mihomo path is not an executable regular file")]
    ExecutableNotExecutable,

    #[error("failed to initialize the private runtime directory")]
    RuntimeInitialization,

    #[error("failed to seed bundled Mihomo data files")]
    BundledData,

    #[error("failed to read the managed profile")]
    ProfileRead,

    #[error("the managed profile exceeds 16 MiB")]
    ProfileTooLarge,

    #[error("the managed profile is not valid Mihomo YAML")]
    InvalidProfile,

    #[error("failed to serialize the isolated runtime configuration")]
    ConfigurationSerialization,

    #[error("failed to write the private runtime configuration")]
    ConfigurationWrite,

    #[error("failed to launch Mihomo configuration validation")]
    ValidationLaunch,

    #[error("Mihomo configuration validation timed out")]
    ValidationTimeout,

    #[error("Mihomo rejected the isolated configuration (exit code {code:?})")]
    ValidationFailed { code: Option<i32> },

    #[error("failed to launch the isolated Mihomo child")]
    Launch,

    #[error("the managed child wrapper could not initialize")]
    ChildInitialization,

    #[error("the MihoTerm parent exited before the managed child initialized")]
    ParentExited,

    #[error("failed to replace the managed child with Mihomo")]
    ChildExec,

    #[error("failed to inspect the isolated Mihomo child")]
    ProcessStatus,

    #[error("failed to stop the isolated Mihomo child")]
    Stop,

    #[error("the isolated Mihomo controller did not become ready")]
    ControllerUnavailable,

    #[error("failed to initialize the isolated Mihomo API client")]
    ApiInitialization,

    #[error("isolated Mihomo exited unexpectedly (exit code {code:?})")]
    UnexpectedExit { code: Option<i32> },

    #[error("failed to clean the private runtime directory")]
    RuntimeCleanup,

    #[error("failed to lock the managed proxy session")]
    SessionLock,

    #[error("failed to read the managed proxy session")]
    SessionRead,

    #[error("the managed proxy session descriptor is invalid")]
    InvalidSession,

    #[error("failed to write the managed proxy session")]
    SessionWrite,

    #[error("a different managed profile is already running; stop it before switching profiles")]
    SessionProfileConflict,

    #[error("no managed proxy session is running")]
    SessionNotRunning,

    #[error("failed to stop the managed proxy session")]
    SessionStop,
}
