//! Bounded, opt-in recovery. No authentication material or model content is read.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    time::Duration,
};

use fs4::FileExt;
use futures_util::{StreamExt, stream};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::{
    mihomo::{ApiClient, ProxiesResponse},
    probe::ProbeTarget,
    profile::FallbackPolicy,
    state::{DesiredStateStore, unix_seconds_now},
};

use super::{RecoveryOutcome, RuntimeError};

const CHECK_INTERVAL: u64 = 15;
const SWITCH_COOLDOWN: u64 = 120;
const QUARANTINE_SECONDS: u64 = 300;
const SIGNAL_WINDOW: u64 = 600;
const MAX_STATE_BYTES: u64 = 128 * 1024;

#[derive(Default, Serialize, Deserialize)]
pub struct RecoveryState {
    pub profile: String,
    pub group: String,
    pub status: String,
    pub active_node: Option<String>,
    pub checked_at: u64,
    pub switched_at: u64,
    pub feedback_status: String,
    pub events: Vec<RecoveryEvent>,
    #[serde(default)]
    failures: u32,
    #[serde(default)]
    feedback_count: u32,
    #[serde(default)]
    feedback_since: u64,
    #[serde(default)]
    cursor: Option<i64>,
    #[serde(default)]
    database_identity: Option<(u64, u64)>,
    #[serde(default)]
    quarantine: BTreeMap<String, u64>,
}

#[derive(Serialize, Deserialize)]
pub struct RecoveryEvent {
    pub at: u64,
    pub reason: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

pub fn read_state(root: &Path) -> Result<RecoveryState, RuntimeError> {
    let path = root.join("recovery.json");
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RecoveryState::default()),
        Ok(m)
            if m.is_file()
                && m.uid() == rustix::process::geteuid().as_raw()
                && m.len() <= MAX_STATE_BYTES
                && m.mode() & 0o077 == 0 =>
        {
            serde_json::from_slice(&fs::read(path).map_err(|_| RuntimeError::PersistentState)?)
                .map_err(|_| RuntimeError::PersistentState)
        }
        _ => Err(RuntimeError::PersistentState),
    }
}

fn save(root: &Path, state: &RecoveryState) -> Result<(), RuntimeError> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|_| RuntimeError::PersistentState)?;
    super::session::replace_private(&root.join("recovery.json"), &bytes)
        .map_err(|_| RuntimeError::PersistentState)
}

fn event(state: &mut RecoveryState, now: u64, reason: &str, to: Option<String>) {
    state.events.push(RecoveryEvent {
        at: now,
        reason: reason.into(),
        from: state.active_node.clone(),
        to,
    });
    if state.events.len() > 32 {
        state.events.remove(0);
    }
}

/// Resolve the selected leaf, detecting loops and missing controller entries.
pub fn selected_leaf(proxies: &ProxiesResponse, group: &str) -> Option<String> {
    let mut name = group;
    let mut seen = BTreeSet::new();
    while seen.insert(name) && seen.len() <= 32 {
        let proxy = proxies.proxies.get(name)?;
        match proxy.now.as_deref() {
            Some(next) => name = next,
            None if proxy.all.is_empty() => return Some(name.to_owned()),
            None => return None,
        }
    }
    None
}

/// Deliberately narrow adapter for current Codex diagnostic events. Unknown
/// errors, auth failures, quotas and service responses never request rotation.
fn is_transport_signal(body: &str) -> bool {
    let message = body.rsplit_once("}: ").map_or(body, |(_, message)| message);
    let lower = message.to_ascii_lowercase();
    if [
        "http 429",
        "http 401",
        "http 403",
        "http 502",
        "http 503",
        "status: 429",
        "status: 401",
        "status: 403",
        "status: 502",
        "status: 503",
        "rate limit",
        "rate_limit",
        "quota",
        "unauthorized",
        "authentication",
        "response.failed",
        "server_error",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        return false;
    }
    [
        "idle timeout waiting for sse",
        "error decoding response body",
        "connection failed: error sending request",
        "stream disconnected before completion: error sending request",
        "websocket stream idle timeout",
        "connection reset by peer",
    ]
    .iter()
    .any(|s| lower.contains(s))
}

