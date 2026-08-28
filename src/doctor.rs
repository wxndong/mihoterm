//! Bounded, read-only diagnostics for clients that inherited an old session.

use std::{
    fs::{self, File},
    io::Read,
    os::unix::fs::MetadataExt,
    path::Path,
};

const MAX_PROCESSES: usize = 8 * 1024;
const MAX_ENVIRONMENT_BYTES: u64 = 256 * 1024;
const MAX_TOTAL_ENVIRONMENT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_COMMAND_BYTES: u64 = 256;
const SESSION_PREFIX: &[u8] = b"MIHOTERM_PROXY_SESSION=";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleClient {
    pub pid: u32,
    pub command: String,
}

#[must_use]
pub fn stale_clients(current_session: Option<&str>) -> Vec<StaleClient> {
    stale_clients_in(Path::new("/proc"), current_session)
}

fn stale_clients_in(proc_root: &Path, current_session: Option<&str>) -> Vec<StaleClient> {
    stale_clients_in_with_limits(
        proc_root,
        current_session,
        MAX_PROCESSES,
        MAX_ENVIRONMENT_BYTES,
        MAX_TOTAL_ENVIRONMENT_BYTES,
    )
}

fn stale_clients_in_with_limits(
    proc_root: &Path,
    current_session: Option<&str>,
    max_processes: usize,
    max_environment_bytes: u64,
    mut remaining_environment_bytes: u64,
) -> Vec<StaleClient> {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return Vec::new();
    };
    let current_uid = rustix::process::geteuid().as_raw();
    let current_pid = std::process::id();
    let mut clients = Vec::new();
    let mut processes = entries
        .flatten()
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()
                .and_then(|value| value.parse::<u32>().ok())
                .map(|pid| (pid, entry.path()))
        })
        .collect::<Vec<_>>();
    processes.sort_unstable_by_key(|(pid, _)| *pid);

    for (pid, path) in processes.into_iter().take(max_processes) {
        if pid == current_pid
            || fs::metadata(&path).map_or(true, |metadata| metadata.uid() != current_uid)
        {
            continue;
        }
        let Some(marker) = process_session_marker(
            &path,
            max_environment_bytes,
            &mut remaining_environment_bytes,
        ) else {
            if remaining_environment_bytes == 0 {
                break;
            }
            continue;
        };
        if current_session == Some(marker.as_str()) {
            continue;
        }
        clients.push(StaleClient {
            pid,
            command: process_command(&path),
        });
    }
    clients
}

fn process_session_marker(
    process: &Path,
    per_process_limit: u64,
    remaining_bytes: &mut u64,
) -> Option<String> {
    let contents =
        read_bounded_with_budget(&process.join("environ"), per_process_limit, remaining_bytes)?;
    contents
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(SESSION_PREFIX))
        .and_then(|value| std::str::from_utf8(value).ok())
        .filter(|value| valid_marker(value))
        .map(str::to_owned)
}

fn process_command(process: &Path) -> String {
    let raw = read_bounded(&process.join("comm"), MAX_COMMAND_BYTES).unwrap_or_default();
    let text = String::from_utf8_lossy(&raw);
    let sanitized = text
        .trim()
        .chars()
        .take(48)
        .map(|character| {
            if character.is_ascii_graphic() || character == ' ' {
                character
            } else {
                '?'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "unknown".to_owned()
    } else {
        sanitized
    }
}

fn read_bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let mut contents = Vec::new();
    file.take(limit + 1).read_to_end(&mut contents).ok()?;
    (contents.len() as u64 <= limit).then_some(contents)
}

fn read_bounded_with_budget(path: &Path, limit: u64, remaining: &mut u64) -> Option<Vec<u8>> {
    if *remaining == 0 {
        return None;
    }
    let allowed = limit.min(*remaining);
    let read_limit = allowed.saturating_add(1).min(*remaining);
    let file = File::open(path).ok()?;
    let mut contents = Vec::new();
    file.take(read_limit).read_to_end(&mut contents).ok()?;
    *remaining -= contents.len() as u64;
    (contents.len() as u64 <= allowed).then_some(contents)
}

fn valid_marker(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::{stale_clients_in, stale_clients_in_with_limits};

    #[test]
    fn reports_only_other_valid_session_markers_and_sanitizes_commands() {
        let root = temporary_directory();
        fs::create_dir_all(root.join("101")).expect("fixture should be created");
        fs::create_dir(root.join("102")).expect("fixture should be created");
        fs::create_dir(root.join("not-a-pid")).expect("fixture should be created");
        fs::write(
            root.join("101/environ"),
            format!("PATH=/bin\0MIHOTERM_PROXY_SESSION={}\0", "11".repeat(16)),
        )
        .expect("environment should be written");
        fs::write(root.join("101/comm"), b"code\nterminal\n").expect("command should be written");
        fs::write(
            root.join("102/environ"),
            format!("MIHOTERM_PROXY_SESSION={}\0", "22".repeat(16)),
        )
        .expect("environment should be written");
        fs::write(root.join("102/comm"), b"codex\n").expect("command should be written");

        let clients = stale_clients_in(&root, Some(&"22".repeat(16)));

        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].pid, 101);
        assert_eq!(clients[0].command, "code?terminal");
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn ignores_malformed_markers() {
        let root = temporary_directory();
        fs::create_dir_all(root.join("201")).expect("fixture should be created");
        fs::write(
            root.join("201/environ"),
            b"MIHOTERM_PROXY_SESSION=not-a-session\0",
        )
        .expect("environment should be written");

        assert!(stale_clients_in(&root, None).is_empty());
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn honors_a_global_environment_read_budget_in_pid_order() {
        let root = temporary_directory();
        let first = format!("MIHOTERM_PROXY_SESSION={}\0", "11".repeat(16));
        for (pid, marker) in [(301, &first), (302, &first)] {
            fs::create_dir_all(root.join(pid.to_string())).expect("fixture should be created");
            fs::write(root.join(format!("{pid}/environ")), marker)
                .expect("environment should be written");
            fs::write(root.join(format!("{pid}/comm")), b"codex\n")
                .expect("command should be written");
        }

        let clients = stale_clients_in_with_limits(&root, None, 2, 1024, first.len() as u64);

        assert_eq!(
            clients.iter().map(|client| client.pid).collect::<Vec<_>>(),
            vec![301]
        );
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    fn temporary_directory() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should follow the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mihoterm-doctor-test-{}-{nonce}",
            std::process::id()
        ))
    }
}
