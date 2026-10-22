use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{AppState, auth, config::Config, db, feedback, maintenance, now};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    app: Router,
    member: String,
    admin_token: String,
}

impl Fixture {
    fn new(cfg: Config) -> Self {
        Self::with_database(cfg, ":memory:")
    }

    fn with_database(mut cfg: Config, path: &str) -> Self {
        cfg.development = true;
        cfg.database = path.into();
        let state = AppState::new(cfg).unwrap();
        let member = uuid::Uuid::new_v4().to_string();
        let admin_token = format!("hf_admin_{}", hex::encode([21u8; 32]));
        {
            let db = state.conn().unwrap();
            db.execute("INSERT INTO users(id,email,name,role,created_at,updated_at) VALUES(?1,'member@example.invalid','Test member','seeker',?2,?2)", params![member,now()]).unwrap();
            db.execute("INSERT INTO admin_api_tokens(id,name,token_hash,created_at,expires_at) VALUES('feedback-admin','Feedback admin',?1,?2,?3)", params![auth::hash_token(&admin_token),now(),now()+3600]).unwrap();
        }
        let app = hireferry::app(state.clone());
        Self {
            state,
            app,
            member,
            admin_token,
        }
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        cookie: Option<(&str, &str)>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        } else {
            req = req.header("origin", "http://localhost:3000");
        }
        if let Some((cookie, csrf)) = cookie {
            req = req
                .header("cookie", format!("hf_session={cookie}"))
                .header("x-csrf-token", csrf)
                .header("origin", "http://localhost:3000");
        }
        if path == "/mcp" {
            req = req.header("mcp-protocol-version", "2025-11-25");
        }
        let body = if let Some(value) = body {
            req = req.header("content-type", "application/json");
            Body::from(value.to_string())
        } else {
            Body::empty()
        };
        let response = self
            .app
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn cli_identity(&self) -> String {
        let raw = format!("hf_cli_{}", hex::encode([22u8; 32]));
        let db = self.state.conn().unwrap();
        db.execute("INSERT INTO cli_authorizations(id,user_id,admin,client_id,scope,created_at,expires_at,current_access_hash) VALUES('feedback-family',?1,0,'hireferry-cli','identity',?2,?3,?4)",params![self.member,now(),now()+3600,auth::hash_token(&raw)]).unwrap();
        db.execute("INSERT INTO cli_tokens(token_hash,family_id,admin,created_at,expires_at) VALUES(?1,'feedback-family',0,?2,?3)",params![auth::hash_token(&raw),now(),now()+600]).unwrap();
        raw
    }

    fn member_browser(&self) -> (String, String) {
        let cookie = "member_feedback_cookie".to_string();
        let csrf = "member_feedback_csrf".to_string();
        self.state.conn().unwrap().execute("INSERT INTO sessions(token_hash,user_id,csrf,expires_at,recent_auth_at) VALUES(?1,?2,?3,?4,?5)",params![auth::hash_token(&cookie),self.member,csrf,now()+3600,now()]).unwrap();
        (cookie, csrf)
    }
}

