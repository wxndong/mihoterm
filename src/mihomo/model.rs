use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatingMode {
    Rule,
    Global,
    Direct,
}

impl OperatingMode {
    #[must_use]
    pub fn from_api(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "rule" => Some(Self::Rule),
            "global" => Some(Self::Global),
            "direct" => Some(Self::Direct),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Global => "global",
            Self::Direct => "direct",
        }
    }

    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Rule => Self::Global,
            Self::Global => Self::Direct,
            Self::Direct => Self::Rule,
        }
    }

    #[must_use]
    pub const fn previous(self) -> Self {
        match self {
            Self::Rule => Self::Direct,
            Self::Global => Self::Rule,
            Self::Direct => Self::Global,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rule => "规则 Rule",
            Self::Global => "全局 Global",
            Self::Direct => "直连 Direct",
        }
    }

    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Rule => "按订阅规则和策略组分流",
            Self::Global => "所有流量使用 GLOBAL 中选择的节点",
            Self::Direct => "所有流量直接连接，不使用代理节点",
        }
    }
}

impl std::fmt::Display for OperatingMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct VersionInfo {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub meta: bool,
    #[serde(default)]
    pub premium: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default, rename = "allow-lan")]
    pub allow_lan: Option<bool>,
    #[serde(default, rename = "mixed-port")]
    pub mixed_port: Option<u16>,
    #[serde(default, rename = "log-level")]
    pub log_level: Option<String>,
    #[serde(default)]
    pub ipv6: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProxiesResponse {
    #[serde(default)]
    pub proxies: BTreeMap<String, ProxyInfo>,
}

impl ProxiesResponse {
    #[must_use]
    pub fn groups(&self) -> Vec<&ProxyInfo> {
        self.proxies
            .values()
            .filter(|proxy| proxy.is_group())
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProxyInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub now: Option<String>,
    #[serde(default)]
    pub all: Vec<String>,
    #[serde(default)]
    pub alive: Option<bool>,
    #[serde(default)]
    pub history: Vec<DelaySample>,
    #[serde(default)]
    pub udp: Option<bool>,
}

impl ProxyInfo {
    #[must_use]
    pub fn is_group(&self) -> bool {
        if !self.all.is_empty() {
            return true;
        }

        matches!(
            self.kind.to_ascii_lowercase().as_str(),
            "selector" | "urltest" | "fallback" | "loadbalance" | "relay"
        )
    }

    #[must_use]
    pub fn latest_delay_ms(&self) -> Option<u32> {
        self.history.last().map(|sample| sample.delay)
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DelaySample {
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub delay: u32,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DelayResponse {
    #[serde(default)]
    pub delay: u32,
    #[serde(default, rename = "meanDelay")]
    pub mean_delay: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ConnectionsResponse {
    #[serde(default, rename = "uploadTotal")]
    pub upload_total: u64,
    #[serde(default, rename = "downloadTotal")]
    pub download_total: u64,
    #[serde(default)]
    pub connections: Vec<Connection>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Connection {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub rule: String,
    #[serde(default, rename = "rulePayload")]
    pub rule_payload: String,
    #[serde(default)]
    pub metadata: ConnectionMetadata,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct ConnectionMetadata {
    #[serde(default)]
    pub network: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default, rename = "sourceIP")]
    pub source_ip: String,
    #[serde(default, rename = "destinationIP")]
    pub destination_ip: String,
    #[serde(default, rename = "sourcePort")]
    pub source_port: String,
    #[serde(default, rename = "destinationPort")]
    pub destination_port: String,
    #[serde(default)]
    pub host: String,
    #[serde(default, rename = "processPath")]
    pub process_path: String,
}

#[cfg(test)]
mod tests {
    use super::{ConnectionsResponse, OperatingMode, ProxiesResponse, ProxyInfo};

    #[test]
    fn classifies_groups_without_relying_only_on_members() {
        let group = ProxyInfo {
            name: "Auto".into(),
            kind: "URLTest".into(),
            now: None,
            all: Vec::new(),
            alive: None,
            history: Vec::new(),
            udp: None,
        };

        assert!(group.is_group());
    }

    #[test]
    fn returns_groups_and_latest_delay() {
        let response: ProxiesResponse = serde_json::from_str(
            r#"{
                "proxies": {
                    "Auto": {
                        "name": "Auto",
                        "type": "URLTest",
                        "all": ["Proxy A"]
                    },
                    "Proxy A": {
                        "name": "Proxy A",
                        "type": "Shadowsocks",
                        "history": [{"time": "2026-01-01T00:00:00Z", "delay": 42}]
                    }
                }
            }"#,
        )
        .expect("fixture should parse");

        assert_eq!(response.groups()[0].name, "Auto");
        assert_eq!(response.proxies["Proxy A"].latest_delay_ms(), Some(42));
    }

    #[test]
    fn cycles_supported_operating_modes() {
        assert_eq!(OperatingMode::Rule.next(), OperatingMode::Global);
        assert_eq!(OperatingMode::Global.next(), OperatingMode::Direct);
        assert_eq!(OperatingMode::Direct.next(), OperatingMode::Rule);
        assert_eq!(OperatingMode::Rule.previous(), OperatingMode::Direct);
        assert_eq!(OperatingMode::Global.label(), "全局 Global");
        assert_eq!(OperatingMode::Rule.label(), "规则 Rule");
        assert_eq!(OperatingMode::Direct.label(), "直连 Direct");
        assert_eq!(OperatingMode::from_api("RULE"), Some(OperatingMode::Rule));
    }

    #[test]
    fn connections_response_parses_totals_and_metadata() {
        let response: ConnectionsResponse = serde_json::from_str(
            r#"{
                "uploadTotal": 2048,
                "downloadTotal": 4096,
                "connections": [
                    {
                        "id": "abc-123",
                        "upload": 100,
                        "download": 200,
                        "start": "2026-08-10T00:00:00Z",
                        "chains": ["Proxy A", "DIRECT"],
                        "rule": "DOMAIN-SUFFIX",
                        "rulePayload": ".com",
                        "metadata": {
                            "network": "tcp",
                            "type": "HTTP",
                            "sourceIP": "127.0.0.1",
                            "destinationIP": "192.0.2.1",
                            "sourcePort": "50000",
                            "destinationPort": "443",
                            "host": "example.com",
                            "processPath": "/usr/bin/curl"
                        }
                    }
                ]
            }"#,
        )
        .expect("connections fixture should parse");

        assert_eq!(response.upload_total, 2048);
        assert_eq!(response.download_total, 4096);
        let connection = &response.connections[0];
        assert_eq!(connection.id, "abc-123");
        assert_eq!(
            connection.chains,
            vec!["Proxy A".to_owned(), "DIRECT".to_owned()]
        );
        assert_eq!(connection.rule_payload, ".com");
        assert_eq!(connection.metadata.network, "tcp");
        assert_eq!(connection.metadata.kind, "HTTP");
        assert_eq!(connection.metadata.host, "example.com");
    }

    #[test]
    fn connections_response_tolerates_missing_fields() {
        let response: ConnectionsResponse =
            serde_json::from_str(r#"{"connections": [{"id": "only-id"}]}"#)
                .expect("a minimal payload should parse with defaults");

        assert_eq!(response.upload_total, 0);
        assert!(response.connections[0].chains.is_empty());
        assert!(response.connections[0].metadata.host.is_empty());
    }
}