fn feedback(path: &Path, state: &mut RecoveryState, now: u64) -> Result<u32, ()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
    if !metadata.is_file() || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(());
    }
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| ())?;
    db.busy_timeout(Duration::from_millis(25)).map_err(|_| ())?;
    let max: i64 = db
        .query_row("SELECT COALESCE(MAX(id),0) FROM logs", [], |r| r.get(0))
        .map_err(|_| ())?;
    let identity = Some((metadata.dev(), metadata.ino()));
    if state.cursor.is_none()
        || state.database_identity != identity
        || state.cursor.is_some_and(|v| v > max)
    {
        state.cursor = Some(max);
        state.database_identity = identity;
        return Ok(0); // Never replay errors predating activation or database rotation.
    }
    let since = now.saturating_sub(SIGNAL_WINDOW).max(state.switched_at);
    let mut statement = db.prepare("SELECT substr(feedback_log_body,1,16384) FROM logs WHERE id>?1 AND id<=?2 AND ts>=?3 AND target='codex_core::responses_retry' AND level='WARN' ORDER BY id LIMIT 128").map_err(|_| ())?;
    let rows = statement
        .query_map((state.cursor, max, since), |r| r.get::<_, String>(0))
        .map_err(|_| ())?;
    let mut count = 0;
    for row in rows {
        if is_transport_signal(&row.map_err(|_| ())?) {
            count += 1;
        }
    }
    state.cursor = Some(max);
    Ok(count)
}

fn target() -> ProbeTarget {
    ProbeTarget::built_in()
        .into_iter()
        .find(|p| p.name() == "Codex")
        .expect("Codex target is built in")
}

async fn confirmed(client: &ApiClient, node: &str, target: &ProbeTarget) -> bool {
    client.probe_delay(node, target).await.is_ok() && client.probe_delay(node, target).await.is_ok()
}

/// A per-tick nonblocking lock serializes the supervisor, an upgrade-time
/// recovery watcher, and explicit repair. Persistent cooldown survives restarts.
pub async fn tick(
    root: &Path,
    profile: &str,
    policy: &FallbackPolicy,
    client: &ApiClient,
    force: bool,
) -> Result<RecoveryOutcome, RuntimeError> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(root.join(".recovery.lock"))
        .map_err(|_| RuntimeError::PersistentState)?;
    if FileExt::try_lock(&lock).is_err() {
        return Ok(RecoveryOutcome::Cooldown);
    }
    let now = unix_seconds_now();
    let mut state = read_state(root)?;
    if state.profile != profile || state.group != policy.group {
        state = RecoveryState {
            profile: profile.into(),
            group: policy.group.clone(),
            ..Default::default()
        };
    }
    if !force
        && now >= state.checked_at
        && now - state.checked_at < CHECK_INTERVAL.saturating_sub(1)
    {
        return Ok(RecoveryOutcome::Cooldown);
    }
    state.checked_at = now;
    let result = recover(root, policy, client, &mut state, now, force).await;
    if result.is_err() {
        state.status = "controller-or-state-error".into();
        event(&mut state, now, "controller-or-state-error", None);
    }
    save(root, &state)?;
    result
}

