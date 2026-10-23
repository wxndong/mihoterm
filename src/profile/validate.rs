use yaml_serde::Value;

use super::ProfileError;

pub fn validate_profile(bytes: &[u8]) -> Result<(), ProfileError> {
    std::str::from_utf8(bytes).map_err(|_| ProfileError::InvalidYamlEncoding)?;
    let value: Value = yaml_serde::from_slice(bytes).map_err(|_| ProfileError::InvalidYaml)?;
    let Value::Mapping(mapping) = value else {
        return Err(ProfileError::InvalidYamlRoot);
    };

    let has_proxy_content = ["proxies", "proxy-providers", "proxy-groups"]
        .into_iter()
        .any(|key| mapping.contains_key(Value::String(key.into())));
    if !has_proxy_content {
        return Err(ProfileError::MissingProxyContent);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_profile;
    use crate::profile::ProfileError;

    #[test]
    fn accepts_a_mihomo_proxy_document() {
        let yaml = br#"
proxies:
  - name: Proxy A
    type: ss
    server: 192.0.2.1
    port: 443
    cipher: aes-128-gcm
    password: fixture-only
"#;

        assert_eq!(validate_profile(yaml), Ok(()));
    }

    #[test]
    fn rejects_html_that_happens_to_be_utf8() {
        let result = validate_profile(b"<html>not a subscription</html>");

        assert_eq!(result, Err(ProfileError::InvalidYamlRoot));
    }

    #[test]
    fn provider_download_access_message_is_not_accepted_as_a_profile() {
        let response =
            b"Subscription downloads are disabled; enable access on the provider website.";
        assert_eq!(
            validate_profile(response),
            Err(ProfileError::InvalidYamlRoot)
        );
        let message = ProfileError::InvalidYamlRoot.to_string();
        assert!(message.contains("provider"));
        assert!(!message.contains("Subscription downloads are disabled"));
    }

    #[test]
    fn rejects_yaml_without_proxy_content() {
        let result = validate_profile(b"mode: rule\nrules: []\n");

        assert_eq!(result, Err(ProfileError::MissingProxyContent));
    }
}
