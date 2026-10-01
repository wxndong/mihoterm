use std::{collections::BTreeSet, path::PathBuf};

use serde::{Deserialize, Serialize};
use yaml_serde::{Mapping, Value};

use crate::probe::ProbeTarget;

use super::{ProfileError, source::MAX_PROFILE_BYTES};

/// Rebuild a bounded fallback from a subscription group's permitted leaf nodes.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FallbackPolicy {
    pub group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_proxy: Option<String>,
    /// Enable bounded managed recovery and require the selected group at activation.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub managed: bool,
    /// Optional local Codex diagnostic database; never read authentication files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_log_db: Option<PathBuf>,
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
        if self.codex_log_db.as_ref().is_some_and(|p| !p.is_absolute()) {
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
        // Managed policies resolve only descendants of the explicitly selected
        // group. Legacy policies retain their direct-leaf semantics.
        let expanded = if self.managed {
            let mut out = Vec::new();
            collect_leaves(&self.group, groups, &leaves, &mut BTreeSet::new(), &mut out)?;
            Some(out)
        } else {
            None
        };
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
        if let Some(expanded) = expanded {
            nodes = expanded;
        }
        if nodes.is_empty() {
            return Err(ProfileError::InvalidFallbackPolicy);
        }
        if let Some(preferred) = &self.preferred_proxy
            && let Some(index) = nodes.iter().position(|node| node == preferred)
        {
            let first = nodes.remove(index);
            nodes.insert(0, first);
        }
        if self.managed {
            for node in &nodes {
                if !members.iter().any(|m| m.as_str() == Some(node)) {
                    members.push(Value::String(node.clone()));
                }
            }
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
            (
                "interval",
                Value::Number(if self.managed { 30 } else { 60 }.into()),
            ),
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

fn collect_leaves(
    name: &str,
    groups: &[Value],
    leaves: &BTreeSet<String>,
    visiting: &mut BTreeSet<String>,
    out: &mut Vec<String>,
) -> Result<(), ProfileError> {
    if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err(ProfileError::InvalidFallbackPolicy);
    }
    if leaves.contains(name) {
        if !out.iter().any(|n| n == name) {
            out.push(name.to_owned());
        }
        return Ok(());
    }
    if matches!(name, "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS") {
        return Ok(());
    }
    if visiting.len() >= 32 || !visiting.insert(name.to_owned()) {
        return Err(ProfileError::InvalidFallbackPolicy);
    }
    let group = groups
        .iter()
        .find(|g| g.get("name").and_then(Value::as_str) == Some(name))
        .ok_or(ProfileError::InvalidFallbackPolicy)?;
    // Provider membership/filters require explicit materialization first. Never
    // widen to include-all or silently omit a dynamic portion of the boundary.
    if group.get("use").is_some()
        || [
            "include-all",
            "include-all-proxies",
            "include-all-providers",
        ]
        .iter()
        .any(|k| group.get(*k).and_then(Value::as_bool) == Some(true))
    {
        return Err(ProfileError::InvalidFallbackPolicy);
    }
    for member in group
        .get("proxies")
        .and_then(Value::as_sequence)
        .ok_or(ProfileError::InvalidFallbackPolicy)?
    {
        collect_leaves(
            member.as_str().ok_or(ProfileError::InvalidFallbackPolicy)?,
            groups,
            leaves,
            visiting,
            out,
        )?;
    }
    visiting.remove(name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::FallbackPolicy;
    use crate::profile::ProfileError;
    use yaml_serde::Value;

    const SOURCE: &str = "proxies:\n- {name: A, type: http}\n- {name: B, type: http}\n- {name: Outside, type: http}\nproxy-groups:\n- {name: AI, type: select, proxies: [Nested, A, B, DIRECT]}\n- {name: Nested, type: select, proxies: [Outside]}\n";

    #[test]
    fn managed_scope_resolves_nested_members_without_expanding_outside_it() {
        let policy = FallbackPolicy {
            group: "AI".into(),
            preferred_proxy: None,
            managed: true,
            codex_log_db: None,
        };
        let source = SOURCE.replace(
            "- {name: Outside, type: http}",
            "- {name: Outside, type: http}\n- {name: Unrelated, type: http}",
        );
        let value: Value =
            yaml_serde::from_slice(&policy.apply(source.as_bytes()).unwrap()).unwrap();
        let nodes = &value["proxy-groups"][2]["proxies"];
        assert_eq!(
            *nodes,
            yaml_serde::from_str::<Value>("[Outside, A, B]").unwrap()
        );
        assert!(
            value["proxy-groups"][0]["proxies"]
                .as_sequence()
                .unwrap()
                .iter()
                .any(|n| n.as_str() == Some("Outside"))
        );
        for bad in [
            SOURCE.replace("[Outside]", "[AI]"),
            SOURCE.replace("[Outside]", "[Missing]"),
            SOURCE.replace(
                "type: select, proxies: [Outside]",
                "type: select, use: [provider], proxies: [Outside]",
            ),
        ] {
            assert_eq!(
                policy.apply(bad.as_bytes()),
                Err(ProfileError::InvalidFallbackPolicy)
            );
        }
    }

    #[test]
    fn fallback_is_bounded_to_direct_group_leaves_and_survives_node_changes() {
        let policy = FallbackPolicy {
            group: "AI".into(),
            preferred_proxy: Some("B".into()),
            managed: false,
            codex_log_db: None,
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
            managed: false,
            codex_log_db: None,
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
