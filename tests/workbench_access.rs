//! Workbench discovery is bounded, administrative and read-only. Fixtures use
//! in-memory SQLite and synthetic credentials; no external services are called.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use hireferry::{
    AppState,
    access::{self, Principal},
    admin, api, auth, chat, commerce,
    config::Config,
    models::{self, Job},
    workbench,
};
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
struct Token {
    id: String,
    raw: String,
}
struct Reply {
    status: StatusCode,
    value: Value,
    text: String,
}

fn admin() -> Principal {
    Principal {
        user_id: None,
        admin: true,
        via_ai: false,
        actor: "admin".into(),
    }
}
fn uid(p: &Principal) -> &str {
    p.user_id.as_deref().unwrap()
}
impl Fixture {
    fn new() -> Self {
        Self::with_limits(hireferry::config::Limits::default())
    }
    fn with_limits(limits: hireferry::config::Limits) -> Self {
        let mut cfg = Config {
            development: true,
            database: ":memory:".into(),
            limits,
            ..Default::default()
        };
        cfg.webhooks.enabled = false;
        let state = AppState::new(cfg).unwrap();
        Self {
            app: hireferry::app(state.clone()),
            state,
        }
    }
    fn member(&self, role: &str) -> Principal {
        let id = uuid::Uuid::new_v4().to_string();
        self.state.conn().unwrap().execute(
            "INSERT INTO users(id,email,name,role,employer_paid,resume_md,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?7)",
            params![id,format!("workbench-{id}@example.invalid"),format!("Private {role} {id}"),role,role=="employer",
                "# PRIVATE_RESUME_SENTINEL\n<script>alert('unsafe')</script>\n[unsafe](javascript:alert(1))\n![tracker](https://example.invalid/tracker)",hireferry::now()],
        ).unwrap();
        Principal {
            user_id: Some(id.clone()),
            admin: false,
            via_ai: false,
            actor: id,
        }
    }
    fn mandate(&self, seeker: &Principal, status: &str) {
        chat::save_mandate(
            &self.state,
            seeker,
            None,
            chat::MandateInput {
                target_industry: "Technology".into(),
                target_roles: "Rust engineer".into(),
                salary_min_cents: Some(250_000),
                salary_max_cents: Some(500_000),
                salary_currency: None,
                location: "Remote".into(),
                preferences: "PRIVATE_PREFERENCES_SENTINEL".into(),
                status: status.into(),
                allow_external: false,
            },
        )
        .unwrap();
    }
    fn seeker(&self) -> Principal {
        let seeker = self.member("seeker");
        self.mandate(&seeker, "active");
        seeker
    }
    fn job(&self, owner: &Principal, headcount: i64) -> Job {
        let id = uuid::Uuid::new_v4().to_string();
        let db = self.state.conn().unwrap();
        db.execute(
            "INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,headcount,status,created_at,updated_at) VALUES(?1,?2,'Rust role','PRIVATE_JOB_SENTINEL',300000,?3,'published',?4,?4)",
            params![id,uid(owner),headcount,hireferry::now()],
        ).unwrap();
        db.execute(
            "INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,status,snapshot_json,created_at,paid_at) VALUES(?1,?2,'publish',?3,30000,'stripe','paid','{\"max_edits\":3,\"edit_bps\":100}',?4,?4)",
            params![format!("paid-{id}"),uid(owner),id,hireferry::now()],
        ).unwrap();
        models::load_job(&db, &id).unwrap()
    }
    fn token(&self, p: &Principal) -> Token {
        let token = Token {
            id: uuid::Uuid::new_v4().to_string(),
            raw: format!(
                "{}{}",
                if p.admin { "hf_admin_" } else { "hf_" },
                hex::encode(rand::random::<[u8; 32]>())
            ),
        };
        let db = self.state.conn().unwrap();
        if p.admin {
            db.execute(
                "INSERT INTO admin_api_tokens(id,name,token_hash,created_at,expires_at) VALUES(?1,'workbench fixture',?2,?3,?4)",
                params![token.id,auth::hash_token(&token.raw),hireferry::now(),hireferry::now()+3600],
            ).unwrap();
        } else {
            db.execute(
                "INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES(?1,?2,'workbench fixture',?3,?4,?5)",
                params![token.id,uid(p),auth::hash_token(&token.raw),hireferry::now(),hireferry::now()+3600],
            ).unwrap();
        }
        token
    }
    fn browser(&self, p: &Principal, bootstrap: bool) -> Browser {
        let browser = Browser {
            raw: format!("workbench_cookie_{}", uuid::Uuid::new_v4()),
            csrf: format!("workbench_csrf_{}", uuid::Uuid::new_v4()),
        };
        self.state.conn().unwrap().execute(
            "INSERT INTO sessions(token_hash,user_id,admin,bootstrap,csrf,expires_at,recent_auth_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![auth::hash_token(&browser.raw),p.user_id,p.admin && !bootstrap,bootstrap,browser.csrf,hireferry::now()+3600,hireferry::now()],
        ).unwrap();
        browser
    }
    fn introduce(&self, seeker: &Principal, job: &Job) -> String {
        admin::add_match(
            &self.state,
            &admin(),
            admin::MatchInput {
                seeker_id: uid(seeker).into(),
                job_id: job.id.clone(),
                note: "PRIVATE_INTRODUCTION_SENTINEL".into(),
            },
        )
        .unwrap()["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn counts(&self) -> (i64, i64, i64, i64) {
        self.state.conn().unwrap().query_row(
            "SELECT (SELECT count(*) FROM matches),(SELECT count(*) FROM mandates),(SELECT count(*) FROM outreach),(SELECT count(*) FROM audit)",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).unwrap()
    }
}

async fn request(
    f: &Fixture,
    method: &str,
    path: &str,
    browser: Option<&Browser>,
    token: Option<&Token>,
    webmcp: bool,
    body: Option<Value>,
) -> Reply {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(browser) = browser {
        req = req
            .header("cookie", format!("hf_session={}", browser.raw))
            .header("origin", &f.state.cfg.public_url)
            .header("x-csrf-token", &browser.csrf);
    }
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {}", token.raw));
    }
    if webmcp {
        req = req.header("x-hireferry-ai", "webmcp");
    }
    if path == "/mcp" {
        req = req.header("mcp-protocol-version", "2025-11-25");
    }
    let body = match body {
        Some(value) => {
            req = req.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = f
        .app
        .clone()
        .oneshot(req.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        value: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        text: String::from_utf8(bytes.to_vec()).unwrap(),
    }
}
async fn rpc(f: &Fixture, token: &Token, method: &str, params: Value) -> Reply {
    request(
        f,
        "POST",
        "/mcp",
        None,
        Some(token),
        false,
        Some(json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})),
    )
    .await
}
async fn call(f: &Fixture, token: &Token, name: &str, args: Value) -> Reply {
    rpc(
        f,
        token,
        "tools/call",
        json!({"name":name,"arguments":args}),
    )
    .await
}
fn success(reply: &Reply) -> &Value {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.value["result"]["isError"], false, "{}", reply.text);
    &reply.value["result"]["structuredContent"]
}
fn tool_error(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert!(
        reply.value["result"]["isError"] == true || reply.value["error"].is_object(),
        "{}",
        reply.text
    );
}
fn denied(reply: &Reply) {
    assert!(
        matches!(
            reply.status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ),
        "{}: {}",
        reply.status,
        reply.text
    );
    assert!(!reply.text.contains("PRIVATE_RESUME_SENTINEL"));
    assert!(!reply.text.contains("PRIVATE_JOB_SENTINEL"));
    assert!(!reply.text.contains("PRIVATE_INTRODUCTION_SENTINEL"));
}

