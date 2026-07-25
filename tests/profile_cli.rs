use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn local_profile_cli_adds_updates_and_rolls_back() {
    let base = temporary_directory();
    let state = base.join("state");
    let source = base.join("source.yaml");
    fs::create_dir(&base).expect("test directory should be created");
    fs::write(&source, fixture("Proxy A")).expect("source should be written");

    let added = mihoterm(
        &state,
        ["profile", "add", "primary", "--file"],
        Some(&source),
    );
    assert_success(&added);
    assert_eq!(
        String::from_utf8(added.stdout).expect("stdout should be UTF-8"),
        "Added profile primary from local-file.\n"
    );

    fs::write(&source, fixture("Proxy B")).expect("source should be updated");
    assert_success(&mihoterm(&state, ["profile", "update", "primary"], None));

    let listed = mihoterm(&state, ["profile", "list"], None);
    assert_success(&listed);
    assert_eq!(
        String::from_utf8(listed.stdout).expect("stdout should be UTF-8"),
        "primary (backup available)\n"
    );

    assert_success(&mihoterm(&state, ["profile", "rollback", "primary"], None));
    let profile = state.join("profiles/primary/profile.yaml");
    assert!(
        fs::read_to_string(profile)
            .expect("profile should be readable")
            .contains("Proxy A")
    );

    fs::remove_dir_all(base).expect("test directory should be removed");
}

fn mihoterm<const N: usize>(
    state: &Path,
    arguments: [&str; N],
    trailing_path: Option<&Path>,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mihoterm"));
    command.arg("--state-dir").arg(state).args(arguments);
    if let Some(path) = trailing_path {
        command.arg(path);
    }
    command.output().expect("mihoterm should run")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
}

fn fixture(name: &str) -> String {
    format!(
        "proxies:\n  - name: {name}\n    type: ss\n    server: 192.0.2.1\n    port: 443\n    cipher: aes-128-gcm\n    password: fixture-only\n"
    )
}

fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should follow the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "mihoterm-profile-cli-test-{}-{nonce}",
        std::process::id()
    ))
}
