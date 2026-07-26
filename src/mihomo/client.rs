use std::{fmt, sync::Arc, time::Duration};

use reqwest::{Method, RequestBuilder};
use secrecy::{ExposeSecret, SecretString};
use serde::{Serialize, de::DeserializeOwned};
use url::Url;

use crate::probe::ProbeTarget;

use super::{
    error::{ApiError, RequestFailure},
    model::{DelayResponse, OperatingMode, ProxiesResponse, RuntimeConfig, VersionInfo},
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_JSON_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct ApiClient {
    base_url: Url,
    secret: Arc<SecretString>,
    http: reqwest::Client,
}

impl ApiClient {
    pub fn new(controller_url: &str, secret: Option<String>) -> Result<Self, ApiError> {
        Self::with_timeout(controller_url, secret, DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(
        controller_url: &str,
        secret: Option<String>,
        timeout: Duration,
    ) -> Result<Self, ApiError> {
        let base_url = normalize_controller_url(controller_url)?;
        let connect_timeout = timeout.min(DEFAULT_TIMEOUT);
        let http = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(connect_timeout)
            .timeout(timeout)
            .user_agent(concat!("mihoterm/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ApiError::ClientInitialization)?;
        let secret = SecretString::new(secret.unwrap_or_default().into_boxed_str());

        Ok(Self {
            base_url,
            secret: Arc::new(secret),
            http,
        })
    }

    #[must_use]
    pub fn controller_url(&self) -> &Url {
        &self.base_url
    }

    pub async fn version(&self) -> Result<VersionInfo, ApiError> {
        self.get_json("get version", "version").await
    }

    pub async fn configuration(&self) -> Result<RuntimeConfig, ApiError> {
        self.get_json("get configuration", "configs").await
    }

    pub async fn proxies(&self) -> Result<ProxiesResponse, ApiError> {
        self.get_json("get proxies", "proxies").await
    }

    pub async fn select_proxy(&self, group: &str, proxy: &str) -> Result<(), ApiError> {
        #[derive(Serialize)]
        struct Selection<'a> {
            name: &'a str,
        }

        let operation = "select proxy";
        let endpoint = self.endpoint_segments(operation, &["proxies", group])?;
        let request = self.authorize(
            self.http
                .request(Method::PUT, endpoint)
                .json(&Selection { name: proxy }),
        );
        self.send_empty(operation, request).await
    }

    pub async fn set_mode(&self, mode: OperatingMode) -> Result<(), ApiError> {
        #[derive(Serialize)]
        struct ModeSelection<'a> {
            mode: &'a str,
        }

        let operation = "set mode";
        let endpoint = self.endpoint_segments(operation, &["configs"])?;
        let request = self.authorize(self.http.request(Method::PATCH, endpoint).json(
            &ModeSelection {
                mode: mode.as_str(),
            },
        ));
        self.send_empty(operation, request).await
    }

    pub async fn probe_delay(
        &self,
        proxy: &str,
        target: &ProbeTarget,
    ) -> Result<DelayResponse, ApiError> {
        let operation = "probe proxy";
        let mut endpoint = self.endpoint_segments(operation, &["proxies", proxy, "delay"])?;
        endpoint
            .query_pairs_mut()
            .append_pair("url", target.url().as_str())
            .append_pair("timeout", &target.timeout_ms().to_string())
            .append_pair("expected", target.expected());
        let request = self.authorize(self.http.request(Method::GET, endpoint));

        self.send_json(operation, request).await
    }

    async fn get_json<T>(&self, operation: &'static str, path: &str) -> Result<T, ApiError>
    where
        T: DeserializeOwned,
    {
        let endpoint = self
            .base_url
            .join(path)
            .map_err(|_| ApiError::InvalidEndpoint { operation })?;
        let request = self.authorize(self.http.request(Method::GET, endpoint));
        self.send_json(operation, request).await
    }

    async fn send_json<T>(
        &self,
        operation: &'static str,
        request: RequestBuilder,
    ) -> Result<T, ApiError>
    where
        T: DeserializeOwned,
    {
        let response = request.send().await.map_err(|error| ApiError::Request {
            operation,
            kind: classify_request_error(&error),
        })?;

        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::UnexpectedStatus {
                operation,
                status: status.as_u16(),
            });
        }

        if response
            .content_length()
            .is_some_and(|length| length > MAX_JSON_BYTES as u64)
        {
            return Err(ApiError::ResponseTooLarge {
                operation,
                limit_bytes: MAX_JSON_BYTES,
            });
        }

        let body = response.bytes().await.map_err(|error| ApiError::Request {
            operation,
            kind: classify_request_error(&error),
        })?;
        if body.len() > MAX_JSON_BYTES {
            return Err(ApiError::ResponseTooLarge {
                operation,
                limit_bytes: MAX_JSON_BYTES,
            });
        }

        serde_json::from_slice(&body).map_err(|_| ApiError::InvalidResponse { operation })
    }

    async fn send_empty(
        &self,
        operation: &'static str,
        request: RequestBuilder,
    ) -> Result<(), ApiError> {
        let response = request.send().await.map_err(|error| ApiError::Request {
            operation,
            kind: classify_request_error(&error),
        })?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(ApiError::UnexpectedStatus {
                operation,
                status: response.status().as_u16(),
            })
        }
    }

    fn endpoint_segments(
        &self,
        operation: &'static str,
        segments: &[&str],
    ) -> Result<Url, ApiError> {
        let mut endpoint = self.base_url.clone();
        let mut path = endpoint
            .path_segments_mut()
            .map_err(|_| ApiError::InvalidEndpoint { operation })?;
        path.pop_if_empty();
        for segment in segments {
            path.push(segment);
        }
        drop(path);
        Ok(endpoint)
    }

    fn authorize(&self, request: RequestBuilder) -> RequestBuilder {
        if self.secret.expose_secret().is_empty() {
            request
        } else {
            request.bearer_auth(self.secret.expose_secret())
        }
    }
}

impl fmt::Debug for ApiClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiClient")
            .field("base_url", &self.base_url.as_str())
            .field("secret", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

fn normalize_controller_url(controller_url: &str) -> Result<Url, ApiError> {
    let mut url = Url::parse(controller_url).map_err(|_| ApiError::InvalidControllerUrl)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ApiError::UnsupportedControllerScheme);
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::UnsafeControllerUrl);
    }
    if url.cannot_be_a_base() {
        return Err(ApiError::InvalidControllerUrl);
    }

    let mut path = url.path().to_owned();
    if !path.ends_with('/') {
        path.push('/');
        url.set_path(&path);
    }

    Ok(url)
}

fn classify_request_error(error: &reqwest::Error) -> RequestFailure {
    if error.is_timeout() {
        RequestFailure::Timeout
    } else if error.is_connect() {
        RequestFailure::Connect
    } else if error.is_redirect() {
        RequestFailure::Redirect
    } else if error.is_body() {
        RequestFailure::Body
    } else {
        RequestFailure::Other
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiClient, ApiError};

    #[test]
    fn normalizes_a_controller_path() {
        let client = ApiClient::new("http://127.0.0.1:9090/controller", None).expect("valid URL");

        assert_eq!(
            client.controller_url().as_str(),
            "http://127.0.0.1:9090/controller/"
        );
    }

    #[test]
    fn rejects_credentials_in_controller_url() {
        let result = ApiClient::new("http://user:password@127.0.0.1:9090", None);

        assert_eq!(
            result.expect_err("credentials must fail"),
            ApiError::UnsafeControllerUrl
        );
    }

    #[test]
    fn debug_output_redacts_the_secret() {
        let client = ApiClient::new(
            "http://127.0.0.1:9090",
            Some("controller-test-secret".into()),
        )
        .expect("valid client");
        let output = format!("{client:?}");

        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("controller-test-secret"));
    }
}
