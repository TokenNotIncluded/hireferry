//! Real HTTP/SQLite tests for one-time browser-to-WebView login.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use hireferry::{AppState, auth, config::Config, now};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    app: Router,
}
struct Seen {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
    json: Value,
}
impl Fixture {
    fn new() -> Self {
        let mut cfg = Config {
            development: true,
            database: ":memory:".into(),
            ..Default::default()
        };
        cfg.google.enabled = true;
        cfg.google.client_id = "test-google-client".into();
        cfg.google.client_secret = "fixture-not-a-real-secret".into();
        let state = AppState::new(cfg).unwrap();
        let db = state.conn().unwrap();
        db.execute("INSERT INTO users(id,email,name,role,created_at,updated_at) VALUES('member','member@example.invalid','Test member','seeker',?1,?1)", [now()]).unwrap();
        db.execute("INSERT INTO sessions(token_hash,user_id,csrf,expires_at,recent_auth_at) VALUES(?1,'member','csrf-member',?2,?3)", params![auth::hash_token("browser-session"),now()+3600,now()]).unwrap();
        db.execute("INSERT INTO sessions(token_hash,admin,csrf,expires_at,recent_auth_at) VALUES(?1,1,'csrf-admin',?2,?3)", params![auth::hash_token("admin-session"),now()+3600,now()]).unwrap();
        drop(db);
        let app = hireferry::app(state.clone());
        Self { state, app }
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Value,
        cookie: &str,
        origin: Option<&str>,
        csrf: Option<&str>,
        bearer: bool,
    ) -> Seen {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if !cookie.is_empty() {
            builder = builder.header("cookie", cookie);
        }
        if let Some(value) = origin {
            builder = builder.header("origin", value);
        }
        if let Some(value) = csrf {
            builder = builder.header("x-csrf-token", value);
        }
        if bearer {
            builder = builder.header("authorization", "Bearer explicitly-not-a-session");
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        let json = serde_json::from_str(&body).unwrap_or(Value::Null);
        Seen {
            status,
            headers,
            body,
            json,
        }
    }
    async fn post(&self, path: &str, body: Value, cookie: &str) -> Seen {
        self.request(
            "POST",
            path,
            body,
            cookie,
            Some("http://localhost:3000"),
            Some("csrf-member"),
            false,
        )
        .await
    }
    async fn start(&self) -> (Value, String) {
        let seen = self.post("/api/auth/app/start", json!({}), "").await;
        assert_eq!(seen.status, StatusCode::OK, "{}", seen.body);
        let cookie = cookie_value(&seen, "hf_app_login");
        (seen.json, cookie)
    }
    async fn approve(&self, request: &Value, approve: bool) -> Seen {
        self.post(
            "/api/auth/app/decide",
            json!({"request":request["request"],"approve":approve}),
            "hf_session=browser-session",
        )
        .await
    }
    async fn redeem(&self, request: &Value, cookie: &str) -> Seen {
        self.post(
            "/api/auth/app/redeem",
            json!({"request":request["request"]}),
            cookie,
        )
        .await
    }
}
fn cookie_value(seen: &Seen, name: &str) -> String {
    seen.headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("Missing {name}: {:?}", seen.headers))
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn start_requires_exact_origin_and_never_returns_verifier_in_json() {
    let f = Fixture::new();
    for origin in [None, Some("https://unrelated.example"), Some("null")] {
        let r = f
            .request(
                "POST",
                "/api/auth/app/start",
                json!({}),
                "",
                origin,
                None,
                false,
            )
            .await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
        assert!(!r.headers.contains_key("set-cookie"));
    }
    let (request, cookie) = f.start().await;
    assert_eq!(request["request"].as_str().unwrap().len(), 43);
    assert!(
        !request
            .to_string()
            .contains(cookie.split_once('=').unwrap().1)
    );
    let again = f.post("/api/auth/app/start", json!({}), &cookie).await;
    assert_eq!(again.json["request"], request["request"]);
    assert_eq!(again.json["expires_at"], request["expires_at"]);
    let header = again.headers["set-cookie"].to_str().unwrap();
    assert!(header.contains("HttpOnly; SameSite=Strict"));
    assert!(header.contains("Path=/api/auth/app"));
    assert_eq!(again.headers["cache-control"], "no-store");
    assert_eq!(again.headers["pragma"], "no-cache");
}

#[tokio::test]
async fn public_request_alone_never_creates_or_redeems_a_session() {
    let f = Fixture::new();
    let (request, cookie) = f.start().await;
    let pending = f.redeem(&request, &cookie).await;
    assert_eq!(pending.json["status"], "pending");
    assert!(!pending.headers.contains_key("set-cookie"));
    assert_eq!(
        f.redeem(&request, "").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(f.approve(&request, true).await.status, StatusCode::OK);
    assert_eq!(
        f.redeem(&request, "hf_app_login=wrong-cookie").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.redeem(&request, "hf_session=browser-session")
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let completed = f.redeem(&request, &cookie).await;
    assert_eq!(completed.status, StatusCode::OK, "{}", completed.body);
    assert_eq!(
        completed.json,
        json!({"status":"complete","redirect":"/app"})
    );
    let app_session = cookie_value(&completed, "hf_session");
    assert_ne!(app_session, "hf_session=browser-session");
    assert!(
        !completed
            .body
            .contains(app_session.split_once('=').unwrap().1)
    );
    assert!(
        completed
            .headers
            .get_all("set-cookie")
            .iter()
            .any(|v| v.to_str().unwrap().contains("hf_app_login=;"))
    );
    let identity = f
        .request(
            "GET",
            "/api/session",
            Value::Null,
            &app_session,
            None,
            None,
            false,
        )
        .await;
    assert_eq!(identity.json["user"]["id"], "member");
    assert_eq!(identity.json["admin"], false);
    let browser = f
        .request(
            "GET",
            "/api/session",
            Value::Null,
            "hf_session=browser-session",
            None,
            None,
            false,
        )
        .await;
    assert_eq!(browser.json["user"]["id"], "member");
    assert_eq!(
        f.redeem(&request, &cookie).await.status,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn approval_requires_member_browser_and_csrf_and_is_single_use() {
    let f = Fixture::new();
    let (request, _) = f.start().await;
    let body = json!({"request":request["request"],"approve":true});
    for (cookie, csrf, bearer) in [
        ("", None, false),
        ("hf_session=browser-session", None, false),
        ("hf_session=admin-session", Some("csrf-admin"), false),
        ("hf_session=browser-session", Some("csrf-member"), true),
    ] {
        let r = f
            .request(
                "POST",
                "/api/auth/app/decide",
                body.clone(),
                cookie,
                Some("http://localhost:3000"),
                csrf,
                bearer,
            )
            .await;
        assert!(
            matches!(r.status, StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED),
            "{}",
            r.body
        );
    }
    assert_eq!(f.approve(&request, true).await.status, StatusCode::OK);
    assert_eq!(
        f.approve(&request, true).await.status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn revoked_or_suspended_approver_cannot_be_transferred() {
    for suspend in [false, true] {
        let f = Fixture::new();
        let (request, cookie) = f.start().await;
        assert_eq!(f.approve(&request, true).await.status, StatusCode::OK);
        if suspend {
            f.state
                .conn()
                .unwrap()
                .execute("UPDATE users SET banned=1 WHERE id='member'", [])
                .unwrap();
        } else {
            f.state
                .conn()
                .unwrap()
                .execute(
                    "DELETE FROM sessions WHERE token_hash=?1",
                    [auth::hash_token("browser-session")],
                )
                .unwrap();
        }
        let r = f.redeem(&request, &cookie).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
        assert!(!r.headers.contains_key("set-cookie"));
    }
}

#[tokio::test]
async fn expiration_denial_and_cancellation_do_not_issue_sessions() {
    let f = Fixture::new();
    let (request, cookie) = f.start().await;
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE app_login_grants SET expires_at=?1", [now() - 1])
        .unwrap();
    assert_eq!(
        f.approve(&request, true).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.redeem(&request, &cookie).await.status,
        StatusCode::UNAUTHORIZED
    );
    let (request, cookie) = f.start().await;
    assert_eq!(f.approve(&request, false).await.status, StatusCode::OK);
    let denied = f.redeem(&request, &cookie).await;
    assert_eq!(denied.json["status"], "denied");
    assert!(
        !denied
            .headers
            .get_all("set-cookie")
            .iter()
            .any(|v| v.to_str().unwrap().starts_with("hf_session="))
    );
    let (request, cookie) = f.start().await;
    let cancel = f
        .post(
            "/api/auth/app/cancel",
            json!({"request":request["request"]}),
            &cookie,
        )
        .await;
    assert_eq!(cancel.status, StatusCode::OK);
    assert_eq!(
        f.redeem(&request, &cookie).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn concurrent_redemptions_create_exactly_one_session() {
    let f = Fixture::new();
    let (request, cookie) = f.start().await;
    assert_eq!(f.approve(&request, true).await.status, StatusCode::OK);
    let (a, b) = tokio::join!(f.redeem(&request, &cookie), f.redeem(&request, &cookie));
    assert_eq!(
        [a.status, b.status]
            .iter()
            .filter(|s| **s == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        f.state
            .conn()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sessions WHERE user_id='member'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn cookie_from_another_request_cannot_redeem_or_cancel() {
    let f = Fixture::new();
    let (a, ca) = f.start().await;
    let (_, cb) = f.start().await;
    f.post("/api/auth/app/cancel", json!({"request":a["request"]}), &cb)
        .await;
    assert_eq!(f.approve(&a, true).await.status, StatusCode::OK);
    assert_eq!(f.redeem(&a, &cb).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(f.redeem(&a, &ca).await.status, StatusCode::OK);
}

#[tokio::test]
async fn consent_context_is_current_identity_and_sensitive_response_is_not_cached() {
    let f = Fixture::new();
    let (request, _) = f.start().await;
    let path = format!(
        "/api/auth/app/context?request={}",
        request["request"].as_str().unwrap()
    );
    let anon = f
        .request("GET", &path, Value::Null, "", None, None, false)
        .await;
    assert_eq!(anon.json["user"], Value::Null);
    assert_eq!(anon.json["can_approve"], false);
    let browser = f
        .request(
            "GET",
            &path,
            Value::Null,
            "hf_session=browser-session",
            None,
            None,
            false,
        )
        .await;
    assert_eq!(browser.json["user"]["email"], "member@example.invalid");
    assert_eq!(browser.json["comparison_code"], request["comparison_code"]);
    assert_eq!(browser.headers["cache-control"], "no-store");
    assert_eq!(browser.json["can_approve"], true);
    let admin = f
        .request(
            "GET",
            &path,
            Value::Null,
            "hf_session=admin-session",
            None,
            None,
            false,
        )
        .await;
    assert_eq!(admin.json["can_approve"], false);
    assert_eq!(admin.json["admin_session"], true);
}

#[tokio::test]
async fn google_return_is_bound_to_a_live_request_and_cancel_clears_cookie() {
    let f = Fixture::new();
    let (request, _) = f.start().await;
    let id = request["request"].as_str().unwrap();
    let start = f
        .request(
            "GET",
            &format!("/api/auth/google/start?app_request={id}"),
            Value::Null,
            "",
            None,
            None,
            false,
        )
        .await;
    assert_eq!(start.status, StatusCode::SEE_OTHER);
    let cookie = cookie_value(&start, "hf_google_challenge");
    let state = cookie.split_once('=').unwrap().1;
    let payload: String = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT payload FROM auth_challenges WHERE kind='google'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&payload).unwrap()["app_request"],
        id
    );
    let cancelled = f
        .request(
            "GET",
            &format!("/auth/google/callback?state={state}&error=access_denied"),
            Value::Null,
            &cookie,
            None,
            None,
            false,
        )
        .await;
    assert_eq!(
        cancelled.headers["location"],
        format!("/auth/app/authorize?request={id}")
    );
    assert!(
        cancelled.headers["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let bad = f
        .request(
            "GET",
            "/api/auth/google/start?app_request=https%3A%2F%2Fevil.invalid",
            Value::Null,
            "",
            None,
            None,
            false,
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let none = f
        .request(
            "GET",
            &format!("/api/auth/google/start?app_request={}", "x".repeat(43)),
            Value::Null,
            "",
            None,
            None,
            false,
        )
        .await;
    assert_eq!(none.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn requests_are_bounded_and_unknown_redirect_fields_are_rejected() {
    let f = Fixture::new();
    let (request, cookie) = f.start().await;
    let r = f
        .post(
            "/api/auth/app/redeem",
            json!({"request":request["request"],"redirect":"https://evil.invalid"}),
            &cookie,
        )
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    f.state.conn().unwrap().execute("INSERT INTO auth_limits(key,window_start,count) VALUES('app-login-start',?1,100) ON CONFLICT(key) DO UPDATE SET count=100,window_start=excluded.window_start",[now()]).unwrap();
    let r = f.post("/api/auth/app/start", json!({}), "").await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    // The existing initiating WebView can resume without consuming more slots.
    assert_eq!(
        f.post("/api/auth/app/start", json!({}), &cookie)
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn staff_and_manager_memberships_are_rechecked_at_approval_and_redemption() {
    for manager in [false, true] {
        for after_approval in [false, true] {
            let f = Fixture::new();
            let (request, cookie) = f.start().await;
            if after_approval {
                assert_eq!(f.approve(&request, true).await.status, StatusCode::OK);
            }
            if manager {
                f.state.conn().unwrap().execute("INSERT INTO staff_members(user_id,role,joined_at,updated_at) VALUES('member','manager',?1,?1)",[now()]).unwrap();
            } else {
                f.state.conn().unwrap().execute("INSERT INTO team_members(id,kind,user_id,name,created_by,created_at,updated_at) VALUES('employee','human','member','Employee','test',?1,?1)",[now()]).unwrap();
            }
            if after_approval {
                assert_eq!(
                    f.redeem(&request, &cookie).await.status,
                    StatusCode::FORBIDDEN
                );
            } else {
                let path = format!(
                    "/api/auth/app/context?request={}",
                    request["request"].as_str().unwrap()
                );
                let ctx = f
                    .request(
                        "GET",
                        &path,
                        Value::Null,
                        "hf_session=browser-session",
                        None,
                        None,
                        false,
                    )
                    .await;
                assert_eq!(ctx.json["can_approve"], false);
                assert_eq!(ctx.json["admin_session"], true);
                assert_eq!(
                    f.approve(&request, true).await.status,
                    StatusCode::FORBIDDEN
                );
            }
        }
    }
}

#[tokio::test]
async fn transferred_session_does_not_refresh_authentication_age_or_extend_expiry() {
    let f = Fixture::new();
    let (request, cookie) = f.start().await;
    let recent = now() - 1200;
    let expires = now() + 300;
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE sessions SET expires_at=?1,recent_auth_at=?2 WHERE user_id='member'",
            params![expires, recent],
        )
        .unwrap();
    assert_eq!(f.approve(&request, true).await.status, StatusCode::OK);
    let completed = f.redeem(&request, &cookie).await;
    let session = cookie_value(&completed, "hf_session");
    let hash = auth::hash_token(session.split_once('=').unwrap().1);
    let times: (i64, i64) = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT expires_at,recent_auth_at FROM sessions WHERE token_hash=?1",
            [hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(times, (expires, recent));
}

#[tokio::test]
async fn cancelling_a_committed_poll_revokes_only_its_new_app_session() {
    for using_issued_cookie in [false, true] {
        let f = Fixture::new();
        let (request, cookie) = f.start().await;
        assert_eq!(f.approve(&request, true).await.status, StatusCode::OK);
        let completed = f.redeem(&request, &cookie).await;
        let app_cookie = cookie_value(&completed, "hf_session");
        let cancel = f
            .post(
                "/api/auth/app/cancel",
                json!({"request":request["request"]}),
                if using_issued_cookie {
                    &app_cookie
                } else {
                    &cookie
                },
            )
            .await;
        assert_eq!(cancel.status, StatusCode::OK, "{}", cancel.body);
        let identity = f
            .request(
                "GET",
                "/api/session",
                Value::Null,
                &app_cookie,
                None,
                None,
                false,
            )
            .await;
        assert_eq!(identity.json["user"], Value::Null);
        let source = f
            .request(
                "GET",
                "/api/session",
                Value::Null,
                "hf_session=browser-session",
                None,
                None,
                false,
            )
            .await;
        assert_eq!(source.json["user"]["id"], "member");
        assert_eq!(
            f.redeem(&request, &cookie).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn concurrent_cancel_and_redeem_leave_no_app_session() {
    let f = Fixture::new();
    let (request, cookie) = f.start().await;
    assert_eq!(f.approve(&request, true).await.status, StatusCode::OK);
    let body = json!({"request":request["request"]});
    let (cancel, _) = tokio::join!(
        f.post("/api/auth/app/cancel", body, &cookie),
        f.redeem(&request, &cookie)
    );
    assert_eq!(cancel.status, StatusCode::OK);
    assert_eq!(
        f.state
            .conn()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sessions WHERE user_id='member'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn additive_login_schema_is_atomic_and_keeps_version_thirteen_on_retry() {
    let f = Fixture::new();
    let db = f.state.conn().unwrap();
    db.execute_batch(
        "DROP TABLE app_login_grants; CREATE TABLE app_login_expiry(blocker INTEGER);",
    )
    .unwrap();
    assert!(hireferry::db::migrate(&db).is_err());
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='app_login_grants')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !exists,
        "A failed index creation must roll back the feature table"
    );
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        13
    );
    db.execute_batch("DROP TABLE app_login_expiry;").unwrap();
    hireferry::db::migrate(&db).unwrap();
    hireferry::db::migrate(&db).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM users WHERE id='member'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        13
    );
}
