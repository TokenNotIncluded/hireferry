//! HTTP privacy and MCP authorization tests. Fixtures contain no personal data.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use hireferry::{AppState, api, auth, commerce, config::Config, models, safety};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    app: Router,
}

impl Fixture {
    fn new() -> Self {
        let mut cfg = Config {
            development: true,
            database: ":memory:".into(),
            ..Default::default()
        };
        cfg.payments.stripe.enabled = true;
        cfg.payments.stripe.secret_key = "sk_test_private_config_sentinel".into();
        cfg.payments.stripe.webhook_secret = "whsec_private_config_sentinel".into();
        cfg.google.client_secret = "google_private_config_sentinel".into();
        cfg.smtp.password = "smtp_private_config_sentinel".into();
        let state = AppState::new(cfg).unwrap();
        let app = hireferry::app(state.clone());
        Self { state, app }
    }

    fn user(&self, role: &str, visible: bool, banned: bool) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let avatar = "svg:<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 20 20\"><circle cx=\"10\" cy=\"10\" r=\"8\" fill=\"#22aa88\"/></svg>";
        self.state.conn().unwrap().execute(
            "INSERT INTO users(id,email,google_sub,name,role,employer_paid,avatar,public_email,resume_md,visible,banned,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?12)",
            params![id, login_email(&id), format!("google-sub-{id}"), format!("Profile {id}"), role, role == "employer", avatar,
                format!("public-{id}@example.invalid"), format!("# Resume\n\nResume marker {id}"), visible, banned, hireferry::now()],
        ).unwrap();
        id
    }

    fn token(&self, user: &str) -> (String, String) {
        let id = uuid::Uuid::new_v4().to_string();
        let token = format!("hf_test_{}", uuid::Uuid::new_v4());
        self.state.conn().unwrap().execute(
            "INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES(?1,?2,'privacy test',?3,?4,?5)",
            params![id, user, auth::hash_token(&token), hireferry::now(), hireferry::now() + 600],
        ).unwrap();
        (id, token)
    }

    fn job(&self, owner: &str, status: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.state.conn().unwrap().execute(
            "INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,status,version,created_at,updated_at)
             VALUES(?1,?2,'A public job','Safe description',300000,?3,1,?4,?4)",
            params![id, owner, status, hireferry::now()],
        ).unwrap();
        id
    }
}

fn login_email(id: &str) -> String {
    format!("private-login-{id}@example.invalid")
}

struct Observation {
    status: StatusCode,
    headers: HeaderMap,
    value: Value,
    text: String,
}

async fn observe(app: &Router, request: Request<Body>) -> Observation {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Observation {
        status,
        headers,
        value,
        text,
    }
}

async fn get(app: &Router, path: &str) -> Observation {
    observe(
        app,
        Request::builder().uri(path).body(Body::empty()).unwrap(),
    )
    .await
}

async fn rpc(app: &Router, token: Option<&str>, method: &str, params: Value) -> Observation {
    let mut request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    observe(
        app,
        request
            .body(Body::from(
                json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
            ))
            .unwrap(),
    )
    .await
}

async fn tool(app: &Router, token: &str, name: &str, arguments: Value) -> Observation {
    rpc(
        app,
        Some(token),
        "tools/call",
        json!({"name":name,"arguments":arguments}),
    )
    .await
}

fn no_login_data(response: &Observation, users: &[&str]) {
    assert!(
        !response.text.contains("\"email\""),
        "public output contains login email field: {}",
        response.text
    );
    assert!(!response.text.contains("google_sub"));
    for id in users {
        assert!(!response.text.contains(&login_email(id)));
        assert!(!response.text.contains(&format!("google-sub-{id}")));
    }
}

