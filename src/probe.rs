use std::fmt;

use thiserror::Error;
use url::Url;

const MIN_TIMEOUT_MS: u32 = 100;
const MAX_TIMEOUT_MS: u32 = 60_000;

#[derive(Clone, PartialEq, Eq)]
pub struct ProbeTarget {
    name: String,
    url: Url,
    expected: String,
    timeout_ms: u32,
}

impl ProbeTarget {
    pub fn new(
        name: impl Into<String>,
        url: &str,
        expected: &str,
        timeout_ms: u32,
    ) -> Result<Self, ProbeError> {
        let name = name.into();
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
            return Err(ProbeError::InvalidName);
        }

        let url = Url::parse(url).map_err(|_| ProbeError::InvalidUrl)?;
        if url.scheme() != "https" {
            return Err(ProbeError::InsecureUrl);
        }
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return Err(ProbeError::UnsafeUrl);
        }
        if !valid_expected_status(expected) {
            return Err(ProbeError::InvalidExpectedStatus);
        }
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&timeout_ms) {
            return Err(ProbeError::InvalidTimeout);
        }

        Ok(Self {
            name: name.into(),
            url,
            expected: expected.into(),
            timeout_ms,
        })
    }

    #[must_use]
    pub fn built_in() -> Vec<Self> {
        [
            ("Google", "https://www.gstatic.com/generate_204", "204"),
            ("OpenAI / Codex", "https://api.openai.com/v1/models", "401"),
            ("GitHub", "https://github.com/robots.txt", "200"),
        ]
        .into_iter()
        .map(|(name, url, expected)| {
            Self::new(name, url, expected, 5_000).expect("built-in probe targets must remain valid")
        })
        .collect()
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn expected(&self) -> &str {
        &self.expected
    }

    #[must_use]
    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }
}

impl fmt::Debug for ProbeTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProbeTarget")
            .field("name", &self.name)
            .field("url", &"[REDACTED]")
            .field("expected", &self.expected)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProbeError {
    #[error("probe name must contain 1 to 40 printable characters")]
    InvalidName,

    #[error("probe URL is invalid")]
    InvalidUrl,

    #[error("probe URL must use HTTPS")]
    InsecureUrl,

    #[error("probe URL must not contain credentials or a fragment")]
    UnsafeUrl,

    #[error("expected status must contain HTTP codes or ranges")]
    InvalidExpectedStatus,

    #[error("probe timeout must be between 100 and 60000 milliseconds")]
    InvalidTimeout,
}

fn valid_expected_status(value: &str) -> bool {
    !value.is_empty()
        && value.split('/').all(|part| {
            if let Some((start, end)) = part.split_once('-') {
                valid_status(start)
                    && valid_status(end)
                    && start.parse::<u16>().expect("validated status")
                        <= end.parse::<u16>().expect("validated status")
            } else {
                valid_status(part)
            }
        })
}

fn valid_status(value: &str) -> bool {
    value.len() == 3
        && value
            .parse::<u16>()
            .is_ok_and(|status| (100..=599).contains(&status))
}

#[cfg(test)]
mod tests {
    use super::{ProbeError, ProbeTarget, valid_expected_status};

    #[test]
    fn built_in_targets_have_distinct_expected_statuses() {
        let targets = ProbeTarget::built_in();

        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0].expected(), "204");
        assert_eq!(targets[1].expected(), "401");
        assert_eq!(targets[2].expected(), "200");
    }

    #[test]
    fn supports_status_lists_and_ranges() {
        assert!(valid_expected_status("200/204"));
        assert!(valid_expected_status("200-299"));
        assert!(!valid_expected_status("299-200"));
        assert!(!valid_expected_status("ok"));
    }

    #[test]
    fn requires_https_for_custom_targets() {
        let result = ProbeTarget::new("Local", "http://127.0.0.1/health", "204", 1_000);

        assert_eq!(
            result.expect_err("HTTP should fail"),
            ProbeError::InsecureUrl
        );
    }

    #[test]
    fn debug_output_redacts_the_url() {
        let target = ProbeTarget::new(
            "Private",
            "https://example.com/private?credential=do-not-render",
            "200",
            1_000,
        )
        .expect("target should be valid");
        let output = format!("{target:?}");

        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("do-not-render"));
    }
}
