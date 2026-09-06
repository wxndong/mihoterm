use yaml_serde::{Mapping, Number, Value};

use crate::mihomo::OperatingMode;

use super::RuntimeError;

pub(super) fn build_managed_config(
    profile: &[u8],
    controller_port: u16,
    mixed_port: u16,
    secret: &str,
    proxy_username: &str,
    proxy_password: &str,
    mode: OperatingMode,
) -> Result<Vec<u8>, RuntimeError> {
    std::str::from_utf8(profile).map_err(|_| RuntimeError::InvalidProfile)?;
    let mut value: Value =
        yaml_serde::from_slice(profile).map_err(|_| RuntimeError::InvalidProfile)?;
    let Value::Mapping(root) = &mut value else {
        return Err(RuntimeError::InvalidProfile);
    };
    if !["proxies", "proxy-providers", "proxy-groups"]
        .into_iter()
        .any(|key| root.contains_key(string(key)))
    {
        return Err(RuntimeError::InvalidProfile);
    }

    set_number(root, "port", 0);
    set_number(root, "socks-port", 0);
    set_number(root, "redir-port", 0);
    set_number(root, "tproxy-port", 0);
    set_number(root, "mixed-port", u64::from(mixed_port));
    set_string(root, "mode", mode.as_str());
    set_bool(root, "allow-lan", false);
    set_bool(root, "tcp-concurrent", true);
    set_string(root, "bind-address", "127.0.0.1");
    set_sequence(
        root,
        "authentication",
        vec![Value::String(format!("{proxy_username}:{proxy_password}"))],
    );

    set_string(
        root,
        "external-controller",
        &format!("127.0.0.1:{controller_port}"),
    );
    set_number(root, "external-controller-routing-mark", 0);
    set_string(root, "external-controller-tls", "");
    set_string(root, "external-controller-unix", "");
    set_string(root, "external-controller-pipe", "");
    set_string(root, "external-doh-server", "");
    set_string(root, "external-ui", "");
    set_string(root, "external-ui-url", "");
    set_string(root, "external-ui-name", "");
    set_string(root, "secret", secret);

    set_string(root, "ss-config", "");
    set_string(root, "vmess-config", "");
    set_sequence(root, "listeners", Vec::new());
    set_sequence(root, "tunnels", Vec::new());
    set_disabled_mapping(root, "tun");
    set_disabled_mapping(root, "iptables");
    set_disabled_mapping(root, "tuic-server");
    set_disabled_mapping(root, "ntp");

    if let Some(Value::Mapping(dns)) = root.get_mut(string("dns")) {
        dns.remove(string("listen"));
        set_number(dns, "listen-routing-mark", 0);
        harden_dns_bootstrap(dns);
    }

    yaml_serde::to_string(&value)
        .map(String::into_bytes)
        .map_err(|_| RuntimeError::ConfigurationSerialization)
}

fn harden_dns_bootstrap(dns: &mut Mapping) {
    if dns.get(string("enable")).and_then(Value::as_bool) != Some(true) {
        return;
    }

    if !dns.contains_key(string("respect-rules")) {
        set_bool(dns, "respect-rules", false);
    }

    if dns.contains_key(string("proxy-server-nameserver")) {
        return;
    }

    // Let Mihomo refresh system DNS and race a small, independent resolver pool.
    // Node bootstrap must never depend on the proxy being bootstrapped.
    let defaults = resolver_values(dns.get(string("default-nameserver")));
    let ordinary = resolver_values(dns.get(string("nameserver")));
    let mut bootstrap = vec![string("system")];
    for value in &defaults {
        append_resolver(&mut bootstrap, value);
    }
    dns.insert(string("default-nameserver"), Value::Sequence(bootstrap));

    let mut nodes = vec![string("system")];
    // Preserve encrypted/TCP alternatives instead of dropping them when deriving
    // proxy-server-nameserver from an UDP-only default-nameserver.
    for value in ordinary.iter().chain(&defaults) {
        if safe_direct_resolver(value, true) {
            append_resolver(&mut nodes, value);
        }
    }
    for value in &defaults {
        if safe_direct_resolver(value, false) {
            append_resolver(&mut nodes, value);
            if let Some(address) = value.as_str()
                && address.parse::<std::net::IpAddr>().is_ok()
            {
                let address = if address.contains(':') {
                    format!("tcp://[{address}]")
                } else {
                    format!("tcp://{address}")
                };
                append_resolver(&mut nodes, &string(&address));
            }
        }
    }
    dns.insert(string("proxy-server-nameserver"), Value::Sequence(nodes));
}

const MAX_BOOTSTRAP_RESOLVERS: usize = 8;

fn resolver_values(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::String(value)) if !value.trim().is_empty() => vec![string(value)],
        Some(Value::Sequence(values)) => values.clone(),
        _ => Vec::new(),
    }
}

