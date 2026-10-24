use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{
    AppState,
    api::{self, ProfileInput},
    config::Config,
    models,
};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn public_profile_is_opt_in_and_exposes_only_declared_ai_links() {
    let state = AppState::new(Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    })
    .unwrap();
    let id = "profile-user";
    state.conn().unwrap().execute("INSERT INTO users(id,email,name,handle,role,public_email,resume_md,public_profile,ai_profile_links,created_at,updated_at) VALUES(?1,?2,'Ada','ada','seeker','public@example.invalid','## Hello',1,?3,1,1)", params![id, "ada@example.invalid", json!({"chatgpt":"https://chatgpt.com/g/example","grok":"https://grok.com/share/example"}).to_string()]).unwrap();
    let app = hireferry::app(state.clone());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/public-profiles/ada")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["profile"]["name"], "Ada");
    assert_eq!(
        body["profile"]["ai_profile_links"]["chatgpt"],
        "https://chatgpt.com/g/example"
    );
    assert!(body.to_string().contains("## Hello") == false);
    state
        .conn()
        .unwrap()
        .execute("UPDATE users SET public_profile=0 WHERE id=?1", [id])
        .unwrap();
    let hidden = app
        .oneshot(
            Request::builder()
                .uri("/api/public-profiles/ada")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
    let user = models::load_user(&state.conn().unwrap(), id).unwrap();
    assert_eq!(user.handle, "ada");
}

#[test]
fn generated_placeholder_handle_cannot_be_published() {
    let state = AppState::new(Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    })
    .unwrap();
    let id = "generated-profile-user";
    state
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO users(id,email,name,handle,role,created_at,updated_at) VALUES(?1,?2,'Ada','user_0123456789abcdef','seeker',1,1)",
            params![id, "ada-generated@example.invalid"],
        )
        .unwrap();
    let error = api::save_profile(
        &state,
        id,
        ProfileInput {
            name: "Ada".into(),
            handle: String::new(),
            public_email: String::new(),
            resume_md: "## Hello".into(),
            visible: false,
            public_profile: true,
            ai_profile_links: std::collections::BTreeMap::new(),
        },
    )
    .unwrap_err();
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(error.message.contains("Claim a custom username"));
}