#[tokio::test]
async fn workbench_http_requires_admin_for_every_list_and_targeted_detail() {
    let f = Fixture::new();
    let employer = f.member("employer");
    let seeker = f.seeker();
    let job = f.job(&employer, 1);
    let matched = f.introduce(&seeker, &job);
    let owner_browser = f.browser(&employer, false);
    let seeker_browser = f.browser(&seeker, false);
    let admin_browser = f.browser(&admin(), false);
    let bootstrap = f.browser(&admin(), true);
    let owner_token = f.token(&employer);
    let seeker_token = f.token(&seeker);
    let admin_token = f.token(&admin());
    let paths = [
        "/api/admin/workbench/jobs?limit=1".into(),
        "/api/admin/workbench/candidates?limit=1".into(),
        "/api/admin/workbench/matches?limit=1".into(),
        format!("/api/admin/workbench/jobs/{}", job.id),
        format!(
            "/api/admin/workbench/candidates/{}?job_id={}",
            uid(&seeker),
            job.id
        ),
        format!("/api/admin/workbench/matches/{matched}"),
    ];
    let before = f.counts();
    for path in &paths {
        for (browser, token, ai) in [
            (None, None, false),
            (Some(&owner_browser), None, false),
            (Some(&seeker_browser), None, false),
            (Some(&seeker_browser), None, true),
            (Some(&bootstrap), None, false),
            (None, Some(&owner_token), false),
            (None, Some(&seeker_token), false),
        ] {
            denied(&request(&f, "GET", path, browser, token, ai, None).await);
        }
        for (browser, token, ai) in [
            (Some(&admin_browser), None, false),
            (Some(&admin_browser), None, true),
            (None, Some(&admin_token), false),
        ] {
            let reply = request(&f, "GET", path, browser, token, ai, None).await;
            assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.text);
        }
    }
    assert_eq!(
        f.counts(),
        before,
        "Search/detail reads must not introduce or authorize anyone"
    );

    let detail = request(
        &f,
        "GET",
        &paths[4],
        Some(&admin_browser),
        None,
        false,
        None,
    )
    .await;
    assert_eq!(detail.value["user"]["id"], uid(&seeker));
    assert!(detail.value["user"].get("google_sub").is_none());
    assert_eq!(detail.value["mandate"]["allow_external"], false);
    assert!(detail.value["mandate"]["consented_at"].is_null());
    assert_eq!(detail.value["match"]["id"], matched);
    let html = detail.value["resume_html"].as_str().unwrap();
    assert!(html.contains("PRIVATE_RESUME_SENTINEL"));
    assert!(!html.contains("<script") && !html.contains("javascript:") && !html.contains("<img"));
    let single = request(
        &f,
        "GET",
        &paths[5],
        Some(&admin_browser),
        None,
        false,
        None,
    )
    .await;
    assert_eq!(
        single.value,
        admin::list_matches(&f.state, &admin()).unwrap()["items"][0]
    );
    assert_eq!(single.value["conversation_id"], format!("match_{matched}"));
    let missing = request(
        &f,
        "GET",
        "/api/admin/workbench/matches/missing",
        Some(&admin_browser),
        None,
        false,
        None,
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(missing.value["error"].is_string());
    let unknown = request(
        &f,
        "GET",
        &format!(
            "/api/admin/workbench/candidates/{}?grant=true",
            uid(&seeker)
        ),
        Some(&admin_browser),
        None,
        false,
        None,
    )
    .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    let write = request(
        &f,
        "POST",
        "/api/admin/workbench/candidates",
        Some(&admin_browser),
        None,
        false,
        Some(json!({})),
    )
    .await;
    assert_eq!(write.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(f.counts(), before);

    // Administrative history stays readable even when it is no longer a
    // selectable candidate, without making the archived conversation writable.
    f.mandate(&seeker, "paused");
    admin::patch_user(
        &f.state,
        &admin(),
        uid(&seeker),
        serde_json::from_value(json!({"banned":true})).unwrap(),
    )
    .unwrap();
    let history = request(
        &f,
        "GET",
        &paths[4],
        Some(&admin_browser),
        None,
        false,
        None,
    )
    .await;
    assert_eq!(history.status, StatusCode::OK);
    assert_eq!(history.value["user"]["banned"], true);
    assert_eq!(history.value["mandate"]["status"], "paused");
    assert_eq!(history.value["match"]["id"], matched);
    let candidates = request(
        &f,
        "GET",
        "/api/admin/workbench/candidates",
        Some(&admin_browser),
        None,
        false,
        None,
    )
    .await;
    assert_eq!(candidates.value["items"], json!([]));
    assert!(admin::get_match(&f.state, &admin(), &matched).is_ok());
    assert_eq!(
        chat::send_message(
            &f.state,
            &admin(),
            &format!("match_{matched}"),
            chat::MessageInput {
                body: "Must not send".into(),
                client_id: "archived-admin".into()
            }
        )
        .unwrap_err()
        .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn workbench_business_reads_recheck_admin_token_and_webmcp_revocation() {
    let f = Fixture::new();
    let employer = f.member("employer");
    let seeker = f.seeker();
    let job = f.job(&employer, 1);
    let matched = f.introduce(&seeker, &job);
    let token = f.token(&admin());
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        format!("Bearer {}", token.raw).parse().unwrap(),
    );
    let cached = access::principal(&f.state, &headers, false).unwrap();
    assert!(workbench::search_candidates(&f.state, &cached, &Default::default()).is_ok());
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE admin_api_tokens SET revoked=1 WHERE id=?1",
            [&token.id],
        )
        .unwrap();
    for result in [
        workbench::search_jobs(&f.state, &cached, &Default::default()),
        workbench::search_candidates(&f.state, &cached, &Default::default()),
        workbench::search_matches(&f.state, &cached, &Default::default()),
        workbench::get_job(&f.state, &cached, &job.id),
        admin::candidate_detail(&f.state, &cached, uid(&seeker), Some(&job.id)),
        admin::get_match(&f.state, &cached, &matched),
    ] {
        assert_eq!(result.unwrap_err().status, StatusCode::UNAUTHORIZED);
    }
    denied(
        &request(
            &f,
            "GET",
            "/api/admin/workbench/jobs",
            None,
            Some(&token),
            false,
            None,
        )
        .await,
    );

    let browser = f.browser(&admin(), false);
    let mut headers = HeaderMap::new();
    headers.insert(
        "cookie",
        format!("hf_session={}", browser.raw).parse().unwrap(),
    );
    headers.insert("x-hireferry-ai", "webmcp".parse().unwrap());
    let cached = access::principal(&f.state, &headers, false).unwrap();
    assert!(cached.actor.starts_with("admin_webmcp:"));
    assert!(admin::candidate_detail(&f.state, &cached, uid(&seeker), None).is_ok());
    f.state
        .conn()
        .unwrap()
        .execute(
            "DELETE FROM sessions WHERE token_hash=?1",
            [auth::hash_token(&browser.raw)],
        )
        .unwrap();
    for result in [
        workbench::search_jobs(&f.state, &cached, &Default::default()),
        workbench::search_candidates(&f.state, &cached, &Default::default()),
        workbench::search_matches(&f.state, &cached, &Default::default()),
        workbench::get_job(&f.state, &cached, &job.id),
        admin::candidate_detail(&f.state, &cached, uid(&seeker), None),
        admin::get_match(&f.state, &cached, &matched),
    ] {
        assert_eq!(result.unwrap_err().status, StatusCode::UNAUTHORIZED);
    }
    denied(
        &request(
            &f,
            "GET",
            "/api/admin/workbench/candidates",
            Some(&browser),
            None,
            true,
            None,
        )
        .await,
    );
}

#[tokio::test]
async fn mcp_catalog_and_default_search_use_configured_business_limits() {
    let f = Fixture::with_limits(hireferry::config::Limits {
        workbench_page_size: 2,
        max_job_headcount: 9,
        ..Default::default()
    });
    let admin_token = f.token(&admin());
    let member = f.seeker();
    let member_token = f.token(&member);
    let admin_catalog = rpc(&f, &admin_token, "tools/list", json!({})).await;
    let admin_tools = admin_catalog.value["result"]["tools"].as_array().unwrap();
    for name in [
        "admin_search_jobs",
        "admin_search_candidates",
        "admin_search_matches",
    ] {
        let tool = admin_tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap();
        assert_eq!(tool["inputSchema"]["properties"]["limit"]["default"], 2);
        let response = call(&f, &admin_token, name, json!({})).await;
        assert_eq!(success(&response)["limit"], 2);
    }
    let member_catalog = rpc(&f, &member_token, "tools/list", json!({})).await;
    let member_tools = member_catalog.value["result"]["tools"].as_array().unwrap();
    for name in ["create_job", "set_job_headcount"] {
        let tool = member_tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap();
        assert_eq!(tool["inputSchema"]["properties"]["headcount"]["maximum"], 9);
    }
}

#[tokio::test]
async fn admin_mcp_searches_are_bounded_strict_and_separate_from_member_and_legacy_tools() {
    let f = Fixture::new();
    let employer = f.member("employer");
    let seekers = [f.seeker(), f.seeker(), f.seeker()];
    let jobs = [f.job(&employer, 1), f.job(&employer, 2)];
    f.introduce(&seekers[0], &jobs[0]);
    f.introduce(&seekers[1], &jobs[1]);
    let admin_token = f.token(&admin());
    let member_token = f.token(&seekers[0]);
    let catalog = rpc(&f, &admin_token, "tools/list", json!({})).await;
    let tools = catalog.value["result"]["tools"].as_array().unwrap();
    let member_catalog = rpc(&f, &member_token, "tools/list", json!({})).await;
    for catalog in [
        tools,
        member_catalog.value["result"]["tools"].as_array().unwrap(),
    ] {
        let names: std::collections::BTreeSet<_> = catalog
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), catalog.len(), "Tool names must be unique");
        assert!(names.contains("get_site_config"));
        for name in [
            "get_chat_capabilities",
            "get_conversation",
            "get_chat_events",
            "begin_message",
            "append_message",
            "finish_message",
            "cancel_message",
        ] {
            let matching: Vec<_> = catalog.iter().filter(|tool| tool["name"] == name).collect();
            assert_eq!(
                matching.len(),
                1,
                "Expected exactly one shared chat tool: {name}"
            );
            assert_eq!(matching[0]["inputSchema"]["additionalProperties"], false);
            assert_eq!(matching[0]["outputSchema"]["type"], "object");
        }
    }
    let before = f.counts();
    for name in [
        "admin_search_jobs",
        "admin_search_candidates",
        "admin_search_matches",
    ] {
        let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert_eq!(tool["inputSchema"]["properties"]["limit"]["maximum"], 100);
        assert_eq!(tool["inputSchema"]["properties"]["limit"]["default"], 25);
        assert!(
            !member_catalog.value["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == name)
        );
        tool_error(&call(&f, &member_token, name, json!({})).await);
        let first = call(&f, &admin_token, name, json!({"limit":1})).await;
        let page = success(&first);
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(page["limit"], 1);
        assert!(page["total"].as_i64().unwrap() >= 2);
        let cursor = page["next_cursor"].as_str().unwrap();
        let next = call(&f, &admin_token, name, json!({"limit":1,"cursor":cursor})).await;
        assert_ne!(page["items"][0]["id"], success(&next)["items"][0]["id"]);
        tool_error(&call(&f, &admin_token, name, json!({"limit":101})).await);
        tool_error(&call(&f, &admin_token, name, json!({"grant_access":true})).await);
    }
    tool_error(
        &call(
            &f,
            &admin_token,
            "admin_search_candidates",
            json!({"salary_min_cents":500000,"salary_max_cents":100000}),
        )
        .await,
    );
    let candidates = call(
        &f,
        &admin_token,
        "admin_search_candidates",
        json!({"job_id":jobs[0].id,"salary_min_cents":300000,"salary_max_cents":300000}),
    )
    .await;
    let candidates = success(&candidates);
    assert_eq!(candidates["total"], 2);
    for item in candidates["items"].as_array().unwrap() {
        assert_ne!(item["id"], uid(&seekers[0]));
        assert!(item.get("email").is_none() && item.get("resume_md").is_none());
    }
    for name in ["admin_list_users", "admin_list_jobs"] {
        let reply = call(&f, &admin_token, name, json!({})).await;
        assert!(success(&reply)["items"].is_array());
        assert!(success(&reply)["total"].is_i64());
        assert!(success(&reply).get("next_cursor").is_none());
    }
    let legacy = call(&f, &admin_token, "admin_list_matches", json!({})).await;
    assert!(success(&legacy)["items"].is_array());
    assert!(success(&legacy)["total"].is_i64());
    assert_eq!(success(&legacy)["page"], 1);
    assert_eq!(
        success(&legacy)["page_size"],
        f.state.cfg.limits.list_page_size
    );
    assert!(success(&legacy).get("next_cursor").is_none());
    assert_eq!(f.counts(), before);
}

