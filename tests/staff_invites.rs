//! Real HTTP invitation flow: owner-only initiation, email-bound acceptance,
//! capability separation and immediate revocation. No SMTP traffic is used.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use hireferry::{AppState, auth, config::Config};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tower::ServiceExt;

struct Client {
    app: Router,
    cookies: BTreeMap<String, String>,
    csrf: Option<String>,
}
impl Client {
    fn new(state: &AppState) -> Self {
        Self {
            app: hireferry::app(state.clone()),
            cookies: BTreeMap::new(),
            csrf: None,
        }
    }
    fn owner(state: &AppState) -> Self {
        let mut result = Self::new(state);
        let raw = uuid::Uuid::new_v4().to_string();
        let csrf = uuid::Uuid::new_v4().to_string();
        state
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO sessions(token_hash,admin,csrf,expires_at,recent_auth_at)
             VALUES(?1,1,?2,?3,?4)",
                params![
                    auth::hash_token(&raw),
                    csrf,
                    hireferry::now() + 3600,
                    hireferry::now()
                ],
            )
            .unwrap();
        result.cookies.insert("hf_session".into(), raw);
        result.csrf = Some(csrf);
        result
    }
    async fn request(&mut self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "http://localhost:3000");
        if !self.cookies.is_empty() {
            request = request.header(
                header::COOKIE,
                self.cookies
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if let Some(csrf) = &self.csrf {
            request = request.header("x-csrf-token", csrf);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
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
        let (status, user) = self
            .request(
                "POST",
                "/api/auth/email/verify",
                json!({
                    "email":email,"code":start["dev_code"]
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{user}");
        user["user"]["id"].as_str().unwrap().to_string()
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

fn invite_token(value: &Value) -> String {
    value["invite_url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn owner_invites_only_the_verified_email_and_manager_never_becomes_owner() {
    let state = state();
    let mut owner = Client::owner(&state);
    let mut stranger = Client::new(&state);
    stranger.sign_in("stranger@example.test").await;

    // An anonymous or ordinary user cannot invite a manager.
    let invite = json!({"email":"worker@example.test","role":"manager","send_email":false});
    assert_eq!(
        stranger
            .request("POST", "/api/admin/staff/invitations", invite.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // The default SMTP path is disabled in this fixture: no invitation is persisted.
    assert_eq!(
        owner
            .request(
                "POST",
                "/api/admin/staff/invitations",
                json!({
                    "email":"worker@example.test","role":"manager"
                })
            )
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (status, issued) = owner
        .request("POST", "/api/admin/staff/invitations", invite)
        .await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    assert_eq!(issued["sent"], false);
    let token = invite_token(&issued);
    assert_eq!(token.len(), 43);
    let path = format!("/api/staff/invitations/{token}");
    let accepted = format!("{path}/accept");

    let (_, preview) = stranger.request("GET", &path, json!({})).await;
    assert_eq!(preview["email"], "worker@example.test");
    assert_eq!(
        stranger.request("POST", &accepted, json!({})).await.0,
        StatusCode::FORBIDDEN
    );

    let mut worker = Client::new(&state);
    let id = worker.sign_in("worker@example.test").await;
    // The verified user still must supply a session-bound CSRF token.
    let csrf = worker.csrf.take();
    assert_eq!(
        worker.request("POST", &accepted, json!({})).await.0,
        StatusCode::FORBIDDEN
    );
    worker.csrf = csrf;
    let (status, result) = worker.request("POST", &accepted, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["role"], "manager");

    let (_, session) = worker.request("GET", "/api/session", json!({})).await;
    assert_eq!(session["staff_role"], "manager");
    assert_eq!(session["admin"], false); // No owner Passkey authority.
    assert_eq!(session["super_admin"], false);
    assert_eq!(session["staff"], Value::Null);
    for path in [
        "/api/admin/team",
        "/api/admin/promo-codes",
        "/api/admin/feedback",
    ] {
        assert_eq!(
            worker.request("GET", path, json!({})).await.0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        worker.request("GET", "/api/admin/users", json!({})).await.0,
        StatusCode::OK
    );
    assert_eq!(
        worker.request("GET", "/api/admin/staff", json!({})).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        worker
            .request("GET", "/api/admin/orders", json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        worker
            .request("GET", "/api/admin/overview", json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        worker
            .request(
                "POST",
                "/api/admin/staff/invitations",
                json!({
                    "email":"friend@example.test","role":"manager","send_email":false
                })
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        worker.request("POST", &accepted, json!({})).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        worker.request("GET", &path, json!({})).await.0,
        StatusCode::NOT_FOUND
    );

    // Disabling membership revokes manager operations on the existing session,
    // but preserves the person's separate signed-in account.
    let (status, _) = owner
        .request(
            "PATCH",
            &format!("/api/admin/staff/members/{id}"),
            json!({"active":false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        worker.request("GET", "/api/admin/users", json!({})).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        worker.request("GET", "/api/profile", json!({})).await.0,
        StatusCode::OK
    );
    let (_, session) = worker.request("GET", "/api/session", json!({})).await;
    assert_eq!(session["staff_role"], Value::Null);
}

#[tokio::test]
async fn owner_can_revoke_unaccepted_link_and_listing_never_reveals_tokens() {
    let state = state();
    let mut owner = Client::owner(&state);
    let (_, issued) = owner
        .request(
            "POST",
            "/api/admin/staff/invitations",
            json!({
                "email":"revoke@example.test","role":"manager","send_email":false
            }),
        )
        .await;
    let token = invite_token(&issued);
    let (_, list) = owner.request("GET", "/api/admin/staff", json!({})).await;
    assert_eq!(list["invitations"][0]["email"], "revoke@example.test");
    assert!(!list.to_string().contains(&token));
    let id = issued["id"].as_str().unwrap();
    assert_eq!(
        owner
            .request(
                "DELETE",
                &format!("/api/admin/staff/invitations/{id}"),
                json!({})
            )
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        owner
            .request("GET", &format!("/api/staff/invitations/{token}"), json!({}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let mut worker = Client::new(&state);
    worker.sign_in("revoke@example.test").await;
    assert_eq!(
        worker
            .request(
                "POST",
                &format!("/api/staff/invitations/{token}/accept"),
                json!({})
            )
            .await
            .0,
        StatusCode::CONFLICT
    );
}

#[test]
fn version_ten_feedback_database_upgrades_to_staff_schema_eleven() {
    let state = state();
    let db = state.conn().unwrap();
    db.execute_batch(
        "DROP TABLE staff_invitations; DROP TABLE staff_members; PRAGMA user_version=10;",
    )
    .unwrap();
    hireferry::db::migrate(&db).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        13
    );
    let existing_tables: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table'
             AND name IN ('feedback','staff_members','staff_invitations')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(existing_tables, 3);
    // Reopening a fully upgraded database is idempotent.
    hireferry::db::migrate(&db).unwrap();
}
