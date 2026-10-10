//! Unauthenticated Codex POST probes through private, node-bound loopback listeners.
use super::{ApiError, model::DelayResponse};
use secrecy::{ExposeSecret, SecretString};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::PathBuf,
    time::{Duration, Instant},
};
use yaml_serde::Value;

#[derive(Clone)]
pub(super) struct NodeProbes {
    pub configuration: PathBuf,
    pub username: SecretString,
    pub password: SecretString,
}

impl NodeProbes {
    pub async fn probe(&self, node: &str) -> Result<DelayResponse, ApiError> {
        let metadata = fs::symlink_metadata(&self.configuration)
            .map_err(|_| ApiError::CodexProbeUnavailable)?;
        if !metadata.is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || metadata.len() > 32 * 1024 * 1024
        {
            return Err(ApiError::CodexProbeUnavailable);
        }
        let bytes = fs::read(&self.configuration).map_err(|_| ApiError::CodexProbeUnavailable)?;
        let value: Value =
            yaml_serde::from_slice(&bytes).map_err(|_| ApiError::CodexProbeUnavailable)?;
        let port = value["listeners"]
            .as_sequence()
            .into_iter()
            .flatten()
            .find_map(|entry| {
                (entry["name"]
                    .as_str()
                    .is_some_and(|n| n.starts_with("mihoterm-codex-probe-"))
                    && entry["listen"].as_str() == Some("127.0.0.1")
                    && entry["proxy"].as_str() == Some(node))
                .then(|| {
                    entry["port"]
                        .as_u64()
                        .and_then(|p| u16::try_from(p).ok())
                        .filter(|p| *p != 0)
                })
                .flatten()
            })
            .ok_or(ApiError::CodexProbeUnavailable)?;
        crate::tls::install_default_provider();
        let proxy = reqwest::Proxy::https(format!("http://127.0.0.1:{port}"))
            .map_err(|_| ApiError::ClientInitialization)?
            .basic_auth(self.username.expose_secret(), self.password.expose_secret());
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .proxy(proxy)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("mihoterm/", env!("CARGO_PKG_VERSION")));
        // Honor the same explicitly supplied trust certificates as the managed core.
        for key in ["custom-certifactes", "custom-certificates"] {
            for certificate in value["tls"][key]
                .as_sequence()
                .into_iter()
                .flatten()
                .take(32)
            {
                if let Some(pem) = certificate.as_str() {
                    let certificate = reqwest::Certificate::from_pem(pem.as_bytes())
                        .map_err(|_| ApiError::ClientInitialization)?;
                    builder = builder.add_root_certificate(certificate);
                }
            }
        }
        let http = builder
            .build()
            .map_err(|_| ApiError::ClientInitialization)?;
        let start = Instant::now();
        // No Authorization, cookies, model input or API key is sent.
        let response = http
            .post("https://chatgpt.com/backend-api/codex/responses")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body("")
            .send()
            .await
            .map_err(|_| ApiError::CodexProbeTransport)?;
        classify(
            response.status().as_u16(),
            response
                .headers()
                .get("cf-mitigated")
                .and_then(|v| v.to_str().ok()),
        )?;
        Ok(DelayResponse {
            delay: start.elapsed().as_millis().max(1).min(u128::from(u32::MAX)) as u32,
            mean_delay: None,
        })
    }
}

fn classify(status: u16, mitigation: Option<&str>) -> Result<(), ApiError> {
    if status == 403 && mitigation.is_some_and(|m| m.eq_ignore_ascii_case("challenge")) {
        Err(ApiError::CodexChallenge)
    } else if status == 401 {
        Ok(())
    } else {
        Err(ApiError::UnexpectedStatus {
            operation: "Codex POST probe",
            status,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_endpoint_auth_rejection_is_healthy_and_challenges_are_distinct() {
        assert!(classify(401, None).is_ok());
        assert!(matches!(
            classify(403, Some("challenge")),
            Err(ApiError::CodexChallenge)
        ));
        for status in [200, 204, 302, 403, 405, 429, 500] {
            assert!(classify(status, None).is_err());
        }
    }
}