#[tokio::test]
async fn anonymous_visitors_cannot_discover_profiles_jobs_or_avatars() {
    let f = Fixture::new();
    // Legacy visible=true is deliberately not a public-discovery permission.
    let public = f.user("seeker", true, false);
    let private = f.user("seeker", false, false);
    let banned = f.user("seeker", true, true);
    let employer = f.user("employer", false, false);
    let job = f.job(&employer, "published");
    for path in ["/api/people", "/api/jobs"] {
        let response = get(&f.app, path).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
        for id in [&public, &private, &banned, &employer, &job] {
            assert!(!response.text.contains(id));
        }
    }
    let missing = get(&f.app, "/api/people/missing").await;
    let missing_avatar = get(&f.app, "/api/avatars/missing").await;
    for user in [&public, &private, &banned] {
        let detail = get(&f.app, &format!("/api/people/{user}")).await;
        assert_eq!(detail.status, StatusCode::NOT_FOUND);
        assert_eq!(detail.value, missing.value);
    }
    for user in [&public, &private, &banned, &employer] {
        let avatar = get(&f.app, &format!("/api/avatars/{user}")).await;
        assert_eq!(avatar.status, StatusCode::NOT_FOUND);
        assert_eq!(avatar.value, missing_avatar.value);
    }
    let detail = get(&f.app, &format!("/api/jobs/{job}")).await;
    assert_eq!(detail.status, StatusCode::NOT_FOUND);
    let config = get(&f.app, "/api/config").await;
    assert_eq!(config.status, StatusCode::OK);
    assert!(!config.text.contains("private_config_sentinel"));
}