fn append_resolver(values: &mut Vec<Value>, value: &Value) {
    if values.len() < MAX_BOOTSTRAP_RESOLVERS
        && value.as_str().is_some_and(|value| !value.trim().is_empty())
        && !values.contains(value)
    {
        values.push(value.clone());
    }
}

fn safe_direct_resolver(value: &Value, reliable_transport: bool) -> bool {
    let Some(value) = value.as_str() else {
        return false;
    };
    if !reliable_transport && value.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    matches!(url.scheme(), "https" | "tls" | "tcp")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none_or(|fragment| fragment == "DIRECT")
}

fn set_disabled_mapping(root: &mut Mapping, key: &str) {
    let mut value = Mapping::new();
    set_bool(&mut value, "enable", false);
    root.insert(string(key), Value::Mapping(value));
}

fn set_string(root: &mut Mapping, key: &str, value: &str) {
    root.insert(string(key), Value::String(value.into()));
}

fn set_bool(root: &mut Mapping, key: &str, value: bool) {
    root.insert(string(key), Value::Bool(value));
}

fn set_number(root: &mut Mapping, key: &str, value: u64) {
    root.insert(string(key), Value::Number(Number::from(value)));
}

fn set_sequence(root: &mut Mapping, key: &str, value: Vec<Value>) {
    root.insert(string(key), Value::Sequence(value));
}

fn string(value: &str) -> Value {
    Value::String(value.into())
}

#[cfg(test)]
mod tests {
    use yaml_serde::Value;

    use crate::mihomo::OperatingMode;

    use super::build_managed_config;

    #[test]
    fn runtime_configuration_disables_unsafe_inbounds_and_system_changes() {
        let input = br#"
tcp-concurrent: false
mixed-port: 7890
port: 7891
socks-port: 7892
redir-port: 7893
tproxy-port: 7894
allow-lan: true
bind-address: "*"
external-controller: 0.0.0.0:9090
external-controller-tls: 0.0.0.0:9443
external-controller-unix: mihomo.sock
external-doh-server: /dns-query
external-ui: ../../outside
external-ui-url: https://example.com/ui.zip
secret: old-secret
ss-config: inbound.yaml
vmess-config: inbound-vmess.yaml
listeners:
  - name: unsafe
    type: mixed
    port: 8080
tunnels:
  - tcp,0.0.0.0:7000,example.com:443,DIRECT
tun:
  enable: true
  auto-route: true
iptables:
  enable: true
tuic-server:
  enable: true
  listen: 0.0.0.0:443
ntp:
  enable: true
  write-to-system: true
dns:
  enable: true
  listen: 0.0.0.0:53
proxies:
  - name: Direct
    type: direct
"#;

        let output = build_managed_config(
            input,
            41001,
            41002,
            "new-secret",
            "mihoterm-user",
            "proxy-password",
            OperatingMode::Global,
        )
        .expect("configuration should be derived");
        let value: Value = yaml_serde::from_slice(&output).expect("output should be YAML");

        assert_eq!(value["mixed-port"].as_u64(), Some(41002));
        assert_eq!(value["mode"].as_str(), Some("global"));
        assert_eq!(value["port"].as_u64(), Some(0));
        assert_eq!(value["socks-port"].as_u64(), Some(0));
        assert_eq!(value["redir-port"].as_u64(), Some(0));
        assert_eq!(value["tproxy-port"].as_u64(), Some(0));
        assert_eq!(value["allow-lan"].as_bool(), Some(false));
        assert_eq!(value["tcp-concurrent"].as_bool(), Some(true));
        assert_eq!(value["bind-address"].as_str(), Some("127.0.0.1"));
        assert_eq!(
            value["authentication"][0].as_str(),
            Some("mihoterm-user:proxy-password")
        );
        assert_eq!(
            value["external-controller"].as_str(),
            Some("127.0.0.1:41001")
        );
        assert_eq!(value["secret"].as_str(), Some("new-secret"));
        assert_eq!(value["tun"]["enable"].as_bool(), Some(false));
        assert_eq!(value["iptables"]["enable"].as_bool(), Some(false));
        assert_eq!(value["tuic-server"]["enable"].as_bool(), Some(false));
        assert_eq!(value["ntp"]["enable"].as_bool(), Some(false));
        assert!(value["listeners"].as_sequence().is_some_and(Vec::is_empty));
        assert!(value["tunnels"].as_sequence().is_some_and(Vec::is_empty));
        assert!(value["dns"]["listen"].is_null());
        assert!(
            !String::from_utf8(output)
                .expect("output should be UTF-8")
                .contains("old-secret")
        );
    }

    #[test]
    fn runtime_configuration_requires_proxy_content() {
        let result = build_managed_config(
            b"mode: rule\nrules: []\n",
            41001,
            41002,
            "secret",
            "user",
            "password",
            OperatingMode::Global,
        );

        assert!(result.is_err());
    }

