//! Employee business access never becomes ownership of platform credentials.
//! All identities and credentials in this file are isolated local fixtures.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use hireferry::{AppState, access, admin, auth, chat, config::Config};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    app: Router,
}

struct Employee {
    id: String,
    user_id: String,
    session: String,
    csrf: String,
}

impl Employee {
    fn headers(&self, webmcp: bool) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            format!("hf_session={}", self.session).parse().unwrap(),
        );
        headers.insert("origin", "http://localhost:3000".parse().unwrap());
        headers.insert("x-csrf-token", self.csrf.parse().unwrap());
        if webmcp {
            headers.insert("x-hireferry-ai", "webmcp".parse().unwrap());
        }
        headers
    }
}

impl Fixture {
    fn new() -> Self {
        let state = AppState::new(Config {
            development: true,
            database: ":memory:".into(),
            ..Default::default()
        })
        .unwrap();
        let app = hireferry::app(state.clone());
        Self { state, app }
    }

    fn employee(&self) -> Employee {
        let employee = Employee {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            session: uuid::Uuid::new_v4().to_string(),
            csrf: uuid::Uuid::new_v4().to_string(),
        };
        let db = self.state.conn().unwrap();
        db.execute(
            "INSERT INTO users(id,email,name,role,created_at,updated_at) VALUES(?1,?2,'Employee','unassigned',?3,?3)",
            params![employee.user_id, format!("{}@example.invalid", employee.user_id), hireferry::now()],
        ).unwrap();
        db.execute(
            "INSERT INTO sessions(token_hash,user_id,csrf,expires_at,recent_auth_at) VALUES(?1,?2,?3,?4,?5)",
            params![auth::hash_token(&employee.session), employee.user_id, employee.csrf, hireferry::now()+3600, hireferry::now()],
        ).unwrap();
        db.execute(
            "INSERT INTO team_members(id,kind,user_id,name,created_by,created_at,updated_at) VALUES(?1,'human',?2,'Employee','test',?3,?3)",
            params![employee.id, employee.user_id, hireferry::now()],
        ).unwrap();
        employee
    }

    fn ai(&self) -> (String, String, String) {
        let member = uuid::Uuid::new_v4().to_string();
        let token_id = uuid::Uuid::new_v4().to_string();
        let token = format!("hf_staff_{}", hex::encode(rand::random::<[u8; 32]>()));
        let db = self.state.conn().unwrap();
        db.execute(
            "INSERT INTO team_members(id,kind,name,created_by,created_at,updated_at) VALUES(?1,'ai','AI assistant','test',?2,?2)",
            params![member, hireferry::now()],
        ).unwrap();
        db.execute(
            "INSERT INTO team_ai_tokens(id,member_id,token_hash,created_at,expires_at) VALUES(?1,?2,?3,?4,?5)",
            params![token_id, member, auth::hash_token(&token), hireferry::now(), hireferry::now()+3600],
        ).unwrap();
        (member, token_id, token)
    }
}

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    headers: HeaderMap,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    *builder.headers_mut().unwrap() = headers;
    if path == "/mcp" {
        builder = builder
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25");
    }
    let body = if let Some(body) = body {
        builder = builder.header("content-type", "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn bearer(raw: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {raw}").parse().unwrap());
    headers
}

