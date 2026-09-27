use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use yaml_serde::{Mapping, Value};

use crate::probe::ProbeTarget;

use super::{ProfileError, source::MAX_PROFILE_BYTES};

/// Rebuild a bounded fallback from a subscription group's direct leaf nodes.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct FallbackPolicy {
    pub group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_proxy: Option<String>,
}

impl FallbackPolicy {
    pub fn validate(&self) -> Result<(), ProfileError> {
        let valid_name = |name: &str| {
            !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
        };
        if !valid_name(&self.group)
            || self
                .preferred_proxy
                .as_deref()
                .is_some_and(|name| !valid_name(name))
        {
            return Err(ProfileError::InvalidFallbackPolicy);
        }
        Ok(())
    }

    pub fn apply(&self, contents: &[u8]) -> Result<Vec<u8>, ProfileError> {
        self.validate()?;
        let mut value: Value =
            yaml_serde::from_slice(contents).map_err(|_| ProfileError::InvalidYaml)?;
        let root = value
            .as_mapping_mut()
            .ok_or(ProfileError::InvalidYamlRoot)?;
        let leaves = root
            .get("proxies")
            .and_then(Value::as_sequence)
            .ok_or(ProfileError::InvalidFallbackPolicy)?
            .iter()
            .filter(|proxy| {
                !matches!(
                    proxy.get("type").and_then(Value::as_str),
                    Some("direct" | "reject")
                )
            })
            .filter_map(|proxy| proxy.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let groups = root
            .get_mut("proxy-groups")
            .and_then(Value::as_sequence_mut)
            .ok_or(ProfileError::InvalidFallbackPolicy)?;
        let fallback_name = format!("{} Auto", self.group);
        if leaves.contains(&fallback_name)
            || groups
                .iter()
                .any(|group| group.get("name").and_then(Value::as_str) == Some(&fallback_name))
        {
            return Err(ProfileError::InvalidFallbackPolicy);
        }
        let group = groups
            .iter_mut()
            .find(|group| group.get("name").and_then(Value::as_str) == Some(&self.group))
            .ok_or(ProfileError::InvalidFallbackPolicy)?;
        if group.get("type").and_then(Value::as_str) != Some("select") {
            return Err(ProfileError::InvalidFallbackPolicy);
        }
        let members = group
            .get_mut("proxies")
            .and_then(Value::as_sequence_mut)
            .ok_or(ProfileError::InvalidFallbackPolicy)?;
        let mut seen = BTreeSet::new();
        let mut nodes = members
            .iter()
            .filter_map(Value::as_str)
            .filter(|name| leaves.contains(*name) && seen.insert((*name).to_owned()))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if nodes.is_empty() {
            return Err(ProfileError::InvalidFallbackPolicy);
        }
        if let Some(preferred) = &self.preferred_proxy
            && let Some(index) = nodes.iter().position(|node| node == preferred)
        {
            let first = nodes.remove(index);
            nodes.insert(0, first);
        }
        members.insert(0, Value::String(fallback_name.clone()));
        let target = ProbeTarget::built_in()
            .into_iter()
            .find(|target| target.name() == "Codex")
            .expect("the Codex health target must exist");
        let mut fallback = Mapping::new();
        for (key, value) in [
            ("name", Value::String(fallback_name)),
            ("type", Value::String("fallback".into())),
            (
                "proxies",
                Value::Sequence(nodes.into_iter().map(Value::String).collect()),
            ),
            ("url", Value::String(target.url().as_str().to_owned())),
            (
                "expected-status",
                Value::String(target.expected().to_owned()),
            ),
            ("interval", Value::Number(60.into())),
            ("timeout", Value::Number(6_000.into())),
            ("lazy", Value::Bool(false)),
            ("max-failed-times", Value::Number(2.into())),
        ] {
            fallback.insert(Value::String(key.into()), value);
        }
        groups.push(Value::Mapping(fallback));
        let output = yaml_serde::to_string(&value)
            .map_err(|_| ProfileError::InvalidYaml)?
            .into_bytes();
        if output.len() > MAX_PROFILE_BYTES {
            return Err(ProfileError::ProfileTooLarge);
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::FallbackPolicy;
    use crate::profile::ProfileError;
    use yaml_serde::Value;

    const SOURCE: &str = "proxies:\n- {name: A, type: http}\n- {name: B, type: http}\n- {name: Outside, type: http}\nproxy-groups:\n- {name: AI, type: select, proxies: [Nested, A, B, DIRECT]}\n- {name: Nested, type: select, proxies: [Outside]}\n";

    #[test]
    fn fallback_is_bounded_to_direct_group_leaves_and_survives_node_changes() {
        let policy = FallbackPolicy {
            group: "AI".into(),
            preferred_proxy: Some("B".into()),
        };
        let derived = policy
            .apply(SOURCE.as_bytes())
            .expect("valid group should transform");
        let value: Value = yaml_serde::from_slice(&derived).expect("derived YAML should parse");
        assert_eq!(
            value["proxy-groups"][0]["proxies"][0].as_str(),
            Some("AI Auto")
        );
        let fallback = &value["proxy-groups"][2];
        assert_eq!(
            fallback["proxies"],
            yaml_serde::from_str::<Value>("[B, A]").unwrap()
        );
        assert_eq!(fallback["expected-status"].as_str(), Some("401/405"));
        let changed = SOURCE.replace("B", "New");
        let updated = policy
            .apply(changed.as_bytes())
            .expect("removed preference should not block refresh");
        let updated: Value = yaml_serde::from_slice(&updated).unwrap();
        assert_eq!(
            updated["proxy-groups"][2]["proxies"],
            yaml_serde::from_str::<Value>("[A, New]").unwrap()
        );
        assert_eq!(
            policy.apply(&derived),
            Err(ProfileError::InvalidFallbackPolicy)
        );
    }

    #[test]
    fn missing_groups_empty_nodes_and_name_collisions_fail_closed() {
        let policy = FallbackPolicy {
            group: "AI".into(),
            preferred_proxy: None,
        };
        for source in [
            SOURCE.replace("name: AI,", "name: Renamed,"),
            SOURCE.replace("[Nested, A, B, DIRECT]", "[Nested, DIRECT]"),
            SOURCE.replace("name: Outside,", "name: AI Auto,"),
        ] {
            assert_eq!(
                policy.apply(source.as_bytes()),
                Err(ProfileError::InvalidFallbackPolicy)
            );
        }
    }
}
