use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
    response::Response,
};
use hireferry::{AppState, config::Config};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app() -> Router {
    hireferry::app(
        AppState::new(Config {
            development: true,
            database: ":memory:".into(),
            ..Config::default()
        })
        .unwrap(),
    )
}

async fn request(app: &Router, method: Method, path: &str, headers: &[(&str, &str)]) -> Response {
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app.clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn body(response: Response) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

async fn revision(app: &Router) -> String {
    let response = request(app, Method::GET, "/cli", &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-cache");
    let html = String::from_utf8(body(response).await).unwrap();
    let script = html
        .split('"')
        .find(|url| url.starts_with("/static/") && url.ends_with("/app.js"))
        .expect("versioned application entry");
    let revision = script
        .strip_prefix("/static/")
        .unwrap()
        .strip_suffix("/app.js")
        .unwrap();
    assert_eq!(revision.len(), 64);
    assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(
        html.split("/static/")
            .skip(1)
            .all(|url| url.starts_with(&format!("{revision}/")))
    );
    revision.into()
}

#[tokio::test]
async fn release_urls_cache_the_entire_module_graph_without_changing_legacy_assets() {
    let app = app();
    let revision = revision(&app).await;
    for asset in [
        "app.js",
        "route-styles.js",
        "cli-guide.js",
        "runtime/timeline.js",
        "theme.css",
        "favicon.svg",
        "chat-renderer.js",
        "chat-renderer.css",
        "chat-openui.json",
        "chat-renderer-fonts/KaTeX_Main-Regular.woff2",
    ] {
        let versioned = request(
            &app,
            Method::GET,
            &format!("/static/{revision}/{asset}"),
            &[],
        )
        .await;
        assert_eq!(versioned.status(), StatusCode::OK);
        assert_eq!(
            versioned.headers()["cache-control"],
            "public, max-age=31536000, immutable"
        );
        assert!(versioned.headers().get("set-cookie").is_none());
        let legacy = request(&app, Method::GET, &format!("/static/{asset}"), &[]).await;
        assert_eq!(legacy.status(), StatusCode::OK);
        assert_eq!(legacy.headers()["cache-control"], "no-cache");
        assert_eq!(versioned.headers()["etag"], legacy.headers()["etag"]);
        assert_eq!(body(versioned).await, body(legacy).await);
    }
    let html = request(
        &app,
        Method::GET,
        &format!("/static/{revision}/index.html"),
        &[],
    )
    .await;
    assert_eq!(html.status(), StatusCode::OK);
    assert_eq!(html.headers()["cache-control"], "no-cache");
}

#[tokio::test]
async fn home_shell_keeps_route_css_out_of_the_blocking_head() {
    let response = request(&app(), Method::GET, "/", &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(body(response).await).unwrap();
    assert_eq!(html.matches("rel=\"stylesheet\"").count(), 5);
    assert!(html.contains("route-styles.js"));
    for route_style in [
        "workspace.css",
        "conversation.css",
        "public-directory.css",
        "auth.css",
    ] {
        assert!(
            !html.contains(route_style),
            "{route_style} leaked into home shell"
        );
    }
}

#[tokio::test]
async fn matching_validators_return_empty_304_and_head_keeps_representation_metadata() {
    let app = app();
    let revision = revision(&app).await;
    for path in [
        "/cli".into(),
        "/static/app.js".into(),
        format!("/static/{revision}/app.js"),
    ] {
        let initial = request(&app, Method::GET, &path, &[]).await;
        let etag = initial.headers()["etag"].to_str().unwrap().to_owned();
        let cache = initial.headers()["cache-control"]
            .to_str()
            .unwrap()
            .to_owned();
        let size = body(initial).await.len();
        for validator in [
            etag.clone(),
            format!("\"another-release\", {}", etag.trim_start_matches("W/")),
            "*".into(),
        ] {
            let cached = request(&app, Method::GET, &path, &[("if-none-match", &validator)]).await;
            assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
            assert_eq!(cached.headers()["etag"], etag);
            assert_eq!(cached.headers()["cache-control"], cache);
            assert_eq!(cached.headers()["vary"], "accept-encoding");
            assert!(body(cached).await.is_empty());
        }
        let changed = request(
            &app,
            Method::GET,
            &path,
            &[("if-none-match", "\"old-release\"")],
        )
        .await;
        assert_eq!(changed.status(), StatusCode::OK);
        assert_eq!(body(changed).await.len(), size);
        let head = request(&app, Method::HEAD, &path, &[]).await;
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(
            head.headers()["content-length"].to_str().unwrap(),
            size.to_string()
        );
        assert_eq!(head.headers()["etag"], etag);
        assert!(body(head).await.is_empty());
    }
}

#[tokio::test]
async fn obsolete_versions_and_missing_assets_never_fall_back_to_new_content() {
    let app = app();
    let revision = revision(&app).await;
    let old = if revision.starts_with('0') {
        "1".repeat(64)
    } else {
        "0".repeat(64)
    };
    for path in [
        format!("/static/{old}/app.js"),
        format!("/static/{old}/index.html"),
        format!("/static/{old}/cli"),
        format!("/static/{revision}/missing"),
        "/static/runtime/missing".into(),
    ] {
        let response = request(&app, Method::GET, &path, &[]).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(body(response).await.is_empty());
    }
    let method = request(&app, Method::POST, "/static/app.js", &[]).await;
    assert_eq!(method.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn static_text_is_compressed_but_session_and_private_api_responses_remain_uncached() {
    let app = app();
    let revision = revision(&app).await;
    let path = format!("/static/{revision}/app.js");
    let plain = request(&app, Method::GET, &path, &[]).await;
    let etag = plain.headers()["etag"].to_str().unwrap().to_owned();
    let plain_size = body(plain).await.len();
    let zipped = request(&app, Method::GET, &path, &[("accept-encoding", "gzip")]).await;
    assert_eq!(zipped.headers()["content-encoding"], "gzip");
    assert!(
        zipped.headers()["vary"]
            .to_str()
            .unwrap()
            .to_ascii_lowercase()
            .contains("accept-encoding")
    );
    assert_eq!(zipped.headers()["etag"], etag);
    let compressed = body(zipped).await;
    assert!(compressed.starts_with(&[0x1f, 0x8b]));
    assert!(compressed.len() < plain_size);
    for path in [
        "/api/session",
        "/api/profile",
        "/api/missing",
        "/v1/missing",
        "/oauth/connections",
    ] {
        let response = request(&app, Method::GET, path, &[("accept-encoding", "gzip")]).await;
        assert_eq!(response.headers()["cache-control"], "no-store", "{path}");
        assert!(
            response.headers().get("content-encoding").is_none(),
            "{path}"
        );
        assert!(response.headers().get("etag").is_none(), "{path}");
    }
}

#[tokio::test]
async fn public_pages_allow_github_star_reads_without_loading_third_party_scripts() {
    let response = request(&app(), Method::GET, "/", &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let policy = response.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    let directive = |name: &str| {
        policy
            .split(';')
            .map(str::trim)
            .find(|value| value.starts_with(name))
            .unwrap()
    };
    assert_eq!(
        directive("connect-src "),
        "connect-src 'self' https://api.github.com"
    );
    assert_eq!(directive("script-src "), "script-src 'self'");
    assert_eq!(directive("default-src "), "default-src 'self'");
}