fn denied(response: &(StatusCode, Value)) {
    assert!(
        matches!(response.0, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
        "expected an access denial, got {response:?}"
    );
}

#[tokio::test]
async fn employee_enters_console_without_receiving_super_admin_credentials() {
    let f = Fixture::new();
    let employee = f.employee();
    let session = request(&f.app, "GET", "/api/session", employee.headers(false), None).await;
    assert_eq!(session.0, StatusCode::OK);
    assert_eq!(session.1["admin"], true);
    assert_eq!(session.1["super_admin"], false);
    assert_eq!(session.1["staff"]["id"], employee.id);
    assert_eq!(session.1["user"]["role"], "unassigned");
    assert!(
        !auth::require_session(&f.state, &employee.headers(false))
            .unwrap()
            .admin
    );
    assert!(auth::require_admin(&f.state, &employee.headers(false)).is_err());
    let users = request(
        &f.app,
        "GET",
        "/api/admin/users",
        employee.headers(false),
        None,
    )
    .await;
    assert_eq!(users.0, StatusCode::OK);
    for (method, path, body) in [
        (
            "POST",
            "/api/admin/auth/bootstrap",
            Some(json!({"password":"employee-has-no-bootstrap-secret"})),
        ),
        ("GET", "/api/admin/passkeys", None),
        (
            "POST",
            "/api/admin/passkeys/register/start",
            Some(json!({"name":"Employee key"})),
        ),
        ("GET", "/api/admin/tokens", None),
        (
            "POST",
            "/api/admin/tokens",
            Some(json!({"name":"Unauthorized token"})),
        ),
        ("GET", "/api/admin/team", None),
        (
            "POST",
            "/api/admin/team/ai",
            Some(json!({"name":"Unauthorized AI"})),
        ),
        (
            "POST",
            "/api/admin/team/invitations",
            Some(
                json!({"email":"unauthorized@example.invalid","name":"Unauthorized","send_email":false}),
            ),
        ),
    ] {
        denied(&request(&f.app, method, path, employee.headers(false), body).await);
    }
    denied(
        &request(
            &f.app,
            "POST",
            "/api/cli/authorization",
            employee.headers(false),
            Some(json!({
                "flow":"oauth","client_id":"hireferry-cli","response_type":"code",
                "redirect_uri":"http://127.0.0.1:49999/callback","scope":"admin",
                "state":"employee-administrator-scope-denial",
                "code_challenge":"A".repeat(43),"code_challenge_method":"S256","approve":true
            })),
        )
        .await,
    );
    let granted: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM cli_grants WHERE admin=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(granted, 0);
    let (_, _, ai_token) = f.ai();
    let mut mixed = employee.headers(false);
    mixed.insert(
        "authorization",
        format!("Bearer {ai_token}").parse().unwrap(),
    );
    for headers in [mixed, employee.headers(true)] {
        denied(
            &request(
                &f.app,
                "POST",
                "/api/team/invitations/accept",
                headers,
                Some(json!({"token":"hf_invite_invalid"})),
            )
            .await,
        );
    }
    let target = f.employee();
    let path = format!(
        "/api/chat/conversations/platform_{}/messages",
        target.user_id
    );
    let mut no_csrf = employee.headers(false);
    no_csrf.remove("x-csrf-token");
    denied(
        &request(
            &f.app,
            "POST",
            &path,
            no_csrf,
            Some(json!({"body":"hello","client_id":"missing-csrf"})),
        )
        .await,
    );
}

#[tokio::test]
async fn employee_personal_bearer_never_inherits_console_access() {
    let f = Fixture::new();
    let employee = f.employee();
    let token = format!("hf_{}", hex::encode(rand::random::<[u8; 32]>()));
    {
        let db = f.state.conn().unwrap();
        db.execute(
            "UPDATE users SET role='seeker' WHERE id=?1",
            [&employee.user_id],
        )
        .unwrap();
        db.execute(
            "INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES(?1,?2,'Personal',?3,?4,?5)",
            params![uuid::Uuid::new_v4().to_string(), employee.user_id, auth::hash_token(&token), hireferry::now(), hireferry::now()+3600],
        ).unwrap();
    }
    let p = access::principal(&f.state, &bearer(&token), false).unwrap();
    assert!(!p.admin);
    denied(&request(&f.app, "GET", "/api/admin/users", bearer(&token), None).await);
    let catalog = request(
        &f.app,
        "POST",
        "/mcp",
        bearer(&token),
        Some(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})),
    )
    .await;
    assert_eq!(catalog.0, StatusCode::OK);
    assert!(
        catalog.1["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| !tool["name"].as_str().unwrap().starts_with("admin_"))
    );

    // Removing an employee does not suspend their customer account, and
    // neither that account's Cookie nor its personal token can restore staff.
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE team_members SET active=0 WHERE id=?1",
            [&employee.id],
        )
        .unwrap();
    let session = request(&f.app, "GET", "/api/session", employee.headers(false), None).await;
    assert_eq!(session.0, StatusCode::OK);
    assert_eq!(session.1["admin"], false);
    assert_eq!(session.1["staff"], Value::Null);
    assert_eq!(session.1["user"]["id"], employee.user_id);
    for headers in [
        employee.headers(false),
        employee.headers(true),
        bearer(&token),
    ] {
        denied(&request(&f.app, "GET", "/api/admin/users", headers, None).await);
    }
    assert!(
        !access::principal(&f.state, &bearer(&token), false)
            .unwrap()
            .admin
    );
}

