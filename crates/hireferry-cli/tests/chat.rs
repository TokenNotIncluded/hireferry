use clap::Parser;
use hireferry_cli::{
    Cli, now, run,
    server::Server,
    settings::StorageChoice,
    storage::{AccountSubject, Credential, Store},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

struct Mock {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn request(socket: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 2048];
    loop {
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0 && bytes.len() < 65536);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return String::from_utf8(bytes).unwrap();
            }
        }
    }
}

async fn mock(scope: &'static str) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let received = requests.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let incoming = request(&mut socket).await;
            let (content_type, body) = if incoming.starts_with("GET /oauth/userinfo ") {
                ("application/json", json!({"admin":false,"user":{"id":"member","name":"Member","email":"member@example.test","role":"seeker"},"scope":scope,"expires_at":now()+900,"capabilities":{"identity":true,"mcp":scope=="identity mcp","admin":false}}).to_string())
            } else if incoming.starts_with("GET /api/chat/events/stream") {
                (
                    "text/event-stream",
                    "id: 8\nevent: message.delta\ndata: {\"delta\":\"聘渡\",\"sequence\":1}\n\n"
                        .into(),
                )
            } else if incoming.starts_with("POST /mcp ") {
                let value: Value =
                    serde_json::from_str(incoming.split("\r\n\r\n").nth(1).unwrap()).unwrap();
                if value.get("id").is_none() {
                    received.lock().unwrap().push(incoming);
                    socket.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    continue;
                }
                (
                    "application/json",
                    json!({"jsonrpc":"2.0","id":value["id"],"result":{"tools":[]}}).to_string(),
                )
            } else {
                (
                    "application/json",
                    json!({"id":7,"items":[],"format":"markdown"}).to_string(),
                )
            };
            received.lock().unwrap().push(incoming);
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    Mock {
        origin,
        requests,
        task,
    }
}

fn seed(mock: &Mock, directory: &std::path::Path, scope: &str) {
    Store::new(
        directory.to_path_buf(),
        &mock.origin,
        "hireferry-cli",
        StorageChoice::File,
    )
    .save(&Credential {
        server: mock.origin.clone(),
        client_id: "hireferry-cli".into(),
        account: Some(AccountSubject::User("member".into())),
        scope: scope.into(),
        access_token: "chat-access-secret".into(),
        expires_at: now() + 900,
        refresh_token: "chat-refresh-secret".into(),
        refresh_expires_at: now() + 86400,
        refresh_pending: false,
    })
    .unwrap();
}

fn cli(mock: &Mock, directory: &std::path::Path, args: &[&str]) -> Cli {
    let mut command = vec![
        "hfy".to_owned(),
        "--server".into(),
        mock.origin.clone(),
        "--config-dir".into(),
        directory.display().to_string(),
        "--storage".into(),
        "file".into(),
    ];
    command.extend(args.iter().map(|s| s.to_string()));
    Cli::try_parse_from(command).unwrap()
}

#[tokio::test]
async fn chat_uses_verified_identity_paths_formats_and_saved_cursor() {
    let mock = mock("identity mcp").await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("private");
    seed(&mock, &directory, "identity mcp");
    for args in [
        vec!["chat", "list"],
        vec!["chat", "get", "platform_member"],
        vec![
            "chat",
            "messages",
            "platform_member",
            "--after",
            "3",
            "--limit",
            "20",
        ],
        vec![
            "chat",
            "send",
            "platform_member",
            "root = Text(\"Hello\")",
            "--request-id",
            "stable-retry-id",
            "--format",
            "openui",
        ],
        vec![
            "chat",
            "watch",
            "--conversation-id",
            "platform_member",
            "--after",
            "7",
            "--once",
        ],
    ] {
        let result = run(cli(&mock, &directory, &args)).await.unwrap();
        assert!(!result.to_string().contains("chat-access-secret"));
    }
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 10);
    assert!(requests[1].starts_with("GET /api/chat/conversations "));
    assert!(requests[3].starts_with("GET /api/chat/conversations/platform_member "));
    assert!(requests[5].contains("limit=20&after=3"));
    let sent: Value = serde_json::from_str(requests[7].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(sent["format"], "openui");
    assert_eq!(sent["client_id"], "stable-retry-id");
    assert!(requests[9].to_lowercase().contains("last-event-id: 7"));
    for request in requests.iter() {
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer chat-access-secret")
        );
    }
}

