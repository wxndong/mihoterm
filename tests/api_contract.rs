mod support;

use std::process::Command;

use mihoterm::{
    mihomo::{ApiClient, ApiError, OperatingMode},
    probe::ProbeTarget,
};
use support::{spawn_json_once, spawn_snapshot_server};
use url::Url;

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

#[tokio::test]
async fn proxy_selection_encodes_dynamic_paths_and_json() {
    let (controller, request) = spawn_json_once("204 No Content", "").await;
    let client = ApiClient::new(&controller, None).expect("client should initialize");

    client
        .select_proxy("Primary / Auto", "Proxy B")
        .await
        .expect("selection should succeed");
    let request = request.await.expect("mock should capture request");
    let (headers, body) = request
        .split_once("\r\n\r\n")
        .expect("request should contain a body separator");

    assert!(headers.starts_with("PUT /api/proxies/Primary%20%2F%20Auto HTTP/1.1"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body).expect("body should be JSON"),
        serde_json::json!({"name": "Proxy B"})
    );
}

#[tokio::test]
async fn mode_update_uses_the_configs_patch() {
    let (controller, request) = spawn_json_once("204 No Content", "").await;
    let client = ApiClient::new(&controller, None).expect("client should initialize");

    client
        .set_mode(OperatingMode::Global)
        .await
        .expect("mode should update");
    let request = request.await.expect("mock should capture request");

    assert!(request.starts_with("PATCH /api/configs HTTP/1.1\r\n"));
    assert!(request.ends_with(r#"{"mode":"global"}"#));
}

#[tokio::test]
async fn probe_request_preserves_target_and_expected_status() {
    let (controller, request) = spawn_json_once("200 OK", r#"{"delay":47,"meanDelay":51}"#).await;
    let client = ApiClient::new(&controller, None).expect("client should initialize");
    let target = ProbeTarget::new(
        "Example",
        "https://example.com/health?region=test",
        "200-299",
        3_000,
    )
    .expect("probe should be valid");

    let result = client
        .probe_delay("Proxy A", &target)
        .await
        .expect("probe should succeed");
    let request = request.await.expect("mock should capture request");
    let request_target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("request target should exist");
    let request_url =
        Url::parse(&format!("http://mock{request_target}")).expect("target should parse");
    let query = request_url
        .query_pairs()
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(request_url.path(), "/api/proxies/Proxy%20A/delay");
    assert_eq!(
        query.get("url").map(|value| value.as_ref()),
        Some("https://example.com/health?region=test")
    );
    assert_eq!(
        query.get("timeout").map(|value| value.as_ref()),
        Some("3000")
    );
    assert_eq!(
        query.get("expected").map(|value| value.as_ref()),
        Some("200-299")
    );
    assert_eq!(result.delay, 47);
    assert_eq!(result.mean_delay, Some(51));
}
