use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{AppState, auth, config::Config, now};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt;

fn fixture() -> AppState {
    let state = AppState::new(Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    })
    .unwrap();
    let db = state.conn().unwrap();
    db.execute(
        "INSERT INTO users(id,email,role,created_at,updated_at) VALUES('member','member@example.invalid','seeker',1,1)",
        [],
    ).unwrap();
    for (raw, admin, user_id, csrf) in [
        ("admin-session", true, None, "admin-csrf"),
        ("user-session", false, Some("member"), "user-csrf"),
    ] {
        db.execute(
            "INSERT INTO sessions(token_hash,user_id,admin,bootstrap,csrf,expires_at,recent_auth_at) VALUES(?1,?2,?3,0,?4,?5,?5)",
            params![auth::hash_token(raw), user_id, admin, csrf, now() + 3600],
        ).unwrap();
    }
    drop(db);
    state
}

async fn request(
    state: &AppState,
    method: &str,
    path: &str,
    session: &str,
    csrf: bool,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("origin", "http://localhost:3000")
        .header("content-type", "application/json")
        .header("cookie", format!("hf_session={session}"));
    if csrf {
        request = request.header(
            "x-csrf-token",
            if session == "admin-session" {
                "admin-csrf"
            } else {
                "user-csrf"
            },
        );
    }
    let response = hireferry::app(state.clone())
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn announcements_are_versioned_sanitized_and_acknowledged() {
    let state = fixture();
    let initial = request(
        &state,
        "GET",
        "/api/admin/announcement",
        "admin-session",
        false,
        json!({}),
    )
    .await;
    assert_eq!(initial.0, StatusCode::OK);
    assert_eq!(initial.1["announcement"]["enabled"], false);
    assert_eq!(initial.1["announcement"]["version"], 1);

    let disabled = request(
        &state,
        "GET",
        "/api/announcement",
        "user-session",
        false,
        json!({}),
    )
    .await;
    assert_eq!(disabled.0, StatusCode::OK);
    assert!(disabled.1["announcement"].is_null());

    let update = json!({
        "enabled": true,
        "title": "重要更新",
        "title_en": "Important update",
        "body_md": "**请阅读** <script>alert(1)</script>",
        "body_md_en": "Please read this update.",
        "require_ack": true,
        "expected_version": 1
    });
    assert_eq!(
        request(
            &state,
            "PUT",
            "/api/admin/announcement",
            "admin-session",
            false,
            update.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let updated = request(
        &state,
        "PUT",
        "/api/admin/announcement",
        "admin-session",
        true,
        update,
    )
    .await;
    assert_eq!(updated.0, StatusCode::OK);
    assert_eq!(updated.1["announcement"]["version"], 2);

    let visible = request(
        &state,
        "GET",
        "/api/announcement",
        "user-session",
        false,
        json!({}),
    )
    .await;
    assert_eq!(visible.0, StatusCode::OK);
    assert_eq!(visible.1["acknowledged"], false);
    assert!(
        visible.1["announcement"]["body_html"]
            .as_str()
            .unwrap()
            .contains("<strong>")
    );
    assert!(
        !visible.1["announcement"]["body_html"]
            .as_str()
            .unwrap()
            .contains("script")
    );

    assert_eq!(
        request(
            &state,
            "POST",
            "/api/announcement/ack",
            "user-session",
            false,
            json!({"version": 2})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let acknowledged = request(
        &state,
        "POST",
        "/api/announcement/ack",
        "user-session",
        true,
        json!({"version": 2}),
    )
    .await;
    assert_eq!(acknowledged.0, StatusCode::OK);
    assert_eq!(acknowledged.1["acknowledged"], true);
    let after = request(
        &state,
        "GET",
        "/api/announcement",
        "user-session",
        false,
        json!({}),
    )
    .await;
    assert_eq!(after.1["acknowledged"], true);

    let stale = json!({
        "enabled": true,
        "title": "stale",
        "title_en": "stale",
        "body_md": "body",
        "body_md_en": "body",
        "require_ack": false,
        "expected_version": 1
    });
    assert_eq!(
        request(
            &state,
            "PUT",
            "/api/admin/announcement",
            "admin-session",
            true,
            stale
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn announcement_management_is_not_available_to_members() {
    let state = fixture();
    assert_eq!(
        request(
            &state,
            "GET",
            "/api/admin/announcement",
            "user-session",
            false,
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let body = json!({"enabled":true,"title":"x","title_en":"x","body_md":"x","body_md_en":"x","require_ack":false,"expected_version":1});
    assert_eq!(
        request(
            &state,
            "PUT",
            "/api/admin/announcement",
            "user-session",
            true,
            body
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
