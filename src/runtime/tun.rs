//! Explicit, owner-scoped Linux TUN settings, never imported from subscriptions.
use std::{
    fs,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
    process::Command,
};

use super::RuntimeError;
use serde::{Deserialize, Serialize};
use yaml_serde::{Mapping, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunSettings {
    pub enabled: bool,
    pub uid: u32,
    #[serde(default)]
    pub core: Option<PathBuf>,
}

impl Default for TunSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            uid: rustix::process::geteuid().as_raw(),
            core: None,
        }
    }
}

impl TunSettings {
    pub fn load(root: &Path) -> Result<Self, RuntimeError> {
        let path = root.join("tun.json");
        let metadata = match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Ok(metadata) => metadata,
            Err(_) => return Err(RuntimeError::PersistentState),
        };
        if !metadata.is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || metadata.len() > 4096
        {
            return Err(RuntimeError::PersistentState);
        }
        let settings: Self =
            serde_json::from_slice(&fs::read(path).map_err(|_| RuntimeError::PersistentState)?)
                .map_err(|_| RuntimeError::PersistentState)?;
        if settings.uid != rustix::process::geteuid().as_raw() {
            return Err(RuntimeError::TunScope);
        }
        Ok(settings)
    }

    pub fn core_path(&self, explicit: Option<&Path>) -> std::path::PathBuf {
        explicit
            .map(Path::to_owned)
            .or_else(|| self.enabled.then(|| self.core.clone()).flatten())
            .unwrap_or_else(super::default_mihomo_executable)
    }

    pub fn save(&self, root: &Path) -> Result<(), RuntimeError> {
        if self.uid != rustix::process::geteuid().as_raw() {
            return Err(RuntimeError::TunScope);
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|_| RuntimeError::PersistentState)?;
        super::session::replace_private(&root.join("tun.json"), &bytes)
            .map_err(|_| RuntimeError::PersistentState)
    }

    pub fn validate_permissions(&self, mihomo: &Path) -> Result<(), RuntimeError> {
        if self.enabled && !permissions_available(mihomo) {
            return Err(RuntimeError::TunPermission);
        }
        Ok(())
    }

    pub(super) fn apply(&self, configuration: Vec<u8>) -> Result<Vec<u8>, RuntimeError> {
        if !self.enabled {
            return Ok(configuration);
        }
        if self.uid != rustix::process::geteuid().as_raw() {
            return Err(RuntimeError::TunScope);
        }
        let mut value: Value =
            yaml_serde::from_slice(&configuration).map_err(|_| RuntimeError::InvalidProfile)?;
        let root = value.as_mapping_mut().ok_or(RuntimeError::InvalidProfile)?;
        let mut tun = Mapping::new();
        for key in ["enable", "auto-route", "auto-detect-interface"] {
            tun.insert(key.into(), Value::Bool(true));
        }
        tun.insert("auto-redirect".into(), Value::Bool(false));
        tun.insert("device".into(), format!("miho{}", self.uid).into());
        tun.insert("stack".into(), "mixed".into());
        tun.insert(
            "include-uid".into(),
            Value::Sequence(vec![Value::Number(self.uid.into())]),
        );
        tun.insert(
            "dns-hijack".into(),
            Value::Sequence(vec!["any:53".into(), "tcp://any:53".into()]),
        );
        // Preserve SSH, LAN services and loopback while capturing this user's public traffic.
        tun.insert(
            "route-exclude-address".into(),
            Value::Sequence(
                [
                    "10.0.0.0/8",
                    "172.16.0.0/12",
                    "192.168.0.0/16",
                    "127.0.0.0/8",
                    "169.254.0.0/16",
                    "::1/128",
                    "fc00::/7",
                    "fe80::/10",
                ]
                .into_iter()
                .map(Value::from)
                .collect(),
            ),
        );
        root.insert("tun".into(), Value::Mapping(tun));
        root.insert("ipv6".into(), Value::Bool(true));
        let dns = root
            .entry(Value::from("dns"))
            .or_insert_with(|| Value::Mapping(Mapping::new()));
        if !dns.is_mapping() {
            *dns = Value::Mapping(Mapping::new());
        }
        let dns = dns.as_mapping_mut().ok_or(RuntimeError::InvalidProfile)?;
        dns.insert("enable".into(), Value::Bool(true));
        dns.insert("ipv6".into(), Value::Bool(true));
        dns.entry(Value::from("nameserver"))
            .or_insert_with(|| Value::Sequence(vec!["1.1.1.1".into(), "8.8.8.8".into()]));
        yaml_serde::to_string(&value)
            .map(String::into_bytes)
            .map_err(|_| RuntimeError::ConfigurationSerialization)
    }
}

pub fn permissions_available(mihomo: &Path) -> bool {
    if !fs::metadata("/dev/net/tun").is_ok_and(|m| m.file_type().is_char_device()) {
        return false;
    }
    if rustix::process::geteuid().is_root() {
        return true;
    }
    // File capabilities apply only to the core. MihoTerm itself never invokes sudo.
    let Ok(output) = Command::new("getcap").arg(mihomo).output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let caps = String::from_utf8_lossy(&output.stdout);
    caps.split_whitespace().skip(1).any(|field| {
        field.split_once('=').is_some_and(|(names, flags)| {
            ["cap_net_admin", "cap_net_raw"]
                .iter()
                .all(|required| names.split(',').any(|name| name == *required))
                && flags.contains('e')
                && flags.contains('p')
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_tun_is_uid_scoped_and_excludes_private_routes() {
        let settings = TunSettings {
            enabled: true,
            ..Default::default()
        };
        let result = settings.apply(b"tun: {enable: false, include-uid: [0], auto-redirect: true}\ndns: {enable: false}\n".to_vec()).unwrap();
        let value: Value = yaml_serde::from_slice(&result).unwrap();
        assert_eq!(value["tun"]["enable"].as_bool(), Some(true));
        assert_eq!(value["tun"]["auto-redirect"].as_bool(), Some(false));
        assert_eq!(
            value["tun"]["include-uid"].as_sequence().unwrap(),
            &vec![Value::Number(settings.uid.into())]
        );
        assert!(
            value["tun"]["route-exclude-address"]
                .as_sequence()
                .unwrap()
                .contains(&Value::from("10.0.0.0/8"))
        );
        assert_eq!(value["dns"]["enable"].as_bool(), Some(true));
        assert_eq!(value["ipv6"].as_bool(), Some(true));
        assert_eq!(value["dns"]["ipv6"].as_bool(), Some(true));
    }

    #[test]
    fn managed_starts_reuse_the_enabled_core_path_and_allow_explicit_override() {
        let settings = TunSettings {
            enabled: true,
            core: Some("/trusted/mihomo".into()),
            ..Default::default()
        };
        assert_eq!(settings.core_path(None), PathBuf::from("/trusted/mihomo"));
        assert_eq!(
            settings.core_path(Some(Path::new("/override/mihomo"))),
            PathBuf::from("/override/mihomo")
        );
    }

    #[test]
    fn disabled_tun_preserves_the_sanitized_configuration() {
        let bytes = b"tun: {enable: false}\n".to_vec();
        assert_eq!(TunSettings::default().apply(bytes.clone()).unwrap(), bytes);
    }

    #[test]
    fn tun_refuses_another_users_scope() {
        let settings = TunSettings {
            enabled: true,
            uid: rustix::process::geteuid().as_raw().wrapping_add(1),
            core: None,
        };
        assert!(matches!(
            settings.apply(b"{}".to_vec()),
            Err(RuntimeError::TunScope)
        ));
    }
}
