use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn first_run_requires_a_terminal_and_creates_private_state() {
    let base = temporary_directory();
    let state = base.join("state");
    let runtime = base.join("runtime");

    let output = Command::new(env!("CARGO_BIN_EXE_mihoterm"))
        .arg("--state-dir")
        .arg(&state)
        .arg("--runtime-dir")
        .arg(runtime)
        .output()
        .expect("mihoterm should run");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(error.contains("first-run setup requires an interactive terminal"));
    assert_eq!(
        fs::metadata(&state)
            .expect("state should exist")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    fs::remove_dir_all(base).expect("test directory should be removed");
}

fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should follow the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "mihoterm-onboarding-cli-test-{}-{nonce}",
        std::process::id()
    ))
}
