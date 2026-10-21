//! Administrator AI credentials are separate from both member identities and
//! Passkey management. All fixtures are local test identities, never live keys.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use hireferry::{AppState, access, admin, admin_ai, auth, config::Config, models};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    app: Router,
}

struct Browser {
    raw: String,
    csrf: String,
}

impl Fixture {
    fn new() -> Self {
        let cfg = Config {
            development: true,
            database: ":memory:".into(),
            ..Default::default()
        };
        let state = AppState::new(cfg).unwrap();
        let app = hireferry::app(state.clone());
        Self { state, app }
    }

    fn admin_browser(&self, recent_at: i64, bootstrap: bool) -> Browser {
        let browser = Browser {
            raw: format!("test_admin_cookie_{}", uuid::Uuid::new_v4()),
            csrf: format!("test_csrf_{}", uuid::Uuid::new_v4()),
        };
        self.state.conn().unwrap().execute(
            "INSERT INTO sessions(token_hash,admin,bootstrap,csrf,expires_at,recent_auth_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![auth::hash_token(&browser.raw), !bootstrap, bootstrap, browser.csrf, hireferry::now() + 3600, recent_at],
        ).unwrap();
        browser
    }

    fn member(&self) -> (String, String) {
        let id = uuid::Uuid::new_v4().to_string();
        let token = format!("hf_{}", hex::encode(rand::random::<[u8; 32]>()));
        let db = self.state.conn().unwrap();
        db.execute(
            "INSERT INTO users(id,email,name,role,resume_md,created_at,updated_at) VALUES(?1,?2,'Private candidate','seeker','Private resume',?3,?3)",
            params![id, format!("private-{id}@example.invalid"), hireferry::now()],
        ).unwrap();
        db.execute(
            "INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES(?1,?2,'member test',?3,?4,?5)",
            params![uuid::Uuid::new_v4().to_string(), id, auth::hash_token(&token), hireferry::now(), hireferry::now() + 3600],
        ).unwrap();
        (id, token)
    }

    async fn issue(&self, browser: &Browser, name: &str) -> (String, String) {
        let result = request(
            &self.app,
            "POST",
            "/api/admin/tokens",
            Some(browser),
            None,
            Some(json!({"name":name})),
        )
        .await;
        assert_eq!(result.status, StatusCode::OK, "{}", result.text);
        (
            result.value["id"].as_str().unwrap().into(),
            result.value["token"].as_str().unwrap().into(),
        )
    }
}

