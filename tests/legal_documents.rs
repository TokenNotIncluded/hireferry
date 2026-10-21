use hireferry::{AppState, config::Config, legal};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[test]
fn policies_follow_configured_identity_pricing_and_retention_in_both_languages() {
    let mut cfg = Config::default();
    cfg.site.name = "Example Hiring".into();
    cfg.site.support_email = "help@example.com".into();
    cfg.public_url = "https://example.com".into();
    cfg.pricing.onboarding_cents = 23500;
    cfg.pricing.publish_bps = 800;
    cfg.pricing.edit_bps = 200;
    cfg.pricing.max_edits = 4;
    cfg.legal.values.extend([
        (
            "operator_name".into(),
            "Example <script>alert(1)</script>".into(),
        ),
        ("refund_days".into(), "11".into()),
        ("deletion_days".into(), "45".into()),
        ("version".into(), "custom-version".into()),
    ]);
    for language in ["zh", "en"] {
        let terms = legal::document(&cfg, "terms", language).unwrap();
        let html = terms["html"].as_str().unwrap();
        assert!(html.contains("CNY 235.00"));
        assert!(html.contains("Example Hiring"));
        assert!(html.contains("8%") && html.contains("2%"));
        assert!(html.contains("11") && html.contains("help@example.com"));
        assert!(html.contains("id=\"refunds\""));
        assert!(html.contains("&lt;script&gt;") && !html.contains("<script>"));
        assert!(!html.contains("{{") && !html.contains("hireferry.lmm.best"));
        assert!(!html.contains("**"));
        assert_eq!(terms["version"], "custom-version");
        let privacy = legal::document(&cfg, "privacy", language).unwrap();
        assert!(privacy["html"].as_str().unwrap().contains("45"));
        assert!(!privacy["html"].as_str().unwrap().contains("**"));
    }
}

#[test]
fn editable_markdown_is_loaded_relative_to_config_and_sanitized() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("privacy.md"), "# Custom privacy\n\n## Contact {#contact}\n\n{{operator_name}}\n\n{{support_contact}}\n\n<script>alert('unsafe')</script>\n\n[unsafe](javascript:alert)\n").unwrap();
    let config_path = directory.path().join("config.toml");
    std::fs::write(&config_path, "development=true\npublic_url='http://localhost:3000'\n[site]\nsupport_email='privacy@example.com'\n[legal.values]\noperator_name='Configured operator'\nversion='review-2'\n[legal.document_files]\n'privacy.zh'='privacy.md'\n").unwrap();
    let cfg = Config::load(&config_path).unwrap();
    let result = legal::document(&cfg, "privacy", "zh").unwrap();
    let html = result["html"].as_str().unwrap();
    assert!(html.contains("Custom privacy") && html.contains("Configured operator"));
    assert!(html.contains("privacy@example.com") && html.contains("id=\"contact\""));
    assert!(!html.contains("<script") && !html.contains("javascript:"));
    std::fs::write(
        directory.path().join("privacy.md"),
        "# Changed without recompiling\n",
    )
    .unwrap();
    let reloaded = Config::load(&config_path).unwrap();
    assert!(
        legal::document(&reloaded, "privacy", "zh").unwrap()["html"]
            .as_str()
            .unwrap()
            .contains("Changed without recompiling")
    );
    std::fs::write(directory.path().join("privacy.md"), "# {{unknown_value}}\n").unwrap();
    assert!(Config::load(&config_path).is_err());
}

#[tokio::test]
async fn public_document_endpoint_only_serves_known_documents_and_languages() {
    let directory = tempfile::tempdir().unwrap();
    let cfg = Config {
        development: true,
        database: directory.path().join("db.sqlite3").to_str().unwrap().into(),
        ..Config::default()
    };
    let app = hireferry::app(AppState::new(cfg).unwrap());
    for (path, status) in [
        ("/api/legal/terms?lang=en", 200),
        ("/api/legal/privacy?lang=zh", 200),
        ("/api/legal/config?lang=zh", 404),
        ("/api/legal/privacy?lang=../../config", 404),
    ] {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        if status == 200 {
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert!(value["html"].as_str().unwrap().contains("<h1>"));
            assert!(value.get("document_files").is_none());
        }
    }
}