#[tokio::test]
async fn guest_and_identity_cli_can_submit_but_only_administrators_can_read() {
    let f = Fixture::new(Config::default());
    let (status, first) = f.request("POST","/api/feedback",None,None,
        Some(json!({"category":"bug","message":"The login button is broken","page":"/oauth/connections"}))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(first["id"].as_i64().unwrap() > 0);
    let identity = f.cli_identity();
    let (status, second) = f
        .request(
            "POST",
            "/api/feedback",
            Some(&identity),
            None,
            Some(json!({"category":"idea","message":"Simplify the CLI flow"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = f
        .request("GET", "/api/admin/feedback", None, None, None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = f
        .request("GET", "/api/admin/feedback", Some(&identity), None, None)
        .await;
    assert_ne!(status, StatusCode::OK);
    let (status, admin) = f
        .request(
            "GET",
            "/api/admin/feedback?limit=1",
            Some(&f.admin_token),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(admin["items"].as_array().unwrap().len(), 1);
    assert_eq!(admin["items"][0]["id"], second["id"]);
    assert_eq!(admin["items"][0]["source"], "cli");
    assert_eq!(admin["next_before_id"], second["id"]);
    let before = second["id"].as_i64().unwrap();
    let (status, page) = f
        .request(
            "GET",
            &format!("/api/admin/feedback?before_id={before}"),
            Some(&f.admin_token),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["items"][0]["id"], first["id"]);

    let rpc = |name: &str, args: Value| json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}});
    let (_, list) = f
        .request(
            "POST",
            "/mcp",
            Some(&f.admin_token),
            None,
            Some(rpc("admin_list_feedback", json!({"limit":10}))),
        )
        .await;
    assert_eq!(list["result"]["isError"], false, "{list}");
    assert_eq!(
        list["result"]["structuredContent"]["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let (denied_status, denied) = f
        .request(
            "POST",
            "/mcp",
            Some(&identity),
            None,
            Some(rpc("admin_list_feedback", json!({}))),
        )
        .await;
    assert_eq!(denied_status, StatusCode::FORBIDDEN);
    assert!(denied["error"].is_string());
    let (_, resolved) = f
        .request(
            "POST",
            "/mcp",
            Some(&f.admin_token),
            None,
            Some(rpc(
                "admin_update_feedback",
                json!({"id":before,"status":"resolved"}),
            )),
        )
        .await;
    assert_eq!(
        resolved["result"]["structuredContent"]["status"],
        "resolved"
    );
}

#[tokio::test]
async fn browser_csrf_validation_plain_text_bounds_and_rate_limit() {
    let mut cfg = Config::default();
    cfg.feedback.max_guest_per_hour = 2;
    cfg.feedback.max_per_account_per_hour = 1;
    let f = Fixture::new(cfg);
    let (cookie, csrf) = f.member_browser();
    let body = json!({"category":"other","message":"Feature request"});
    assert_eq!(
        f.request(
            "POST",
            "/api/feedback",
            None,
            Some((&cookie, "wrong")),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.request(
            "POST",
            "/api/feedback",
            None,
            Some((&cookie, &csrf)),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        f.request(
            "POST",
            "/api/feedback",
            None,
            Some((&cookie, &csrf)),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.request(
            "POST",
            "/api/feedback",
            None,
            Some((&cookie, &csrf)),
            Some(json!({"category":"other","message":"Another feature"}))
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    for i in 0..2 {
        assert_eq!(
            f.request(
                "POST",
                "/api/feedback",
                None,
                None,
                Some(json!({"category":"other","message":format!("Guest report {i}")}))
            )
            .await
            .0,
            StatusCode::CREATED
        );
    }
    assert_eq!(
        f.request("POST", "/api/feedback", None, None, Some(body.clone()))
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    for invalid in [
        json!({"category":"unsafe","message":"hello"}),
        json!({"category":"bug","message":"x"}),
        json!({"category":"bug","message":"valid","page":"//evil.example"}),
        json!({"category":"bug","message":"valid","page":"/login?token=secret"}),
        json!({"category":"bug","message":"valid","extra":"unknown"}),
    ] {
        let expected = if invalid.get("extra").is_some() {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            StatusCode::BAD_REQUEST
        };
        assert_eq!(
            f.request("POST", "/api/feedback", None, None, Some(invalid))
                .await
                .0,
            expected
        );
    }
}

#[test]
fn cleanup_bounds_feedback_and_ephemeral_state_without_touching_business_records() {
    let mut cfg = Config::default();
    cfg.feedback.max_records = 100;
    let f = Fixture::new(cfg);
    let at = now();
    {
        let db = f.state.conn().unwrap();
        for i in 0..120 {
            db.execute("INSERT INTO feedback(actor_key,actor_kind,source,category,message,created_at,updated_at) VALUES('guest','guest','web','bug','test',?1,?1)",[at-i]).unwrap();
        }
        db.execute("INSERT INTO audit(actor,action,target,created_at) VALUES('admin','review','unchanged',?1)",[at-1_000_000]).unwrap();
        db.execute("INSERT INTO auth_challenges(id,kind,subject,expires_at) VALUES('expired-feedback-test','email','example',?1)",[at-90_000]).unwrap();
        db.execute("INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES('backing-feedback-token',?1,'CLI backing','old-backing-hash',?2,?3)",params![f.member,at-70*86_400,at-35*86_400]).unwrap();
        db.execute("INSERT INTO cli_authorizations(id,user_id,admin,client_id,scope,created_at,expires_at,current_access_hash) VALUES('long-lived-feedback-family',?1,0,'hireferry-cli','identity mcp',?2,?3,'old-cli-hash')",params![f.member,at-70*86_400,at+20*86_400]).unwrap();
        db.execute("INSERT INTO cli_tokens(token_hash,token_id,family_id,admin,created_at,expires_at) VALUES('old-cli-hash','backing-feedback-token','long-lived-feedback-family',0,?1,?2)",params![at-70*86_400,at-35*86_400]).unwrap();
        maintenance::prune(&db, &f.state.cfg, at).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM feedback", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            120,
            "lowering capacity must not silently evict unexpired feedback"
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM auth_challenges WHERE id='expired-feedback-test'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM audit WHERE target='unchanged'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM api_tokens WHERE id='backing-feedback-token'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1,
            "housekeeping must not invalidate live CLI refresh families"
        );
        db.execute(
            "UPDATE cli_authorizations SET expires_at=?1 WHERE id='long-lived-feedback-family'",
            [at - 1],
        )
        .unwrap();
        maintenance::prune(&db, &f.state.cfg, at).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM api_tokens WHERE id='backing-feedback-token'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute(
            "UPDATE feedback SET created_at=?1,updated_at=?1",
            [at - 181 * 86400],
        )
        .unwrap();
        feedback::cleanup(&db, &f.state.cfg.feedback, at).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM feedback", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute_batch("DROP TABLE feedback; PRAGMA user_version=9;")
            .unwrap();
        db::migrate(&db).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            12
        );
    }
}

fn insert_report(state: &AppState, status: &str, created: i64, updated: i64) -> i64 {
    let db = state.conn().unwrap();
    db.execute("INSERT INTO feedback(actor_key,actor_kind,source,category,message,status,created_at,updated_at) VALUES('guest','guest','web','bug','existing report',?1,?2,?3)",params![status,created,updated]).unwrap();
    db.last_insert_rowid()
}

#[tokio::test]
async fn capacity_rejects_new_reports_without_evicting_open_or_resolved_records() {
    let mut cfg = Config::default();
    cfg.feedback.max_records = 100;
    let f = Fixture::new(cfg);
    for i in 0..100 {
        insert_report(
            &f.state,
            if i == 0 { "resolved" } else { "open" },
            now() - 7200,
            now(),
        );
    }
    let (status, body) = f
        .request(
            "POST",
            "/api/feedback",
            None,
            None,
            Some(json!({"category":"bug","message":"This report must not evict another"})),
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body["error"].as_str().unwrap().contains("full"));
    let db = f.state.conn().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM feedback", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        100
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM feedback WHERE id=1 AND status='resolved'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM feedback_limits", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn concurrent_submissions_share_atomic_capacity_and_duplicate_checks() {
    let mut cfg = Config::default();
    cfg.feedback.max_records = 100;
    let f = Fixture::new(cfg);
    for _ in 0..99 {
        insert_report(&f.state, "open", now() - 7200, now());
    }
    let report = |text: &str| Some(json!({"category":"bug","message":text}));
    let (first, second) = tokio::join!(
        f.request(
            "POST",
            "/api/feedback",
            None,
            None,
            report("The last available place")
        ),
        f.request(
            "POST",
            "/api/feedback",
            None,
            None,
            report("The last available place")
        ),
    );
    let statuses = [first.0, second.0];
    assert_eq!(
        statuses
            .iter()
            .filter(|&&s| s == StatusCode::CREATED)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|&&s| s == StatusCode::CONFLICT)
            .count(),
        1
    );
    assert_eq!(
        f.request(
            "POST",
            "/api/feedback",
            None,
            None,
            report("A distinct report")
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        f.state
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM feedback", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        100
    );
}

#[test]
fn retention_uses_status_deadlines_and_deletes_only_one_bounded_batch() {
    let f = Fixture::new(Config::default());
    let at = now();
    let open_expired = insert_report(&f.state, "open", at - 181 * 86400, at - 181 * 86400);
    let open_live = insert_report(&f.state, "open", at - 179 * 86400, at - 100 * 86400);
    let resolved_live = insert_report(&f.state, "resolved", at - 365 * 86400, at - 29 * 86400);
    let resolved_expired = insert_report(&f.state, "resolved", at - 40 * 86400, at - 31 * 86400);
    let db = f.state.conn().unwrap();
    assert_eq!(
        feedback::cleanup_batch(&db, &f.state.cfg.feedback, at, 1).unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM feedback", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    feedback::cleanup_batch(&db, &f.state.cfg.feedback, at, 1).unwrap();
    for id in [open_expired, resolved_expired] {
        assert_eq!(
            db.query_row("SELECT count(*) FROM feedback WHERE id=?1", [id], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    for id in [open_live, resolved_live] {
        assert_eq!(
            db.query_row("SELECT count(*) FROM feedback WHERE id=?1", [id], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn counters_and_dedup_survive_restart_and_report_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("feedback.sqlite3");
    let mut cfg = Config::default();
    cfg.feedback.max_guest_per_hour = 1;
    let f = Fixture::with_database(cfg, path.to_str().unwrap());
    let payload = json!({"category":"bug","message":"The same report after restart"});
    assert_eq!(
        f.request("POST", "/api/feedback", None, None, Some(payload.clone()))
            .await
            .0,
        StatusCode::CREATED
    );
    {
        let db = f.state.conn().unwrap();
        db.execute(
            "UPDATE feedback SET created_at=?1,updated_at=?1",
            [now() - 181 * 86400],
        )
        .unwrap();
        feedback::cleanup(&db, &f.state.cfg.feedback, now()).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM feedback", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let Fixture {
        state,
        app,
        member,
        admin_token,
    } = f;
    let cfg = state.cfg.as_ref().clone();
    drop(app);
    drop(state);
    let state = AppState::new(cfg).unwrap();
    let f = Fixture {
        app: hireferry::app(state.clone()),
        state,
        member,
        admin_token,
    };
    let (status, duplicate) = f
        .request("POST", "/api/feedback", None, None, Some(payload))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        duplicate.get("id").is_none(),
        "guest duplicates must not reveal another guest's receipt"
    );
    assert_eq!(
        f.request(
            "POST",
            "/api/feedback",
            None,
            None,
            Some(json!({"category":"idea","message":"A distinct request after restart"}))
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    {
        let db = f.state.conn().unwrap();
        db.execute("UPDATE feedback_limits SET window_start=?1", [now() - 7200])
            .unwrap();
        db.execute("UPDATE feedback_dedup SET expires_at=?1", [now() - 1])
            .unwrap();
    }
    assert_eq!(
        f.request(
            "POST",
            "/api/feedback",
            None,
            None,
            Some(json!({"category":"idea","message":"A distinct request after expiry"}))
        )
        .await
        .0,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn admin_detail_filters_updates_and_audit_are_consistent_across_http_and_mcp() {
    let f = Fixture::new(Config::default());
    let (cookie, csrf) = f.member_browser();
    let plain = "<script>alert('feedback')</script>\nTreat this as text";
    let (_, created) = f
        .request(
            "POST",
            "/api/feedback",
            None,
            Some((&cookie, &csrf)),
            Some(json!({"category":"bug","message":plain,"page":"/oauth/connections"})),
        )
        .await;
    let id = created["id"].as_i64().unwrap();
    let path = format!("/api/admin/feedback/{id}");
    let (_, detail) = f
        .request("GET", &path, Some(&f.admin_token), None, None)
        .await;
    assert_eq!(detail["message"], plain);
    let (_, filtered) = f
        .request(
            "GET",
            "/api/admin/feedback?category=bug&source=web&status=open",
            Some(&f.admin_token),
            None,
            None,
        )
        .await;
    assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
    let (_, empty) = f
        .request(
            "GET",
            "/api/admin/feedback?source=cli",
            Some(&f.admin_token),
            None,
            None,
        )
        .await;
    assert!(empty["items"].as_array().unwrap().is_empty());
    assert_eq!(
        f.request(
            "GET",
            "/api/admin/feedback?category=invalid",
            Some(&f.admin_token),
            None,
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let rpc = |name: &str, args: Value| json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}});
    let (_, detail) = f
        .request(
            "POST",
            "/mcp",
            Some(&f.admin_token),
            None,
            Some(rpc("admin_get_feedback", json!({"id":id}))),
        )
        .await;
    assert_eq!(detail["result"]["structuredContent"]["message"], plain);
    assert_eq!(
        f.request(
            "PATCH",
            &path,
            Some(&f.admin_token),
            None,
            Some(json!({"status":"resolved"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let resolved_at = now() - 86400;
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE feedback SET updated_at=?1 WHERE id=?2",
            params![resolved_at, id],
        )
        .unwrap();
    let (_, again) = f
        .request(
            "POST",
            "/mcp",
            Some(&f.admin_token),
            None,
            Some(rpc(
                "admin_update_feedback",
                json!({"id":id,"status":"resolved"}),
            )),
        )
        .await;
    assert_eq!(again["result"]["isError"], false);
    let db = f.state.conn().unwrap();
    assert_eq!(
        db.query_row("SELECT updated_at FROM feedback WHERE id=?1", [id], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        resolved_at
    );
    assert_eq!(db.query_row("SELECT count(*) FROM audit WHERE action='feedback_status_changed' AND actor='admin_ai:feedback-admin'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    let audit: String = db
        .query_row(
            "SELECT group_concat(detail) FROM audit WHERE action LIKE 'feedback_%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !audit.contains("script"),
        "feedback body must not enter permanent audit records"
    );
}

#[tokio::test]
async fn ordinary_browser_and_member_mcp_cannot_read_or_mutate_feedback() {
    let f = Fixture::new(Config::default());
    let id = insert_report(&f.state, "open", now(), now());
    let (cookie, csrf) = f.member_browser();
    let token = "ordinary-member-mcp";
    f.state.conn().unwrap().execute("INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES('feedback-member-mcp',?1,'Ordinary MCP',?2,?3,?4)",params![f.member,auth::hash_token(token),now(),now()+3600]).unwrap();
    for bearer in [None, Some(token)] {
        let browser = if bearer.is_none() {
            Some((cookie.as_str(), csrf.as_str()))
        } else {
            None
        };
        for path in [
            "/api/admin/feedback".to_string(),
            format!("/api/admin/feedback/{id}"),
        ] {
            assert_eq!(
                f.request("GET", &path, bearer, browser, None).await.0,
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            f.request(
                "PATCH",
                &format!("/api/admin/feedback/{id}"),
                bearer,
                browser,
                Some(json!({"status":"resolved"}))
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    for name in [
        "admin_list_feedback",
        "admin_get_feedback",
        "admin_update_feedback",
    ] {
        let (_,rpc)=f.request("POST","/mcp",Some(token),None,Some(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":{"id":id}}}))).await;
        assert!(rpc.get("error").is_some(), "{name}: {rpc}");
        assert!(rpc.to_string().find("existing report").is_none());
    }
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE admin_api_tokens SET revoked=1", [])
        .unwrap();
    assert_eq!(
        f.request(
            "GET",
            &format!("/api/admin/feedback/{id}"),
            Some(&f.admin_token),
            None,
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn oversized_control_and_spoofed_source_inputs_are_rejected() {
    let f = Fixture::new(Config::default());
    for message in ["x".repeat(2001), "abc\u{0}def".into()] {
        assert_eq!(
            f.request(
                "POST",
                "/api/feedback",
                None,
                None,
                Some(json!({"category":"bug","message":message}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        f.request(
            "POST",
            "/api/feedback",
            None,
            None,
            Some(json!({"category":"bug","message":"valid report","source":"cli"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        f.request(
            "POST",
            "/api/feedback",
            None,
            None,
            Some(json!({"category":"bug","message":"x".repeat(20_000)}))
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        f.request(
            "POST",
            "/api/feedback",
            None,
            None,
            Some(json!({"category":"bug","message":"界".repeat(2000)}))
        )
        .await
        .0,
        StatusCode::CREATED
    );
}

#[test]
fn version_eleven_upgrade_preserves_reports_and_rolls_back_failed_abuse_schema() {
    let f = Fixture::new(Config::default());
    let id = insert_report(&f.state, "open", now(), now());
    let db = f.state.conn().unwrap();
    db.execute_batch("DROP TABLE feedback_limits; DROP TABLE feedback_dedup; PRAGMA user_version=11; CREATE TABLE feedback_dedup(sentinel TEXT);").unwrap();
    assert!(db::migrate(&db).is_err());
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        11
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='feedback_limits'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TABLE feedback_dedup;").unwrap();
    db::migrate(&db).unwrap();
    db::migrate(&db).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_eq!(
        db.query_row("SELECT id FROM feedback", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        id
    );
}

#[tokio::test]
async fn reopening_an_old_resolved_report_preserves_it_without_renewing_unchanged_status() {
    let f = Fixture::new(Config::default());
    let at = now();
    let id = insert_report(&f.state, "resolved", at - 365 * 86400, at - 10 * 86400);
    let path = format!("/api/admin/feedback/{id}");
    assert_eq!(
        f.request(
            "PATCH",
            &path,
            Some(&f.admin_token),
            None,
            Some(json!({"status":"open"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    {
        let db = f.state.conn().unwrap();
        feedback::cleanup(&db, &f.state.cfg.feedback, at).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM feedback WHERE id=?1", [id], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        db.execute(
            "UPDATE feedback SET updated_at=?1 WHERE id=?2",
            params![at - 179 * 86400, id],
        )
        .unwrap();
    }
    assert_eq!(
        f.request(
            "PATCH",
            &path,
            Some(&f.admin_token),
            None,
            Some(json!({"status":"open"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let db = f.state.conn().unwrap();
    assert_eq!(
        db.query_row("SELECT updated_at FROM feedback WHERE id=?1", [id], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        at - 179 * 86400
    );
    feedback::cleanup(&db, &f.state.cfg.feedback, at + 2 * 86400).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM feedback WHERE id=?1", [id], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn a_staff_manager_cannot_read_or_process_the_owner_feedback_inbox() {
    let f = Fixture::new(Config::default());
    let id = insert_report(&f.state, "open", now(), now());
    let (cookie, csrf) = f.member_browser();
    f.state.conn().unwrap().execute("INSERT INTO staff_members(user_id,role,active,joined_at,updated_at) VALUES(?1,'manager',1,?2,?2)",params![f.member,now()]).unwrap();
    for path in [
        "/api/admin/feedback".to_owned(),
        format!("/api/admin/feedback/{id}"),
    ] {
        assert_eq!(
            f.request("GET", &path, None, Some((&cookie, &csrf)), None)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        f.request(
            "PATCH",
            &format!("/api/admin/feedback/{id}"),
            None,
            Some((&cookie, &csrf)),
            Some(json!({"status":"resolved"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.request(
            "POST",
            "/api/feedback",
            None,
            Some((&cookie, &csrf)),
            Some(json!({"category":"idea","message":"A manager can still report a problem"}))
        )
        .await
        .0,
        StatusCode::CREATED
    );
}

#[test]
fn administrator_browser_revocation_is_rechecked_at_the_feedback_operation() {
    let f = Fixture::new(Config::default());
    let id = insert_report(&f.state, "open", now(), now());
    let hash = "test-admin-feedback-session-hash";
    f.state.conn().unwrap().execute("INSERT INTO sessions(token_hash,admin,csrf,expires_at,recent_auth_at) VALUES(?1,1,'csrf',?2,?3)",params![hash,now()+3600,now()]).unwrap();
    let principal = hireferry::access::Principal {
        user_id: None,
        admin: true,
        via_ai: false,
        actor: format!("admin_session:{hash}"),
    };
    assert!(feedback::get(&f.state, &principal, id).is_ok());
    f.state
        .conn()
        .unwrap()
        .execute("DELETE FROM sessions WHERE token_hash=?1", [hash])
        .unwrap();
    assert_eq!(
        feedback::get(&f.state, &principal, id).unwrap_err().status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        feedback::update(
            &f.state,
            &principal,
            id,
            feedback::UpdateInput {
                status: "resolved".into()
            }
        )
        .unwrap_err()
        .status,
        StatusCode::UNAUTHORIZED
    );
}