struct Response {
    status: StatusCode,
    value: Value,
    text: String,
}

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    browser: Option<&Browser>,
    bearer: Option<&str>,
    body: Option<Value>,
) -> Response {
    let mut request = Request::builder().method(method).uri(path);
    if path == "/mcp" {
        request = request
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25");
    }
    if let Some(browser) = browser {
        request = request
            .header("cookie", format!("hf_session={}", browser.raw))
            .header("origin", "http://localhost:3000")
            .header("x-csrf-token", &browser.csrf);
    }
    if let Some(bearer) = bearer {
        request = request.header("authorization", format!("Bearer {bearer}"));
    }
    let body = if let Some(body) = body {
        request = request.header("content-type", "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Response {
        status,
        value,
        text,
    }
}

fn denied(response: &Response) {
    assert!(
        matches!(
            response.status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ),
        "expected authentication/authorization rejection, got {}: {}",
        response.status,
        response.text
    );
}

async fn rpc(app: &Router, token: &str, method: &str, params: Value) -> Response {
    request(
        app,
        "POST",
        "/mcp",
        None,
        Some(token),
        Some(json!({
            "jsonrpc":"2.0", "id":1, "method":method, "params":params
        })),
    )
    .await
}

#[tokio::test]
async fn admin_token_is_hashed_30_days_and_bearer_cannot_manage_credentials() {
    let f = Fixture::new();
    let browser = f.admin_browser(hireferry::now(), false);
    let before = hireferry::now();
    let (id, raw) = f.issue(&browser, "Platform assistant").await;
    assert!(raw.starts_with("hf_admin_"));
    assert_eq!(raw.len(), 73);
    let (stored_hash, expires_at): (String, i64) = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT token_hash,expires_at FROM admin_api_tokens WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored_hash, auth::hash_token(&raw));
    assert_ne!(stored_hash, raw);
    assert!((before + 30 * 86400..=hireferry::now() + 30 * 86400).contains(&expires_at));
    assert_eq!(admin_ai::verify(&f.state, &raw).unwrap(), Some(id.clone()));
    let listed = request(
        &f.app,
        "GET",
        "/api/admin/tokens",
        Some(&browser),
        None,
        None,
    )
    .await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.value["items"][0]["id"], id);
    assert!(listed.value["items"][0]["last_used_at"].is_i64());
    assert!(!listed.text.contains(&raw));
    assert!(!listed.text.contains("token_hash"));

    for (method, path, body) in [
        ("GET", "/api/admin/tokens".to_string(), None),
        (
            "POST",
            "/api/admin/tokens".to_string(),
            Some(json!({"name":"Unauthorized new token"})),
        ),
        ("DELETE", format!("/api/admin/tokens/{id}"), None),
        ("GET", "/api/admin/passkeys".to_string(), None),
        (
            "POST",
            "/api/admin/passkeys/register/start".to_string(),
            Some(json!({"name":"Unauthorized key"})),
        ),
    ] {
        denied(&request(&f.app, method, &path, None, Some(&raw), body).await);
    }
    // Even a request that attaches both credentials must use the dedicated
    // interactive flow, instead of silently escalating an agent credential.
    denied(
        &request(
            &f.app,
            "POST",
            "/api/admin/tokens",
            Some(&browser),
            Some(&raw),
            Some(json!({"name":"Mixed credentials"})),
        )
        .await,
    );
    assert_eq!(admin_ai::verify(&f.state, &raw).unwrap(), Some(id));
}

#[tokio::test]
async fn issuance_requires_a_real_recent_admin_cookie_and_valid_csrf() {
    let f = Fixture::new();
    denied(
        &request(
            &f.app,
            "POST",
            "/api/admin/tokens",
            None,
            None,
            Some(json!({"name":"Unsigned"})),
        )
        .await,
    );
    let bootstrap = f.admin_browser(hireferry::now(), true);
    denied(
        &request(
            &f.app,
            "POST",
            "/api/admin/tokens",
            Some(&bootstrap),
            None,
            Some(json!({"name":"Bootstrap"})),
        )
        .await,
    );
    let stale = f.admin_browser(hireferry::now() - 601, false);
    denied(
        &request(
            &f.app,
            "POST",
            "/api/admin/tokens",
            Some(&stale),
            None,
            Some(json!({"name":"Stale authentication"})),
        )
        .await,
    );
    let mut tampered = f.admin_browser(hireferry::now(), false);
    tampered.csrf = "incorrect-csrf".into();
    denied(
        &request(
            &f.app,
            "POST",
            "/api/admin/tokens",
            Some(&tampered),
            None,
            Some(json!({"name":"Wrong CSRF"})),
        )
        .await,
    );
    let count: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM admin_api_tokens", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let recent = f.admin_browser(hireferry::now(), false);
    f.issue(&recent, "Recently authenticated").await;
}

