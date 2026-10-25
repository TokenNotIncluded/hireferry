//! Cross-surface hiring counts and recovery rules. All identities, payment
//! records and outbound transports are isolated fixtures; no provider is called.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{
    AppState,
    access::Principal,
    admin, auth, chat, commerce,
    config::{Config, Webhooks},
    models::{self, Job},
    webhooks::{self, DeliveryFailure, OutboundRequest, Transport, TransportFuture},
};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Mutex};
use tower::ServiceExt;

struct Fixture {
    state: AppState,
}
impl Fixture {
    fn new() -> Self {
        let mut cfg = Config {
            development: true,
            database: ":memory:".into(),
            ..Default::default()
        };
        cfg.payments.stripe.enabled = true;
        cfg.payments.stripe.secret_key = "sk_test_headcount_fixture".into();
        cfg.payments.stripe.webhook_secret = "whsec_headcount_fixture".into();
        Self {
            state: AppState::new(cfg).unwrap(),
        }
    }
    fn member(&self, role: &str) -> Principal {
        let id = uuid::Uuid::new_v4().to_string();
        self.state.conn().unwrap().execute(
            "INSERT INTO users(id,email,name,role,employer_paid,created_at,updated_at) VALUES(?1,?2,'Private fixture',?3,?4,?5,?5)",
            params![id, format!("headcount-{id}@example.invalid"), role, role == "employer", hireferry::now()],
        ).unwrap();
        Principal {
            user_id: Some(id.clone()),
            admin: false,
            via_ai: false,
            actor: id,
        }
    }
    fn seeker(&self) -> Principal {
        let p = self.member("seeker");
        chat::save_mandate(
            &self.state,
            &p,
            None,
            chat::MandateInput {
                target_industry: "Technology".into(),
                target_roles: "Engineer".into(),
                salary_min_cents: None,
                salary_max_cents: None,
                salary_currency: None,
                location: "Remote".into(),
                preferences: String::new(),
                status: "active".into(),
                allow_external: false,
            },
        )
        .unwrap();
        p
    }
    fn job(&self, owner: &Principal, headcount: i64) -> Job {
        let id = uuid::Uuid::new_v4().to_string();
        let db = self.state.conn().unwrap();
        db.execute(
            "INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,headcount,status,version,edits_used,created_at,updated_at) VALUES(?1,?2,'Private hiring title','PRIVATE_JD_SENTINEL',300000,?3,'published',7,2,?4,?4)",
            params![id, uid(owner), headcount, hireferry::now()],
        ).unwrap();
        db.execute(
            "INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,status,snapshot_json,created_at,paid_at) VALUES(?1,?2,'publish',?3,30000,'stripe','paid','{\"max_edits\":3,\"edit_bps\":100}',?4,?4)",
            params![format!("paid-{id}"), uid(owner), id, hireferry::now()],
        ).unwrap();
        models::load_job(&db, &id).unwrap()
    }
    fn token(&self, p: &Principal) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let raw = if p.admin {
            format!("hf_admin_{}", hex::encode(rand::random::<[u8; 32]>()))
        } else {
            format!("hf_{}", hex::encode(rand::random::<[u8; 32]>()))
        };
        let db = self.state.conn().unwrap();
        if p.admin {
            db.execute("INSERT INTO admin_api_tokens(id,name,token_hash,created_at,expires_at) VALUES(?1,'fixture',?2,?3,?4)",params![id,auth::hash_token(&raw),hireferry::now(),hireferry::now()+3600]).unwrap();
        } else {
            db.execute("INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES(?1,?2,'fixture',?3,?4,?5)",params![id,uid(p),auth::hash_token(&raw),hireferry::now(),hireferry::now()+3600]).unwrap();
        }
        raw
    }
    fn load(&self, id: &str) -> Job {
        models::load_job(&self.state.conn().unwrap(), id).unwrap()
    }
    fn scalar(&self, sql: &str) -> i64 {
        self.state
            .conn()
            .unwrap()
            .query_row(sql, [], |r| r.get(0))
            .unwrap()
    }
}
fn uid(p: &Principal) -> &str {
    p.user_id.as_deref().unwrap()
}
fn admin() -> Principal {
    Principal {
        user_id: None,
        admin: true,
        via_ai: false,
        actor: "admin".into(),
    }
}
fn patch(f: &Fixture, id: &str, input: Value) -> hireferry::AppResult<Job> {
    admin::patch_job(
        &f.state,
        &admin(),
        id,
        serde_json::from_value(input).unwrap(),
    )
}
fn introduction(f: &Fixture, seeker: &Principal, job: &Job) -> hireferry::AppResult<Value> {
    admin::add_match(
        &f.state,
        &admin(),
        admin::MatchInput {
            seeker_id: uid(seeker).into(),
            job_id: job.id.clone(),
            note: "Introduction".into(),
        },
    )
}
fn stage(f: &Fixture, id: &str, status: &str) -> hireferry::AppResult<()> {
    admin::patch_match(
        &f.state,
        &admin(),
        id,
        admin::MatchPatch {
            status: status.into(),
            note: String::new(),
        },
    )
}
async fn request(
    f: &Fixture,
    token: &str,
    method: &str,
    path: &str,
    value: Value,
) -> (StatusCode, Value) {
    let response = hireferry::app(f.state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("mcp-protocol-version", "2025-11-25")
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn rpc(f: &Fixture, token: &str, method: &str, params: Value) -> Value {
    let (status, value) = request(
        f,
        token,
        "POST",
        "/mcp",
        json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value
}
async fn call(f: &Fixture, token: &str, name: &str, arguments: Value) -> Value {
    rpc(
        f,
        token,
        "tools/call",
        json!({"name":name,"arguments":arguments}),
    )
    .await
}
fn ok(reply: &Value) -> &Value {
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    &reply["result"]["structuredContent"]
}
fn tool_error(reply: &Value) {
    assert!(
        reply["result"]["isError"] == true || reply["error"].is_object(),
        "{reply}"
    );
}

#[tokio::test]
async fn administrator_headcount_is_free_audited_and_independent_of_pending_jd_revision() {
    let f = Fixture::new();
    let employer = f.member("employer");
    let job = f.job(&employer, 3);
    let token = f.token(&admin());
    f.state.conn().unwrap().execute(
        "INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,status,snapshot_json,created_at) VALUES('pending-edit',?1,'edit',?2,3000,'stripe','pending','{\"version\":7,\"jd_md\":\"Pending description\"}',?3)",
        params![uid(&employer),job.id,hireferry::now()],
    ).unwrap();
    for count in [0, 2, 1_000_000, 4] {
        let (status, result) = request(
            &f,
            &token,
            "PATCH",
            &format!("/api/admin/jobs/{}", job.id),
            json!({"headcount":count}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["headcount"], count);
        assert_eq!(result["status"], "published");
        assert_eq!(result["version"], job.version);
        assert_eq!(result["edits_used"], job.edits_used);
        assert_eq!(result["title"], job.title);
        assert_eq!(result["salary_cents"], job.salary_cents);
        let totals =
            admin::overview_data(&f.state, &admin(), &admin::QueryInput::default()).unwrap();
        assert_eq!(
            totals["stats"]["active_jobs"],
            if count > 0 { 1 } else { 0 }
        );
    }
    assert_eq!(f.scalar("SELECT count(*) FROM orders"), 2);
    assert_eq!(f.scalar("SELECT count(*) FROM ledger"), 0);
    assert_eq!(
        f.scalar("SELECT count(*) FROM audit WHERE action='job_update'"),
        0
    );
    assert_eq!(
        f.scalar("SELECT count(*) FROM audit WHERE action='job_headcount_update'"),
        4
    );
    let (actor, detail):(String,String) = f.state.conn().unwrap().query_row(
        "SELECT actor,detail FROM audit WHERE action='job_headcount_update' ORDER BY id LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    assert!(actor.starts_with("admin_ai:"));
    assert_eq!(
        serde_json::from_str::<Value>(&detail).unwrap(),
        json!({"before":3,"after":0})
    );
    patch(&f, &job.id, json!({"headcount":4})).unwrap();
    assert_eq!(
        f.scalar("SELECT count(*) FROM audit WHERE action='job_headcount_update'"),
        4
    );
    for count in [-1, 1_000_001] {
        assert_eq!(
            patch(&f, &job.id, json!({"headcount":count}))
                .unwrap_err()
                .status,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        patch(&f, &job.id, json!({"jd_md":"Changed while pending"}))
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE orders SET status='cancelled' WHERE id='pending-edit'",
            [],
        )
        .unwrap();
    let changed = patch(
        &f,
        &job.id,
        json!({"jd_md":"Moderated description","headcount":4}),
    )
    .unwrap();
    assert_eq!(changed.version, job.version + 1);
    assert_eq!(changed.edits_used, job.edits_used);
}

#[tokio::test]
async fn administrator_recovery_uses_member_rules_and_permanent_deletion_checks_version() {
    let f = Fixture::new();
    let employer = f.member("employer");
    let job = f.job(&employer, 2);
    let token = f.token(&admin());
    let deleted = commerce::delete_job(&f.state, uid(&employer), &job.id, job.version).unwrap();
    assert_eq!(deleted.delete_count, 1);
    let restore_path = format!("/api/admin/jobs/{}/restore", job.id);
    assert_eq!(
        request(
            &f,
            &token,
            "POST",
            &restore_path,
            json!({"expected_version":job.version})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, restored) = request(
        &f,
        &token,
        "POST",
        &restore_path,
        json!({"expected_version":deleted.version}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(restored["status"], "published");
    assert_eq!(restored["headcount"], 2);
    assert_eq!(restored["delete_count"], 1);
    let delete_path = format!("/api/admin/jobs/{}", job.id);
    assert!(
        request(&f, &token, "DELETE", &delete_path, json!({}))
            .await
            .0
            .is_client_error()
    );
    assert_eq!(
        request(
            &f,
            &token,
            "DELETE",
            &delete_path,
            json!({"expected_version":deleted.version})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, erased) = request(
        &f,
        &token,
        "DELETE",
        &delete_path,
        json!({"expected_version":restored["version"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{erased}");
    assert_eq!(erased["status"], "deleted");
    assert_eq!(erased["delete_count"], 2);
    assert_eq!(erased["headcount"], 0);
    assert_eq!(erased["jd_md"], "");
    assert_ne!(erased["title"], job.title);
    assert_eq!(f.scalar("SELECT count(*) FROM orders"), 1);
    assert!(
        request(
            &f,
            &token,
            "POST",
            &restore_path,
            json!({"expected_version":erased["version"]})
        )
        .await
        .0
        .is_client_error()
    );
    assert!(patch(&f, &job.id, json!({"status":"published","headcount":1})).is_err());

    let closed = f.job(&employer, 5);
    let closed = patch(&f, &closed.id, json!({"status":"closed"})).unwrap();
    assert_eq!(closed.headcount, 0);
    assert!(patch(&f, &closed.id, json!({"headcount":1})).is_err());
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE orders SET status='refunded',refunded_cents=amount_cents WHERE job_id=?1",
            [&closed.id],
        )
        .unwrap();
    assert!(patch(&f, &closed.id, json!({"status":"published"})).is_err());

    let suspended_job = f.job(&employer, 1);
    let suspended_job = commerce::delete_job(
        &f.state,
        uid(&employer),
        &suspended_job.id,
        suspended_job.version,
    )
    .unwrap();
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id=?1", [uid(&employer)])
        .unwrap();
    assert!(
        admin::restore_job_data(&f.state, &admin(), &suspended_job.id, suspended_job.version)
            .is_err()
    );
    assert_eq!(f.load(&suspended_job.id).status, "deleted");
}

#[test]
fn zero_headcount_blocks_new_matches_without_consuming_openings_or_hiding_existing_chat() {
    let f = Fixture::new();
    let employer = f.member("employer");
    let job = f.job(&employer, 1);
    let seeker = f.seeker();
    let second = f.seeker();
    let third = f.seeker();
    let first_match = introduction(&f, &seeker, &job).unwrap();
    let second_match = introduction(&f, &second, &job).unwrap();
    assert_eq!(f.load(&job.id).headcount, 1); // Planned openings do not cap introductions.
    let first_id = first_match["id"].as_str().unwrap();
    let second_id = second_match["id"].as_str().unwrap();
    stage(&f, first_id, "hired").unwrap();
    assert_eq!(f.load(&job.id).headcount, 1); // HR maintains the plan manually.
    commerce::set_headcount(&f.state, uid(&employer), &job.id, 0).unwrap();
    assert_eq!(
        introduction(&f, &third, &job).unwrap_err().status,
        StatusCode::BAD_REQUEST
    );
    stage(&f, second_id, "interview").unwrap();
    let room = first_match["conversation_id"].as_str().unwrap();
    chat::send_message(
        &f.state,
        &seeker,
        room,
        chat::MessageInput {
            body: "Existing discussion continues".into(),
            client_id: uuid::Uuid::new_v4().to_string(),
        },
    )
    .unwrap();
    assert_eq!(
        chat::messages(&f.state, &employer, room, -1, 100).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    stage(&f, second_id, "declined").unwrap();
    assert!(stage(&f, second_id, "introduced").is_err());
    commerce::set_headcount(&f.state, uid(&employer), &job.id, 2).unwrap();
    stage(&f, second_id, "introduced").unwrap();
    introduction(&f, &third, &job).unwrap();
    assert_eq!(f.load(&job.id).headcount, 2);

    let current = f.load(&job.id);
    let deleted = commerce::delete_job(&f.state, uid(&employer), &job.id, current.version).unwrap();
    assert_eq!(
        chat::messages(&f.state, &seeker, room, -1, 100)
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let restored =
        commerce::restore_job(&f.state, uid(&employer), &job.id, deleted.version).unwrap();
    assert_eq!(restored.headcount, 2);
    assert_eq!(
        chat::messages(&f.state, &seeker, room, -1, 100).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn mcp_counts_remain_free_and_recovery_tools_keep_roles_versions_and_ai_eligibility() {
    let f = Fixture::new();
    let employer = f.member("employer");
    let qualifying = f.job(&employer, 1);
    let token = f.token(&employer);
    let admin_token = f.token(&admin());
    let member_tools = rpc(&f, &token, "tools/list", json!({})).await;
    let tools = member_tools["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 40);
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(names.len(), tools.len(), "Tool names must be unique");
    for name in [
        "get_chat_capabilities",
        "get_conversation",
        "get_chat_events",
        "begin_message",
        "append_message",
        "finish_message",
        "cancel_message",
    ] {
        assert!(names.contains(name), "Missing shared chat tool: {name}");
    }
    let site_config = call(&f, &token, "get_site_config", json!({})).await;
    assert_eq!(ok(&site_config)["pricing"]["currency"], "CNY");
    assert_eq!(ok(&site_config)["pricing"]["currency_decimals"], 2);
    assert!(ok(&site_config).get("payments").is_none());
    let count_tool = tools
        .iter()
        .find(|t| t["name"] == "set_job_headcount")
        .unwrap();
    assert_eq!(
        count_tool["inputSchema"]["properties"]["headcount"]["maximum"],
        1_000_000
    );
    let create_tool = tools.iter().find(|t| t["name"] == "create_job").unwrap();
    assert_eq!(
        create_tool["inputSchema"]["properties"]["headcount"]["default"],
        1
    );
    for name in ["delete_job", "permanently_delete_job"] {
        let tool = tools.iter().find(|t| t["name"] == name).unwrap();
        assert_eq!(tool["annotations"]["destructiveHint"], true);
        assert!(
            tool["inputSchema"]["required"]
                .as_array()
                .unwrap()
                .contains(&json!("expected_version"))
        );
    }
    let admin_tools = rpc(&f, &admin_token, "tools/list", json!({})).await;
    assert_eq!(
        admin_tools["result"]["tools"].as_array().unwrap().len(),
        46 + hireferry::relation_views::tools().len()
    );
    assert!(
        admin_tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "get_chat_capabilities")
    );
    let admin_config = call(&f, &admin_token, "get_site_config", json!({})).await;
    assert_eq!(ok(&admin_config), ok(&site_config));
    assert!(
        admin_tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "admin_restore_job")
    );
    let draft = call(&f,&token,"create_job",json!({"title":"New draft","jd_md":"New description","salary_cents":300000,"provider":"stripe"})).await;
    let draft = ok(&draft).clone();
    assert_eq!(draft["job"]["headcount"], 1);
    let large = call(&f,&token,"create_job",json!({"title":"Larger plan","jd_md":"Same pricing","salary_cents":300000,"provider":"stripe","headcount":1_000_000})).await;
    let large = ok(&large).clone();
    assert_eq!(large["job"]["headcount"], 1_000_000);
    assert_eq!(
        large["order"]["amount_cents"],
        draft["order"]["amount_cents"]
    );
    assert_eq!(large["order"]["amount_cents"], 30000);
    let orders = f.scalar("SELECT count(*) FROM orders");
    for count in [0, 3] {
        let updated = call(
            &f,
            &token,
            "set_job_headcount",
            json!({"id":qualifying.id,"headcount":count}),
        )
        .await;
        assert_eq!(ok(&updated)["headcount"], count);
        assert_eq!(ok(&updated)["version"], qualifying.version);
        let profile = call(&f, &token, "get_profile", json!({})).await;
        assert_eq!(ok(&profile)["id"], uid(&employer));
    }
    assert_eq!(f.scalar("SELECT count(*) FROM orders"), orders);
    for invalid in [
        json!(-1),
        json!(1_000_001),
        json!(1.5),
        json!("2"),
        Value::Null,
    ] {
        tool_error(
            &call(
                &f,
                &token,
                "set_job_headcount",
                json!({"id":qualifying.id,"headcount":invalid}),
            )
            .await,
        );
    }
    tool_error(
        &call(
            &f,
            &token,
            "set_job_headcount",
            json!({"id":qualifying.id,"headcount":2,"salary_cents":100}),
        )
        .await,
    );
    let stranger = f.member("employer");
    f.job(&stranger, 1);
    let stranger_token = f.token(&stranger);
    tool_error(
        &call(
            &f,
            &stranger_token,
            "set_job_headcount",
            json!({"id":qualifying.id,"headcount":1}),
        )
        .await,
    );
    tool_error(
        &call(
            &f,
            &admin_token,
            "set_job_headcount",
            json!({"id":qualifying.id,"headcount":1}),
        )
        .await,
    );
    tool_error(
        &call(
            &f,
            &token,
            "admin_update_job",
            json!({"id":qualifying.id,"headcount":1}),
        )
        .await,
    );

    let id = draft["job"]["id"].as_str().unwrap();
    let deleted = call(
        &f,
        &token,
        "delete_job",
        json!({"id":id,"expected_version":draft["job"]["version"]}),
    )
    .await;
    let deleted = ok(&deleted).clone();
    assert_eq!(deleted["delete_count"], 1);
    let trash = call(&f, &token, "list_deleted_jobs", json!({})).await;
    assert!(
        ok(&trash)["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|j| j["id"] == id)
    );
    let restored = call(
        &f,
        &token,
        "restore_job",
        json!({"id":id,"expected_version":deleted["version"]}),
    )
    .await;
    let restored = ok(&restored).clone();
    assert_eq!(restored["status"], "draft");
    assert_eq!(restored["delete_count"], 1);
    tool_error(
        &call(
            &f,
            &token,
            "delete_job",
            json!({"id":id,"expected_version":draft["job"]["version"]}),
        )
        .await,
    );
    let erased = call(
        &f,
        &token,
        "delete_job",
        json!({"id":id,"expected_version":restored["version"]}),
    )
    .await;
    let erased = ok(&erased).clone();
    assert_eq!(erased["delete_count"], 2);
    assert_eq!(erased["jd_md"], "");
    tool_error(
        &call(
            &f,
            &token,
            "restore_job",
            json!({"id":id,"expected_version":erased["version"]}),
        )
        .await,
    );
    let permanent = call(
        &f,
        &token,
        "permanently_delete_job",
        json!({"id":large["job"]["id"],"expected_version":large["job"]["version"]}),
    )
    .await;
    assert_eq!(ok(&permanent)["delete_count"], 2);
    let trash = call(&f, &token, "list_deleted_jobs", json!({})).await;
    assert!(ok(&trash)["items"].as_array().unwrap().is_empty());
}

struct Recorder(Mutex<Vec<OutboundRequest>>);
impl Transport for Recorder {
    fn resolve<'a>(
        &'a self,
        _: &'a url::Url,
        _: &'a Webhooks,
    ) -> TransportFuture<'a, Result<Vec<SocketAddr>, DeliveryFailure>> {
        Box::pin(async { Ok(vec!["8.8.8.8:443".parse().unwrap()]) })
    }
    fn post<'a>(
        &'a self,
        request: OutboundRequest,
        _: &'a Webhooks,
    ) -> TransportFuture<'a, Result<u16, DeliveryFailure>> {
        self.0.lock().unwrap().push(request);
        Box::pin(async { Ok(204) })
    }
}

#[tokio::test]
async fn upgraded_webhook_trigger_notifies_count_changes_without_leaking_fields_or_revoking_matches()
 {
    let f = Fixture::new();
    let employer = f.member("employer");
    let seeker = f.seeker();
    let stranger = f.seeker();
    let job = f.job(&employer, 3);
    introduction(&f, &seeker, &job).unwrap();
    {
        let db = f.state.conn().unwrap();
        for p in [&employer, &seeker, &stranger] {
            db.execute("INSERT INTO webhook_endpoints(id,user_id,name,url,events_json,signing_secret,created_at,updated_at) VALUES(?1,?2,'fixture','https://hooks.example.com/test','[\"job.updated\"]','hf_wh_fixture',?3,?3)",params![format!("endpoint-{}",uid(p)),uid(p),hireferry::now()]).unwrap();
        }
        // Reproduce the installed v3 predicate, then run the real upgrade.
        db.execute_batch("DROP TRIGGER webhook_capture_jobs_UPDATE;
            CREATE TRIGGER webhook_capture_jobs_UPDATE AFTER UPDATE ON jobs
            WHEN OLD.status IS NOT NEW.status OR OLD.version IS NOT NEW.version OR OLD.jd_md IS NOT NEW.jd_md
            BEGIN INSERT INTO webhook_events(event_type,resource_id,data_json) VALUES('job.updated',NEW.id,json_object('id',NEW.id,'status',NEW.status)); END;").unwrap();
        webhooks::migrate(&db).unwrap();
        webhooks::migrate(&db).unwrap(); // Reopening must not duplicate triggers.
    }
    commerce::set_headcount(&f.state, uid(&employer), &job.id, 0).unwrap();
    commerce::set_headcount(&f.state, uid(&employer), &job.id, 0).unwrap();
    assert_eq!(
        f.scalar("SELECT count(*) FROM webhook_events WHERE event_type='job.updated'"),
        1
    );
    assert_eq!(f.scalar("SELECT count(*) FROM webhook_deliveries"), 2);
    let recipients: Vec<String> = {
        let db = f.state.conn().unwrap();
        db.prepare("SELECT e.user_id FROM webhook_deliveries d JOIN webhook_endpoints e ON e.id=d.endpoint_id").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    };
    assert!(recipients.contains(&uid(&employer).to_owned()));
    assert!(recipients.contains(&uid(&seeker).to_owned()));
    assert!(!recipients.contains(&uid(&stranger).to_owned()));
    let transport = Recorder(Mutex::new(Vec::new()));
    for _ in 0..2 {
        assert!(
            webhooks::run_once_with(&f.state, &transport, hireferry::now() + 1)
                .await
                .unwrap()
        );
    }
    assert!(
        !webhooks::run_once_with(&f.state, &transport, hireferry::now() + 1)
            .await
            .unwrap()
    );
    for sent in transport.0.lock().unwrap().iter() {
        let event: Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(event["data"], json!({"id":job.id,"status":"published"}));
        assert!(!String::from_utf8_lossy(&sent.body).contains("PRIVATE_JD_SENTINEL"));
        assert!(!String::from_utf8_lossy(&sent.body).contains("headcount"));
    }
    assert_eq!(
        f.scalar("SELECT count(*) FROM webhook_deliveries WHERE status='delivered'"),
        2
    );
}
