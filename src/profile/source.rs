use std::{
    fmt, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use futures_util::StreamExt;
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use url::Url;

use super::ProfileError;

pub(crate) const MAX_PROFILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_URL_FILE_BYTES: u64 = 16 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSource {
    source: SourceKind,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "kebab-case")]
enum SourceKind {
    Https { url: String },
    LocalFile { path: PathBuf },
}

impl ProfileSource {
    pub fn from_url_file(path: &Path) -> Result<Self, ProfileError> {
        let metadata = fs::metadata(path).map_err(|_| ProfileError::SourceMetadata)?;
        if !metadata.is_file() {
            return Err(ProfileError::SourceNotFile);
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ProfileError::InsecureUrlFile);
        }
        if metadata.len() > MAX_URL_FILE_BYTES {
            return Err(ProfileError::UrlFileTooLarge);
        }

        let bytes = fs::read(path).map_err(|_| ProfileError::SourceRead)?;
        let mut value = String::from_utf8(bytes).map_err(|_| ProfileError::UrlEncoding)?;
        value.truncate(value.trim_end_matches(['\r', '\n']).len());
        Self::from_url(SecretString::from(value))
    }

    pub fn from_url(value: SecretString) -> Result<Self, ProfileError> {
        let url = validate_subscription_url(value.expose_secret())?;

        Ok(Self {
            source: SourceKind::Https {
                url: url.to_string(),
            },
        })
    }

    pub fn from_local_file(path: &Path) -> Result<Self, ProfileError> {
        let path = path.canonicalize().map_err(|_| ProfileError::SourceRead)?;
        let metadata = fs::metadata(&path).map_err(|_| ProfileError::SourceMetadata)?;
        if !metadata.is_file() {
            return Err(ProfileError::SourceNotFile);
        }
        if metadata.len() > MAX_PROFILE_BYTES as u64 {
            return Err(ProfileError::ProfileTooLarge);
        }

        Ok(Self {
            source: SourceKind::LocalFile { path },
        })
    }

    #[must_use]
    pub fn kind(&self) -> &'static str {
        match &self.source {
            SourceKind::Https { .. } => "https",
            SourceKind::LocalFile { .. } => "local-file",
        }
    }

    pub(crate) fn descriptor(&self) -> Result<String, ProfileError> {
        toml::to_string(self).map_err(|_| ProfileError::Storage)
    }

    pub(crate) fn from_descriptor(value: &str) -> Result<Self, ProfileError> {
        let source: Self =
            toml::from_str(value).map_err(|_| ProfileError::InvalidSourceDescriptor)?;
        source.revalidate()?;
        Ok(source)
    }

    pub(crate) async fn load(&self, client: &Client) -> Result<Vec<u8>, ProfileError> {
        match &self.source {
            SourceKind::Https { url } => load_https(client, url).await,
            SourceKind::LocalFile { path } => load_local(path),
        }
    }

    fn revalidate(&self) -> Result<(), ProfileError> {
        match &self.source {
            SourceKind::Https { url } => {
                validate_subscription_url(url)?;
                Ok(())
            }
            SourceKind::LocalFile { path } => {
                if path.is_absolute() {
                    Ok(())
                } else {
                    Err(ProfileError::InvalidSourceDescriptor)
                }
            }
        }
    }
}

impl fmt::Debug for ProfileSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileSource")
            .field("kind", &self.kind())
            .field("value", &"[REDACTED]")
            .finish()
    }
}

fn validate_subscription_url(value: &str) -> Result<Url, ProfileError> {
    let url = Url::parse(value).map_err(|_| ProfileError::InvalidSubscriptionUrl)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(ProfileError::InvalidSubscriptionUrl);
    }
    Ok(url)
}

fn load_local(path: &Path) -> Result<Vec<u8>, ProfileError> {
    let metadata = fs::metadata(path).map_err(|_| ProfileError::SourceMetadata)?;
    if !metadata.is_file() {
        return Err(ProfileError::SourceNotFile);
    }
    if metadata.len() > MAX_PROFILE_BYTES as u64 {
        return Err(ProfileError::ProfileTooLarge);
    }
    fs::read(path).map_err(|_| ProfileError::SourceRead)
}

async fn load_https(client: &Client, url: &str) -> Result<Vec<u8>, ProfileError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| ProfileError::DownloadRequest)?;
    if !response.status().is_success() {
        return Err(ProfileError::DownloadStatus(response.status().as_u16()));
    }
    if response.url().scheme() != "https" {
        return Err(ProfileError::InvalidSubscriptionUrl);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROFILE_BYTES as u64)
    {
        return Err(ProfileError::ProfileTooLarge);
    }

    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ProfileError::DownloadRequest)?;
        if bytes.len() + chunk.len() > MAX_PROFILE_BYTES {
            return Err(ProfileError::ProfileTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::ProfileSource;
    use crate::profile::ProfileError;

    #[test]
    fn url_debug_output_is_redacted() {
        let path = temporary_path();
        fs::write(&path, "https://example.com/sub?credential=private\n")
            .expect("fixture should be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("permissions should be set");

        let source = ProfileSource::from_url_file(&path).expect("URL should load");
        fs::remove_file(path).expect("fixture should be removed");
        let output = format!("{source:?}");

        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("private"));
    }

    #[test]
    fn url_file_must_be_owner_only() {
        let path = temporary_path();
        fs::write(&path, "https://example.com/sub\n").expect("fixture should be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
            .expect("permissions should be set");

        let result = ProfileSource::from_url_file(&path);
        fs::remove_file(path).expect("fixture should be removed");

        assert_eq!(
            result.expect_err("permissions should fail"),
            ProfileError::InsecureUrlFile
        );
    }

    fn temporary_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mihoterm-url-source-test-{}-{nonce}",
            std::process::id()
        ))
    }
}
