use std::net::{IpAddr, SocketAddr};

use url::{Host, Url};
use yaml_serde::{Mapping, Number, Value};

use super::RuntimeError;

// Keep the subscription's resolver pipeline. In particular, adding `system`
// here makes its answers race provider DNS and bypasses split-DNS policies.
pub(super) fn prepare_managed_dns(dns: &mut Mapping) -> Result<(), RuntimeError> {
    let listen = dns.remove(key("listen"));
    dns.insert(key("listen-routing-mark"), Value::Number(Number::from(0)));
    if dns.get(key("enable")).and_then(Value::as_bool) != Some(true) {
        return Ok(());
    }
    let Some(listener) = listen.as_ref().and_then(Value::as_str).and_then(endpoint) else {
        return Ok(());
    };

    // A reference from the main/bootstrap pipeline to its own removed listener
    // cannot be expanded without keeping a DNS cycle.
    for field in [
        "nameserver",
        "default-nameserver",
        "fallback",
        "nameserver-policy",
    ] {
        if contains_self(dns.get(key(field)), &listener) {
            return Err(RuntimeError::DnsListenerCycle);
        }
    }

    let node_self = contains_self(dns.get(key("proxy-server-nameserver")), &listener);
    let policy_self = contains_self(dns.get(key("proxy-server-nameserver-policy")), &listener);
    let direct_self = contains_self(dns.get(key("direct-nameserver")), &listener);
    if !node_self && !policy_self && !direct_self {
        return Ok(());
    }
    for field in [
        "proxy-server-nameserver",
        "proxy-server-nameserver-policy",
        "direct-nameserver",
    ] {
        if has_self_options(dns.get(key(field)), &listener) {
            return Err(RuntimeError::DnsListenerPolicy);
        }
    }

    let upstreams = values(dns.get(key("nameserver")));
    if upstreams.is_empty() {
        return Err(RuntimeError::DnsListenerUpstreams);
    }
    for field in ["nameserver", "fallback", "nameserver-policy"] {
        if !all_direct(dns.get(key(field))) {
            return Err(RuntimeError::DnsListenerUpstreams);
        }
    }

    // A DNS listener applies Mihomo's host aliases and other service-level
    // transformations. Pointing node lookups directly at its upstreams skips
    // that processing, so keep the configured resolver path on loopback.
    let private_listener = private_listener(&listener)?;
    dns.insert(key("listen"), key(&private_listener.to_string()));
    for field in [
        "proxy-server-nameserver",
        "proxy-server-nameserver-policy",
        "direct-nameserver",
    ] {
        if let Some(value) = dns.get_mut(key(field)) {
            rewrite_self_references(value, &listener, private_listener);
        }
    }

    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
struct Endpoint {
    host: Host<String>,
    port: u16,
    options: bool,
}

fn private_listener(listener: &Endpoint) -> Result<SocketAddr, RuntimeError> {
    if listener.port == 0 {
        return Err(RuntimeError::DnsListenerPolicy);
    }
    let ip = match &listener.host {
        Host::Ipv4(ip) if ip.is_loopback() => IpAddr::V4(*ip),
        Host::Ipv6(ip) if ip.is_loopback() => IpAddr::V6(*ip),
        Host::Ipv6(_) => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        Host::Ipv4(_) | Host::Domain(_) => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
    };
    Ok(SocketAddr::new(ip, listener.port))
}

fn rewrite_self_references(value: &mut Value, listener: &Endpoint, private: SocketAddr) {
    match value {
        Value::Mapping(mapping) => {
            for child in mapping.values_mut() {
                rewrite_self_references(child, listener, private);
            }
        }
        Value::Sequence(sequence) => {
            for child in sequence {
                rewrite_self_references(child, listener, private);
            }
        }
        Value::String(address) if is_self(&Value::String(address.clone()), listener) => {
            let scheme = address.split_once("://").map_or("", |(scheme, _)| {
                if scheme.eq_ignore_ascii_case("tcp") {
                    "tcp://"
                } else {
                    "udp://"
                }
            });
            let fragment = address
                .split_once('#')
                .map_or(String::new(), |(_, fragment)| format!("#{fragment}"));
            *address = format!("{scheme}{private}{fragment}");
        }
        _ => {}
    }
}

pub(super) struct DnsListenerReservation {
    _tcp: std::net::TcpListener,
    _udp: std::net::UdpSocket,
}

// Reserve a newly required listener before starting or applying a profile.
// Reuse the existing listener on same-port hot reloads; it belongs to the
// running managed core. The sockets are released just before Mihomo binds.
pub(super) fn reserve_new_listener(
    configuration: &[u8],
    previous: Option<&[u8]>,
) -> Result<Option<DnsListenerReservation>, RuntimeError> {
    let Some(address) = configured_listener(configuration)? else {
        return Ok(None);
    };
    if let Some(previous) = previous
        && configured_listener(previous)? == Some(address)
    {
        return Ok(None);
    }
    let error = || RuntimeError::DnsListenerReservation { address };
    let domain = if address.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };
    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, None).map_err(|_| error())?;
    socket.set_reuse_address(true).map_err(|_| error())?;
    socket.bind(&address.into()).map_err(|_| error())?;
    socket.listen(128).map_err(|_| error())?;
    let tcp = std::net::TcpListener::from(socket);
    let udp = std::net::UdpSocket::bind(address).map_err(|_| error())?;
    Ok(Some(DnsListenerReservation {
        _tcp: tcp,
        _udp: udp,
    }))
}

