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

    let complex = !values(dns.get(key("fallback"))).is_empty()
        || dns
            .get(key("nameserver-policy"))
            .and_then(Value::as_mapping)
            .is_some_and(|v| !v.is_empty());
    let has_node_policy = dns
        .get(key("proxy-server-nameserver-policy"))
        .and_then(Value::as_mapping)
        .is_some_and(|v| !v.is_empty());
    let respect_rules = dns.get(key("respect-rules")).and_then(Value::as_bool) == Some(true);

    for field in ["proxy-server-nameserver", "direct-nameserver"] {
        let resolvers = values(dns.get(key(field)));
        if !resolvers.iter().any(|v| is_self(v, &listener)) {
            continue;
        }
        // With no separate node policy, Mihomo's ordinary resolver preserves
        // nameserver-policy and fallback semantics, unlike flattening them.
        let can_inherit = resolvers.iter().all(|v| is_self(v, &listener))
            && (field == "direct-nameserver" || (!has_node_policy && !respect_rules));
        if can_inherit {
            dns.remove(key(field));
        } else {
            if complex {
                return Err(RuntimeError::DnsListenerPolicy);
            }
            let rewritten = expand(&resolvers, &listener, &upstreams);
            dns.insert(key(field), Value::Sequence(rewritten));
        }
    }
    if policy_self {
        if complex {
            return Err(RuntimeError::DnsListenerPolicy);
        }
        if let Some(Value::Mapping(policy)) = dns.get_mut(key("proxy-server-nameserver-policy")) {
            for (_, value) in policy.iter_mut() {
                if contains_self(Some(value), &listener) {
                    *value = Value::Sequence(expand(&values(Some(value)), &listener, &upstreams));
                }
            }
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
        options: !matches!(url.fragment(), None | Some("DIRECT")),
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

fn expand(resolvers: &[Value], listener: &Endpoint, upstreams: &[Value]) -> Vec<Value> {
    let mut result = Vec::new();
    for resolver in resolvers {
        for value in if is_self(resolver, listener) {
            upstreams
        } else {
            std::slice::from_ref(resolver)
        } {
            if !result.contains(value) {
                result.push(value.clone());
            }
        }
    }
    result
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
    fn self_reference_inherits_full_provider_policy_and_fallback() {
        let dns = prepare(
            "enable: true\nlisten: 127.0.0.1:15353\nproxy-server-nameserver: [udp://127.0.0.1:15353]\nnameserver: [https://resolver.example/dns-query]\nnameserver-policy: {'+.nodes.test': [tcp://192.0.2.53]}\nfallback: [tls://192.0.2.54]\n",
        );
        assert!(!dns.contains_key(key("proxy-server-nameserver")));
        assert!(!dns.contains_key(key("listen")));
        assert_eq!(
            dns[key("nameserver-policy")]["+.nodes.test"][0].as_str(),
            Some("tcp://192.0.2.53")
        );
        assert_eq!(dns[key("fallback")][0].as_str(), Some("tls://192.0.2.54"));
    }

    #[test]
    fn loopback_aliases_and_wildcard_listeners_are_not_port_specific() {
        for (listener, resolver) in [
            ("127.0.0.1:15353", "udp://127.0.0.1:15353"),
            ("0.0.0.0:2053", "127.0.0.1:2053"),
            (":53", "127.0.0.1"),
            ("[::]:2053", "tcp://[::1]:2053"),
            ("[::1]:2053", "udp://[::1]:2053"),
            ("localhost:2053", "udp://127.0.0.1:2053"),
            ("127.0.0.1:2053", "tcp://LOCALHOST.:2053"),
            ("127.0.0.1:2053", "udp://[::ffff:127.0.0.1]:2053"),
        ] {
            let dns = prepare(&format!(
                "enable: true\nlisten: '{listener}'\nproxy-server-nameserver: ['{resolver}']\nnameserver: [192.0.2.53]\n"
            ));
            assert!(
                !dns.contains_key(key("proxy-server-nameserver")),
                "{listener} -> {resolver}"
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
    fn mixed_node_pool_uses_provider_upstreams_and_retains_independent_entries() {
        let dns = prepare(
            "enable: true\nlisten: 127.0.0.1:2053\nproxy-server-nameserver: [tls://192.0.2.53, udp://127.0.0.1:2053]\nnameserver: [https://resolver.example/dns-query#DIRECT, tcp://192.0.2.54]\n",
        );
        let actual = values(dns.get(key("proxy-server-nameserver")));
        assert_eq!(
            actual,
            vec![
                key("tls://192.0.2.53"),
                key("https://resolver.example/dns-query#DIRECT"),
                key("tcp://192.0.2.54")
            ]
        );
    }

    #[test]
    fn respect_rules_keeps_an_independent_provider_node_pool() {
        let dns = prepare(
            "enable: true\nlisten: 127.0.0.1:2053\nrespect-rules: true\nproxy-server-nameserver: [udp://127.0.0.1:2053]\nnameserver: [https://resolver.example/dns-query]\n",
        );
        assert_eq!(dns[key("respect-rules")].as_bool(), Some(true));
        assert_eq!(dns[key("proxy-server-nameserver")], dns[key("nameserver")]);
    }

    #[test]
    fn node_policy_self_reference_uses_the_same_provider_upstream() {
        let dns = prepare(
            "enable: true\nlisten: 127.0.0.1:2053\nproxy-server-nameserver: [tls://192.0.2.53]\nproxy-server-nameserver-policy: {'+.nodes.test': udp://127.0.0.1:2053}\nnameserver: [https://resolver.example/dns-query]\n",
        );
        assert_eq!(
            dns[key("proxy-server-nameserver-policy")]["+.nodes.test"],
            dns[key("nameserver")]
        );
    }

    #[test]
    fn direct_self_reference_inherits_provider_dns() {
        let dns = prepare(
            "enable: true\nlisten: 127.0.0.1:2053\ndirect-nameserver: [udp://127.0.0.1:2053]\nnameserver: [192.0.2.53]\nnameserver-policy: {'+.test': tls://192.0.2.54}\n",
        );
        assert!(!dns.contains_key(key("direct-nameserver")));
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
    fn complex_mixed_pipeline_is_rejected_instead_of_flattened() {
        let mut dns = yaml(
            "enable: true\nlisten: 127.0.0.1:2053\nproxy-server-nameserver: [udp://127.0.0.1:2053, tls://192.0.2.53]\nnameserver: [192.0.2.54]\nfallback: [192.0.2.55]\n",
        );
        assert!(matches!(
            prepare_managed_dns(&mut dns),
            Err(RuntimeError::DnsListenerPolicy)
        ));
    }

    #[test]
    fn parameterized_self_reference_is_rejected_instead_of_silently_losing_options() {
        for fragment in ["ecs=192.0.2.0/24", "DIRECT&ecs=192.0.2.0/24", "RULES"] {
            let mut dns = yaml(&format!(
                "enable: true\nlisten: 127.0.0.1:2053\nproxy-server-nameserver: ['udp://127.0.0.1:2053#{fragment}']\nnameserver: [192.0.2.53]\n"
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
