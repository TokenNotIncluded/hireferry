use axum::{body::Body, http::Request};
use hireferry::{AppState, config::Config};
use http_body_util::BodyExt;
use rusqlite::params;
use tower::ServiceExt;

fn app(english: bool) -> axum::Router {
    let mut cfg = Config {
        database: ":memory:".into(),
        public_url: "https://hireferry.example".into(),
        ..Config::default()
    };
    cfg.webauthn_rp_id = "hireferry.example".into();
    cfg.site.default_language = if english { "en" } else { "zh" }.into();
    hireferry::app(AppState::new(cfg).unwrap())
}

async fn read(
    app: &axum::Router,
    path: &str,
) -> (axum::http::StatusCode, axum::http::HeaderMap, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("host", "untrusted.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let html = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    (status, headers, html)
}

#[tokio::test]
async fn sitemap_discovers_only_public_canonical_pages_with_crawlable_content() {
    let app = app(false);
    let (status, headers, xml) = read(&app, "/sitemap.xml").await;
    assert_eq!(status, 200);
    assert_eq!(headers["content-type"], "application/xml; charset=utf-8");
    let xml = roxmltree::Document::parse(&xml).unwrap();
    let urls: Vec<_> = xml
        .descendants()
        .filter(|n| n.has_tag_name(("http://www.sitemaps.org/schemas/sitemap/0.9", "loc")))
        .map(|n| n.text().unwrap().to_owned())
        .collect();
    assert_eq!(urls.len(), 11);
    let mut titles = std::collections::BTreeSet::new();
    for url in urls {
        let path = url.strip_prefix("https://hireferry.example").unwrap();
        assert!(!path.contains(['?', '#']));
        let (status, headers, html) = read(&app, path).await;
        assert_eq!(status, 200, "{path}");
        assert!(!headers.contains_key("x-robots-tag"));
        assert!(html.contains(&format!("<link rel=\"canonical\" href=\"{url}\">")));
        assert!(html.contains("<meta name=\"robots\" content=\"index, follow"));
        assert!(html.contains("<h1"), "crawler-readable content: {path}");
        assert!(html.contains("href=\"/contact\""));
        let title = html
            .split("<title>")
            .nth(1)
            .unwrap()
            .split("</title>")
            .next()
            .unwrap()
            .to_owned();
        if path == "/for-employers" {
            assert_eq!(title, "AI 自动筛简历与免费发布岗位 · HireFerry · 聘渡");
            assert!(!title.contains("聘渡 HireFerry · HireFerry"));
        }
        if path == "/for-job-tools" {
            assert_eq!(
                title,
                "本地求职工具与 HireFerry 公开发现层 · HireFerry · 聘渡"
            );
        }
        assert!(titles.insert(title), "unique page title: {path}");
    }
    let (_, _, robots) = read(&app, "/robots.txt").await;
    assert!(robots.contains("User-agent: *\nAllow: /"));
    assert!(robots.contains("Sitemap: https://hireferry.example/sitemap.xml"));
    assert!(!robots.contains("Disallow:"));
}

#[tokio::test]
async fn query_variants_share_canonical_and_private_pages_are_noindex() {
    let app = app(false);
    let (_, headers, home) = read(&app, "/").await;
    let (_, variant_headers, variant) = read(&app, "/?utm_source=example&role=seeker").await;
    assert_eq!(home, variant);
    assert_eq!(headers["etag"], variant_headers["etag"]);
    assert!(!home.contains("https://untrusted.example"));
    for path in [
        "/login",
        "/admin",
        "/app?tab=matches",
        "/jobs/private-id",
        "/people/private-id",
        "/human/private-link",
        "/team/invite",
        "/billing/return?order_id=example",
        "/oauth/device",
        "/api/session",
        "/static/index.html",
    ] {
        let (_, headers, html) = read(&app, path).await;
        assert_eq!(headers["x-robots-tag"], "noindex, follow", "{path}");
        assert!(!html.contains("rel=\"canonical\""), "{path}");
    }
    let (status, headers, _) = read(&app, "/does-not-exist").await;
    assert_eq!(status, 404);
    assert_eq!(headers["x-robots-tag"], "noindex, follow");
}

#[tokio::test]
async fn configured_language_and_actual_legal_document_are_in_initial_html() {
    let app = app(true);
    let (_, _, html) = read(&app, "/privacy").await;
    assert!(html.contains("lang=\"en\""));
    assert!(html.contains("<title>Privacy policy"));
    assert!(html.contains("<h1>"));
    assert!(html.contains("Who is responsible for your information"));
    assert!(html.contains("Information we collect"));
    assert!(!html.contains("{{"));
    let (_, _, home) = read(&app, "/").await;
    assert!(home.contains("itemtype=\"https://schema.org/WebSite\""));
    assert!(home.contains("itemprop=\"url\" href=\"https://hireferry.example\""));
    assert!(home.contains("Free for job seekers"));
}

#[tokio::test]
async fn downloads_are_public_and_release_links_are_crawlable_without_javascript() {
    let app = app(true);
    let (status, headers, html) = read(&app, "/downloads").await;
    assert_eq!(status, 200);
    assert!(!headers.contains_key("x-robots-tag"));
    assert!(html.contains("<title>Software downloads and installation"));
    assert!(html.contains("href=\"https://hireferry.example/downloads\""));
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("../static/downloads.json")).unwrap();
    for asset in catalog["assets"].as_array().unwrap() {
        assert!(html.contains(asset["url"].as_str().unwrap()));
        assert!(html.contains(asset["name"].as_str().unwrap()));
    }
    assert!(html.contains("not formally released yet"));
}

#[tokio::test]
async fn public_job_detail_initial_html_uses_public_metadata_and_json_ld() {
    let mut detail_config = Config {
        database: ":memory:".into(),
        public_url: "https://hireferry.example".into(),
        ..Config::default()
    };
    detail_config.webauthn_rp_id = "hireferry.example".into();
    let state = hireferry::AppState::new(detail_config).unwrap();
    state
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO users(id,email,name,handle,role,employer_paid,banned,created_at,updated_at) VALUES(?1,?2,?3,'open-source','employer',1,0,1,1)",
            params!["seo-employer", "seo-employer@example.test", "Open Source Co"],
        )
        .unwrap();
    state
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,currency,ai_min_salary_cents,headcount,status,created_at,updated_at) VALUES('seo-job','seo-employer','Rust API maintainer','## Build public APIs',250000,'CNY',300000,2,'published',1,1)",
            [],
        )
        .unwrap();
    let app = hireferry::app(state);
    let (status, _, html) = read(&app, "/discover/jobs/seo-job").await;
    assert_eq!(status, 200);
    assert!(html.contains("<title>Rust API maintainer · HireFerry · 聘渡</title>"));
    assert!(html.contains("<meta name=\"robots\" content=\"index, follow"));
    assert!(html.contains("data-hireferry-jsonld"));
    assert!(html.contains("\"@type\":\"JobPosting\""));
    assert!(html.contains("Build public APIs"));
    assert!(!html.contains("seo-employer@example.test"));
    let (_, _, sitemap) = read(&app, "/sitemap.xml").await;
    assert!(sitemap.contains("https://hireferry.example/discover/jobs/seo-job"));
}
