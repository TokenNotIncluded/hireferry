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
    fn new(mut cfg: Config) -> Self {
        cfg.development = true;
        cfg.database = ":memory:".into();
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
        StatusCode::TOO_MANY_REQUESTS
    );
    for _ in 0..2 {
        assert_eq!(
            f.request("POST", "/api/feedback", None, None, Some(body.clone()))
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
            100
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
        db.execute("UPDATE feedback SET created_at=?1", [at - 91 * 86400])
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
            10
        );
    }
}