#[test]
fn employee_revocation_logout_and_suspension_invalidate_captured_principals() {
    let f = Fixture::new();
    let employee = f.employee();
    let headers = employee.headers(false);
    let p = access::principal(&f.state, &headers, false).unwrap();
    assert!(admin::get_user(&f.state, &p, &employee.user_id).is_ok());
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE team_members SET active=0 WHERE id=?1",
            [&employee.id],
        )
        .unwrap();
    assert!(admin::get_user(&f.state, &p, &employee.user_id).is_err());
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE team_members SET active=1 WHERE id=?1",
            [&employee.id],
        )
        .unwrap();
    assert!(admin::get_user(&f.state, &p, &employee.user_id).is_ok());
    f.state
        .conn()
        .unwrap()
        .execute(
            "DELETE FROM sessions WHERE token_hash=?1",
            [auth::hash_token(&employee.session)],
        )
        .unwrap();
    assert!(admin::get_user(&f.state, &p, &employee.user_id).is_err());
    assert!(access::principal(&f.state, &headers, false).is_err());

    let employee = f.employee();
    let p = access::principal(&f.state, &employee.headers(true), false).unwrap();
    assert!(p.admin && p.via_ai);
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id=?1", [&employee.user_id])
        .unwrap();
    assert!(admin::get_user(&f.state, &p, &employee.user_id).is_err());
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=0 WHERE id=?1", [&employee.user_id])
        .unwrap();
    assert!(admin::get_user(&f.state, &p, &employee.user_id).is_err());
}

#[tokio::test]
async fn ai_employee_uses_mcp_with_separate_revocable_identity() {
    let f = Fixture::new();
    let target = f.employee();
    let (member, token_id, token) = f.ai();
    let p = access::principal(&f.state, &bearer(&token), false).unwrap();
    assert!(p.admin && p.via_ai);
    assert_eq!(p.actor, format!("staff_ai:{token_id}"));
    let response = request(&f.app, "POST", "/mcp", bearer(&token), Some(json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"send_message","arguments":{
            "conversation_id":format!("platform_{}",target.user_id),"body":"AI staff reply","client_id":"ai-reply"
        }}
    }))).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["result"]["isError"], false);
    assert_eq!(response.1["result"]["structuredContent"]["via_ai"], true);
    for path in [
        "/api/admin/passkeys",
        "/api/admin/tokens",
        "/api/admin/team",
    ] {
        denied(&request(&f.app, "GET", path, bearer(&token), None).await);
    }
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE team_members SET active=0 WHERE id=?1", [&member])
        .unwrap();
    assert!(admin::get_user(&f.state, &p, &target.user_id).is_err());
    denied(&request(&f.app, "GET", "/api/admin/users", bearer(&token), None).await);
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE team_members SET active=1 WHERE id=?1", [&member])
        .unwrap();
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE team_ai_tokens SET revoked=1 WHERE id=?1",
            [&token_id],
        )
        .unwrap();
    assert!(admin::get_user(&f.state, &p, &target.user_id).is_err());
    assert!(access::principal(&f.state, &bearer(&token), false).is_err());
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE team_ai_tokens SET revoked=0,expires_at=?2 WHERE id=?1",
            params![token_id, hireferry::now() - 1],
        )
        .unwrap();
    assert!(access::principal(&f.state, &bearer(&token), false).is_err());
}

