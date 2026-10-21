//! Suspension uses synthetic accounts and the real HTTP/SQLite paths. No mail,
//! Google traffic, webhook delivery or real user account is involved.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use hireferry::{AppState, access::Principal, admin, auth, chat, config::Config};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tower::ServiceExt;

struct Client {
    app: Router,
    cookies: BTreeMap<String, String>,
    csrf: Option<String>,
    token: Option<String>,
}
impl Client {
    fn new(state: &AppState) -> Self {
        Self {
            app: hireferry::app(state.clone()),
            cookies: BTreeMap::new(),
            csrf: None,
            token: None,
        }
    }
    async fn request(&mut self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "http://localhost:3000");
        if !self.cookies.is_empty() {
            builder = builder.header(
                header::COOKIE,
                self.cookies
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if let Some(csrf) = &self.csrf {
            builder = builder.header("x-csrf-token", csrf);
        }
        if let Some(token) = &self.token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        for value in response.headers().get_all(header::SET_COOKIE) {
            let (key, value) = value
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .split_once('=')
                .unwrap();
            if value.is_empty() {
                self.cookies.remove(key);
            } else {
                self.cookies.insert(key.to_owned(), value.to_owned());
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if let Some(csrf) = value["csrf"].as_str() {
            self.csrf = Some(csrf.to_owned());
        }
        (status, value)
    }
    async fn sign_in(&mut self, email: &str) -> String {
        let (status, start) = self
            .request("POST", "/api/auth/email/start", json!({"email":email}))
            .await;
        assert_eq!(status, StatusCode::OK, "{start}");
        let (status, result) = self
            .request(
                "POST",
                "/api/auth/email/verify",
                json!({"email":email,"code":start["dev_code"]}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        result["user"]["id"].as_str().unwrap().to_owned()
    }
    fn administrator(state: &AppState) -> Self {
        let mut result = Self::new(state);
        let raw = uuid::Uuid::new_v4().to_string();
        let csrf = uuid::Uuid::new_v4().to_string();
        state.conn().unwrap().execute("INSERT INTO sessions(token_hash,admin,csrf,expires_at,recent_auth_at) VALUES(?1,1,?2,?3,?4)", params![auth::hash_token(&raw),csrf,hireferry::now()+3600,hireferry::now()]).unwrap();
        result.cookies.insert("hf_session".into(), raw);
        result.csrf = Some(csrf);
        result
    }
}
fn state() -> AppState {
    AppState::new(Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    })
    .unwrap()
}
fn clear_limits(state: &AppState) {
    state
        .conn()
        .unwrap()
        .execute("DELETE FROM auth_limits", [])
        .unwrap();
}

#[tokio::test]
async fn suspension_revokes_credentials_and_webhooks_without_resurrecting_them_on_restore() {
    let state = state();
    let mut member = Client::new(&state);
    let id = member.sign_in("suspended@example.test").await;
    state
        .conn()
        .unwrap()
        .execute("UPDATE users SET role='seeker' WHERE id=?1", [&id])
        .unwrap();
    let mut other = Client::new(&state);
    let other_id = other.sign_in("unaffected@example.test").await;
    let mut operator = Client::administrator(&state);
    let path = format!("/api/admin/users/{id}/access");
    let change = json!({"banned":true,"reason":"Repeated unsolicited messages"});

    assert_eq!(
        member.request("POST", &path, change.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    let csrf = operator.csrf.take();
    assert_eq!(
        operator.request("POST", &path, change.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    operator.csrf = csrf;
    assert_eq!(
        operator
            .request("POST", &path, json!({"banned":true,"reason":"  "}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        operator.request("GET", &path, json!({})).await.1["access"]["banned"],
        false
    );

    let (status, issued) = member
        .request("POST", "/api/tokens", json!({"name":"Synthetic client"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    let token_id = issued["id"].as_str().unwrap().to_owned();
    let mut ai = Client::new(&state);
    ai.token = Some(issued["token"].as_str().unwrap().to_owned());
    assert_eq!(
        ai.request(
            "POST",
            "/mcp",
            json!({"jsonrpc":"2.0","id":1,"method":"ping"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, endpoint) = member.request("POST", "/api/webhooks", json!({"name":"Synthetic endpoint","url":"https://hooks.example.com/hireferry","events":["message.created"]})).await;
    assert_eq!(status, StatusCode::OK, "{endpoint}");
    let endpoint_id = endpoint["endpoint"]["id"].as_str().unwrap();
    let (status, queued) = member
        .request(
            "POST",
            &format!("/api/webhooks/{endpoint_id}/test"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{queued}");

    clear_limits(&state);
    let mut pending = Client::new(&state);
    let (_, start) = pending
        .request(
            "POST",
            "/api/auth/email/start",
            json!({"email":"suspended@example.test"}),
        )
        .await;
    let old_code = start["dev_code"].as_str().unwrap().to_owned();
    let (status, result) = operator.request("POST", &path, change).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["user"]["banned"], true);
    assert_eq!(result["access"]["reason"], "Repeated unsolicited messages");
    assert_eq!(result["access"]["changed_by"], "admin");
    {
        let db = state.conn().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sessions WHERE user_id=?1",
                [&id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert!(
            db.query_row(
                "SELECT revoked FROM api_tokens WHERE id=?1",
                [&token_id],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        assert_eq!(db.query_row("SELECT count(*) FROM auth_challenges WHERE kind='email' AND subject='suspended@example.test'", [], |r|r.get::<_,i64>(0)).unwrap(), 0);
        assert!(
            !db.query_row(
                "SELECT enabled FROM webhook_endpoints WHERE id=?1",
                [endpoint_id],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        assert_eq!(
            db.query_row(
                "SELECT status FROM webhook_deliveries WHERE endpoint_id=?1",
                [endpoint_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "cancelled"
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM audit WHERE action='user_suspend' AND target=?1",
                [&id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    assert!(member.request("GET", "/api/session", json!({})).await.1["user"].is_null());
    assert_eq!(
        member.request("GET", "/api/profile", json!({})).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        member.request("GET", "/api/webhooks", json!({})).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        ai.request(
            "POST",
            "/mcp",
            json!({"jsonrpc":"2.0","id":2,"method":"ping"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        other.request("GET", "/api/session", json!({})).await.1["user"]["id"],
        other_id
    );
    let stale = Principal {
        user_id: Some(id.clone()),
        admin: false,
        via_ai: false,
        actor: id.clone(),
    };
    assert!(
        chat::send_message(
            &state,
            &stale,
            &format!("platform_{id}"),
            chat::MessageInput {
                body: "Forbidden".into(),
                client_id: "banned-attempt".into()
            }
        )
        .is_err()
    );
    clear_limits(&state);
    let mut blocked = Client::new(&state);
    let (status, neutral) = blocked
        .request(
            "POST",
            "/api/auth/email/start",
            json!({"email":"suspended@example.test"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(neutral, json!({"sent":true}));
    assert!(!blocked.cookies.contains_key("hf_email_challenge"));

    let (status, restored) = operator
        .request(
            "POST",
            &path,
            json!({"banned":false,"reason":"Appeal reviewed; access restored"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(restored["access"]["banned"], false);
    assert_eq!(
        member.request("GET", "/api/profile", json!({})).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        ai.request(
            "POST",
            "/mcp",
            json!({"jsonrpc":"2.0","id":3,"method":"ping"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        pending
            .request(
                "POST",
                "/api/auth/email/verify",
                json!({"email":"suspended@example.test","code":old_code})
            )
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        !state
            .conn()
            .unwrap()
            .query_row(
                "SELECT enabled FROM webhook_endpoints WHERE id=?1",
                [endpoint_id],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
    clear_limits(&state);
    assert_eq!(member.sign_in("suspended@example.test").await, id);
    assert_eq!(
        other.request("GET", &path, json!({})).await.0,
        StatusCode::FORBIDDEN
    );
}

#[test]
fn admin_filters_and_pages_follow_configured_limits() {
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.limits.list_page_size = 2;
    cfg.limits.list_max_page = 3;
    let state = AppState::new(cfg).unwrap();
    {
        let db = state.conn().unwrap();
        for i in 0..5 {
            db.execute("INSERT INTO users(id,email,role,banned,created_at,updated_at) VALUES(?1,?2,'seeker',?3,?4,?4)", params![format!("user-{i}"),format!("page-{i}@example.test"), i==0, i]).unwrap();
        }
    }
    let admin = Principal {
        user_id: None,
        admin: true,
        via_ai: false,
        actor: "admin".into(),
    };
    let active = admin::list_users(
        &state,
        &admin,
        &admin::QueryInput {
            status: Some("active".into()),
            page: Some(2),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(active["total"], 4);
    assert_eq!(active["page_size"], 2);
    assert_eq!(active["page"], 2);
    assert_eq!(active["items"].as_array().unwrap().len(), 2);
    let suspended = admin::list_users(
        &state,
        &admin,
        &admin::QueryInput {
            status: Some("suspended".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(suspended["total"], 1);
    assert_eq!(suspended["items"][0]["id"], "user-0");
    let capped = admin::list_users(
        &state,
        &admin,
        &admin::QueryInput {
            page: Some(i64::MAX),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(capped["page"], 3);
    assert_eq!(capped["items"].as_array().unwrap().len(), 1);
}