#[tokio::test]
async fn admin_and_member_tokens_keep_distinct_privileges_and_actor_audit() {
    let f = Fixture::new();
    let browser = f.admin_browser(hireferry::now(), false);
    let (token_id, admin_token) = f.issue(&browser, "Platform worker").await;
    let (user, member_token) = f.member();
    assert_eq!(admin_ai::verify(&f.state, &member_token).unwrap(), None);
    for path in [
        "/api/admin/overview",
        "/api/admin/users",
        "/api/admin/orders",
    ] {
        denied(&request(&f.app, "GET", path, None, Some(&member_token), None).await);
        assert_eq!(
            request(&f.app, "GET", path, None, Some(&admin_token), None)
                .await
                .status,
            StatusCode::OK
        );
    }
    for path in ["/api/profile", "/api/orders", "/api/tokens"] {
        denied(&request(&f.app, "GET", path, None, Some(&admin_token), None).await);
    }
    denied(
        &request(
            &f.app,
            "PATCH",
            &format!("/api/admin/users/{user}"),
            None,
            Some(&member_token),
            Some(json!({"name":"Member escalation"})),
        )
        .await,
    );
    let changed = request(
        &f.app,
        "PATCH",
        &format!("/api/admin/users/{user}"),
        None,
        Some(&admin_token),
        Some(json!({"name":"Administrator reviewed"})),
    )
    .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.value["name"], "Administrator reviewed");
    assert_eq!(
        changed.value["email"],
        format!("private-{user}@example.invalid")
    );
    let actor: String = f.state.conn().unwrap().query_row(
        "SELECT actor FROM audit WHERE action='user_update' AND target=?1 ORDER BY id DESC LIMIT 1",
        [&user], |r| r.get(0),
    ).unwrap();
    assert_eq!(actor, format!("admin_ai:{token_id}"));
}