fn configured_listener(configuration: &[u8]) -> Result<Option<SocketAddr>, RuntimeError> {
    let root: Value =
        yaml_serde::from_slice(configuration).map_err(|_| RuntimeError::InvalidProfile)?;
    let Some(dns) = root.get(key("dns")).and_then(Value::as_mapping) else {
        return Ok(None);
    };
    if dns.get(key("enable")).and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    let Some(listen) = dns.get(key("listen")) else {
        return Ok(None);
    };
    let address: SocketAddr = listen
        .as_str()
        .and_then(|value| value.parse().ok())
        .ok_or(RuntimeError::DnsListenerPolicy)?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(RuntimeError::DnsListenerPolicy);
    }
    Ok(Some(address))
}

fn endpoint(value: &str) -> Option<Endpoint> {
    let value = if let Ok(ip) = value.parse::<IpAddr>() {
        SocketAddr::new(ip, 53).to_string()
    } else if value.starts_with(':') {
        format!("0.0.0.0{value}")
    } else {
        value.to_owned()
    };
    let value = if value.contains("://") {
        value
    } else {
        format!("udp://{value}")
    };
    let url = Url::parse(&value).ok()?;
    if !matches!(url.scheme(), "udp" | "tcp")
        || !url.username().is_empty()
        || url.password().is_some()
        || !url
            .fragment()
            .is_none_or(|v| direct_fragment(v) || v == "RULES")
    {
        return None;
    }
    let raw_host = url
        .host_str()?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let host = match raw_host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => Host::Ipv4(ip),
        Ok(IpAddr::V6(ip)) => match ip.to_ipv4_mapped() {
            Some(ip) => Host::Ipv4(ip),
            None => Host::Ipv6(ip),
        },
        Err(_)
            if raw_host.eq_ignore_ascii_case("localhost")
                || raw_host.eq_ignore_ascii_case("localhost.") =>
        {
            Host::Domain("localhost".to_owned())
        }
        _ => return None,
    };
    Some(Endpoint {
        host,
        port: url.port().unwrap_or(53),
        options: url.query().is_some()
            || !url.path().is_empty()
            || !matches!(url.fragment(), None | Some("DIRECT")),
    })
}