    #[test]
    fn runtime_configuration_bootstraps_proxy_node_dns_from_profile_defaults() {
        let input = br#"
dns:
  enable: true
  default-nameserver:
    - 223.5.5.5
    - 119.29.29.29
  nameserver:
    - https://dns.example/dns-query
proxies:
  - name: Proxy
    type: direct
"#;

        let output = build_managed_config(
            input,
            41001,
            41002,
            "secret",
            "user",
            "password",
            OperatingMode::Global,
        )
        .expect("configuration should be derived");
        let value: Value = yaml_serde::from_slice(&output).expect("output should be YAML");

        assert_eq!(value["dns"]["respect-rules"].as_bool(), Some(false));
        assert_eq!(
            value["dns"]["proxy-server-nameserver"][0].as_str(),
            Some("system")
        );
        assert_eq!(
            value["dns"]["proxy-server-nameserver"][1].as_str(),
            Some("https://dns.example/dns-query")
        );
    }

    #[test]
    fn runtime_configuration_preserves_explicit_proxy_node_dns_policy() {
        let input = br#"
dns:
  enable: true
  respect-rules: true
  default-nameserver:
    - 223.5.5.5
  proxy-server-nameserver:
    - tls://1.1.1.1
proxies:
  - name: Proxy
    type: direct
"#;

        let output = build_managed_config(
            input,
            41001,
            41002,
            "secret",
            "user",
            "password",
            OperatingMode::Rule,
        )
        .expect("configuration should be derived");
        let value: Value = yaml_serde::from_slice(&output).expect("output should be YAML");

        assert_eq!(value["dns"]["respect-rules"].as_bool(), Some(true));
        assert_eq!(value["mode"].as_str(), Some("rule"));
        assert_eq!(
            value["dns"]["proxy-server-nameserver"][0].as_str(),
            Some("tls://1.1.1.1")
        );
    }

    #[test]
    fn runtime_configuration_uses_dynamic_system_dns_without_profile_defaults() {
        let input = br#"
dns:
  enable: true
  nameserver:
    - https://dns.example/dns-query
proxies:
  - name: Proxy
    type: direct
"#;

        let output = build_managed_config(
            input,
            41001,
            41002,
            "secret",
            "user",
            "password",
            OperatingMode::Global,
        )
        .expect("configuration should be derived");
        let value: Value = yaml_serde::from_slice(&output).expect("output should be YAML");

        assert_eq!(value["dns"]["respect-rules"].as_bool(), Some(false));
        assert_eq!(
            value["dns"]["proxy-server-nameserver"][0].as_str(),
            Some("system")
        );
    }

    #[test]
    fn runtime_configuration_preserves_an_explicit_empty_proxy_nameserver() {
        let input = br#"
dns:
  enable: true
  default-nameserver:
    - 223.5.5.5
  proxy-server-nameserver: []
proxies:
  - name: Proxy
    type: direct
"#;

        let output = build_managed_config(
            input,
            41001,
            41002,
            "secret",
            "user",
            "password",
            OperatingMode::Global,
        )
        .expect("configuration should be derived");
        let value: Value = yaml_serde::from_slice(&output).expect("output should be YAML");

        assert!(
            value["dns"]["proxy-server-nameserver"]
                .as_sequence()
                .is_some_and(Vec::is_empty)
        );
    }
    #[test]
    fn bootstrap_pool_excludes_proxy_routing_and_is_bounded() {
        let mut dns: yaml_serde::Mapping = yaml_serde::from_str(
            r#"
enable: true
default-nameserver: [223.5.5.5, 119.29.29.29]
nameserver:
  - https://dns.example/dns-query#Proxy
  - https://dns.example/dns-query#RULES
  - https://dns.example/dns-query#skip-cert-verify
  - https://dns.example/dns-query#DIRECT
  - tcp://192.0.2.1
"#,
        )
        .unwrap();
        super::harden_dns_bootstrap(&mut dns);
        let nodes = dns[super::string("proxy-server-nameserver")]
            .as_sequence()
            .unwrap();
        assert_eq!(nodes[0].as_str(), Some("system"));
        assert!(nodes.iter().any(|v| v.as_str() == Some("tcp://223.5.5.5")));
        assert!(
            nodes
                .iter()
                .any(|v| v.as_str() == Some("https://dns.example/dns-query#DIRECT"))
        );
        assert!(!nodes.iter().any(|v| v.as_str().unwrap().contains("#RULES")));
        assert!(!nodes.iter().any(|v| v.as_str().unwrap().contains("#Proxy")));
        assert!(
            !nodes
                .iter()
                .any(|v| v.as_str().unwrap().contains("skip-cert-verify"))
        );
        assert!(nodes.len() <= super::MAX_BOOTSTRAP_RESOLVERS);
        let once = dns.clone();
        super::harden_dns_bootstrap(&mut dns);
        assert_eq!(dns, once);
    }
}
