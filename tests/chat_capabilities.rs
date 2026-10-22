use axum::{body::Body, http::Request};
use hireferry::{AppState, config::Config};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn welcome_configuration_is_localized_sanitized_and_public_without_account_data() {
    let config: Config = toml::from_str(
        r###"
        development = true
        database = ":memory:"
        [chat.welcome.platform_seeker]
        title = "欢迎"
        title_en = "Welcome"
        body_md = "**说明** <script>alert(1)</script> [bad](javascript:alert(1))"
        body_md_en = "## Start here\nTell us what you need."
        [[chat.welcome.platform_seeker.prompts]]
        label = "聊聊方向"
        body = "我想聊聊求职方向。"
        label_en = "My next role"
        body_en = "I would like to discuss my next role."
    "###,
    )
    .unwrap();
    let state = AppState::new(config).unwrap();
    let response = hireferry::app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/chat/capabilities")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(
        body["formats"],
        serde_json::json!(["plain", "markdown", "openui"])
    );
    assert_eq!(body["openui"]["library"], "hireferry@1");
    assert_eq!(body["welcome"]["platform_seeker"]["title_en"], "Welcome");
    let html = body["welcome"]["platform_seeker"]["body_html"]
        .as_str()
        .unwrap();
    assert!(html.contains("<strong>"));
    assert!(!html.contains("<script"));
    assert!(!html.contains("javascript:"));
    assert!(
        body["welcome"]["platform_seeker"]["body_html_en"]
            .as_str()
            .unwrap()
            .contains("<h2>")
    );
    assert!(body.get("user_id").is_none());
    assert_eq!(
        hireferry::api::public_config(&state)["chat"]["welcome"],
        body["welcome"]
    );
}

#[test]
fn welcome_limits_reject_misconfiguration_and_allow_empty_prompt_lists() {
    let mut config = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    config.chat.welcome.insert(
        "matched".into(),
        hireferry::config::ChatWelcome {
            prompts: Some(vec![]),
            ..Default::default()
        },
    );
    config.validate().unwrap();
    config.chat.welcome.get_mut("matched").unwrap().title = Some("x".repeat(161));
    assert!(config.validate().is_err());
    config.chat.welcome.clear();
    config
        .chat
        .welcome
        .insert("typo".into(), Default::default());
    assert!(config.validate().is_err());
    let config: Config =
        toml::from_str("[chat.welcome.staff]\nprompts=[{label='Button',body=''}]").unwrap();
    assert!(config.chat.validate().is_err());
}
