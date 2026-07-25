use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
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
        let mut request = Vec::with_capacity(1024);
        let mut buffer = [0_u8; 1024];

        loop {
            let read = stream.read(&mut buffer).await.expect("mock should read");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
            assert!(
                request.len() < 16 * 1024,
                "request headers are unexpectedly large"
            );
        }

        let request = String::from_utf8(request).expect("request should be UTF-8");
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
