//! Exercise the complete employee invitation flow against the real router and
//! SQLite. Email sign-in uses local development codes; no external mail is sent.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use hireferry::{AppState, auth, config::Config, now};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tower::ServiceExt;

struct Client {
    app: Router,
    cookies: BTreeMap<String, String>,
    csrf: Option<String>,
    extra: Vec<(&'static str, String)>,
}

impl Client {
    fn new(state: &AppState) -> Self {
        Self {
            app: hireferry::app(state.clone()),
            cookies: BTreeMap::new(),
            csrf: None,
            extra: vec![],
        }
    }

    fn owner(state: &AppState) -> Self {
        let mut client = Self::new(state);
        let raw = uuid::Uuid::new_v4().to_string();
        let csrf = uuid::Uuid::new_v4().to_string();
        state.conn().unwrap().execute(
            "INSERT INTO sessions(token_hash,admin,csrf,expires_at,recent_auth_at) VALUES(?1,1,?2,?3,?4)",
            params![auth::hash_token(&raw),csrf,now()+3600,now()],
        ).unwrap();
        client.cookies.insert("hf_session".into(), raw);
        client.csrf = Some(csrf);
        client
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
        for (name, value) in &self.extra {
            request = request.header(*name, value);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        assert_eq!(response.headers()["cache-control"], "no-store");
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
                self.cookies.insert(key.into(), value.into());
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if let Some(csrf) = value["csrf"].as_str() {
            self.csrf = Some(csrf.into());
        }
        (status, value)
    }

    async fn sign_in(&mut self, email: &str) -> Value {
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
        result["user"].clone()
    }

    async fn invite(&mut self, email: &str) -> Value {
        let (status, result) = self
            .request(
                "POST",
                "/api/admin/team/invitations",
                json!({"email":email,"name":"HR colleague","send_email":false}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        result
    }

    async fn accept(&mut self, invitation: &Value) -> (StatusCode, Value) {
        self.request(
            "POST",
            "/api/team/invitations/accept",
            json!({"token":token(invitation)}),
        )
        .await
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

fn token(invitation: &Value) -> &str {
    invitation["invitation_url"]
        .as_str()
        .unwrap()
        .split_once('#')
        .unwrap()
        .1
}

#[tokio::test]
async fn invitation_requires_current_owner_browser_csrf_and_recent_passkey() {
    let s = state();
    let mut anonymous = Client::new(&s);
    let body = json!({"email":"colleague@example.invalid"});
    assert_eq!(
        anonymous
            .request("POST", "/api/admin/team/invitations", body.clone())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let mut owner = Client::owner(&s);
    let csrf = owner.csrf.take();
    assert_eq!(
        owner
            .request("POST", "/api/admin/team/invitations", body.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    owner.csrf = csrf;
    owner.extra.push(("x-hireferry-ai", "webmcp".into()));
    assert_eq!(
        owner
            .request("POST", "/api/admin/team/invitations", body.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    owner.extra = vec![(
        "authorization",
        "Bearer hf_admin_not_an_owner_browser".into(),
    )];
    assert_eq!(
        owner
            .request("POST", "/api/admin/team/invitations", body.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    owner.extra.clear();
    s.conn()
        .unwrap()
        .execute(
            "UPDATE sessions SET recent_auth_at=?1 WHERE admin=1",
            [now() - 601],
        )
        .unwrap();
    assert_eq!(
        owner
            .request("POST", "/api/admin/team/invitations", body.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        s.conn()
            .unwrap()
            .query_row("SELECT count(*) FROM team_invitations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn new_employee_verifies_email_then_explicitly_accepts_once_without_customer_role() {
    let s = state();
    let mut owner = Client::owner(&s);
    let invitation = owner.invite(" New.Staff@Example.Invalid ").await;
    assert!(
        invitation["invitation_url"]
            .as_str()
            .unwrap()
            .starts_with("http://localhost:3000/team/invite#hf_invite_")
    );
    assert_eq!(invitation["delivery_status"], "not_sent");
    let mut employee = Client::new(&s);
    for _ in 0..2 {
        let (status, preview) = employee
            .request(
                "POST",
                "/api/team/invitations/preview",
                json!({"token":token(&invitation)}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(preview["status"], "pending");
        assert!(preview.get("email").is_none());
        assert_eq!(preview["email_matches"], Value::Null);
    }
    assert_eq!(
        employee.accept(&invitation).await.0,
        StatusCode::UNAUTHORIZED
    );
    let user = employee.sign_in("new.staff@example.invalid").await;
    assert_eq!(user["role"], "unassigned");
    let csrf = employee.csrf.take();
    assert_eq!(employee.accept(&invitation).await.0, StatusCode::FORBIDDEN);
    employee.csrf = csrf;
    let (status, accepted) = employee.accept(&invitation).await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted["member"]["role"], "staff");
    assert_eq!(employee.accept(&invitation).await.0, StatusCode::CONFLICT);
    let (_, session) = employee.request("GET", "/api/session", Value::Null).await;
    assert_eq!(session["admin"], true);
    assert_eq!(session["super_admin"], false);
    assert_eq!(session["user"]["role"], "unassigned");
    assert_eq!(session["staff"]["id"], accepted["member"]["id"]);
    assert_eq!(
        employee
            .request("GET", "/api/admin/users", Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        employee
            .request("GET", "/api/admin/team", Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (_, listed) = owner.request("GET", "/api/admin/team", Value::Null).await;
    assert_eq!(listed["members"].as_array().unwrap().len(), 1);
    assert_eq!(listed["invitations"][0]["status"], "accepted");
    assert!(!listed.to_string().contains(token(&invitation)));
    let stored: String = s
        .conn()
        .unwrap()
        .query_row("SELECT token_hash FROM team_invitations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, auth::hash_token(token(&invitation)));
}

#[tokio::test]
async fn wrong_email_root_and_ai_cannot_claim_a_human_invitation() {
    let s = state();
    let mut owner = Client::owner(&s);
    let invitation = owner.invite("right@example.invalid").await;
    assert_eq!(owner.accept(&invitation).await.0, StatusCode::FORBIDDEN);
    let mut wrong = Client::new(&s);
    wrong.sign_in("wrong@example.invalid").await;
    assert_eq!(wrong.accept(&invitation).await.0, StatusCode::FORBIDDEN);
    let mut right = Client::new(&s);
    right.sign_in("right@example.invalid").await;
    right
        .extra
        .push(("authorization", "Bearer hf_staff_other_identity".into()));
    assert_eq!(right.accept(&invitation).await.0, StatusCode::FORBIDDEN);
    right.extra = vec![("x-hireferry-ai", "webmcp".into())];
    assert_eq!(right.accept(&invitation).await.0, StatusCode::FORBIDDEN);
    right.extra.clear();
    assert_eq!(right.accept(&invitation).await.0, StatusCode::OK);
}

#[tokio::test]
async fn resend_rotates_the_link_and_revoke_or_expiry_prevents_joining() {
    let s = state();
    let mut owner = Client::owner(&s);
    let invitation = owner.invite("rotate@example.invalid").await;
    let id = invitation["invitation"]["id"].as_str().unwrap();
    let route = format!("/api/admin/team/invitations/{id}/resend");
    assert_eq!(
        owner
            .request("POST", &route, json!({"send_email":false}))
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    s.conn()
        .unwrap()
        .execute("UPDATE team_invitations SET updated_at=?1", [now() - 61])
        .unwrap();
    let (status, reissued) = owner
        .request("POST", &route, json!({"send_email":false}))
        .await;
    assert_eq!(status, StatusCode::OK, "{reissued}");
    assert_ne!(token(&invitation), token(&reissued));
    let mut employee = Client::new(&s);
    employee.sign_in("rotate@example.invalid").await;
    assert_eq!(employee.accept(&invitation).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        owner
            .request(
                "DELETE",
                &format!("/api/admin/team/invitations/{id}"),
                Value::Null
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(employee.accept(&reissued).await.0, StatusCode::CONFLICT);
    assert_eq!(
        owner
            .request("POST", &route, json!({"send_email":false}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let expired = owner.invite("rotate@example.invalid").await;
    s.conn()
        .unwrap()
        .execute(
            "UPDATE team_invitations SET expires_at=?1 WHERE status='pending'",
            [now() - 1],
        )
        .unwrap();
    assert_eq!(employee.accept(&expired).await.0, StatusCode::CONFLICT);
    let replacement = owner.invite("rotate@example.invalid").await;
    assert_eq!(employee.accept(&replacement).await.0, StatusCode::OK);
    assert_eq!(employee.accept(&expired).await.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn duplicate_and_concurrent_accepts_do_not_overfill_the_team() {
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.team.max_members = 1;
    let s = AppState::new(cfg).unwrap();
    let mut owner = Client::owner(&s);
    let first = owner.invite("first@example.invalid").await;
    assert_eq!(
        owner
            .request(
                "POST",
                "/api/admin/team/invitations",
                json!({"email":"FIRST@example.invalid"})
            )
            .await
            .0,
        StatusCode::CONFLICT
    );
    let second = owner.invite("second@example.invalid").await;
    let mut one = Client::new(&s);
    let mut two = Client::new(&s);
    one.sign_in("first@example.invalid").await;
    two.sign_in("second@example.invalid").await;
    let (a, b) = tokio::join!(one.accept(&first), two.accept(&second));
    assert_eq!(
        [a.0, b.0].iter().filter(|&&s| s == StatusCode::OK).count(),
        1
    );
    assert_eq!(
        [a.0, b.0]
            .iter()
            .filter(|&&s| s == StatusCode::CONFLICT)
            .count(),
        1
    );
    assert_eq!(
        s.conn()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM team_members WHERE active=1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn disabled_smtp_does_not_claim_to_send_and_pending_limits_are_configurable() {
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.team.max_pending_invitations = 1;
    cfg.team.invitation_ttl_hours = 2;
    let s = AppState::new(cfg).unwrap();
    let mut owner = Client::owner(&s);
    let (status, _) = owner
        .request(
            "POST",
            "/api/admin/team/invitations",
            json!({"email":"mail@example.invalid","send_email":true}),
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let invitation = owner.invite("link@example.invalid").await;
    assert_eq!(
        invitation["invitation"]["expires_at"].as_i64().unwrap()
            - invitation["invitation"]["created_at"].as_i64().unwrap(),
        7200
    );
    assert_eq!(
        owner
            .request(
                "POST",
                "/api/admin/team/invitations",
                json!({"email":"over-limit@example.invalid"})
            )
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        s.conn()
            .unwrap()
            .query_row("SELECT count(*) FROM team_invitations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn failed_local_smtp_preserves_copyable_invitation_and_reports_failure() {
    // The listener closes before SMTP negotiation, so no recipient or message
    // can be delivered. This validates the real transport failure path.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mail_server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        drop(socket);
    });
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.smtp.enabled = true;
    cfg.smtp.host = "127.0.0.1".into();
    cfg.smtp.port = port;
    cfg.smtp.from = "HireFerry <no-reply@example.invalid>".into();
    cfg.smtp.reply_to = "support@example.invalid".into();
    let s = AppState::new(cfg).unwrap();
    let mut owner = Client::owner(&s);
    let (status, invitation) = owner
        .request(
            "POST",
            "/api/admin/team/invitations",
            json!({"email":"no-delivery@example.invalid","send_email":true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{invitation}");
    assert_eq!(invitation["delivery_status"], "failed");
    assert_eq!(invitation["invitation"]["status"], "pending");
    let mut anonymous = Client::new(&s);
    let (_, preview) = anonymous
        .request(
            "POST",
            "/api/team/invitations/preview",
            json!({"token":token(&invitation)}),
        )
        .await;
    assert_eq!(preview["status"], "pending");
    mail_server.await.unwrap();
}

#[tokio::test]
async fn ai_credentials_are_individual_and_never_revived_by_reenabling_a_member() {
    let s = state();
    let mut owner = Client::owner(&s);
    let (status, ai) = owner
        .request(
            "POST",
            "/api/admin/team/ai",
            json!({"name":"Matching assistant"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{ai}");
    let id = ai["member"]["id"].as_str().unwrap();
    let raw = ai["token"].as_str().unwrap();
    assert!(raw.starts_with("hf_staff_"));
    assert!(
        hireferry::team::verify_ai_token(&s.conn().unwrap(), raw)
            .unwrap()
            .is_some()
    );
    let route = format!("/api/admin/team/members/{id}");
    assert_eq!(
        owner
            .request("PATCH", &route, json!({"active":false}))
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        hireferry::team::verify_ai_token(&s.conn().unwrap(), raw)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        owner
            .request("PATCH", &route, json!({"active":true}))
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        hireferry::team::verify_ai_token(&s.conn().unwrap(), raw)
            .unwrap()
            .is_none()
    );
    let (status, rotated) = owner
        .request("POST", &format!("{route}/token"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    assert!(
        hireferry::team::verify_ai_token(&s.conn().unwrap(), rotated["token"].as_str().unwrap())
            .unwrap()
            .is_some()
    );
    let (_, listed) = owner.request("GET", "/api/admin/team", Value::Null).await;
    assert!(!listed.to_string().contains(raw));
    assert!(
        !listed
            .to_string()
            .contains(rotated["token"].as_str().unwrap())
    );
}

#[tokio::test]
async fn revoking_during_smtp_does_not_block_or_return_a_superseded_link() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (connected, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = tokio::sync::oneshot::channel();
    let mail_server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        connected.send(()).unwrap();
        wait.await.unwrap();
        drop(socket);
    });
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.smtp.enabled = true;
    cfg.smtp.host = "127.0.0.1".into();
    cfg.smtp.port = port;
    cfg.smtp.from = "HireFerry <no-reply@example.invalid>".into();
    let s = AppState::new(cfg).unwrap();
    let mut sending_owner = Client::owner(&s);
    let mut revoking_owner = Client::owner(&s);
    let sending = tokio::spawn(async move {
        sending_owner
            .request(
                "POST",
                "/api/admin/team/invitations",
                json!({"email":"cancel-during-send@example.invalid","send_email":true}),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), ready)
        .await
        .unwrap()
        .unwrap();
    let id: String = s
        .conn()
        .unwrap()
        .query_row("SELECT id FROM team_invitations", [], |r| r.get(0))
        .unwrap();
    let (status, _) = revoking_owner
        .request(
            "DELETE",
            &format!("/api/admin/team/invitations/{id}"),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    release.send(()).unwrap();
    let (status, response) = sending.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT, "{response}");
    assert!(response.get("invitation_url").is_none());
    assert_eq!(
        s.conn()
            .unwrap()
            .query_row("SELECT status FROM team_invitations", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "revoked"
    );
    mail_server.await.unwrap();
}

#[test]
fn schema_nine_upgrade_preserves_existing_identities_and_rolls_back_on_failure() {
    let s = state();
    let _owner = Client::owner(&s);
    let db = s.conn().unwrap();
    let downgrade="DROP TRIGGER team_member_disabled; DROP TRIGGER team_account_suspended;
        DROP TABLE team_ai_tokens; DROP TABLE team_invitations; DROP TABLE team_members; PRAGMA user_version=8;";
    db.execute_batch(downgrade).unwrap();
    hireferry::db::migrate(&db).unwrap();
    hireferry::db::migrate(&db).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        9
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM sessions WHERE admin=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute_batch(downgrade).unwrap();
    db.execute_batch("CREATE TABLE team_invitations(sentinel TEXT);")
        .unwrap();
    assert!(hireferry::db::migrate(&db).is_err());
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        8
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='team_members'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM sessions WHERE admin=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn customer_moderation_cannot_be_used_to_manage_other_employees() {
    let s = state();
    let mut owner = Client::owner(&s);
    let mut colleague = Client::new(&s);
    colleague.sign_in("operator@example.invalid").await;
    let invitation = owner.invite("operator@example.invalid").await;
    assert_eq!(colleague.accept(&invitation).await.0, StatusCode::OK);
    let mut target = Client::new(&s);
    let target_user = target.sign_in("employee@example.invalid").await;
    let invitation = owner.invite("employee@example.invalid").await;
    assert_eq!(target.accept(&invitation).await.0, StatusCode::OK);
    let mut pending = Client::new(&s);
    let pending_user = pending.sign_in("invited@example.invalid").await;
    owner.invite("invited@example.invalid").await;
    let (_, ai) = owner
        .request("POST", "/api/admin/team/ai", json!({"name":"AI colleague"}))
        .await;
    let mut ai_client = Client::new(&s);
    ai_client.extra.push((
        "authorization",
        format!("Bearer {}", ai["token"].as_str().unwrap()),
    ));
    let (_, legacy) = owner
        .request(
            "POST",
            "/api/admin/tokens",
            json!({"name":"Legacy platform AI"}),
        )
        .await;
    let mut legacy_client = Client::new(&s);
    legacy_client.extra.push((
        "authorization",
        format!("Bearer {}", legacy["token"].as_str().unwrap()),
    ));
    for client in [&mut colleague, &mut ai_client, &mut legacy_client] {
        for user in [&target_user, &pending_user] {
            let id = user["id"].as_str().unwrap();
            assert_eq!(
                client
                    .request(
                        "POST",
                        &format!("/api/admin/users/{id}/access"),
                        json!({"banned":true,"reason":"Must not manage a teammate"})
                    )
                    .await
                    .0,
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                client
                    .request(
                        "PATCH",
                        &format!("/api/admin/users/{id}"),
                        json!({"banned":true})
                    )
                    .await
                    .0,
                StatusCode::FORBIDDEN
            );
        }
    }
    // Moderating an ordinary customer remains a staff business permission.
    let mut customer = Client::new(&s);
    let customer = customer.sign_in("customer@example.invalid").await;
    let id = customer["id"].as_str().unwrap();
    assert_eq!(
        colleague
            .request(
                "POST",
                &format!("/api/admin/users/{id}/access"),
                json!({"banned":true,"reason":"Customer moderation"})
            )
            .await
            .0,
        StatusCode::OK
    );
    let id = target_user["id"].as_str().unwrap();
    assert_eq!(
        owner
            .request(
                "POST",
                &format!("/api/admin/users/{id}/access"),
                json!({"banned":true,"reason":"Owner suspends employee"})
            )
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        owner
            .request(
                "POST",
                &format!("/api/admin/users/{id}/access"),
                json!({"banned":false,"reason":"Owner restores account"})
            )
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        hireferry::team::human_member(&s.conn().unwrap(), id)
            .unwrap()
            .is_none()
    );
}