fn is_self(value: &Value, listener: &Endpoint) -> bool {
    let Some(address) = value.as_str().and_then(endpoint) else {
        return false;
    };
    if address.port != listener.port {
        return false;
    }
    address.host == listener.host
        || match (&listener.host, &address.host) {
            (Host::Domain(_), Host::Ipv4(ip)) | (Host::Ipv4(ip), Host::Domain(_)) => {
                *ip == std::net::Ipv4Addr::LOCALHOST || ip.is_unspecified()
            }
            (Host::Domain(_), Host::Ipv6(ip)) | (Host::Ipv6(ip), Host::Domain(_)) => {
                *ip == std::net::Ipv6Addr::LOCALHOST || ip.is_unspecified()
            }
            (Host::Ipv4(listen_ip), Host::Ipv4(ip)) => {
                listen_ip.is_unspecified() && ip.is_loopback()
            }
            (Host::Ipv6(listen_ip), Host::Ipv6(ip)) => {
                listen_ip.is_unspecified() && ip.is_loopback()
            }
            // A dual-stack IPv6 wildcard listener may serve IPv4 loopback too.
            (Host::Ipv6(listen_ip), Host::Ipv4(ip)) => {
                listen_ip.is_unspecified() && ip.is_loopback()
            }
            _ => false,
        }
}

fn contains_self(value: Option<&Value>, listener: &Endpoint) -> bool {
    match value {
        Some(Value::Mapping(mapping)) => mapping.values().any(|v| contains_self(Some(v), listener)),
        Some(Value::Sequence(sequence)) => {
            sequence.iter().any(|v| contains_self(Some(v), listener))
        }
        Some(value) => is_self(value, listener),
        None => false,
    }
}

fn has_self_options(value: Option<&Value>, listener: &Endpoint) -> bool {
    match value {
        Some(Value::Mapping(mapping)) => mapping
            .values()
            .any(|v| has_self_options(Some(v), listener)),
        Some(Value::Sequence(sequence)) => {
            sequence.iter().any(|v| has_self_options(Some(v), listener))
        }
        Some(value) => {
            is_self(value, listener) && value.as_str().and_then(endpoint).is_some_and(|e| e.options)
        }
        None => false,
    }
}

fn direct_fragment(fragment: &str) -> bool {
    fragment.split('&').all(|part| {
        part.contains('=')
            || matches!(
                part,
                "DIRECT" | "h3" | "skip-cert-verify" | "disable-ipv4" | "disable-ipv6"
            )
    })
}

fn all_direct(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::Mapping(mapping)) => mapping.values().all(|v| all_direct(Some(v))),
        Some(Value::Sequence(sequence)) => sequence.iter().all(|v| all_direct(Some(v))),
        Some(Value::String(value)) => {
            let Some((_, fragment)) = value.split_once('#') else {
                return true;
            };
            direct_fragment(fragment)
        }
        _ => false,
    }
}

fn values(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::String(value)) if !value.trim().is_empty() => vec![key(value)],
        Some(Value::Sequence(values)) => values.clone(),
        _ => Vec::new(),
    }
}