#[test]
fn employees_keep_independent_chat_reads_and_message_idempotency() {
    let f = Fixture::new();
    let first = f.employee();
    let second = f.employee();
    let target = f.employee();
    let a = access::principal(&f.state, &first.headers(false), false).unwrap();
    let a_webmcp = access::principal(&f.state, &first.headers(true), false).unwrap();
    let b = access::principal(&f.state, &second.headers(false), false).unwrap();
    let (_, _, token) = f.ai();
    let ai = access::principal(&f.state, &bearer(&token), false).unwrap();
    let thread = format!("platform_{}", target.user_id);
    let mut ids = Vec::new();
    for (p, body) in [
        (&a, "First employee"),
        (&b, "Second employee"),
        (&ai, "AI employee"),
    ] {
        ids.push(
            chat::send_message(
                &f.state,
                p,
                &thread,
                chat::MessageInput {
                    body: body.into(),
                    client_id: "same-client-id".into(),
                },
            )
            .unwrap()["id"]
                .as_i64()
                .unwrap(),
        );
    }
    assert!(ids[0] < ids[1] && ids[1] < ids[2]);
    let retry = chat::send_message(
        &f.state,
        &a_webmcp,
        &thread,
        chat::MessageInput {
            body: "First employee".into(),
            client_id: "same-client-id".into(),
        },
    )
    .unwrap();
    assert_eq!(retry["id"], ids[0]);
    assert_eq!(
        chat::mark_read(&f.state, &a, &thread, ids[2]).unwrap()["unread_count"],
        0
    );
    assert_eq!(
        chat::get_conversation(&f.state, &a_webmcp, &thread).unwrap()["unread_count"],
        0
    );
    assert_eq!(
        chat::get_conversation(&f.state, &b, &thread).unwrap()["unread_count"],
        2
    );
    let actors = f
        .state
        .conn()
        .unwrap()
        .prepare("SELECT DISTINCT sender_actor FROM chat_messages WHERE conversation_id=?1")
        .unwrap()
        .query_map([thread], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(actors.len(), 3);
    assert!(actors.contains(&a.actor) && actors.contains(&b.actor) && actors.contains(&ai.actor));
}

#[test]
fn staff_chat_attribution_is_viewer_specific_and_hidden_from_customers() {
    let f = Fixture::new();
    let first = f.employee();
    let second = f.employee();
    let customer = f.employee();
    {
        let db = f.state.conn().unwrap();
        for (id, name) in [(&first.id, "Internal Alex"), (&second.id, "Internal Bea")] {
            db.execute(
                "UPDATE team_members SET name=?2 WHERE id=?1",
                params![id, name],
            )
            .unwrap();
        }
        db.execute(
            "UPDATE team_members SET active=0 WHERE id=?1",
            [&customer.id],
        )
        .unwrap();
    }
    let a = access::principal(&f.state, &first.headers(false), false).unwrap();
    let b = access::principal(&f.state, &second.headers(false), false).unwrap();
    let client = access::principal(&f.state, &customer.headers(false), false).unwrap();
    assert!(!client.admin);
    let (ai_id, _, token) = f.ai();
    let ai = access::principal(&f.state, &bearer(&token), false).unwrap();
    let owner = access::Principal {
        user_id: None,
        admin: true,
        via_ai: false,
        actor: "admin".into(),
    };
    let thread = format!("platform_{}", customer.user_id);
    for (index, (p, name)) in [
        (&a, "Internal Alex"),
        (&b, "Internal Bea"),
        (&ai, "AI assistant"),
        (&owner, "超级管理员"),
    ]
    .into_iter()
    .enumerate()
    {
        let sent = chat::send_message(
            &f.state,
            p,
            &thread,
            chat::MessageInput {
                body: format!("Reply {index}"),
                client_id: format!("reply-{index}"),
            },
        )
        .unwrap();
        assert_eq!(sent["is_own"], true);
        assert_eq!(sent["staff_name"], name);
        assert_eq!(sent["sender_kind"], "platform");
    }
    let customer_sent = chat::send_message(
        &f.state,
        &client,
        &thread,
        chat::MessageInput {
            body: "Customer reply".into(),
            client_id: "customer-reply".into(),
        },
    )
    .unwrap();
    assert!(customer_sent.get("is_own").is_none());
    assert!(customer_sent.get("staff_name").is_none());

    for (viewer, own_index) in [(&a, 0), (&b, 1)] {
        let page = chat::messages(&f.state, viewer, &thread, 0, 100).unwrap();
        let updates = chat::updates(&f.state, viewer, 0, 100).unwrap();
        for result in [&page, &updates] {
            let items = result["items"].as_array().unwrap();
            assert_eq!(items.len(), 5);
            assert_eq!(items[0]["staff_name"], "Internal Alex");
            assert_eq!(items[1]["staff_name"], "Internal Bea");
            assert_eq!(items[2]["staff_name"], "AI assistant");
            assert_eq!(items[3]["staff_name"], "超级管理员");
            assert_eq!(items[4]["staff_name"], Value::Null);
            for (index, message) in items.iter().enumerate() {
                assert_eq!(message["is_own"], index == own_index);
                assert!(message.get("sender_actor").is_none());
                assert!(message.get("sender_key").is_none());
            }
        }
    }
    let history = chat::messages_before(&f.state, &a, &thread, i64::MAX, 100).unwrap();
    assert_eq!(history["items"][1]["staff_name"], "Internal Bea");
    for result in [
        chat::messages(&f.state, &client, &thread, 0, 100).unwrap(),
        chat::updates(&f.state, &client, 0, 100).unwrap(),
    ] {
        for message in result["items"].as_array().unwrap() {
            assert!(message.get("staff_name").is_none());
            assert!(message.get("is_own").is_none());
            assert!(message.get("sender_actor").is_none());
        }
        let serialized = result.to_string();
        for private in [
            "Internal Alex",
            "Internal Bea",
            "AI assistant",
            &a.actor,
            &b.actor,
            &token,
        ] {
            assert!(!serialized.contains(private));
        }
    }
    // Old registrations sometimes used an email as a fallback name. It must
    // never become internal chat attribution either.
    {
        let db = f.state.conn().unwrap();
        db.execute("UPDATE team_members SET name=(SELECT email FROM users WHERE id=team_members.user_id) WHERE id=?1", [&first.id]).unwrap();
        db.execute(
            "UPDATE team_members SET name='robot@example.invalid' WHERE id=?1",
            [&ai_id],
        )
        .unwrap();
    }
    let page = chat::messages(&f.state, &b, &thread, 0, 100).unwrap();
    assert_eq!(page["items"][0]["staff_name"], "员工");
    assert_eq!(page["items"][2]["staff_name"], "平台AI");
    assert!(
        !page["items"][0]["staff_name"]
            .as_str()
            .unwrap()
            .contains('@')
    );
}
