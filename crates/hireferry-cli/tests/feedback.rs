use clap::Parser;
use hireferry_cli::{
    Cli, now, run,
    settings::StorageChoice,
    storage::{AccountSubject, Credential, Store},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

struct Mock {
    origin: String,
    scope: &'static str,
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
        assert!(count > 0 && bytes.len() < 16384);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
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
    let admin = scope == "identity admin";
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let received = requests.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = request(&mut socket).await;
            let identity = request.starts_with("GET /oauth/userinfo ");
            received.lock().unwrap().push(request);
            let body = if identity {
                json!({"admin":admin,"user":if admin {Value::Null} else {json!({"id":"member","name":"Member","email":"member@example.test","role":"unassigned"})},
                    "scope":scope,"expires_at":now()+900,
                    "capabilities":{"identity":true,"mcp":scope=="identity mcp","admin":admin}})
            } else { json!({"id":99,"items":[],"status":"resolved"}) }.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    Mock {
        origin,
        scope,
        requests,
        task,
    }
}

fn seed(mock: &Mock, directory: &Path) {
    Store::new(
        directory.to_path_buf(),
        &mock.origin,
        "hireferry-cli",
        StorageChoice::File,
    )
    .save(&Credential {
        server: mock.origin.clone(),
        client_id: "hireferry-cli".into(),
        account: Some(if mock.scope == "identity admin" {
            AccountSubject::Administrator
        } else {
            AccountSubject::User("member".into())
        }),
        scope: mock.scope.into(),
        access_token: "access-secret".into(),
        expires_at: now() + 900,
        refresh_token: "refresh-secret".into(),
        refresh_expires_at: now() + 86400,
        refresh_pending: false,
    })
    .unwrap();
}

fn cli(mock: &Mock, directory: &Path, arguments: &[&str]) -> Cli {
    let mut command = vec![
        "hfy".to_owned(),
        "--server".into(),
        mock.origin.clone(),
        "--config-dir".into(),
        directory.to_string_lossy().into_owned(),
        "--storage".into(),
        "file".into(),
        "--scope".into(),
        if mock.scope == "identity admin" {
            "admin".into()
        } else {
            "identity".into()
        },
        "feedback".into(),
    ];
    command.extend(arguments.iter().map(|value| value.to_string()));
    Cli::try_parse_from(command).unwrap()
}

#[tokio::test]
async fn member_feedback_uses_the_verified_cli_identity_and_plain_text() {
    let mock = mock("identity").await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("private");
    seed(&mock, &directory);
    let result = run(cli(
        &mock,
        &directory,
        &["send", "页面 <script>test</script>", "--category", "idea"],
    ))
    .await
    .unwrap();
    assert_eq!(result["feedback"]["id"], 99);
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].starts_with("POST /api/feedback "));
    assert!(
        requests[1]
            .to_lowercase()
            .contains("authorization: bearer access-secret")
    );
    let body: Value = serde_json::from_str(requests[1].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        body,
        json!({"category":"idea","message":"页面 <script>test</script>"})
    );
    assert!(!result.to_string().contains("access-secret"));
    assert!(!result.to_string().contains("refresh-secret"));
}

#[tokio::test]
async fn administrator_feedback_supports_filters_detail_and_status_changes() {
    let mock = mock("identity admin").await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("private");
    seed(&mock, &directory);
    for arguments in [
        vec![
            "list",
            "--limit",
            "25",
            "--before-id",
            "100",
            "--status",
            "open",
            "--category",
            "bug",
            "--source",
            "cli",
        ],
        vec!["get", "99"],
        vec!["resolve", "99"],
        vec!["resolve", "99", "--reopen"],
    ] {
        run(cli(&mock, &directory, &arguments)).await.unwrap();
    }
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 8);
    assert!(requests[1].starts_with("GET /api/admin/feedback?"));
    let url = url::Url::parse(&format!(
        "{}{}",
        mock.origin,
        requests[1].split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
    for (key, value) in [
        ("limit", "25"),
        ("before_id", "100"),
        ("status", "open"),
        ("category", "bug"),
        ("source", "cli"),
    ] {
        assert_eq!(query.get(key).map(|v| v.as_ref()), Some(value));
    }
    assert!(requests[3].starts_with("GET /api/admin/feedback/99 "));
    for (position, status) in [(5, "resolved"), (7, "open")] {
        assert!(requests[position].starts_with("PATCH /api/admin/feedback/99 "));
        let body: Value =
            serde_json::from_str(requests[position].split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body, json!({"status":status}));
    }
}

#[tokio::test]
async fn member_and_member_mcp_scopes_never_request_administrator_feedback() {
    for scope in ["identity", "identity mcp"] {
        let mock = mock(scope).await;
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("private");
        seed(&mock, &directory);
        for arguments in [vec!["list"], vec!["get", "99"], vec!["resolve", "99"]] {
            assert_eq!(
                run(cli(&mock, &directory, &arguments))
                    .await
                    .unwrap_err()
                    .code,
                "admin_required"
            );
        }
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(
            requests
                .iter()
                .all(|request| request.starts_with("GET /oauth/userinfo "))
        );
    }
}

#[tokio::test]
async fn malformed_feedback_is_rejected_before_sending_a_report() {
    let mock = mock("identity").await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("private");
    seed(&mock, &directory);
    for message in ["a".into(), "x".repeat(2001), "bad\u{001b}text".into()] {
        assert_eq!(
            run(cli(&mock, &directory, &["send", &message]))
                .await
                .unwrap_err()
                .code,
            "invalid_feedback"
        );
    }
    assert!(
        mock.requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.starts_with("GET /oauth/userinfo "))
    );
    for arguments in [
        vec!["hfy", "feedback", "get", "0"],
        vec!["hfy", "feedback", "list", "--source", "other"],
        vec!["hfy", "feedback", "list", "--category", "urgent"],
    ] {
        assert!(Cli::try_parse_from(arguments).is_err());
    }
}
