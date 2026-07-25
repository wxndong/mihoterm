use crate::mihomo::{ApiClient, ApiError, ProxiesResponse, RuntimeConfig, VersionInfo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub version: String,
    pub mode: String,
    pub groups: Vec<PolicyGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyGroup {
    pub name: String,
    pub kind: String,
    pub selected: Option<String>,
    pub proxies: Vec<ProxyRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyRow {
    pub name: String,
    pub kind: String,
    pub alive: Option<bool>,
    pub delay_ms: Option<u32>,
}

impl Snapshot {
    fn from_api(version: VersionInfo, config: RuntimeConfig, proxies: ProxiesResponse) -> Self {
        let groups = proxies
            .proxies
            .iter()
            .filter(|(_, proxy)| proxy.is_group())
            .map(|(map_name, group)| {
                let name = if group.name.is_empty() {
                    map_name.clone()
                } else {
                    group.name.clone()
                };
                let members = group
                    .all
                    .iter()
                    .map(|member_name| {
                        let member = proxies.proxies.get(member_name);
                        ProxyRow {
                            name: member_name.clone(),
                            kind: member.map_or_else(String::new, |proxy| proxy.kind.clone()),
                            alive: member.and_then(|proxy| proxy.alive),
                            delay_ms: member.and_then(|proxy| proxy.latest_delay_ms()),
                        }
                    })
                    .collect();

                PolicyGroup {
                    name,
                    kind: group.kind.clone(),
                    selected: group.now.clone(),
                    proxies: members,
                }
            })
            .collect();

        Self {
            version: fallback_text(version.version, "unknown"),
            mode: fallback_text(config.mode.unwrap_or_default(), "unknown"),
            groups,
        }
    }
}

fn fallback_text(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.into()
    } else {
        value
    }
}

pub async fn fetch_snapshot(client: &ApiClient) -> Result<Snapshot, ApiError> {
    let (version, config, proxies) =
        tokio::join!(client.version(), client.configuration(), client.proxies());

    Ok(Snapshot::from_api(version?, config?, proxies?))
}
