use clap::Parser;
use hireferry_cli::{
    Cli, now, run,
    settings::StorageChoice,
    storage::{AccountSubject, Credential, Store},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

struct Mock {
    origin: String,
    refreshes: Arc<AtomicUsize>,
    authorizations: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn mock(lose_refresh_response: bool) -> Mock {
    mock_with_identity(lose_refresh_response, "identity", "member").await
}

async fn mock_with_identity(
    lose_refresh_response: bool,
    scope: &'static str,
    user_id: &'static str,
) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let refreshes = Arc::new(AtomicUsize::new(0));
    let authorizations = Arc::new(AtomicUsize::new(0));
    let refresh_counter = refreshes.clone();
    let authorization_counter = authorizations.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let path = request.split_whitespace().nth(1).unwrap();
            let body = match path {
                "/oauth/userinfo" => json!({
                    "admin":false,"user":{"id":user_id,"name":"New member","email":"member@example.test","role":"unassigned"},
                    "scope":scope,"expires_at":now()+900,
                    "ai_eligibility":{"eligible":false,"reason":"choose_role_first","role":"unassigned"},
                    "capabilities":{"identity":true,"mcp":scope == "identity mcp","admin":false}
                }),
                "/oauth/token" => {
                    let form = request.split("\r\n\r\n").nth(1).unwrap();
                    let fields: std::collections::HashMap<_, _> = url::form_urlencoded::parse(form.as_bytes()).collect();
                    assert_eq!(fields.get("grant_type").unwrap(), "refresh_token");
                    assert_eq!(fields.get("client_id").unwrap(), "hireferry-cli");
                    let count = refresh_counter.fetch_add(1, Ordering::SeqCst) + 1;
                    if lose_refresh_response { continue; }
                    json!({"access_token":format!("new-access-{count}"),"token_type":"Bearer","expires_in":900,
                        "refresh_token":format!("new-refresh-{count}"),"refresh_expires_in":86400,"scope":scope})
                }
                "/oauth/revoke" => json!({}),
                _ => { authorization_counter.fetch_add(1, Ordering::SeqCst); json!({"unexpected":true}) }
            }.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    Mock {
        origin,
        refreshes,
        authorizations,
        task,
    }
}

async fn read_request(socket: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 2048];
    loop {
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0);
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

fn cli(mock: &Mock, directory: &Path, command: &[&str]) -> Cli {
    let mut arguments = vec![
        "hfy".to_owned(),
        "--server".into(),
        mock.origin.clone(),
        "--config-dir".into(),
        directory.to_string_lossy().into_owned(),
        "--storage".into(),
        "file".into(),
        "--scope".into(),
        "identity".into(),
    ];
    arguments.extend(command.iter().map(|value| value.to_string()));
    Cli::try_parse_from(arguments).unwrap()
}

fn seed(mock: &Mock, directory: &Path, expired: bool) -> Store {
    let store = Store::new(
        directory.to_path_buf(),
        &mock.origin,
        "hireferry-cli",
        StorageChoice::File,
    );
    store
        .save(&Credential {
            server: mock.origin.clone(),
            client_id: "hireferry-cli".into(),
            account: Some(AccountSubject::User("member".into())),
            scope: "identity".into(),
            access_token: "old-access-secret".into(),
            expires_at: now() + if expired { -1 } else { 900 },
            refresh_token: "old-refresh-secret".into(),
            refresh_expires_at: now() + 86400,
            refresh_pending: false,
        })
        .unwrap();
    store
}

#[tokio::test]
async fn valid_login_reuses_the_session_without_starting_consent() {
    let mock = mock(false).await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("private");
    seed(&mock, &directory, false);
    let result = run(cli(&mock, &directory, &["auth", "login", "--device"]))
        .await
        .unwrap();
    assert_eq!(result["authenticated"], true);
    assert_eq!(result["reused"], true);
    assert_eq!(result["identity"]["capabilities"]["identity"], true);
    assert_eq!(result["identity"]["capabilities"]["mcp"], false);
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 0);
    assert_eq!(mock.authorizations.load(Ordering::SeqCst), 0);
    assert!(!result.to_string().contains("old-access-secret"));
    assert!(!result.to_string().contains("old-refresh-secret"));
}

#[tokio::test]
async fn concurrent_commands_refresh_only_once_and_save_both_rotated_tokens() {
    let mock = mock(false).await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("private");
    let store = seed(&mock, &directory, true);
    let original_deadline = store.load().unwrap().unwrap().refresh_expires_at;
    let (first, second) = tokio::join!(
        run(cli(&mock, &directory, &["whoami"])),
        run(cli(&mock, &directory, &["capabilities"])),
    );
    assert!(first.is_ok());
    assert!(second.is_ok());
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 1);
    let credential = store.load().unwrap().unwrap();
    assert_eq!(credential.access_token, "new-access-1");
    assert_eq!(credential.refresh_token, "new-refresh-1");
    assert!(!credential.refresh_pending);
    assert!(credential.refresh_expires_at <= original_deadline);
    let result = run(cli(&mock, &directory, &["auth", "refresh"]))
        .await
        .unwrap();
    assert_eq!(result["refreshed"], true);
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_lost_refresh_response_is_never_replayed_automatically() {
    let mock = mock(true).await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("private");
    let store = seed(&mock, &directory, true);
    let first = run(cli(&mock, &directory, &["whoami"])).await.unwrap_err();
    assert_eq!(first.code, "refresh_uncertain");
    assert!(store.load().unwrap().unwrap().refresh_pending);
    let next = run(cli(&mock, &directory, &["auth", "refresh"]))
        .await
        .unwrap_err();
    assert_eq!(next.code, "refresh_uncertain");
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 1);
    let output: Value = serde_json::to_value(first).unwrap();
    assert!(!output.to_string().contains("old-refresh-secret"));
}

#[tokio::test]
async fn identity_login_reuses_a_wider_grant_and_reports_the_actual_scope() {
    let mock = mock_with_identity(false, "identity mcp", "member").await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("private");
    let store = seed(&mock, &directory, true);
    let mut credential = store.load().unwrap().unwrap();
    credential.scope = "identity mcp".into();
    store.save(&credential).unwrap();
    let result = run(cli(&mock, &directory, &["auth", "login", "--device"]))
        .await
        .unwrap();
    assert_eq!(result["reused"], true);
    assert_eq!(result["identity"]["scope"], "identity mcp");
    assert_eq!(result["identity"]["capabilities"]["mcp"], true);
    assert_eq!(mock.authorizations.load(Ordering::SeqCst), 0);
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_rotated_session_cannot_change_its_bound_account() {
    let mock = mock_with_identity(false, "identity", "different-member").await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("private");
    let store = seed(&mock, &directory, true);
    let error = run(cli(&mock, &directory, &["whoami"])).await.unwrap_err();
    assert_eq!(error.code, "account_mismatch");
    assert!(store.load().unwrap().is_none());
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 1);
    let output = serde_json::to_string(&error).unwrap();
    for secret in [
        "old-access-secret",
        "old-refresh-secret",
        "new-access-1",
        "new-refresh-1",
    ] {
        assert!(!output.contains(secret));
    }
}