#[tokio::test]
async fn discovery_does_not_authorize_or_match_and_zero_headcount_keeps_existing_chat_rules() {
    let f = Fixture::new();
    let employer = f.member("employer");
    let seeker = f.seeker();
    let other = f.seeker();
    let no_mandate = f.member("seeker");
    let paused = f.seeker();
    f.mandate(&paused, "paused");
    let job = f.job(&employer, 1);
    let zero = f.job(&employer, 0);
    let admin_browser = f.browser(&admin(), false);
    let private = chat::platform_conversation(&f.state, &seeker, None).unwrap();
    chat::send_message(
        &f.state,
        &seeker,
        private["id"].as_str().unwrap(),
        chat::MessageInput {
            body: "PRIVATE_PLATFORM_ONLY".into(),
            client_id: "private-before-search".into(),
        },
    )
    .unwrap();
    let before = f.counts();
    let results = request(
        &f,
        "GET",
        &format!("/api/admin/workbench/candidates?job_id={}", job.id),
        Some(&admin_browser),
        None,
        false,
        None,
    )
    .await;
    assert_eq!(results.status, StatusCode::OK, "{}", results.text);
    let items = results.value["items"].as_array().unwrap();
    assert!(items.iter().any(|row| row["id"] == uid(&seeker)));
    assert!(
        !items
            .iter()
            .any(|row| row["id"] == uid(&no_mandate) || row["id"] == uid(&paused))
    );
    let detail = admin::candidate_detail(&f.state, &admin(), uid(&seeker), Some(&job.id)).unwrap();
    assert!(detail["match"].is_null());
    assert_eq!(detail["mandate"]["allow_external"], false);
    assert_eq!(f.counts(), before);
    assert_eq!(
        api::person_detail(&f.state, uid(&seeker), Some(uid(&employer)))
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    assert!(!access::can_read_job(&f.state.conn().unwrap(), uid(&seeker), &job.id).unwrap());
    for (candidate, target) in [(&no_mandate, &job), (&paused, &job), (&seeker, &zero)] {
        let response = request(
            &f,
            "POST",
            "/api/admin/matches",
            Some(&admin_browser),
            None,
            false,
            Some(json!({"seeker_id":uid(candidate),"job_id":target.id,"note":"Explicit action"})),
        )
        .await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{}",
            response.text
        );
    }
    assert_eq!(f.counts(), before);
    let created = request(
        &f,
        "POST",
        "/api/admin/matches",
        Some(&admin_browser),
        None,
        false,
        Some(json!({"seeker_id":uid(&seeker),"job_id":job.id,"note":"Explicit introduction"})),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    let matched = created.value["id"].as_str().unwrap();
    let room = format!("match_{matched}");
    let messages = chat::messages(&f.state, &employer, &room, 0, 100).unwrap();
    assert!(!messages.to_string().contains("PRIVATE_PLATFORM_ONLY"));
    assert_eq!(
        chat::messages(&f.state, &employer, private["id"].as_str().unwrap(), 0, 100)
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    commerce::set_headcount(&f.state, uid(&employer), &job.id, 0).unwrap();
    assert!(chat::messages(&f.state, &employer, &room, 0, 100).is_ok());
    let history = request(
        &f,
        "GET",
        &format!("/api/admin/workbench/matches/{matched}"),
        Some(&admin_browser),
        None,
        false,
        None,
    )
    .await;
    assert_eq!(history.status, StatusCode::OK);
    let paused_job = request(
        &f,
        "GET",
        &format!("/api/admin/workbench/jobs/{}", job.id),
        Some(&admin_browser),
        None,
        false,
        None,
    )
    .await;
    assert_eq!(paused_job.status, StatusCode::NOT_FOUND);
    let rejected = request(
        &f,
        "POST",
        "/api/admin/matches",
        Some(&admin_browser),
        None,
        false,
        Some(json!({"seeker_id":uid(&other),"job_id":job.id})),
    )
    .await;
    assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
    admin::patch_match(
        &f.state,
        &admin(),
        matched,
        admin::MatchPatch {
            status: "declined".into(),
            note: "Closed introduction".into(),
        },
    )
    .unwrap();
    assert_eq!(
        chat::messages(&f.state, &seeker, &room, 0, 100)
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let reopen = request(
        &f,
        "PATCH",
        &format!("/api/admin/matches/{matched}"),
        Some(&admin_browser),
        None,
        false,
        Some(json!({"status":"introduced"})),
    )
    .await;
    assert_eq!(reopen.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        admin::get_match(&f.state, &admin(), matched).unwrap()["status"],
        "declined"
    );
    commerce::set_headcount(&f.state, uid(&employer), &job.id, 1).unwrap();
    let candidates = request(
        &f,
        "GET",
        &format!("/api/admin/workbench/candidates?job_id={}", job.id),
        Some(&admin_browser),
        None,
        false,
        None,
    )
    .await;
    let candidate = candidates.value["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == uid(&seeker))
        .unwrap();
    assert_eq!(candidate["history_match"]["id"], matched);
    assert_eq!(candidate["history_match"]["status"], "declined");
    assert_eq!(
        admin::get_match(&f.state, &admin(), matched).unwrap()["status"],
        "declined",
        "Searching a declined candidate must not reopen the match"
    );
}

#[tokio::test]
async fn bulk_introductions_require_admin_and_retries_keep_existing_notes_and_progress() {
    let f = Fixture::new();
    let employer = f.member("employer");
    let first = f.seeker();
    let second = f.seeker();
    let job = f.job(&employer, 1);
    let owner_browser = f.browser(&employer, false);
    let admin_browser = f.browser(&admin(), false);
    let body =
        json!({"job_id":job.id,"seeker_ids":[uid(&first),uid(&second)],"note":"Reviewed together"});
    for browser in [None, Some(&owner_browser)] {
        let response = request(
            &f,
            "POST",
            "/api/admin/workbench/introductions",
            browser,
            None,
            false,
            Some(body.clone()),
        )
        .await;
        denied(&response);
    }
    let created = request(
        &f,
        "POST",
        "/api/admin/workbench/introductions",
        Some(&admin_browser),
        None,
        false,
        Some(body.clone()),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    assert_eq!(created.value["created"], 2);
    assert_eq!(created.value["reused"], 0);
    assert_eq!(created.value["matches"].as_array().unwrap().len(), 2);
    let id = created.value["matches"][0]["id"].as_str().unwrap();
    admin::patch_match(
        &f.state,
        &admin(),
        id,
        admin::MatchPatch {
            status: "interview".into(),
            note: "Interview already arranged".into(),
        },
    )
    .unwrap();
    let before_retry = f.counts();
    let retry = request(
        &f,
        "POST",
        "/api/admin/workbench/introductions",
        Some(&admin_browser),
        None,
        false,
        Some(body),
    )
    .await;
    assert_eq!(retry.status, StatusCode::OK, "{}", retry.text);
    assert_eq!(retry.value["created"], 0);
    assert_eq!(retry.value["reused"], 2);
    assert_eq!(retry.value["matches"][0]["id"], id);
    assert_eq!(retry.value["matches"][0]["status"], "interview");
    assert_eq!(retry.value["matches"][0]["created"], false);
    assert_eq!(
        f.counts(),
        before_retry,
        "Retries must not create audit or match duplicates"
    );
    assert_eq!(
        admin::get_match(&f.state, &admin(), id).unwrap()["note"],
        "Interview already arranged"
    );
    assert!(
        chat::get_conversation(
            &f.state,
            &first,
            retry.value["matches"][0]["conversation_id"]
                .as_str()
                .unwrap()
        )
        .is_ok()
    );
}

#[test]
fn bulk_introductions_roll_back_if_any_candidate_is_ineligible_or_previously_declined() {
    let f = Fixture::new();
    let employer = f.member("employer");
    let first = f.seeker();
    let second = f.seeker();
    let job = f.job(&employer, 3);
    f.mandate(&second, "paused");
    let input = || admin::BulkMatchInput {
        job_id: job.id.clone(),
        seeker_ids: vec![uid(&first).into(), uid(&second).into()],
        note: "Atomic introduction".into(),
    };
    let before = f.counts();
    assert_eq!(
        admin::add_matches(&f.state, &admin(), input())
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.counts(),
        before,
        "The first insert and its audit must roll back"
    );
    f.mandate(&second, "active");
    let previous = f.introduce(&second, &job);
    admin::patch_match(
        &f.state,
        &admin(),
        &previous,
        admin::MatchPatch {
            status: "declined".into(),
            note: "Do not silently reopen".into(),
        },
    )
    .unwrap();
    let before = f.counts();
    assert_eq!(
        admin::add_matches(&f.state, &admin(), input())
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(f.counts(), before);
    assert_eq!(
        admin::get_match(&f.state, &admin(), &previous).unwrap()["status"],
        "declined"
    );
    commerce::set_headcount(&f.state, uid(&employer), &job.id, 0).unwrap();
    let before = f.counts();
    assert!(admin::add_matches(&f.state, &admin(), input()).is_err());
    assert_eq!(f.counts(), before);
}

#[test]
fn bulk_introductions_bound_selection_and_serialize_concurrent_retries() {
    let f = Fixture::new();
    let employer = f.member("employer");
    let first = f.seeker();
    let second = f.seeker();
    let job = f.job(&employer, 2);
    for ids in [
        vec![],
        vec![uid(&first).to_owned(); 101],
        vec![uid(&first).to_owned(); 2],
        vec![String::new()],
    ] {
        let before = f.counts();
        let error = admin::add_matches(
            &f.state,
            &admin(),
            admin::BulkMatchInput {
                job_id: job.id.clone(),
                seeker_ids: ids,
                note: String::new(),
            },
        )
        .unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(f.counts(), before);
    }
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let s = f.state.clone();
            let barrier = barrier.clone();
            let input = admin::BulkMatchInput {
                job_id: job.id.clone(),
                seeker_ids: vec![uid(&first).into(), uid(&second).into()],
                note: "Concurrent request".into(),
            };
            std::thread::spawn(move || {
                barrier.wait();
                admin::add_matches(&s, &admin(), input).unwrap()
            })
        })
        .collect();
    let responses: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(
        responses
            .iter()
            .map(|r| r["created"].as_u64().unwrap())
            .sum::<u64>(),
        2
    );
    assert_eq!(
        responses
            .iter()
            .map(|r| r["reused"].as_u64().unwrap())
            .sum::<u64>(),
        2
    );
    assert_eq!(
        responses[0]["matches"][0]["id"],
        responses[1]["matches"][0]["id"]
    );
    assert_eq!(
        responses[0]["matches"][1]["id"],
        responses[1]["matches"][1]["id"]
    );
    assert_eq!(f.counts().0, 2);
}
