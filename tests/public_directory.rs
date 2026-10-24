use axum::{body::Body, http::Request};
use hireferry::{AppState, config::Config};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

fn state() -> AppState {
    AppState::new(Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    })
    .unwrap()
}

async fn json_request(
    app: &axum::Router,
    path: &str,
    body: Value,
) -> (axum::http::StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
        .unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn public_directory_only_exposes_explicit_resumes_and_active_published_roles() {
    let state = state();
    let db = state.conn().unwrap();
    db.execute("INSERT INTO users(id,email,name,handle,role,public_email,resume_md,public_profile,ai_profile_links,created_at,updated_at) VALUES('seeker-public','public-seeker@example.test','Ada','ada','seeker','ada-public@example.test','## Rust\n\nI build APIs.',1,?1,1,1)", [json!({"chatgpt":"https://chatgpt.com/g/ada"}).to_string()]).unwrap();
    db.execute("INSERT INTO users(id,email,name,handle,role,created_at,updated_at) VALUES('seeker-private','private-seeker@example.test','Private','private','seeker',1,1)", []).unwrap();
    db.execute("INSERT INTO users(id,email,name,handle,role,created_at,updated_at) VALUES('employer','employer@example.test','Open Source Co','opensource','employer',1,1)", []).unwrap();
    db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,currency,ai_min_salary_cents,headcount,status,created_at,updated_at) VALUES('job-public','employer','Rust API maintainer','## Build public APIs',250000,'CNY',300000,2,'published',1,1)", []).unwrap();
    db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,currency,ai_min_salary_cents,headcount,status,created_at,updated_at) VALUES('job-paused','employer','Paused role','Paused',250000,'CNY',300000,0,'published',1,1)", []).unwrap();
    db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,currency,ai_min_salary_cents,headcount,status,created_at,updated_at) VALUES('job-closed','employer','Closed role','Closed',250000,'CNY',300000,0,'closed',1,1)", []).unwrap();
    drop(db);

    let app = hireferry::app(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/public/jobs?q=Rust")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let jobs: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(jobs["total"], 1);
    assert_eq!(jobs["items"][0]["id"], "job-public");
    assert!(
        jobs["items"][0]["jd_html"]
            .as_str()
            .unwrap()
            .contains("Build public APIs")
    );
    assert!(jobs["items"][0].get("public_email").is_none());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/public/people")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let people: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(people["total"], 1);
    assert_eq!(people["items"][0]["handle"], "ada");
    assert_eq!(
        people["items"][0]["ai_profile_links"]["chatgpt"],
        "https://chatgpt.com/g/ada"
    );

    let hidden = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/public/people/private")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hidden.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn public_mcp_is_unauthenticated_read_only_and_documented() {
    let state = state();
    state
        .conn()
        .unwrap()
        .execute("INSERT INTO users(id,email,name,handle,role,resume_md,public_profile,created_at,updated_at) VALUES('seeker','seeker@example.test','Ada','ada','seeker','Rust maintainer',1,1,1)", [])
        .unwrap();
    let app = hireferry::app(state);

    let (status, listed) = json_request(
        &app,
        "/mcp/public",
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(
        listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "hireferry_public_search_people")
    );
    assert!(listed.to_string().contains("Authorization") == false);

    let (status, called) = json_request(
        &app,
        "/mcp/public",
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"hireferry_public_search_people","arguments":{}}}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(called["result"]["isError"], false);
    assert!(called["result"]["structuredContent"]["items"][0]["handle"] == "ada");

    let (status, private) = json_request(
        &app,
        "/mcp/public",
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_profile","arguments":{}}}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(private["error"]["code"], -32602);
}