#[tokio::test]
async fn identity_only_connection_cannot_call_chat_but_config_contains_no_tokens() {
    let mock = mock("identity").await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("private");
    seed(&mock, &directory, "identity");
    assert_eq!(
        run(cli(&mock, &directory, &["chat", "list"]))
            .await
            .unwrap_err()
            .code,
        "mcp_scope_required"
    );
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
    let config = run(cli(&mock, &directory, &["mcp", "config"]))
        .await
        .unwrap();
    assert_eq!(config["mcpServers"]["hireferry"]["command"], "hfy");
    assert!(!config.to_string().contains("secret"));
}

#[tokio::test]
async fn mcp_http_bridge_preserves_ids_notifications_and_protocol_version() {
    let mock = mock("identity mcp").await;
    let server = Server::new(&mock.origin).unwrap();
    let response = server
        .mcp_rpc(
            "bridge-token",
            &json!({"jsonrpc":"2.0","id":"agent-42","method":"tools/list"}),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response["id"], "agent-42");
    assert!(
        server
            .mcp_rpc(
                "bridge-token",
                &json!({"jsonrpc":"2.0","method":"notifications/initialized"})
            )
            .await
            .unwrap()
            .is_none()
    );
    let requests = mock.requests.lock().unwrap();
    assert!(
        requests[0]
            .to_lowercase()
            .contains("mcp-protocol-version: 2025-11-25")
    );
    assert!(
        requests[0]
            .to_lowercase()
            .contains("authorization: bearer bridge-token")
    );
}

#[test]
fn conversation_ids_cannot_change_api_path_or_query() {
    assert_eq!(
        Server::conversation_path("platform_member", Some("messages")).unwrap(),
        "/api/chat/conversations/platform_member/messages"
    );
    let path =
        Server::conversation_path("other?token=oops#part/../admin", Some("messages")).unwrap();
    assert!(!path.contains('?') && !path.contains('#'));
    assert!(path.ends_with("/messages"));
    assert!(path.contains("%2F"));
}

#[tokio::test]
async fn native_upload_receives_an_ack_before_stdin_finishes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = Server::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut received = Vec::new();
        let mut buffer = [0u8; 4096];
        while !received.windows(7).any(|bytes| bytes == b"\"start\"") {
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            received.extend_from_slice(&buffer[..count]);
        }
        assert!(
            String::from_utf8_lossy(&received)
                .starts_with("POST /api/chat/conversations/platform_member/stream ")
        );
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
        let ack = "{\"type\":\"ack\",\"message_id\":9,\"sequence\":0,\"status\":\"streaming\"}\n";
        socket
            .write_all(format!("{:x}\r\n{ack}\r\n", ack.len()).as_bytes())
            .await
            .unwrap();
        while !received.windows(8).any(|bytes| bytes == b"\"finish\"") {
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            received.extend_from_slice(&buffer[..count]);
        }
        let ack = "{\"type\":\"ack\",\"message_id\":9,\"sequence\":1,\"status\":\"complete\"}\n";
        socket
            .write_all(format!("{:x}\r\n{ack}\r\n0\r\n\r\n", ack.len()).as_bytes())
            .await
            .unwrap();
    });
    let (mut input, reader) = tokio::io::duplex(4096);
    let (release, wait) = tokio::sync::oneshot::channel::<()>();
    let producer = tokio::spawn(async move {
        input
            .write_all(b"{\"type\":\"start\",\"client_id\":\"native\",\"body\":\"first chunk\"}\n")
            .await
            .unwrap();
        wait.await.unwrap();
        input
            .write_all(b"{\"type\":\"finish\",\"sequence\":1}\n")
            .await
            .unwrap();
    });
    let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(reader));
    let mut response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        server.chat_upload("upload-token", "platform_member", body),
    )
    .await
    .unwrap()
    .unwrap();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), response.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&first).contains("streaming"));
    release.send(()).unwrap();
    let second = tokio::time::timeout(std::time::Duration::from_secs(5), response.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&second).contains("complete"));
    peer.await.unwrap();
    producer.await.unwrap();
}
