//! Shared workbench state uses real SQLite, real operator checks and no network.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{AppState, access::Principal, auth, config::Config, relation_views as views};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt;

fn state() -> AppState {
    let mut config = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    config.webhooks.enabled = false;
    AppState::new(config).unwrap()
}
fn root() -> Principal {
    Principal {
        user_id: None,
        admin: true,
        via_ai: false,
        actor: "admin".into(),
    }
}
fn tool(s: &AppState, p: &Principal, name: &str, args: Value) -> hireferry::AppResult<Value> {
    views::call(s, p, name, args)
}
fn active(s: &AppState, p: &Principal) -> Value {
    tool(s, p, "hf_view_open", json!({"response":"state"})).unwrap()
}
fn patch(
    s: &AppState,
    p: &Principal,
    id: &str,
    rev: i64,
    op: &str,
    commands: Value,
) -> hireferry::AppResult<Value> {
    tool(
        s,
        p,
        "hf_view_patch",
        json!({"view_id":id,"if_view_rev":rev,"operation_id":op,"commands":commands}),
    )
}
fn seed(s: &AppState, count: usize) {
    let db = s.conn().unwrap();
    db.execute("INSERT INTO users(id,email,name,role,created_at,updated_at) VALUES('employer','employer@example.invalid','Employer','employer',1,1)",[]).unwrap();
    db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,status,created_at,updated_at) VALUES('job','employer','Rust role','PRIVATE_JD_SENTINEL',300000,'published',1,1)",[]).unwrap();
    for i in 0..count {
        let id = format!("seeker-{i:03}");
        db.execute("INSERT INTO users(id,email,name,role,resume_md,created_at,updated_at) VALUES(?1,?2,?3,'seeker','PRIVATE_RESUME_SENTINEL',?4,?4)",
            params![id,format!("{id}@example.invalid"),format!("Candidate {i:03} {}","x".repeat(120)),i as i64+10]).unwrap();
        db.execute("INSERT INTO matches(id,seeker_id,job_id,status,note,created_at,updated_at) VALUES(?1,?2,'job','introduced','PRIVATE_NOTE_SENTINEL',1,1)",params![format!("match-{i:03}"),id]).unwrap();
    }
}
fn ai_staff(s: &AppState, name: &str) -> Principal {
    let db = s.conn().unwrap();
    db.execute("INSERT INTO team_members(id,kind,name,created_by,created_at,updated_at) VALUES(?1,'ai',?1,'admin',1,1)",[name]).unwrap();
    db.execute("INSERT INTO team_ai_tokens(id,member_id,token_hash,created_at,expires_at) VALUES(?1,?1,?2,1,?3)",
        params![name,format!("hash-{name}"),hireferry::now()+3600]).unwrap();
    Principal {
        user_id: None,
        admin: true,
        via_ai: true,
        actor: format!("staff_ai:{name}"),
    }
}
fn ai_root(s: &AppState) -> Principal {
    s.conn().unwrap().execute("INSERT INTO admin_api_tokens(id,name,token_hash,created_at,expires_at) VALUES('root-ai','root test','root-token-hash',1,?1)",[hireferry::now()+3600]).unwrap();
    Principal {
        user_id: None,
        admin: true,
        via_ai: true,
        actor: "admin_ai:root-ai".into(),
    }
}