async fn recover(
    root: &Path,
    policy: &FallbackPolicy,
    client: &ApiClient,
    state: &mut RecoveryState,
    now: u64,
    force: bool,
) -> Result<RecoveryOutcome, RuntimeError> {
    let config = client
        .configuration()
        .await
        .map_err(|_| RuntimeError::ControllerUnavailable)?;
    let proxies = client
        .proxies()
        .await
        .map_err(|_| RuntimeError::ControllerUnavailable)?;
    let auto = format!("{} Auto", policy.group);
    let nodes = proxies
        .proxies
        .get(&auto)
        .map(|p| p.all.clone())
        .unwrap_or_default();
    // Recovery never changes GLOBAL, enters another group's known-good history,
    // or changes the user's chosen operating mode.
    if config.mode.as_deref() != Some("global")
        || proxies.proxies.get("GLOBAL").and_then(|p| p.now.as_deref())
            != Some(policy.group.as_str())
    {
        state.status = "inactive-policy-selection".into();
        return Ok(RecoveryOutcome::NotApplicable);
    }
    if nodes.is_empty() || !proxies.proxies.contains_key(&policy.group) {
        state.status = "policy-group-missing".into();
        return Ok(RecoveryOutcome::Degraded);
    }
    let node = selected_leaf(&proxies, &policy.group);
    if node != state.active_node {
        event(state, now, "observed-selection-change", node.clone());
        state.active_node = node.clone();
        state.feedback_count = 0;
        state.feedback_since = now;
        state.failures = 0;
        // Attribute only future feedback to this newly observed node.
        state.cursor = None;
    }
    if let Some(path) = &policy.codex_log_db {
        match feedback(path, state, now) {
            Ok(count) => {
                state.feedback_status = "watching".into();
                if now < state.feedback_since || now - state.feedback_since > SIGNAL_WINDOW {
                    state.feedback_since = now;
                    state.feedback_count = 0;
                }
                state.feedback_count = state.feedback_count.saturating_add(count);
            }
            Err(()) => state.feedback_status = "unavailable-probes-only".into(),
        }
    } else {
        state.feedback_status = "disabled".into();
    }
    let target = target();
    let healthy = client.probe_delay(&policy.group, &target).await.is_ok();
    state.failures = if healthy {
        0
    } else {
        state.failures.saturating_add(1)
    };
    let stream_trial = healthy && state.feedback_count >= 2;
    if healthy && !stream_trial {
        state.status = "short-probes-healthy-stream-unverified".into();
        return Ok(RecoveryOutcome::AlreadyHealthy);
    }
    state.status = if stream_trial {
        "stream-suspect"
    } else {
        "probe-failed"
    }
    .into();
    if !force && state.failures < 2 && !stream_trial {
        return Ok(RecoveryOutcome::Degraded);
    }
    if !force
        && state.switched_at != 0
        && now >= state.switched_at
        && now - state.switched_at < SWITCH_COOLDOWN
    {
        state.status = "switch-cooldown".into();
        return Ok(RecoveryOutcome::Cooldown);
    }
    if stream_trial
        && state
            .events
            .iter()
            .filter(|e| {
                e.reason == "stream-feedback-trial-switch"
                    && e.at <= now
                    && now - e.at < SIGNAL_WINDOW
            })
            .count()
            >= 2
    {
        state.status = "stream-trial-budget-exhausted".into();
        return Ok(RecoveryOutcome::Degraded);
    }
    state
        .quarantine
        .retain(|_, until| *until > now && *until <= now + QUARANTINE_SECONDS);
    let eligible: Vec<_> = nodes
        .iter()
        .filter(|n| Some(n.as_str()) != node.as_deref() && !state.quarantine.contains_key(*n))
        .cloned()
        .collect();
    let checks = stream::iter(eligible.into_iter().map(|n| {
        let client = client.clone();
        let target = target.clone();
        async move {
            let good = confirmed(&client, &n, &target).await;
            (n, good)
        }
    }))
    .buffered(4)
    .collect::<Vec<_>>()
    .await;
    for (candidate, good) in checks {
        if !good {
            continue;
        }
        // A user action or concurrent hot reload wins over an in-flight recovery.
        let fresh = client
            .proxies()
            .await
            .map_err(|_| RuntimeError::ControllerUnavailable)?;
        if selected_leaf(&fresh, &policy.group) != node
            || fresh.proxies.get("GLOBAL").and_then(|p| p.now.as_deref())
                != Some(policy.group.as_str())
            || !fresh
                .proxies
                .get(&auto)
                .is_some_and(|p| p.all.contains(&candidate))
        {
            state.status = "selection-changed-during-check".into();
            return Ok(RecoveryOutcome::NotApplicable);
        }
        let previous = fresh.proxies.get(&policy.group).and_then(|p| p.now.clone());
        client
            .select_proxy(&policy.group, &candidate)
            .await
            .map_err(|_| RuntimeError::SessionReload)?;
        if !confirmed(client, &policy.group, &target).await {
            if let Some(previous) = previous {
                let _ = client.select_proxy(&policy.group, &previous).await;
            }
            continue;
        }
        if DesiredStateStore::new(root.to_owned())
            .and_then(|s| s.record_selection(&policy.group, &candidate))
            .is_err()
        {
            if let Some(previous) = previous {
                let _ = client.select_proxy(&policy.group, &previous).await;
            }
            return Err(RuntimeError::PersistentState);
        }
        if let Some(node) = &node {
            state
                .quarantine
                .insert(node.clone(), now + QUARANTINE_SECONDS);
        }
        event(
            state,
            now,
            if stream_trial {
                "stream-feedback-trial-switch"
            } else {
                "confirmed-probe-failover"
            },
            Some(candidate.clone()),
        );
        state.active_node = Some(candidate);
        state.switched_at = now;
        state.failures = 0;
        state.feedback_count = 0;
        state.feedback_since = now;
        state.status = if stream_trial {
            "trial-switched-stream-unverified"
        } else {
            "recovered-short-probes"
        }
        .into();
        return Ok(RecoveryOutcome::RecoveredByKnownGood);
    }
    state.status = "no-healthy-in-scope-candidate".into();
    event(state, now, "no-healthy-in-scope-candidate", None);
    Ok(RecoveryOutcome::Degraded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_reader_skips_history_and_never_replays_warning_ids() {
        let dir = std::env::temp_dir().join(format!(
            "mihoterm-feedback-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("logs.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE logs(id INTEGER PRIMARY KEY,ts INTEGER,target TEXT,level TEXT,feedback_log_body TEXT);
            INSERT INTO logs VALUES(1,1000,'codex_core::responses_retry','WARN','idle timeout waiting for SSE');").unwrap();
        let mut state = RecoveryState::default();
        assert_eq!(feedback(&path, &mut state, 1000), Ok(0));
        db.execute_batch("INSERT INTO logs VALUES(2,1001,'codex_core::responses_retry','WARN','error decoding response body');
            INSERT INTO logs VALUES(3,1001,'codex_core::responses_retry','WARN','HTTP 429 error decoding response body');
            INSERT INTO logs VALUES(4,1001,'unrelated','WARN','error decoding response body');
            INSERT INTO logs VALUES(5,1,'codex_core::responses_retry','WARN','error decoding response body');").unwrap();
        assert_eq!(feedback(&path, &mut state, 1001), Ok(1));
        assert_eq!(feedback(&path, &mut state, 1002), Ok(0));
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM logs", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            5
        );
        drop(db);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn feedback_requires_transport_evidence_and_excludes_service_errors() {
        for body in [
            "idle timeout waiting for SSE",
            "Transport error: network error: error decoding response body",
            "Connection failed: error sending request",
        ] {
            assert!(is_transport_signal(body));
        }
        for body in [
            "response.failed error decoding response body",
            "HTTP 429 idle timeout waiting for SSE",
            "HTTP 401 error sending request",
            "HTTP 503 connection reset by peer",
            "unknown failure",
            "completed",
        ] {
            assert!(!is_transport_signal(body));
        }
    }

    #[test]
    fn selected_leaf_rejects_cycles_and_missing_members() {
        let p: ProxiesResponse = serde_json::from_value(serde_json::json!({"proxies": {
            "Root": {"now":"Auto","all":["Auto"]}, "Auto":{"now":"A","all":["A"]},
            "A":{"type":"Http"}, "Loop":{"now":"Loop"}, "Missing":{"now":"Absent"}
        }}))
        .unwrap();
        assert_eq!(selected_leaf(&p, "Root").as_deref(), Some("A"));
        assert_eq!(selected_leaf(&p, "Loop"), None);
        assert_eq!(selected_leaf(&p, "Missing"), None);
    }
}
