use std::time::{Duration, Instant};

use crate::mihomo::{
    ApiClient, ApiError, ConnectionMetadata, ConnectionsResponse, ProxiesResponse, RuntimeConfig,
    VersionInfo,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub version: String,
    pub mode: String,
    pub groups: Vec<PolicyGroup>,
    pub connections: Vec<ConnectionRow>,
    pub traffic_rate: Option<TrafficRate>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRow {
    pub id: String,
    pub host: String,
    pub network: String,
    pub chains: Vec<String>,
    pub rule: String,
    pub upload: u64,
    pub download: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrafficRate {
    pub up_bytes_per_sec: u64,
    pub down_bytes_per_sec: u64,
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
            connections: Vec::new(),
            traffic_rate: None,
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

/// Folds live connection state into a snapshot fetched by `fetch_snapshot`.
///
/// The rate is derived from the delta of mihomo's monotonic `uploadTotal` /
/// `downloadTotal` counters between two successful samples. `prev` carries the
/// previous sample plus its timestamp; it is rebaselined to `None` whenever the
/// connection endpoint fails or the primary snapshot errors so the next success
/// never produces a delta measured across a gap.
pub(crate) async fn enrich_with_connections(
    client: &ApiClient,
    mut snapshot: Snapshot,
    prev: &mut Option<(u64, u64, Instant)>,
) -> Snapshot {
    match client.connections().await {
        Ok(response) => {
            let now = Instant::now();
            let rate = prev.map(|(up_prev, down_prev, at)| TrafficRate {
                up_bytes_per_sec: rate_per_sec(up_prev, response.upload_total, now - at),
                down_bytes_per_sec: rate_per_sec(down_prev, response.download_total, now - at),
            });
            snapshot.connections = connection_rows(&response);
            snapshot.traffic_rate = rate;
            *prev = Some((response.upload_total, response.download_total, now));
        }
        Err(_) => *prev = None,
    }
    snapshot
}

fn connection_rows(response: &ConnectionsResponse) -> Vec<ConnectionRow> {
    response
        .connections
        .iter()
        .map(|conn| ConnectionRow {
            id: conn.id.clone(),
            host: connection_host(&conn.metadata),
            network: conn.metadata.network.clone(),
            chains: conn.chains.clone(),
            rule: combined_rule(&conn.rule, &conn.rule_payload),
            upload: conn.upload,
            download: conn.download,
        })
        .collect()
}

fn connection_host(metadata: &ConnectionMetadata) -> String {
    if !metadata.host.is_empty() {
        metadata.host.clone()
    } else if metadata.destination_ip.is_empty() {
        String::new()
    } else if metadata.destination_port.is_empty() {
        metadata.destination_ip.clone()
    } else {
        format!("{}:{}", metadata.destination_ip, metadata.destination_port)
    }
}

fn combined_rule(rule: &str, payload: &str) -> String {
    match (rule.is_empty(), payload.is_empty()) {
        (true, true) => String::new(),
        (true, false) => payload.to_owned(),
        (false, true) => rule.to_owned(),
        (false, false) => format!("{rule} {payload}"),
    }
}

/// Bytes per second represented by a monotonic counter moving from `prev_total`
/// to `now_total` over `elapsed`. Saturates to zero on counter resets and uses a
/// millisecond denominator so sub-second intervals are not silently dropped.
pub(super) fn rate_per_sec(prev_total: u64, now_total: u64, elapsed: Duration) -> u64 {
    let delta = u128::from(now_total.saturating_sub(prev_total));
    let millis = elapsed.as_millis().max(1);
    u64::try_from(delta * 1000 / millis).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::mihomo::{Connection, ConnectionMetadata, ConnectionsResponse};

    use super::{combined_rule, connection_host, connection_rows, rate_per_sec};

    #[test]
    fn rate_uses_elapsed_time_and_saturates_on_reset() {
        // 1500 bytes over 1.5 s = 1000 B/s.
        assert_eq!(rate_per_sec(0, 1500, Duration::from_millis(1500)), 1000);
        // A counter reset (now < prev) reads as zero, never negative.
        assert_eq!(rate_per_sec(5000, 1000, Duration::from_secs(1)), 0);
        // Sub-second intervals are not rounded away to zero.
        assert_eq!(rate_per_sec(0, 500, Duration::from_millis(500)), 1000);
    }

    #[test]
    fn connection_rows_prefer_host_then_destination() {
        let response = ConnectionsResponse {
            upload_total: 0,
            download_total: 0,
            connections: vec![
                Connection {
                    id: "with-host".into(),
                    upload: 10,
                    download: 20,
                    start: String::new(),
                    chains: vec!["Proxy A".into(), "DIRECT".into()],
                    rule: "DOMAIN-SUFFIX".into(),
                    rule_payload: ".com".into(),
                    metadata: ConnectionMetadata {
                        network: "tcp".into(),
                        host: "example.com".into(),
                        ..ConnectionMetadata::default()
                    },
                },
                Connection {
                    id: "ip-only".into(),
                    upload: 0,
                    download: 0,
                    start: String::new(),
                    chains: Vec::new(),
                    rule: String::new(),
                    rule_payload: String::new(),
                    metadata: ConnectionMetadata {
                        network: "udp".into(),
                        destination_ip: "192.0.2.1".into(),
                        destination_port: "443".into(),
                        ..ConnectionMetadata::default()
                    },
                },
            ],
        };

        let rows = connection_rows(&response);

        assert_eq!(rows[0].host, "example.com");
        assert_eq!(rows[0].rule, "DOMAIN-SUFFIX .com");
        assert_eq!(
            rows[0].chains,
            vec!["Proxy A".to_owned(), "DIRECT".to_owned()]
        );
        assert_eq!(rows[0].upload, 10);
        assert_eq!(rows[1].host, "192.0.2.1:443");
        assert!(rows[1].rule.is_empty());
    }

    #[test]
    fn host_falls_back_to_empty_without_destination() {
        let metadata = ConnectionMetadata {
            network: "tcp".into(),
            ..ConnectionMetadata::default()
        };

        assert_eq!(connection_host(&metadata), "");
    }

    #[test]
    fn combined_rule_joins_rule_and_payload() {
        assert_eq!(combined_rule("DOMAIN-SUFFIX", ".com"), "DOMAIN-SUFFIX .com");
        assert_eq!(combined_rule("DIRECT", ""), "DIRECT");
        assert_eq!(combined_rule("", ""), "");
    }
}
