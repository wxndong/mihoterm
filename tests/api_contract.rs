mod support;

use std::process::Command;

use mihoterm::mihomo::{ApiClient, ApiError};
use support::{spawn_json_once, spawn_snapshot_server};

#[tokio::test]
async fn version_request_uses_prefix_and_bearer_auth() {
    let (controller, request) = spawn_json_once(
        "200 OK",
        r#"{"version":"v1.19.29","meta":true,"premium":false}"#,
    )
    .await;
    let client = ApiClient::new(&controller, Some("test-controller-secret".into()))
        .expect("client should initialize");

    let version = client.version().await.expect("version should load");
    let request = request.await.expect("mock should capture request");

    assert_eq!(version.version, "v1.19.29");
    assert!(version.meta);
    assert!(request.starts_with("GET /api/version HTTP/1.1\r\n"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer test-controller-secret\r\n")
    );
}

#[tokio::test]
async fn proxies_response_exposes_groups_and_delays() {
    let (controller, _) = spawn_json_once(
        "200 OK",
        r#"{
            "proxies": {
                "Auto": {
                    "name": "Auto",
                    "type": "URLTest",
                    "now": "Proxy A",
                    "all": ["Proxy A", "Proxy B"],
                    "alive": true
                },
                "Proxy A": {
                    "name": "Proxy A",
                    "type": "Shadowsocks",
                    "alive": true,
                    "history": [
                        {"time": "2026-07-25T00:00:00Z", "delay": 38}
                    ]
                }
            }
        }"#,
    )
    .await;
    let client = ApiClient::new(&controller, None).expect("client should initialize");

    let proxies = client.proxies().await.expect("proxies should load");

    assert_eq!(proxies.groups().len(), 1);
    assert_eq!(proxies.groups()[0].now.as_deref(), Some("Proxy A"));
    assert_eq!(proxies.proxies["Proxy A"].latest_delay_ms(), Some(38));
}

#[tokio::test]
async fn errors_do_not_include_response_bodies() {
    let (controller, _) = spawn_json_once(
        "401 Unauthorized",
        r#"{"message":"do not expose response-test-secret"}"#,
    )
    .await;
    let client = ApiClient::new(&controller, None).expect("client should initialize");

    let error = client.version().await.expect_err("request should fail");
    let rendered = format!("{error:?} {error}");

    assert_eq!(
        error,
        ApiError::UnexpectedStatus {
            operation: "get version",
            status: 401,
        }
    );
    assert!(!rendered.contains("response-test-secret"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_command_prints_only_sanitized_summary() {
    let controller = spawn_snapshot_server().await;
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_mihoterm"))
            .args(["--controller", &controller, "status"])
            .output()
            .expect("status binary should run")
    })
    .await
    .expect("status task should finish");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        "Mihomo v1.19.29 | mode rule | 1 policy groups\n"
    );
    assert!(output.stderr.is_empty());
}