fn key(value: &str) -> Value {
    Value::String(value.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(source: &str) -> Mapping {
        yaml_serde::from_str(source).unwrap()
    }
    fn prepare(source: &str) -> Mapping {
        let mut dns = yaml(source);
        prepare_managed_dns(&mut dns).unwrap();
        dns
    }

    #[test]
    fn provider_dns_pipeline_is_preserved_without_injecting_system() {
        let source = "enable: true\nipv6: true\ndefault-nameserver: [192.0.2.53]\nnameserver: [https://resolver.example/dns-query]\nnameserver-policy: {'+.nodes.test': [tls://192.0.2.54]}\nfallback: [tcp://192.0.2.55]\nfallback-filter: {geoip: false, domain: ['+.test']}\n";
        let before = yaml(source);
        let mut after = prepare(source);
        after.remove(key("listen-routing-mark"));
        assert_eq!(after, before);
    }

    #[test]
    fn explicit_node_dns_and_empty_node_dns_remain_authoritative() {
        for nodes in ["[tcp://127.0.0.1:2053]", "[]"] {
            let source = format!(
                "enable: true\nlisten: 127.0.0.1:15353\nproxy-server-nameserver: {nodes}\nnameserver: [192.0.2.53]\n"
            );
            let before = yaml(&source);
            let after = prepare(&source);
            assert_eq!(
                after[key("proxy-server-nameserver")],
                before[key("proxy-server-nameserver")]
            );
        }
    }

    #[test]
    fn self_reference_preserves_full_provider_policy_and_fallback() {
        let dns = prepare(
            "enable: true\nlisten: 127.0.0.1:15353\nproxy-server-nameserver: [udp://127.0.0.1:15353]\nnameserver: [https://resolver.example/dns-query]\nnameserver-policy: {'+.nodes.test': [tcp://192.0.2.53]}\nfallback: [tls://192.0.2.54]\n",
        );
        assert_eq!(dns[key("listen")].as_str(), Some("127.0.0.1:15353"));
        assert_eq!(
            dns[key("proxy-server-nameserver")][0].as_str(),
            Some("udp://127.0.0.1:15353")
        );
        assert_eq!(
            dns[key("nameserver-policy")]["+.nodes.test"][0].as_str(),
            Some("tcp://192.0.2.53")
        );
        assert_eq!(dns[key("fallback")][0].as_str(), Some("tls://192.0.2.54"));
    }

    #[test]
    fn loopback_aliases_and_wildcard_listeners_are_not_port_specific() {
        for (listener, resolver, private_listener, private_resolver) in [
            (
                "127.0.0.1:15353",
                "udp://127.0.0.1:15353",
                "127.0.0.1:15353",
                "udp://127.0.0.1:15353",
            ),
            (
                "0.0.0.0:2053",
                "127.0.0.1:2053",
                "127.0.0.1:2053",
                "127.0.0.1:2053",
            ),
            (":53", "127.0.0.1", "127.0.0.1:53", "127.0.0.1:53"),
            (
                "[::]:2053",
                "tcp://[::1]:2053",
                "[::1]:2053",
                "tcp://[::1]:2053",
            ),
            (
                "[::]:2053",
                "udp://127.0.0.1:2053",
                "[::1]:2053",
                "udp://[::1]:2053",
            ),
            (
                "[::1]:2053",
                "udp://[::1]:2053",
                "[::1]:2053",
                "udp://[::1]:2053",
            ),
            (
                "localhost:2053",
                "udp://127.0.0.1:2053",
                "127.0.0.1:2053",
                "udp://127.0.0.1:2053",
            ),
            (
                "127.0.0.1:2053",
                "tcp://LOCALHOST.:2053",
                "127.0.0.1:2053",
                "tcp://127.0.0.1:2053",
            ),
            (
                "127.0.0.1:2053",
                "udp://[::ffff:127.0.0.1]:2053",
                "127.0.0.1:2053",
                "udp://127.0.0.1:2053",
            ),
        ] {
            let dns = prepare(&format!(
                "enable: true\nlisten: '{listener}'\nproxy-server-nameserver: ['{resolver}']\nnameserver: [192.0.2.53]\n"
            ));
            assert_eq!(
                dns[key("listen")].as_str(),
                Some(private_listener),
                "{listener}"
            );
            assert_eq!(
                dns[key("proxy-server-nameserver")][0].as_str(),
                Some(private_resolver),
                "{resolver}"
            );
        }
    }

    #[test]
    fn unrelated_local_service_or_remote_proxy_dns_is_not_rewritten() {
        for resolver in [
            "udp://127.0.0.2:2053",
            "udp://127.0.0.1:5353",
            "udp://127.0.0.1:2053#Remote",
        ] {
            let dns = prepare(&format!(
                "enable: true\nlisten: 127.0.0.1:2053\nproxy-server-nameserver: ['{resolver}']\nnameserver: [192.0.2.53]\n"
            ));
            assert_eq!(
                dns[key("proxy-server-nameserver")][0].as_str(),
                Some(resolver)
            );
        }
    }

    #[test]
    fn mixed_node_pool_preserves_self_and_independent_entries() {
        let source = "enable: true\nlisten: 127.0.0.1:2053\nproxy-server-nameserver: [tls://192.0.2.53, udp://127.0.0.1:2053]\nnameserver: [https://resolver.example/dns-query#DIRECT, tcp://192.0.2.54]\n";
        let before = yaml(source);
        let after = prepare(source);
        assert_eq!(
            after[key("proxy-server-nameserver")],
            before[key("proxy-server-nameserver")]
        );
        assert_eq!(after[key("nameserver")], before[key("nameserver")]);
        assert_eq!(after[key("listen")].as_str(), Some("127.0.0.1:2053"));
    }

    #[test]
    fn respect_rules_keeps_an_independent_provider_node_pool() {
        let dns = prepare(
            "enable: true\nlisten: 127.0.0.1:2053\nrespect-rules: true\nproxy-server-nameserver: [udp://127.0.0.1:2053]\nnameserver: [https://resolver.example/dns-query]\n",
        );
        assert_eq!(dns[key("respect-rules")].as_bool(), Some(true));
        assert_eq!(
            dns[key("proxy-server-nameserver")][0].as_str(),
            Some("udp://127.0.0.1:2053")
        );
        assert_eq!(dns[key("listen")].as_str(), Some("127.0.0.1:2053"));
    }

    #[test]
    fn node_policy_self_reference_keeps_the_dns_service() {
        let dns = prepare(
            "enable: true\nlisten: 127.0.0.1:2053\nproxy-server-nameserver: [tls://192.0.2.53]\nproxy-server-nameserver-policy: {'+.nodes.test': udp://127.0.0.1:2053}\nnameserver: [https://resolver.example/dns-query]\n",
        );
        assert_eq!(
            dns[key("proxy-server-nameserver-policy")]["+.nodes.test"].as_str(),
            Some("udp://127.0.0.1:2053")
        );
        assert_eq!(dns[key("listen")].as_str(), Some("127.0.0.1:2053"));
    }

    #[test]
    fn direct_self_reference_keeps_the_dns_service() {
        let dns = prepare(
            "enable: true\nlisten: 127.0.0.1:2053\ndirect-nameserver: [udp://127.0.0.1:2053]\nnameserver: [192.0.2.53]\nnameserver-policy: {'+.test': tls://192.0.2.54}\n",
        );
        assert_eq!(
            dns[key("direct-nameserver")][0].as_str(),
            Some("udp://127.0.0.1:2053")
        );
        assert_eq!(dns[key("listen")].as_str(), Some("127.0.0.1:2053"));
    }

    #[test]
    fn main_bootstrap_and_policy_cycles_are_rejected() {
        for field in [
            "nameserver",
            "default-nameserver",
            "fallback",
            "nameserver-policy",
        ] {
            let value = if field.ends_with("policy") {
                "{'+.test': udp://127.0.0.1:2053}"
            } else {
                "[udp://127.0.0.1:2053]"
            };
            let mut dns = yaml(&format!(
                "enable: true\nlisten: 127.0.0.1:2053\n{field}: {value}\n"
            ));
            assert!(matches!(
                prepare_managed_dns(&mut dns),
                Err(RuntimeError::DnsListenerCycle)
            ));
        }
    }

    #[test]
    fn missing_or_proxy_dependent_upstreams_are_rejected_without_system_fallback() {
        for upstream in [
            "[]",
            "[https://resolver.example/dns-query#Proxies]",
            "[https://resolver.example/dns-query#RULES]",
        ] {
            let mut dns = yaml(&format!(
                "enable: true\nlisten: 127.0.0.1:2053\nproxy-server-nameserver: [udp://127.0.0.1:2053]\nnameserver: {upstream}\n"
            ));
            assert!(matches!(
                prepare_managed_dns(&mut dns),
                Err(RuntimeError::DnsListenerUpstreams)
            ));
        }
    }

    #[test]
    fn complex_mixed_pipeline_is_preserved_without_flattening() {
        let source = "enable: true\nlisten: 127.0.0.1:2053\nproxy-server-nameserver: [udp://127.0.0.1:2053, tls://192.0.2.53]\nnameserver: [192.0.2.54]\nfallback: [192.0.2.55]\n";
        let before = yaml(source);
        let after = prepare(source);
        assert_eq!(
            after[key("proxy-server-nameserver")],
            before[key("proxy-server-nameserver")]
        );
        assert_eq!(after[key("fallback")], before[key("fallback")]);
        assert_eq!(after[key("listen")].as_str(), Some("127.0.0.1:2053"));
    }

    #[test]
    fn parameterized_self_reference_is_rejected_instead_of_silently_losing_options() {
        for options in [
            "#ecs=192.0.2.0/24",
            "#DIRECT&ecs=192.0.2.0/24",
            "#RULES",
            "?prefer=ipv4",
            "/path",
        ] {
            let mut dns = yaml(&format!(
                "enable: true\nlisten: 127.0.0.1:2053\nproxy-server-nameserver: ['udp://127.0.0.1:2053{options}']\nnameserver: [192.0.2.53]\n"
            ));
            assert!(matches!(
                prepare_managed_dns(&mut dns),
                Err(RuntimeError::DnsListenerPolicy)
            ));
        }
    }

    #[test]
    fn disabled_dns_is_not_enabled_or_replaced() {
        let source = "enable: false\nipv6: false\nlisten: 127.0.0.1:2053\nproxy-server-nameserver: [udp://127.0.0.1:2053]\nnameserver: [192.0.2.53]\n";
        let mut before = yaml(source);
        before.remove(key("listen"));
        before.insert(key("listen-routing-mark"), Value::Number(Number::from(0)));
        assert_eq!(prepare(source), before);
    }

    #[test]
    fn required_listener_checks_tcp_and_udp_conflicts() {
        use std::net::{TcpListener, UdpSocket};
        let occupied_tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = occupied_tcp.local_addr().unwrap();
        let config = format!("dns:\n  enable: true\n  listen: '{address}'\n");
        assert!(matches!(
            reserve_new_listener(config.as_bytes(), None),
            Err(RuntimeError::DnsListenerReservation { .. })
        ));
        // A same-port reload keeps the listener held by the running core.
        assert!(
            reserve_new_listener(config.as_bytes(), Some(config.as_bytes()))
                .unwrap()
                .is_none()
        );
        drop(occupied_tcp);
        let occupied_udp = UdpSocket::bind(address).unwrap();
        assert!(matches!(
            reserve_new_listener(config.as_bytes(), None),
            Err(RuntimeError::DnsListenerReservation { .. })
        ));
        drop(occupied_udp);
        let reservation = reserve_new_listener(config.as_bytes(), None)
            .unwrap()
            .unwrap();
        assert!(TcpListener::bind(address).is_err());
        assert!(UdpSocket::bind(address).is_err());
        drop(reservation);
        assert!(TcpListener::bind(address).is_ok());
        assert!(UdpSocket::bind(address).is_ok());
    }

    #[test]
    fn normalization_is_idempotent_and_keeps_explicit_ipv6_settings() {
        for ipv6 in [true, false] {
            let mut dns = prepare(&format!(
                "enable: true\nipv6: {ipv6}\nlisten: 127.0.0.1:2053\nproxy-server-nameserver: [udp://127.0.0.1:2053]\nnameserver: [192.0.2.53]\n"
            ));
            let once = dns.clone();
            prepare_managed_dns(&mut dns).unwrap();
            assert_eq!(dns, once);
            assert_eq!(dns[key("ipv6")].as_bool(), Some(ipv6));
        }
    }
}