#[tokio::test]
async fn revoked_and_expired_admin_tokens_are_rechecked_inside_business_operations() {
    let f = Fixture::new();
    let browser = f.admin_browser(hireferry::now(), false);
    let (user, _) = f.member();
    for invalidation in ["revoked", "expired"] {
        let (id, raw) = f.issue(&browser, invalidation).await;
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {raw}").parse().unwrap());
        let principal = access::principal(&f.state, &headers, true).unwrap();
        if invalidation == "revoked" {
            let response = request(
                &f.app,
                "DELETE",
                &format!("/api/admin/tokens/{id}"),
                Some(&browser),
                None,
                None,
            )
            .await;
            assert_eq!(response.status, StatusCode::NO_CONTENT);
        } else {
            f.state
                .conn()
                .unwrap()
                .execute(
                    "UPDATE admin_api_tokens SET expires_at=?2 WHERE id=?1",
                    params![id, hireferry::now() - 1],
                )
                .unwrap();
        }
        assert_eq!(admin_ai::verify(&f.state, &raw).unwrap(), None);
        assert!(admin_ai::ensure_token(&f.state.conn().unwrap(), &id).is_err());
        denied(&request(&f.app, "GET", "/api/admin/overview", None, Some(&raw), None).await);
        let patch: admin::UserPatch =
            serde_json::from_value(json!({"name":"Stale agent mutation"})).unwrap();
        let error = admin::patch_user(&f.state, &principal, &user, patch).unwrap_err();
        assert!(matches!(
            error.status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
        assert_ne!(
            models::load_user(&f.state.conn().unwrap(), &user)
                .unwrap()
                .name,
            "Stale agent mutation"
        );
    }
}

#[tokio::test]
async fn admin_token_limit_counts_only_unexpired_unrevoked_tokens() {
    let f = Fixture::new();
    let browser = f.admin_browser(hireferry::now(), false);
    let mut first = String::new();
    for number in 0..10 {
        let (id, _) = f.issue(&browser, &format!("Worker {number}")).await;
        if number == 0 {
            first = id;
        }
    }
    let full = request(
        &f.app,
        "POST",
        "/api/admin/tokens",
        Some(&browser),
        None,
        Some(json!({"name":"Worker 11"})),
    )
    .await;
    assert_eq!(full.status, StatusCode::CONFLICT);
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE admin_api_tokens SET expires_at=0 WHERE id=?1",
            [&first],
        )
        .unwrap();
    let (id, _) = f.issue(&browser, "Replace expired").await;
    assert_eq!(
        request(
            &f.app,
            "DELETE",
            &format!("/api/admin/tokens/{id}"),
            Some(&browser),
            None,
            None
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    f.issue(&browser, "Replace revoked").await;
    let active: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM admin_api_tokens WHERE revoked=0 AND expires_at>?1",
            [hireferry::now()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, 10);
}

#[tokio::test]
async fn private_matching_needs_seeker_consent_and_payment_and_declining_revokes_access() {
    let f = Fixture::new();
    let browser = f.admin_browser(hireferry::now(), false);
    let (_, admin_token) = f.issue(&browser, "Match coordinator").await;
    let (seeker, seeker_token) = f.member();
    let (employer, _) = f.member();
    let (stranger, _) = f.member();
    let job = uuid::Uuid::new_v4().to_string();
    let order = uuid::Uuid::new_v4().to_string();
    {
        let db = f.state.conn().unwrap();
        db.execute(
            "UPDATE users SET role='employer',employer_paid=1 WHERE id=?1",
            [&employer],
        )
        .unwrap();
        db.execute(
            "INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,status,created_at,updated_at) VALUES(?1,?2,'Private Rust role','Job description',300000,'published',?3,?3)",
            params![job, employer, hireferry::now()],
        ).unwrap();
        db.execute(
            "INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,status,created_at) VALUES(?1,?2,'publish',?3,30000,'stripe','pending',?4)",
            params![order, employer, job, hireferry::now()],
        ).unwrap();
    }
    let pair = json!({"seeker_id":seeker,"job_id":job,"note":"Discuss the role together"});
    let missing_consent = request(
        &f.app,
        "POST",
        "/api/admin/matches",
        None,
        Some(&admin_token),
        Some(pair.clone()),
    )
    .await;
    assert_eq!(missing_consent.status, StatusCode::BAD_REQUEST);
    let mandate = json!({
        "target_industry":"Software", "target_roles":"Rust engineer", "salary_min_cents":300000,
        "salary_max_cents":null, "location":"Remote", "preferences":"Keep my identity private until introduction",
        "status":"active", "allow_external":false
    });
    let forged_consent = request(
        &f.app,
        "PATCH",
        &format!("/api/admin/mandates/{seeker}"),
        None,
        Some(&admin_token),
        Some(mandate.clone()),
    )
    .await;
    assert_eq!(forged_consent.status, StatusCode::FORBIDDEN);
    let consent = request(
        &f.app,
        "PUT",
        "/api/mandate",
        None,
        Some(&seeker_token),
        Some(mandate.clone()),
    )
    .await;
    assert_eq!(consent.status, StatusCode::OK, "{}", consent.text);
    let unpaid = request(
        &f.app,
        "POST",
        "/api/admin/matches",
        None,
        Some(&admin_token),
        Some(pair.clone()),
    )
    .await;
    assert_eq!(unpaid.status, StatusCode::BAD_REQUEST);
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE orders SET status='paid',paid_at=?2 WHERE id=?1",
            params![order, hireferry::now()],
        )
        .unwrap();
    let introduced = request(
        &f.app,
        "POST",
        "/api/admin/matches",
        None,
        Some(&admin_token),
        Some(pair),
    )
    .await;
    assert_eq!(introduced.status, StatusCode::OK, "{}", introduced.text);
    let matched = introduced.value["id"].as_str().unwrap();
    let conversation = format!("match_{matched}");
    assert_eq!(introduced.value["conversation_id"], conversation);
    assert_eq!(
        introduced.value["seeker_conversation_id"],
        format!("platform_{seeker}")
    );
    assert_eq!(
        introduced.value["employer_conversation_id"],
        format!("platform_{employer}")
    );
    assert!(
        !models::load_user(&f.state.conn().unwrap(), &seeker)
            .unwrap()
            .visible
    );
    {
        let db = f.state.conn().unwrap();
        assert!(access::matched_pair(&db, &employer, &seeker).unwrap());
        assert!(!access::matched_pair(&db, &stranger, &seeker).unwrap());
        assert!(access::can_read_job(&db, &seeker, &job).unwrap());
        assert!(!access::can_read_job(&db, &stranger, &job).unwrap());
    }
    let thread_path = format!("/api/chat/conversations/{conversation}/messages");
    assert_eq!(
        request(&f.app, "GET", &thread_path, None, Some(&seeker_token), None)
            .await
            .status,
        StatusCode::OK
    );
    let declined = request(
        &f.app,
        "PATCH",
        &format!("/api/admin/matches/{matched}"),
        None,
        Some(&admin_token),
        Some(json!({"status":"declined","note":"Introduction withdrawn"})),
    )
    .await;
    assert_eq!(declined.status, StatusCode::NO_CONTENT);
    {
        let db = f.state.conn().unwrap();
        assert!(!access::matched_pair(&db, &employer, &seeker).unwrap());
        assert!(!access::can_read_job(&db, &seeker, &job).unwrap());
    }
    assert_eq!(
        request(&f.app, "GET", &thread_path, None, Some(&seeker_token), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let mut paused = mandate.clone();
    paused["status"] = json!("paused");
    assert_eq!(
        request(
            &f.app,
            "PUT",
            "/api/mandate",
            None,
            Some(&seeker_token),
            Some(paused)
        )
        .await
        .status,
        StatusCode::OK
    );
    let restore = json!({"status":"introduced","note":"Try to resume"});
    let blocked = request(
        &f.app,
        "PATCH",
        &format!("/api/admin/matches/{matched}"),
        None,
        Some(&admin_token),
        Some(restore.clone()),
    )
    .await;
    assert_eq!(blocked.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        request(
            &f.app,
            "PUT",
            "/api/mandate",
            None,
            Some(&seeker_token),
            Some(mandate)
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &f.app,
            "PATCH",
            &format!("/api/admin/matches/{matched}"),
            None,
            Some(&admin_token),
            Some(restore)
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn admin_mcp_discovery_and_mutations_share_http_authorization() {
    let f = Fixture::new();
    let browser = f.admin_browser(hireferry::now(), false);
    let (token_id, admin_token) = f.issue(&browser, "MCP administrator").await;
    let (user, member_token) = f.member();
    let admin_tools = rpc(&f.app, &admin_token, "tools/list", json!({})).await;
    assert_eq!(admin_tools.status, StatusCode::OK, "{}", admin_tools.text);
    let admin_tools = admin_tools.value["result"]["tools"].as_array().unwrap();
    for required in [
        "admin_overview",
        "admin_get_user",
        "admin_delete_resume",
        "admin_create_match",
        "admin_list_orders",
    ] {
        assert!(
            admin_tools.iter().any(|tool| tool["name"] == required),
            "missing {required}"
        );
    }
    let member_tools = rpc(&f.app, &member_token, "tools/list", json!({})).await;
    let member_tools = member_tools.value["result"]["tools"].as_array().unwrap();
    assert!(
        !member_tools
            .iter()
            .any(|tool| tool["name"].as_str().unwrap().starts_with("admin_"))
    );
    let forbidden = rpc(
        &f.app,
        &member_token,
        "tools/call",
        json!({
            "name":"admin_get_user", "arguments":{"id":user}
        }),
    )
    .await;
    assert!(forbidden.value["error"].is_object() || forbidden.value["result"]["isError"] == true);
    assert!(!forbidden.text.contains("Private resume"));
    let profile = rpc(
        &f.app,
        &admin_token,
        "tools/call",
        json!({
            "name":"admin_get_user", "arguments":{"id":user}
        }),
    )
    .await;
    assert_eq!(
        profile.value["result"]["isError"], false,
        "{}",
        profile.text
    );
    assert_eq!(
        profile.value["result"]["structuredContent"]["resume_md"],
        "Private resume"
    );
    let removed = rpc(
        &f.app,
        &admin_token,
        "tools/call",
        json!({
            "name":"admin_delete_resume", "arguments":{"id":user}
        }),
    )
    .await;
    assert_eq!(
        removed.value["result"]["isError"], false,
        "{}",
        removed.text
    );
    let fetched = request(
        &f.app,
        "GET",
        &format!("/api/admin/users/{user}"),
        None,
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.value["resume_md"], "");
    let actor: String = f.state.conn().unwrap().query_row(
        "SELECT actor FROM audit WHERE action='resume_delete' AND target=?1 ORDER BY id DESC LIMIT 1",
        [&user], |r| r.get(0),
    ).unwrap();
    assert_eq!(actor, format!("admin_ai:{token_id}"));
    assert_eq!(
        request(
            &f.app,
            "DELETE",
            &format!("/api/admin/tokens/{token_id}"),
            Some(&browser),
            None,
            None
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    denied(&rpc(&f.app, &admin_token, "tools/list", json!({})).await);
}