async fn authenticated_get(app: &Router, path: &str, token: &str) -> Observation {
    observe(
        app,
        Request::builder()
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

fn pay_for_job(f: &Fixture, owner: &str, job: &str) {
    f.state.conn().unwrap().execute("INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,status,created_at,paid_at) VALUES(?1,?2,'publish',?3,30000,'stripe','paid',?4,?4)",params![format!("paid-{job}"),owner,job,hireferry::now()]).unwrap();
}

#[tokio::test]
async fn exact_matches_open_only_corresponding_profiles_and_jobs_then_revoke_immediately() {
    let f = Fixture::new();
    let seeker = f.user("seeker", false, false);
    let employer = f.user("employer", false, false);
    let stranger = f.user("seeker", true, false);
    let job = f.job(&employer, "published");
    let other_job = f.job(&employer, "published");
    pay_for_job(&f, &employer, &job);
    let (_, seeker_token) = f.token(&seeker);
    let (_, employer_token) = f.token(&employer);
    let (_, stranger_token) = f.token(&stranger);
    assert_eq!(
        authenticated_get(&f.app, "/api/jobs", &seeker_token)
            .await
            .value["total"],
        0
    );
    assert_eq!(
        authenticated_get(&f.app, "/api/people", &employer_token)
            .await
            .value["total"],
        0
    );
    for (token, path) in [
        (&seeker_token, format!("/api/jobs/{job}")),
        (&employer_token, format!("/api/people/{seeker}")),
    ] {
        assert_eq!(
            authenticated_get(&f.app, &path, token).await.status,
            StatusCode::NOT_FOUND
        );
    }
    f.state.conn().unwrap().execute("INSERT INTO matches(id,seeker_id,job_id,created_at,updated_at) VALUES('exact-match',?1,?2,?3,?3)",params![seeker,job,hireferry::now()]).unwrap();
    let jobs = authenticated_get(&f.app, "/api/jobs", &seeker_token).await;
    assert_eq!(jobs.value["total"], 1);
    assert_eq!(jobs.value["items"][0]["id"], job);
    let people = authenticated_get(&f.app, "/api/people", &employer_token).await;
    assert_eq!(people.value["total"], 1);
    assert_eq!(people.value["items"][0]["id"], seeker);
    no_login_data(&people, &[&seeker, &employer]);
    for (token, path) in [
        (&seeker_token, format!("/api/jobs/{job}")),
        (&employer_token, format!("/api/people/{seeker}")),
    ] {
        let response = authenticated_get(&f.app, &path, token).await;
        assert_eq!(response.status, StatusCode::OK);
        no_login_data(&response, &[&seeker, &employer]);
    }
    let avatar =
        authenticated_get(&f.app, &format!("/api/avatars/{seeker}"), &employer_token).await;
    assert_eq!(avatar.status, StatusCode::OK);
    assert_eq!(avatar.headers["x-content-type-options"], "nosniff");
    assert!(
        avatar.headers["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("object-src 'none'")
    );
    assert_eq!(
        authenticated_get(&f.app, &format!("/api/jobs/{other_job}"), &seeker_token)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        authenticated_get(&f.app, &format!("/api/people/{seeker}"), &stranger_token)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        authenticated_get(&f.app, &format!("/api/avatars/{seeker}"), &stranger_token)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    // Closing a hired role does not erase its existing introduction.
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE jobs SET status='closed' WHERE id=?1", [&job])
        .unwrap();
    assert_eq!(
        authenticated_get(&f.app, &format!("/api/jobs/{job}"), &seeker_token)
            .await
            .status,
        StatusCode::OK
    );
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE jobs SET status='published' WHERE id=?1", [&job])
        .unwrap();
    for revoke in [
        "UPDATE matches SET status='declined' WHERE id='exact-match'",
        "UPDATE users SET banned=1 WHERE role='employer'",
        "UPDATE jobs SET status='deleted' WHERE id IN(SELECT job_id FROM matches)",
    ] {
        f.state.conn().unwrap().execute(revoke, []).unwrap();
        assert_eq!(
            authenticated_get(&f.app, &format!("/api/jobs/{job}"), &seeker_token)
                .await
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            authenticated_get(&f.app, &format!("/api/avatars/{employer}"), &seeker_token)
                .await
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            authenticated_get(&f.app, "/api/matches", &seeker_token)
                .await
                .value["items"],
            json!([])
        );
        f.state
            .conn()
            .unwrap()
            .execute("UPDATE matches SET status='introduced'", [])
            .unwrap();
        f.state
            .conn()
            .unwrap()
            .execute("UPDATE users SET banned=0", [])
            .unwrap();
        f.state
            .conn()
            .unwrap()
            .execute("UPDATE jobs SET status='published'", [])
            .unwrap();
    }
}

#[tokio::test]
async fn employer_ai_requires_paid_published_base_salary_at_least_3000_on_every_request() {
    let f = Fixture::new();
    let employer = f.user("employer", false, false);
    let job = f.job(&employer, "published");
    let (_, token) = f.token(&employer);
    let raw = "employer-eligibility-session";
    f.state.conn().unwrap().execute("INSERT INTO sessions(token_hash,user_id,csrf,expires_at,recent_auth_at) VALUES(?1,?2,'test-csrf',?3,?4)",params![auth::hash_token(raw),employer,hireferry::now()+600,hireferry::now()]).unwrap();
    assert_eq!(
        rpc(&f.app, Some(&token), "tools/list", json!({}))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    pay_for_job(&f, &employer, &job);
    for (salary, status, allowed) in [
        (299999, "published", false),
        (300000, "draft", false),
        (300000, "published", true),
        (300000, "closed", false),
        (600000, "published", true),
    ] {
        f.state
            .conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET salary_cents=?2,status=?3 WHERE id=?1",
                params![job, salary, status],
            )
            .unwrap();
        let expected = if allowed {
            StatusCode::OK
        } else {
            StatusCode::FORBIDDEN
        };
        assert_eq!(
            rpc(&f.app, Some(&token), "tools/list", json!({}))
                .await
                .status,
            expected
        );
        assert_eq!(
            authenticated_get(&f.app, "/api/profile", &token)
                .await
                .status,
            expected
        );
        let webmcp = observe(
            &f.app,
            Request::builder()
                .uri("/api/profile")
                .header("cookie", format!("hf_session={raw}"))
                .header("x-hireferry-ai", "webmcp")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(webmcp.status, expected);
        let interactive = observe(
            &f.app,
            Request::builder()
                .uri("/api/profile")
                .header("cookie", format!("hf_session={raw}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(interactive.status, StatusCode::OK);
    }
    // Changing a receipt to needs-review removes qualification even for a published job.
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE orders SET status='paid_needs_review'", [])
        .unwrap();
    assert_eq!(
        rpc(&f.app, Some(&token), "tools/list", json!({}))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn mcp_requires_a_bearer_token_and_initializes_with_the_declared_tools() {
    let f = Fixture::new();
    let user = f.user("seeker", false, false);
    let (_, token) = f.token(&user);
    let unsigned = rpc(
        &f.app,
        None,
        "initialize",
        json!({"protocolVersion":"2025-11-25"}),
    )
    .await;
    assert_eq!(unsigned.status, StatusCode::UNAUTHORIZED);
    assert!(
        unsigned.headers["www-authenticate"]
            .to_str()
            .unwrap()
            .starts_with("Bearer")
    );
    assert_eq!(
        rpc(&f.app, Some("invalid-token"), "tools/list", json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );

    let initialize = rpc(&f.app, Some(&token), "initialize", json!({
        "protocolVersion":"2025-11-25", "capabilities":{}, "clientInfo":{"name":"privacy-test","version":"1"}
    })).await;
    assert_eq!(initialize.status, StatusCode::OK);
    assert_eq!(initialize.value["jsonrpc"], "2.0");
    assert_eq!(initialize.value["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        initialize.value["result"]["serverInfo"]["name"],
        "hireferry"
    );
    let listed = rpc(&f.app, Some(&token), "tools/list", json!({})).await;
    let tools = listed.value["result"]["tools"].as_array().unwrap();
    for required in [
        "get_profile",
        "update_profile",
        "set_avatar_svg",
        "set_image_avatar",
        "list_people",
        "get_person",
        "get_order",
        "create_job",
        "edit_job",
        "close_job",
        "create_checkout",
    ] {
        assert!(
            tools.iter().any(|t| t["name"] == required),
            "missing {required}"
        );
    }
    assert!(
        !tools
            .iter()
            .any(|t| t["name"].as_str().unwrap().contains("admin"))
    );
    let cross_origin = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("origin", "https://attacker.example.invalid")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string(),
        ))
        .unwrap();
    assert_eq!(
        observe(&f.app, cross_origin).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn mcp_limits_resume_order_and_job_access_to_the_authenticated_account() {
    let f = Fixture::new();
    let owner = f.user("employer", false, false);
    let other = f.user("employer", false, false);
    let private = f.user("seeker", false, false);
    let banned = f.user("seeker", true, true);
    let public = f.user("seeker", true, false);
    let (_, owner_token) = f.token(&owner);
    let (_, other_token) = f.token(&other);
    let published = f.job(&owner, "published");
    pay_for_job(&f, &owner, &published);
    let other_qualification = f.job(&other, "published");
    pay_for_job(&f, &other, &other_qualification);
    let (draft, order) = commerce::new_job(
        &f.state,
        &owner,
        "Private draft title",
        "Private draft description",
        300_000,
        "stripe",
    )
    .unwrap();

    for (name, args) in [
        ("get_person", json!({"id":private})),
        ("get_person", json!({"id":banned})),
        ("get_job", json!({"id":draft.id})),
        ("get_order", json!({"id":order.id})),
        ("create_checkout", json!({"id":order.id})),
        (
            "edit_job",
            json!({"id":published,"jd_md":"Unauthorized replacement","provider":"stripe"}),
        ),
        ("close_job", json!({"id":published})),
    ] {
        let response = tool(&f.app, &other_token, name, args).await;
        assert_eq!(response.status, StatusCode::OK, "{name}");
        assert_eq!(
            response.value["result"]["isError"], true,
            "{name}: {}",
            response.text
        );
        assert!(response.value["result"].get("structuredContent").is_none());
        assert!(!response.text.contains("Private draft description"));
        no_login_data(&response, &[&owner, &other, &private, &banned]);
    }
    let listing = tool(&f.app, &other_token, "list_people", json!({})).await;
    let data = &listing.value["result"]["structuredContent"];
    assert_eq!(data["total"], 0);
    assert_eq!(data["items"], json!([]));
    no_login_data(&listing, &[&owner, &other, &private, &banned, &public]);
    let owned = tool(&f.app, &owner_token, "get_order", json!({"id":order.id})).await;
    assert_eq!(owned.value["result"]["isError"], false);
    assert_eq!(owned.value["result"]["structuredContent"]["id"], order.id);
    let unowned = tool(&f.app, &other_token, "list_orders", json!({})).await;
    assert!(
        unowned.value["result"]["structuredContent"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["user_id"] == other)
    );
    assert!(!unowned.text.contains(&order.id));
    let current = models::load_job(&f.state.conn().unwrap(), &published).unwrap();
    assert_eq!(current.status, "published");
    assert_eq!(current.jd_md, "Safe description");
    let started: Option<i64> = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT checkout_started_at FROM orders WHERE id=?1",
            [&order.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        started.is_none(),
        "cross-account checkout must be rejected before any provider request"
    );
}

#[tokio::test]
async fn revoked_expired_and_banned_tokens_are_rechecked_on_every_mcp_request() {
    let f = Fixture::new();
    let user = f.user("seeker", false, false);
    for invalidation in ["revoked", "expired", "banned"] {
        let (id, token) = f.token(&user);
        assert_eq!(
            rpc(&f.app, Some(&token), "tools/list", json!({}))
                .await
                .status,
            StatusCode::OK
        );
        match invalidation {
            "revoked" => {
                f.state
                    .conn()
                    .unwrap()
                    .execute("UPDATE api_tokens SET revoked=1 WHERE id=?1", [&id])
                    .unwrap();
            }
            "expired" => {
                f.state
                    .conn()
                    .unwrap()
                    .execute("UPDATE api_tokens SET expires_at=0 WHERE id=?1", [&id])
                    .unwrap();
            }
            _ => {
                f.state
                    .conn()
                    .unwrap()
                    .execute("UPDATE users SET banned=1 WHERE id=?1", [&user])
                    .unwrap();
            }
        }
        let denied = tool(&f.app, &token, "update_profile", json!({"name":"Changed by old token","public_email":"contact@example.invalid","resume_md":"Changed","visible":false})).await;
        assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
        assert_ne!(
            models::load_user(&f.state.conn().unwrap(), &user)
                .unwrap()
                .name,
            "Changed by old token"
        );
    }
}

#[tokio::test]
async fn administrator_ban_removes_sessions_and_revokes_all_agent_tokens() {
    let f = Fixture::new();
    let user = f.user("seeker", true, false);
    let (_, first) = f.token(&user);
    let (_, second) = f.token(&user);
    let admin_raw = "test_admin_session_only";
    let csrf = "test_admin_csrf";
    {
        let db = f.state.conn().unwrap();
        // Seed an already authenticated admin; passkey authentication has its own tests.
        db.execute("INSERT INTO sessions(token_hash,admin,bootstrap,csrf,expires_at,recent_auth_at) VALUES(?1,1,0,?2,?3,?4)", params![auth::hash_token(admin_raw),csrf,hireferry::now()+600,hireferry::now()]).unwrap();
        db.execute("INSERT INTO sessions(token_hash,user_id,csrf,expires_at,recent_auth_at) VALUES(?1,?2,'user-csrf',?3,?4)", params![auth::hash_token("test_user_session"),user,hireferry::now()+600,hireferry::now()]).unwrap();
    }
    let request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/admin/users/{user}"))
        .header("content-type", "application/json")
        .header("origin", "http://localhost:3000")
        .header("cookie", format!("hf_session={admin_raw}"))
        .header("x-csrf-token", csrf)
        .body(Body::from(json!({"banned":true}).to_string()))
        .unwrap();
    assert_eq!(observe(&f.app, request).await.status, StatusCode::OK);
    {
        let db = f.state.conn().unwrap();
        let sessions: i64 = db
            .query_row(
                "SELECT count(*) FROM sessions WHERE user_id=?1",
                [&user],
                |r| r.get(0),
            )
            .unwrap();
        let active: i64 = db
            .query_row(
                "SELECT count(*) FROM api_tokens WHERE user_id=?1 AND revoked=0",
                [&user],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sessions, 0);
        assert_eq!(active, 0);
    }
    for token in [first, second] {
        assert_eq!(
            rpc(&f.app, Some(&token), "tools/list", json!({}))
                .await
                .status,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        get(&f.app, &format!("/api/people/{user}")).await.status,
        StatusCode::NOT_FOUND
    );
}

fn avatar_bytes() -> Vec<u8> {
    let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        512,
        256,
        image::Rgb([32, 160, 120]),
    ));
    let mut encoded = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Jpeg)
        .unwrap();
    let mut bytes = encoded.into_inner();
    bytes.extend_from_slice(b"<script>private_avatar_metadata_sentinel</script>");
    bytes
}

fn stored_avatar(f: &Fixture, user: &str) -> String {
    f.state
        .conn()
        .unwrap()
        .query_row("SELECT avatar FROM users WHERE id=?1", [user], |r| r.get(0))
        .unwrap()
}

#[tokio::test]
async fn bearer_image_avatar_is_owned_bounded_and_reencoded_like_http_uploads() {
    let f = Fixture::new();
    let owner = f.user("seeker", false, false);
    let other = f.user("seeker", false, false);
    let (_, token) = f.token(&owner);
    let original = stored_avatar(&f, &owner);
    let other_original = stored_avatar(&f, &other);
    let bytes = avatar_bytes();
    let image_base64 = STANDARD.encode(&bytes);

    // The caller cannot choose a target account, even with a valid bearer token.
    let denied = tool(
        &f.app,
        &token,
        "set_image_avatar",
        json!({
            "image_base64":image_base64,"user_id":other
        }),
    )
    .await;
    assert_eq!(denied.value["result"]["isError"], true);
    assert_eq!(stored_avatar(&f, &owner), original);
    assert_eq!(stored_avatar(&f, &other), other_original);

    let uploaded = tool(
        &f.app,
        &token,
        "set_image_avatar",
        json!({
            "image_base64":image_base64
        }),
    )
    .await;
    assert_eq!(uploaded.status, StatusCode::OK);
    assert_eq!(
        uploaded.value["result"]["isError"], false,
        "{}",
        uploaded.text
    );
    assert_eq!(uploaded.value["result"]["structuredContent"]["id"], owner);
    assert_eq!(
        uploaded.value["result"]["structuredContent"]["avatar"],
        format!("/api/avatars/{owner}")
    );
    assert_eq!(stored_avatar(&f, &other), other_original);

    let stored = stored_avatar(&f, &owner);
    let clean = STANDARD
        .decode(stored.strip_prefix("png:").unwrap())
        .unwrap();
    assert_eq!(
        image::guess_format(&clean).unwrap(),
        image::ImageFormat::Png
    );
    let image = image::load_from_memory(&clean).unwrap();
    assert_eq!((image.width(), image.height()), (256, 128));
    assert!(
        !clean
            .windows(b"private_avatar_metadata_sentinel".len())
            .any(|part| part == b"private_avatar_metadata_sentinel")
    );

    // The unchanged multipart route uses exactly the same sanitization pipeline.
    let boundary = "hireferry-avatar-test";
    let mut multipart = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"avatar\"; filename=\"avatar.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n").into_bytes();
    multipart.extend_from_slice(&bytes);
    multipart.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let http_upload = observe(
        &f.app,
        Request::builder()
            .method("POST")
            .uri("/api/profile/avatar")
            .header("authorization", format!("Bearer {token}"))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(multipart))
            .unwrap(),
    )
    .await;
    assert_eq!(http_upload.status, StatusCode::OK, "{}", http_upload.text);
    assert_eq!(http_upload.value["id"], owner);
    assert_eq!(stored_avatar(&f, &owner), stored);

    // This length has the same encoded length as the largest allowed image;
    // checking Base64 length alone would accept it.
    let oversized = STANDARD.encode(vec![0u8; api::MAX_AVATAR_IMAGE_BYTES + 1]);
    assert_eq!(oversized.len(), api::MAX_AVATAR_BASE64_LEN);
    for invalid in [oversized, format!("data:image/jpeg;base64,{image_base64}")] {
        let rejected = tool(
            &f.app,
            &token,
            "set_image_avatar",
            json!({"image_base64":invalid}),
        )
        .await;
        assert_eq!(rejected.value["result"]["isError"], true);
        assert_eq!(stored_avatar(&f, &owner), stored);
    }
}

#[test]
fn avatar_commit_rechecks_a_ban_that_occurs_after_authentication() {
    let f = Fixture::new();
    let owner = f.user("seeker", false, false);
    let (_, token) = f.token(&owner);
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    let authenticated = hireferry::user_id(&f.state, &headers, true).unwrap();
    let original = stored_avatar(&f, &owner);
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id=?1", [&owner])
        .unwrap();

    // Model the account being suspended while bytes are being read or decoded.
    let image_error =
        api::save_image_avatar(&f.state, &authenticated, &avatar_bytes()).unwrap_err();
    assert_eq!(image_error.status, StatusCode::FORBIDDEN);
    let svg_error = api::save_svg_avatar(&f.state, &authenticated,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><circle cx="10" cy="10" r="8"/></svg>"#).unwrap_err();
    assert_eq!(svg_error.status, StatusCode::FORBIDDEN);
    assert_eq!(stored_avatar(&f, &owner), original);
}

#[test]
fn rendered_markdown_has_no_executable_markup_or_tracking_images() {
    let input = r#"# Safe heading

**Readable text** and [safe link](https://example.invalid/work).

<script src="https://tracker.example.invalid/script.js">alert(1)</script>
<iframe src="https://tracker.example.invalid/frame"></iframe>
<img src="https://tracker.example.invalid/raw-pixel" onerror="alert(1)">

![pixel](https://tracker.example.invalid/markdown-pixel)
[unsafe](javascript:alert%281%29)
[data](data:text/html;base64,PHNjcmlwdD4=)
"#;
    let html = safety::markdown(input);
    assert!(html.contains("<h1>Safe heading</h1>"));
    assert!(html.contains("<strong>Readable text</strong>"));
    assert!(html.contains("href=\"https://example.invalid/work\""));
    assert!(html.contains("noopener noreferrer nofollow"));
    for rejected in [
        "<script",
        "<iframe",
        "<img",
        "onerror=",
        "javascript:",
        "data:text/html",
        "tracker.example.invalid",
    ] {
        assert!(
            !html.to_ascii_lowercase().contains(rejected),
            "unsafe output {rejected}: {html}"
        );
    }
}

#[test]
fn svg_allows_local_shapes_and_safe_animation_but_rejects_active_content_and_external_references() {
    let safe = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><defs><linearGradient id="paint"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></linearGradient></defs><circle cx="10" cy="10" r="8" fill="url(#paint)"/></svg>"##;
    assert!(safety::safe_svg(safe).is_ok());
    let animated = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><g><circle cx="10" cy="10" r="8"/><animateTransform attributeName="transform" attributeType="XML" type="rotate" values="0 10 10;360 10 10" dur="2s" repeatCount="indefinite"/></g></svg>"##;
    assert!(safety::safe_svg(animated).is_ok());
    for unsafe_svg in [
        r#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject><div>HTML</div></foreignObject></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><path xlink:href="https://tracker.example.invalid/image"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><circle fill="url(https://tracker.example.invalid/image)"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><circle fill="u\72l(https://tracker.example.invalid/image)"/></svg>"#,
        r#"<?xml-stylesheet type="text/css" href="https://tracker.example.invalid/style.css"?><svg xmlns="http://www.w3.org/2000/svg"></svg>"#,
        r#"<!DOCTYPE svg [<!ENTITY external SYSTEM "https://tracker.example.invalid/entity">]><svg xmlns="http://www.w3.org/2000/svg"><title>&external;</title></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><circle cx="10" cy="10" r="8"><animate attributeName="href" to="https://tracker.example.invalid/image" dur="1s"/></circle></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><circle cx="10" cy="10" r="8"><animate attributeName="opacity" values="0;1" onbegin="alert(1)" dur="1s"/></circle></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><circle cx="10" cy="10" r="8"><animate attributeName="opacity" values="url(https://tracker.example.invalid/image)" dur="1s"/></circle></svg>"#,
    ] {
        assert!(
            safety::safe_svg(unsafe_svg).is_err(),
            "SVG input was accepted: {unsafe_svg}"
        );
    }
}
