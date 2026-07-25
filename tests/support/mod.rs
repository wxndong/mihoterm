use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

pub async fn spawn_json_once(status: &str, body: &str) -> (String, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock listener should bind");
    let address = listener
        .local_addr()
        .expect("mock listener should have an address");
    let status = status.to_owned();
    let body = body.to_owned();
    let (request_sender, request_receiver) = oneshot::channel();

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("mock should accept");
        let request = read_request(&mut stream).await;
        request_sender.send(request).ok();

        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("mock should respond");
    });

    (format!("http://{address}/api/"), request_receiver)
}

pub async fn spawn_snapshot_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("snapshot listener should bind");
    let address = listener
        .local_addr()
        .expect("snapshot listener should have an address");

    tokio::spawn(async move {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().await.expect("snapshot should accept");
            let request = read_request(&mut stream).await;
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("request path should exist");
            let body = match path {
                "/api/version" => r#"{"version":"v1.19.29","meta":true}"#,
                "/api/configs" => r#"{"mode":"rule","allow-lan":false}"#,
                "/api/proxies" => {
                    r#"{"proxies":{"Auto":{"name":"Auto","type":"URLTest","now":"Proxy A","all":["Proxy A"]},"Proxy A":{"name":"Proxy A","type":"Shadowsocks","alive":true,"history":[{"time":"2026-07-25T00:00:00Z","delay":32}]}}}"#
                }
                unexpected => panic!("unexpected snapshot path: {unexpected}"),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("snapshot should respond");
        }
    });

    format!("http://{address}/api/")
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    let mut expected_length = None;

    loop {
        let read = stream.read(&mut buffer).await.expect("mock should read");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        assert!(request.len() < 64 * 1024, "request is unexpectedly large");

        if expected_length.is_none()
            && let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let body_start = header_end + 4;
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or_default();
            expected_length = Some(body_start + content_length);
        }

        if expected_length.is_some_and(|length| request.len() >= length) {
            break;
        }
    }

    String::from_utf8(request).expect("request should be UTF-8")
}
