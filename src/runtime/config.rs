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
        super::dns::prepare_managed_dns(dns)?;
    }

    install_node_probe_listeners(root, proxy_username, proxy_password)?;

    yaml_serde::to_string(&value)
        .map(String::into_bytes)
        .map_err(|_| RuntimeError::ConfigurationSerialization)
}

fn install_node_probe_listeners(
    root: &mut Mapping,
    username: &str,
    password: &str,
) -> Result<(), RuntimeError> {
    let selected_group = root
        .get(string("mihoterm-managed-probe-group"))
        .and_then(Value::as_str)
        .map(|name| format!("{name} Auto"));
    let leaves = root
        .get(string("proxies"))
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|p| p["name"].as_str())
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>();
    let nodes = root
        .get(string("proxy-groups"))
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter(|g| {
            g["type"].as_str() == Some("select")
                && g["name"]
                    .as_str()
                    .is_some_and(|n| Some(n) == selected_group.as_deref())
        })
        .flat_map(|g| g["proxies"].as_sequence().into_iter().flatten())
        .filter_map(Value::as_str)
        .filter(|n| leaves.contains(*n))
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>();
    if nodes.len() > 64 {
        return Err(RuntimeError::InvalidProfile);
    }
    let mut listeners = Vec::new();
    // Reserve all candidate ports together; the core validates and binds them at start.
    let mut reservations = Vec::new();
    for (index, node) in nodes.into_iter().enumerate() {
        let reservation = std::net::TcpListener::bind(("127.0.0.1", 0))
            .map_err(|_| RuntimeError::PortReservation)?;
        let port = reservation
            .local_addr()
            .map_err(|_| RuntimeError::PortReservation)?
            .port();
        reservations.push(reservation);
        let mut listener = Mapping::new();
        set_string(
            &mut listener,
            "name",
            &format!("mihoterm-codex-probe-{index}"),
        );
        set_string(&mut listener, "type", "http");
        set_string(&mut listener, "listen", "127.0.0.1");
        set_string(&mut listener, "proxy", &node);
        set_number(&mut listener, "port", u64::from(port));
        let mut user = Mapping::new();
        set_string(&mut user, "username", username);
        set_string(&mut user, "password", password);
        set_sequence(&mut listener, "users", vec![Value::Mapping(user)]);
        listeners.push(Value::Mapping(listener));
    }
    set_sequence(root, "listeners", listeners);
    Ok(())
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
    fn node_probe_listeners_require_a_managed_group_and_keep_private_authentication() {
        let source = b"proxies: [{name: A, type: http}, {name: B, type: http}]\nproxy-groups: [{name: AI Auto, type: select, proxies: [A, B]}]\nmihoterm-managed-probe-group: AI\n";
        let bytes = build_managed_config(
            source,
            40001,
            40002,
            "controller",
            "owner",
            "password",
            crate::mihomo::OperatingMode::Global,
        )
        .unwrap();
        let value: yaml_serde::Value = yaml_serde::from_slice(&bytes).unwrap();
        let listeners = value["listeners"].as_sequence().unwrap();
        assert_eq!(listeners.len(), 2);
        assert_ne!(listeners[0]["port"], listeners[1]["port"]);
        for listener in listeners {
            assert_eq!(listener["listen"].as_str(), Some("127.0.0.1"));
            assert_eq!(listener["users"][0]["password"].as_str(), Some("password"));
            assert!(matches!(listener["proxy"].as_str(), Some("A" | "B")));
        }
    }

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
    fn provider_dns_and_ipv6_are_preserved_without_client_overrides() {
        let input = br#"
ipv6: false
unified-delay: true
dns:
  enable: true
  ipv6: false
  nameserver: [https://resolver.example/dns-query]
  nameserver-policy: {'+.nodes.test': [tls://192.0.2.53]}
  default-nameserver: [192.0.2.54]
proxies: [{name: Proxy, type: direct}]
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
        .unwrap();
        let value: Value = yaml_serde::from_slice(&output).unwrap();
        let source: Value = yaml_serde::from_slice(input).unwrap();
        assert_eq!(value["ipv6"], source["ipv6"]);
        assert_eq!(value["unified-delay"], source["unified-delay"]);
        let mut dns = value["dns"].as_mapping().unwrap().clone();
        dns.remove(super::string("listen-routing-mark"));
        assert_eq!(dns, *source["dns"].as_mapping().unwrap());
        assert!(value["dns"]["proxy-server-nameserver"].is_null());
    }
}
