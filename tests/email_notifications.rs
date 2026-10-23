use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{AppState, auth, config::Config};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt;
fn fixture() -> AppState {
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.smtp.enabled = true;
    cfg.smtp.host = "smtp.example.invalid".into();
    cfg.smtp.from = "notify@example.invalid".into();
    let s = AppState::new(cfg).unwrap();
    let db = s.conn().unwrap();
    db.execute("INSERT INTO users(id,email,role,created_at,updated_at) VALUES('member','member@example.invalid','seeker',1,1)",[]).unwrap();
    for (raw, admin, uid) in [
        ("admin-session", true, None),
        ("user-session", false, Some("member")),
    ] {
        db.execute("INSERT INTO sessions(token_hash,user_id,admin,bootstrap,csrf,expires_at,recent_auth_at) VALUES(?1,?2,?3,0,'csrf-token',?4,?5)",params![auth::hash_token(raw),uid,admin,hireferry::now()+3600,hireferry::now()]).unwrap();
    }
    drop(db);
    s
}
async fn request(
    s: &AppState,
    path: &str,
    method: &str,
    session: Option<&str>,
    csrf: bool,
    ai: bool,
    body: Value,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("origin", "http://localhost:3000")
        .header("content-type", "application/json");
    if let Some(raw) = session {
        req = req.header("cookie", format!("hf_session={raw}"));
    }
    if csrf {
        req = req.header("x-csrf-token", "csrf-token");
    }
    if ai {
        req = req.header("x-hireferry-ai", "webmcp");
    }
    let res = hireferry::app(s.clone())
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
async fn native_email_routes_enforce_admin_identity_and_csrf_and_do_not_expose_addresses() {
    let s = fixture();
    let body = json!({"user_id":"member","subject":"Service update","body":"Review your workspace.","request_key":"native-request-1"});
    for (session, csrf, expected) in [
        (None, false, StatusCode::UNAUTHORIZED),
        (Some("user-session"), true, StatusCode::FORBIDDEN),
        (Some("admin-session"), false, StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            request(
                &s,
                "/api/admin/notifications/email",
                "POST",
                session,
                csrf,
                false,
                body.clone()
            )
            .await
            .0,
            expected
        );
    }
    let first = request(
        &s,
        "/api/admin/notifications/email",
        "POST",
        Some("admin-session"),
        true,
        false,
        body.clone(),
    )
    .await;
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(first.1["status"], "pending");
    let retry = request(
        &s,
        "/api/admin/notifications/email",
        "POST",
        Some("admin-session"),
        true,
        false,
        body,
    )
    .await;
    assert_eq!(retry.1["id"], first.1["id"]);
    let log = request(
        &s,
        "/api/admin/notifications/email?user_id=member",
        "GET",
        Some("admin-session"),
        false,
        false,
        json!({}),
    )
    .await;
    assert_eq!(log.1["items"].as_array().unwrap().len(), 1);
    assert!(!log.1.to_string().contains("member@example.invalid"));
    assert!(!log.1.to_string().contains("Review your workspace"));
    assert_eq!(
        request(
            &s,
            "/api/admin/notifications/email",
            "GET",
            Some("user-session"),
            false,
            false,
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn only_the_person_can_change_notification_preferences_and_they_apply_to_new_mail() {
    let s = fixture();
    let prefs = json!({"messages":false,"matches":false,"orders":true,"platform":false});
    for (csrf, ai) in [(false, false), (true, true)] {
        assert_eq!(
            request(
                &s,
                "/api/notifications/email",
                "PUT",
                Some("user-session"),
                csrf,
                ai,
                prefs.clone()
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        request(
            &s,
            "/api/notifications/email",
            "PUT",
            Some("user-session"),
            true,
            false,
            prefs.clone()
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &s,
            "/api/notifications/email",
            "GET",
            Some("user-session"),
            false,
            false,
            json!({})
        )
        .await
        .1["preferences"],
        prefs
    );
    let mail = json!({"user_id":"member","subject":"notice","body":"notice","request_key":"unsubscribed-test"});
    assert_eq!(
        request(
            &s,
            "/api/admin/notifications/email",
            "POST",
            Some("admin-session"),
            true,
            false,
            mail
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn platform_ai_has_native_email_tools_but_member_ai_does_not() {
    let admin = hireferry::mcp::admin_tools();
    let member = hireferry::mcp::tools();
    for name in [
        "admin_send_email_notification",
        "admin_list_email_notifications",
    ] {
        assert!(admin.iter().any(|t| t["name"] == name));
        assert!(!member.iter().any(|t| t["name"] == name));
    }
    let s = fixture();
    let member = hireferry::access::Principal {
        user_id: Some("member".into()),
        admin: false,
        via_ai: true,
        actor: "member".into(),
    };
    assert!(
        hireferry::mcp::call(
            &s,
            &member,
            "admin_send_email_notification",
            json!({"user_id":"member","subject":"x","body":"x","request_key":"denied-123"})
        )
        .await
        .is_err()
    );
}