#[test]
fn active_resume_cas_atomic_commands_retry_and_one_step_undo() {
    let s = state();
    let p = root();
    let first = active(&s, &p);
    let id = first["view_id"].as_str().unwrap();
    assert_eq!(first["view_rev"], 1);
    assert_eq!(first["ui"]["status"], "stored");
    assert_eq!(active(&s, &p)["view_id"], id);
    let resumed = tool(
        &s,
        &p,
        "hf_view_open",
        json!({"mode":"resume","view_id":id,"response":"state"}),
    )
    .unwrap();
    assert_eq!(resumed["view_rev"], 1);
    let commands =
        json!([{"op":"set_mode","mode":"matrix"},{"op":"set_ordering","ordering":"degree"}]);
    let changed = patch(&s, &p, id, 1, "change-1", commands.clone()).unwrap();
    assert_eq!(changed["view_rev"], 2);
    assert_eq!(patch(&s, &p, id, 1, "change-1", commands).unwrap(), changed);
    assert_eq!(
        patch(
            &s,
            &p,
            id,
            2,
            "change-1",
            json!([{"op":"set_mode","mode":"list"}])
        )
        .unwrap_err()
        .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        patch(
            &s,
            &p,
            id,
            1,
            "stale",
            json!([{"op":"set_mode","mode":"list"}])
        )
        .unwrap_err()
        .status,
        StatusCode::CONFLICT
    );
    let bad = patch(
        &s,
        &p,
        id,
        2,
        "atomic-bad",
        json!([
            {"op":"set_mode","mode":"list"},{"op":"set_filters","filters":{"role":"administrator"}}
        ]),
    )
    .unwrap_err();
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let unchanged = views::get_view(&s, &p, Some(id)).unwrap();
    assert_eq!(unchanged["view_rev"], 2);
    assert_eq!(unchanged["state"]["mode"], "matrix");
    let undone = patch(&s, &p, id, 2, "undo", json!([{"op":"undo"}])).unwrap();
    assert_eq!(undone["view_rev"], 3);
    assert_eq!(
        views::get_view(&s, &p, Some(id)).unwrap()["state"]["mode"],
        "overview"
    );
    assert_eq!(
        patch(&s, &p, id, 3, "undo-again", json!([{"op":"undo"}]))
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
}

#[test]
fn focus_validation_uses_the_final_atomic_filter_scope() {
    let s = state();
    seed(&s, 1);
    let p = root();
    let v = active(&s, &p);
    let id = v["view_id"].as_str().unwrap();
    let missing = patch(
        &s,
        &p,
        id,
        1,
        "missing-focus",
        json!([
            {"op":"set_mode","mode":"focus"},
            {"op":"set_focus","focus":{"side":"left","id":"missing-user"}}
        ]),
    )
    .unwrap_err();
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(views::get_view(&s, &p, Some(id)).unwrap()["view_rev"], 1);
    let focused = patch(
        &s,
        &p,
        id,
        1,
        "real-focus",
        json!([
            {"op":"set_mode","mode":"focus"},
            {"op":"set_focus","focus":{"side":"left","id":"seeker-000"}}
        ]),
    )
    .unwrap();
    assert_eq!(focused["view_rev"], 2);
    let excluded = patch(
        &s,
        &p,
        id,
        2,
        "excluded-focus",
        json!([
            {"op":"set_filters","filters":{"role":"employer"}}
        ]),
    )
    .unwrap_err();
    assert_eq!(excluded.status, StatusCode::CONFLICT);
    assert!(excluded.message.contains("excluded_by_filter"));
    let unchanged = views::get_view(&s, &p, Some(id)).unwrap();
    assert_eq!(unchanged["view_rev"], 2);
    assert_eq!(unchanged["state"]["filters"]["role"], Value::Null);
    let cleared = patch(
        &s,
        &p,
        id,
        2,
        "clear-and-filter",
        json!([
            {"op":"set_filters","filters":{"role":"employer"}},
            {"op":"set_focus","focus":null},
            {"op":"set_mode","mode":"overview"}
        ]),
    )
    .unwrap();
    assert_eq!(cleared["view_rev"], 3);
}

#[test]
fn typed_commands_are_bounded_and_visibility_never_modifies_business_records() {
    let s = state();
    seed(&s, 1);
    let p = root();
    let first = active(&s, &p);
    let id = first["view_id"].as_str().unwrap();
    let result = patch(
        &s,
        &p,
        id,
        1,
        "hide",
        json!([
            {"op":"set_visibility","side":"left","ids":["seeker-000"],"visible":false},
            {"op":"set_visibility","side":"edges","ids":["match-000"],"visible":false}
        ]),
    )
    .unwrap();
    assert_eq!(result["summary"]["hidden_nodes"], 1);
    assert_eq!(result["summary"]["hidden_edges"], 1);
    assert_eq!(
        s.conn()
            .unwrap()
            .query_row("SELECT count(*) FROM matches", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let ids: Vec<String> = (0..101).map(|i| format!("id{i}")).collect();
    assert!(
        patch(
            &s,
            &p,
            id,
            2,
            "too-many",
            json!([{"op":"set_visibility","side":"left","ids":ids,"visible":false}])
        )
        .is_err()
    );
    assert!(
        patch(
            &s,
            &p,
            id,
            2,
            "unknown",
            json!([{"op":"run_javascript","source":"alert(1)"}])
        )
        .is_err()
    );
    assert!(
        patch(
            &s,
            &p,
            id,
            2,
            "extra",
            json!([{"op":"set_mode","mode":"list","delete_matches":true}])
        )
        .is_err()
    );
    assert!(
        patch(
            &s,
            &p,
            id,
            2,
            "mixed-undo",
            json!([{"op":"undo"},{"op":"set_mode","mode":"list"}])
        )
        .is_err()
    );
    assert_eq!(views::get_view(&s, &p, Some(id)).unwrap()["view_rev"], 2);
}

#[test]
fn logical_operator_sharing_isolated_team_members_and_revocation() {
    let s = state();
    let a = ai_staff(&s, "agent-a");
    let b = ai_staff(&s, "agent-b");
    let v = active(&s, &a);
    let id = v["view_id"].as_str().unwrap();
    assert_eq!(
        views::get_view(&s, &b, Some(id)).unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(views::list(&s, &b, 20).unwrap()["total"], 0);
    let receipt = patch(
        &s,
        &a,
        id,
        1,
        "owned",
        json!([{"op":"set_mode","mode":"matrix"}]),
    )
    .unwrap();
    assert_eq!(receipt["view_rev"], 2);
    s.conn()
        .unwrap()
        .execute("UPDATE team_ai_tokens SET revoked=1 WHERE id='agent-a'", [])
        .unwrap();
    assert_eq!(
        views::get_view(&s, &a, Some(id)).unwrap_err().status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        patch(
            &s,
            &a,
            id,
            1,
            "owned",
            json!([{"op":"set_mode","mode":"matrix"}])
        )
        .unwrap_err()
        .status,
        StatusCode::UNAUTHORIZED
    );
    let member = Principal {
        user_id: Some("not-an-operator".into()),
        admin: false,
        via_ai: false,
        actor: "not-an-operator".into(),
    };
    assert_eq!(
        views::list(&s, &member, 20).unwrap_err().status,
        StatusCode::FORBIDDEN
    );
    let human = active(&s, &root());
    let ai = ai_root(&s);
    assert_eq!(active(&s, &ai)["view_id"], human["view_id"]);
}

#[test]
fn render_ack_is_browser_only_exact_revision_and_fullscreen_remains_a_gesture() {
    let s = state();
    let p = root();
    let ai = ai_root(&s);
    let first = active(&s, &p);
    let id = first["view_id"].as_str().unwrap();
    let ack = views::AckInput {
        view_rev: 1,
        client_id: "browser-one".into(),
        fullscreen: views::Fullscreen::Windowed,
    };
    assert_eq!(
        views::acknowledge(&s, &ai, id, &ack).unwrap_err().status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        views::acknowledge(&s, &p, id, &ack).unwrap()["ui"]["status"],
        "applied"
    );
    let updated = patch(
        &s,
        &ai,
        id,
        1,
        "fullscreen",
        json!([{"op":"request_fullscreen"}]),
    )
    .unwrap();
    assert_eq!(updated["ui"]["status"], "queued");
    assert_eq!(updated["ui"]["fullscreen"], "requires_user_gesture");
    assert_eq!(
        views::acknowledge(&s, &p, id, &ack).unwrap_err().status,
        StatusCode::CONFLICT
    );
    let ack = views::AckInput {
        view_rev: 2,
        client_id: "browser-one".into(),
        fullscreen: views::Fullscreen::RequiresUserGesture,
    };
    let applied = views::acknowledge(&s, &p, id, &ack).unwrap();
    assert_eq!(applied["ui"]["status"], "applied");
    assert_eq!(applied["ui"]["fullscreen"], "requires_user_gesture");
    s.conn()
        .unwrap()
        .execute("UPDATE relation_views SET seen_at=0 WHERE id=?1", [id])
        .unwrap();
    assert_eq!(
        views::get_view(&s, &p, Some(id)).unwrap()["ui"]["status"],
        "stored"
    );
}

#[test]
fn graph_summaries_handles_projection_budget_pagination_and_snapshot_rejection() {
    let s = state();
    seed(&s, 30);
    let p = root();
    let summary = tool(
        &s,
        &p,
        "hf_graph_query",
        json!({"query":{"mode":"nodes","side":"left","filters":{"role":"seeker"}}}),
    )
    .unwrap();
    assert!(summary.get("items").is_none());
    assert_eq!(summary["summary"]["total"], 30);
    let result = summary["result_id"].as_str().unwrap();
    let mut cursor: Option<String> = None;
    let mut ids = Vec::new();
    loop {
        let mut args =
            json!({"result_id":result,"limit":20,"fields":["id","label"],"max_bytes":2048});
        if let Some(c) = cursor.take() {
            args["cursor"] = json!(c);
        }
        let page = tool(&s, &p, "hf_graph_read", args).unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() <= 2048);
        let items = page["items"].as_array().unwrap();
        assert!(!items.is_empty());
        for item in items {
            assert!(item.get("email").is_none());
            assert!(item.get("resume_md").is_none());
            ids.push(item["id"].as_str().unwrap().to_owned());
        }
        cursor = page["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids.len(), 30);
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 30);
    assert!(
        tool(
            &s,
            &p,
            "hf_graph_read",
            json!({"result_id":result,"fields":["resume_md"]})
        )
        .is_err()
    );
    let another = ai_staff(&s, "other-reader");
    assert_eq!(
        tool(&s, &another, "hf_graph_read", json!({"result_id":result}))
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    s.conn()
        .unwrap()
        .execute("UPDATE users SET name='Changed' WHERE id='seeker-000'", [])
        .unwrap();
    let error = tool(&s, &p, "hf_graph_read", json!({"result_id":result})).unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert!(error.message.contains("resync_required"));
}

#[test]
fn overview_rows_and_cursors_are_bounded_and_projection_bound() {
    let s = state();
    seed(&s, 3);
    let p = root();
    let result = tool(&s, &p, "hf_graph_query", json!({})).unwrap();
    let id = result["result_id"].as_str().unwrap();
    assert_eq!(result["summary"]["edges"], 3);
    let page = tool(
        &s,
        &p,
        "hf_graph_read",
        json!({"result_id":id,"limit":1,"fields":["id","label","count"]}),
    )
    .unwrap();
    assert_eq!(page["returned"], 1);
    let cursor = page["next_cursor"].as_str().unwrap();
    assert!(
        tool(
            &s,
            &p,
            "hf_graph_read",
            json!({"result_id":id,"cursor":cursor,"fields":["id"]})
        )
        .is_err()
    );
    let next = tool(
        &s,
        &p,
        "hf_graph_read",
        json!({"result_id":id,"cursor":cursor,"limit":1,"fields":["id","label","count"]}),
    )
    .unwrap();
    assert_ne!(page["items"][0]["id"], next["items"][0]["id"]);
    s.conn()
        .unwrap()
        .execute("UPDATE relation_results SET expires_at=0 WHERE id=?1", [id])
        .unwrap();
    assert_eq!(
        tool(&s, &p, "hf_graph_read", json!({"result_id":id}))
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
}

#[test]
fn save_is_an_independent_idempotent_copy_and_does_not_switch_active_view() {
    let s = state();
    let p = root();
    let first = active(&s, &p);
    let id = first["view_id"].as_str().unwrap();
    patch(
        &s,
        &p,
        id,
        1,
        "matrix",
        json!([{"op":"set_mode","mode":"matrix"}]),
    )
    .unwrap();
    let args =
        json!({"view_id":id,"if_view_rev":2,"name":"Saved matrix","operation_id":"save-matrix"});
    let saved = tool(&s, &p, "hf_view_save", args.clone()).unwrap();
    assert_eq!(tool(&s, &p, "hf_view_save", args).unwrap(), saved);
    patch(
        &s,
        &p,
        id,
        2,
        "back",
        json!([{"op":"set_mode","mode":"overview"}]),
    )
    .unwrap();
    assert_eq!(active(&s, &p)["view_id"], id);
    let preset = saved["preset_id"].as_str().unwrap();
    assert_eq!(
        views::get_view(&s, &p, Some(preset)).unwrap()["state"]["mode"],
        "matrix"
    );
    let restored = tool(
        &s,
        &p,
        "hf_view_open",
        json!({"mode":"create","preset_id":preset,"operation_id":"restore","response":"state"}),
    )
    .unwrap();
    assert_eq!(restored["state"]["mode"], "matrix");
    assert_ne!(restored["view_id"], preset);
    assert_eq!(views::list(&s, &p, 64).unwrap()["total"], 3);
}

#[test]
fn activate_reuses_existing_views_and_requires_a_fresh_render_ack() {
    let s = state();
    let p = root();
    let v = active(&s, &p);
    let original = v["view_id"].as_str().unwrap();
    let ack = views::AckInput {
        view_rev: 1,
        client_id: "browser".into(),
        fullscreen: views::Fullscreen::Windowed,
    };
    assert_eq!(
        views::acknowledge(&s, &p, original, &ack).unwrap()["ui"]["status"],
        "applied"
    );
    let created = tool(
        &s,
        &p,
        "hf_view_open",
        json!({"mode":"create","name":"Other","operation_id":"create-other"}),
    )
    .unwrap();
    let other = created["view_id"].as_str().unwrap();
    let inactive = views::get_view(&s, &p, Some(original)).unwrap();
    assert_eq!(inactive["active"], false);
    assert_eq!(inactive["ui"]["status"], "stored");
    let args = json!({"mode":"activate","view_id":original,"operation_id":"activate-original","response":"state"});
    let switched = tool(&s, &p, "hf_view_open", args.clone()).unwrap();
    assert_eq!(switched["view_rev"], 1);
    assert_eq!(switched["active"], true);
    assert_eq!(switched["ui"]["status"], "queued");
    assert_eq!(switched["ui"]["ack_view_rev"], Value::Null);
    assert_eq!(tool(&s, &p, "hf_view_open", args).unwrap(), switched);
    assert_eq!(
        views::get_view(&s, &p, Some(other)).unwrap()["active"],
        false
    );
    assert_eq!(views::list(&s, &p, 64).unwrap()["total"], 2);
    assert_eq!(
        views::acknowledge(&s, &p, original, &ack).unwrap()["ui"]["status"],
        "applied"
    );
}

#[tokio::test]
async fn http_views_require_operator_csrf_and_actual_browser_ack() {
    let s = state();
    let raw = "relation-view-browser";
    let csrf = "view-csrf";
    s.conn().unwrap().execute("INSERT INTO sessions(token_hash,user_id,admin,bootstrap,csrf,expires_at,recent_auth_at) VALUES(?1,NULL,1,0,?2,?3,?4)",
        params![auth::hash_token(raw),csrf,hireferry::now()+3600,hireferry::now()]).unwrap();
    let app = hireferry::app(s.clone());
    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/relations/views/active")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let make = |uri: &str, body: Value, valid: bool| {
        let mut r = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .header("origin", s.cfg.public_url.as_str())
            .header("cookie", format!("hf_session={raw}"));
        if valid {
            r = r.header("x-csrf-token", csrf);
        }
        r.body(Body::from(body.to_string())).unwrap()
    };
    let denied = app
        .clone()
        .oneshot(make("/api/admin/relations/views", json!({}), false))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let opened = app
        .clone()
        .oneshot(make(
            "/api/admin/relations/views",
            json!({"response":"state"}),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(opened.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&opened.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = value["view_id"].as_str().unwrap();
    let ack = app
        .clone()
        .oneshot(make(
            &format!("/api/admin/relations/views/{id}/ack"),
            json!({"view_rev":1,"client_id":"test-browser"}),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(ack.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&ack.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["ui"]["status"], "applied");
    let tools = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/relations/tools")
                .header("cookie", format!("hf_session={raw}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tools.status(), StatusCode::OK);
    let catalog: Value =
        serde_json::from_slice(&tools.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(catalog["tools"].as_array().unwrap().len(), 6);
}
