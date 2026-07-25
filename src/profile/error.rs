use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProfileError {
    #[error("profile ID must match [A-Za-z0-9][A-Za-z0-9_-]{{0,39}}")]
    InvalidId,

    #[error("the profile already exists")]
    AlreadyExists,

    #[error("the profile does not exist")]
    NotFound,

    #[error("the profile has no previous version")]
    NoBackup,

    #[error("another profile operation is already in progress")]
    Busy,

    #[error("failed to initialize profile storage")]
    StorageInitialization,

    #[error("profile storage operation failed")]
    Storage,

    #[error("the source path is not a regular file")]
    SourceNotFile,

    #[error("failed to inspect the profile source")]
    SourceMetadata,

    #[error("the URL source file must not be accessible by group or other users")]
    InsecureUrlFile,

    #[error("the URL source file exceeds 16 KiB")]
    UrlFileTooLarge,

    #[error("failed to read the profile source")]
    SourceRead,

    #[error("the URL source file is not valid UTF-8")]
    UrlEncoding,

    #[error("the subscription source must be a valid HTTPS URL")]
    InvalidSubscriptionUrl,

    #[error("the profile source exceeds 16 MiB")]
    ProfileTooLarge,

    #[error("failed to initialize the profile downloader")]
    DownloadInitialization,

    #[error("the subscription request failed")]
    DownloadRequest,

    #[error("the subscription server returned HTTP {0}")]
    DownloadStatus(u16),

    #[error("the subscription response is not valid UTF-8 YAML")]
    InvalidYamlEncoding,

    #[error("the subscription response is not valid YAML")]
    InvalidYaml,

    #[error("the YAML root must be a mapping")]
    InvalidYamlRoot,

    #[error("the YAML does not contain proxies, proxy-providers, or proxy-groups")]
    MissingProxyContent,

    #[error("the stored profile source descriptor is invalid")]
    InvalidSourceDescriptor,
}
