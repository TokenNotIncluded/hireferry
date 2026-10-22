use axum::{body::Body, http::Request};
use hireferry::{AppState, config::Config};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn public_installers_follow_configuration_and_remain_plain_text() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config {
        development: true,
        database: dir
            .path()
            .join("test.sqlite3")
            .to_string_lossy()
            .into_owned(),
        ..Config::default()
    };
    cfg.cli_distribution.release_repository = "Example/custom-cli".into();
    cfg.cli_distribution.release_tag_prefix = "tool-v".into();
    let app = hireferry::app(AppState::new(cfg).unwrap());
    for path in ["/install.sh", "/install.ps1"] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()["content-type"],
            "text/plain; charset=utf-8"
        );
        assert_eq!(response.headers()["cache-control"], "no-cache");
        let text = String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(
            text.contains("Example/custom-cli")
                && text.contains("tool-v")
                && text.contains("http://localhost:3000")
        );
        assert!(!text.contains("{{") && !text.contains("client_secret"));
    }
}

#[test]
fn repository_configuration_cannot_inject_script_content() {
    let mut cfg = Config {
        development: true,
        ..Config::default()
    };
    for repository in [
        "owner",
        "owner/repo/extra",
        "owner/repo';touch /tmp/x",
        "../repo",
        "owner/..",
    ] {
        cfg.cli_distribution.release_repository = repository.into();
        assert!(cfg.validate().is_err(), "{repository}");
    }
    cfg.cli_distribution.release_repository = "Example/custom-cli".into();
    cfg.cli_distribution.release_tag_prefix = "release-v".into();
    cfg.validate().unwrap();
}
