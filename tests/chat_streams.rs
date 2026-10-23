//! Streaming protocol behavior, authorization, replay and resource boundaries.
use std::{convert::Infallible, time::Duration};

use axum::{
    body::{Body, Bytes},
    http::{Request, StatusCode},
};
use hireferry::{
    AppState,
    access::Principal,
    auth,
    chat::{self, FormattedMessageInput, MessageInput},
    chat_stream::{self, StreamAppendInput, StreamBeginInput, StreamEndInput},
    config::Config,
};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    user: Principal,
    token: String,
    room: String,
}
impl Fixture {
    fn new() -> Self {
        let state = AppState::new(Config {
            development: true,
            database: ":memory:".into(),
            ..Default::default()
        })
        .unwrap();
        let user = Principal {
            user_id: Some("seeker".into()),
            actor: "seeker".into(),
            admin: false,
            via_ai: false,
        };
        let token = "hf_test_chat_stream_token".to_owned();
        state.conn().unwrap().execute("INSERT INTO users(id,email,name,role,created_at,updated_at) VALUES('seeker','stream@example.invalid','Stream user','seeker',1,1)",[]).unwrap();
        state.conn().unwrap().execute("INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES('token','seeker','test',?1,1,?2)",params![auth::hash_token(&token),hireferry::now()+3600]).unwrap();
        Self {
            state,
            user,
            token,
            room: "platform_seeker".into(),
        }
    }
    fn start(&self, client: &str, body: &str) -> Value {
        chat_stream::begin(
            &self.state,
            &self.user,
            &self.room,
            StreamBeginInput {
                body: body.into(),
                client_id: client.into(),
                format: "markdown".into(),
            },
        )
        .unwrap()
    }
    fn append(&self, id: i64, sequence: i64, delta: &str) -> hireferry::AppResult<Value> {
        chat_stream::append(
            &self.state,
            &self.user,
            &self.room,
            id,
            StreamAppendInput {
                sequence,
                delta: delta.into(),
            },
        )
    }
    fn req(&self, method: &str, uri: &str, body: Body) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {}", self.token))
            .body(body)
            .unwrap()
    }
    fn matched_room(&self) -> String {
        let db = self.state.conn().unwrap();
        db.execute("INSERT INTO users(id,email,role,employer_paid,created_at,updated_at) VALUES('employer','employer@example.invalid','employer',1,1,1)",[]).unwrap();
        db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,status,created_at,updated_at) VALUES('job','employer','Private job','Private',300000,'published',1,1)",[]).unwrap();
        db.execute("INSERT INTO matches(id,seeker_id,job_id,status,created_at,updated_at) VALUES('intro','seeker','job','introduced',1,1)",[]).unwrap();
        "match_intro".into()
    }
}
fn id(v: &Value) -> i64 {
    v["id"].as_i64().unwrap()
}
async fn frame(body: &mut Body) -> String {
    let value = tokio::time::timeout(Duration::from_secs(2), body.frame())
        .await
        .expect("stream must flush a frame")
        .expect("stream must remain open")
        .unwrap();
    String::from_utf8(value.into_data().unwrap().to_vec()).unwrap()
}

#[test]
fn one_message_identity_sequenced_retries_and_terminal_states() {
    let f = Fixture::new();
    let first = f.start("same", "**Hello");
    let message = id(&first);
    assert_eq!(first["status"], "streaming");
    assert_eq!(first["sequence"], 0);
    assert_eq!(f.start("same", "**Hello")["id"], message);
    let second = f.append(message, 1, " world**").unwrap();
    assert_eq!(second["sequence"], 1);
    assert!(second.get("body").is_none());
    assert_eq!(f.append(message, 1, " world**").unwrap(), second);
    assert_eq!(
        f.append(message, 1, "another").unwrap_err().status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.append(message, 3, "skip").unwrap_err().status,
        StatusCode::CONFLICT
    );
    let complete = chat_stream::finish(
        &f.state,
        &f.user,
        &f.room,
        message,
        StreamEndInput { sequence: 2 },
    )
    .unwrap();
    assert_eq!(complete["status"], "complete");
    assert!(complete.get("body_html").is_none());
    let snapshot = chat::messages(&f.state, &f.user, &f.room, -1, 10).unwrap();
    assert_eq!(
        snapshot["items"][0]["body_html"],
        "<p><strong>Hello world</strong></p>\n"
    );
    assert_eq!(
        chat_stream::finish(
            &f.state,
            &f.user,
            &f.room,
            message,
            StreamEndInput { sequence: 2 }
        )
        .unwrap(),
        complete
    );
    assert!(f.append(message, 2, "late").is_err());
    assert!(
        chat_stream::cancel(
            &f.state,
            &f.user,
            &f.room,
            message,
            StreamEndInput { sequence: 3 }
        )
        .is_err()
    );
    // Retrying a previously accepted append never appends twice, even after finish.
    assert_eq!(
        f.append(message, 1, " world**").unwrap()["status"],
        "complete"
    );
    let rows: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM chat_messages", [], |r| r.get(0))
        .unwrap();
    let events: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM chat_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!((rows, events), (1, 3));
}

#[test]
fn replay_contains_historical_snapshots_compact_deltas_and_atomic_cursor() {
    let f = Fixture::new();
    let message = id(&f.start("replay", "first"));
    f.append(message, 1, " second").unwrap();
    chat_stream::finish(
        &f.state,
        &f.user,
        &f.room,
        message,
        StreamEndInput { sequence: 2 },
    )
    .unwrap();
    let events = chat_stream::events(&f.state, &f.user, Some(&f.room), 0, 100).unwrap();
    assert_eq!(events["items"][0]["message"]["body"], "first");
    assert_eq!(events["items"][0]["message"]["status"], "streaming");
    assert_eq!(events["items"][1]["delta"], " second");
    assert!(events["items"][1].get("message").is_none());
    assert_eq!(events["items"][2]["message"]["body"], "first second");
    let page = chat::messages(&f.state, &f.user, &f.room, -1, 10).unwrap();
    assert_eq!(page["event_cursor"], events["next_cursor"]);
    assert_eq!(
        chat::list_conversations(&f.state, &f.user).unwrap()["event_cursor"],
        events["next_cursor"]
    );
    let first = chat_stream::events(&f.state, &f.user, Some(&f.room), 0, 1).unwrap();
    assert_eq!(first["has_more"], true);
    let second = chat_stream::events(
        &f.state,
        &f.user,
        Some(&f.room),
        first["next_cursor"].as_i64().unwrap(),
        1,
    )
    .unwrap();
    assert_eq!(second["items"][0]["sequence"], 1);
}

#[test]
fn normal_sends_emit_events_keep_markdown_default_and_respect_explicit_format() {
    let f = Fixture::new();
    let legacy = chat::send_message(
        &f.state,
        &f.user,
        &f.room,
        MessageInput {
            body: "**old**".into(),
            client_id: "legacy".into(),
        },
    )
    .unwrap();
    assert_eq!(legacy["format"], "markdown");
    assert_eq!(legacy["status"], "complete");
    let plain = chat::send_formatted_message(
        &f.state,
        &f.user,
        &f.room,
        FormattedMessageInput {
            body: "<script>bad()</script> **plain**".into(),
            client_id: "plain".into(),
            format: "plain".into(),
        },
    )
    .unwrap();
    assert!(!plain["body_html"].as_str().unwrap().contains("<script>"));
    assert!(!plain["body_html"].as_str().unwrap().contains("<strong>"));
    let openui = chat::send_formatted_message(
        &f.state,
        &f.user,
        &f.room,
        FormattedMessageInput {
            body: "root = Col([Text(\"Hello\")])".into(),
            client_id: "ui".into(),
            format: "openui".into(),
        },
    )
    .unwrap();
    assert_eq!(openui["format"], "openui");
    assert!(
        chat::send_formatted_message(
            &f.state,
            &f.user,
            &f.room,
            FormattedMessageInput {
                body: "x".into(),
                client_id: "invalid".into(),
                format: "html".into()
            }
        )
        .is_err()
    );
    assert_eq!(
        chat_stream::events(&f.state, &f.user, None, 0, 100).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn streams_never_cross_owner_or_room_boundaries_and_recheck_bans() {
    let f = Fixture::new();
    f.state.conn().unwrap().execute("INSERT INTO users(id,email,role,created_at,updated_at) VALUES('other','other@example.invalid','seeker',1,1)",[]).unwrap();
    let other = Principal {
        user_id: Some("other".into()),
        actor: "other".into(),
        admin: false,
        via_ai: false,
    };
    let message = id(&f.start("private", "secret"));
    assert!(
        chat_stream::append(
            &f.state,
            &other,
            &f.room,
            message,
            StreamAppendInput {
                sequence: 1,
                delta: "intrusion".into()
            }
        )
        .is_err()
    );
    assert!(
        chat_stream::append(
            &f.state,
            &other,
            "platform_other",
            message,
            StreamAppendInput {
                sequence: 1,
                delta: "intrusion".into()
            }
        )
        .is_err()
    );
    assert!(chat_stream::events(&f.state, &other, Some(&f.room), 0, 100).is_err());
    assert_eq!(
        chat_stream::events(&f.state, &other, None, 0, 100).unwrap()["items"],
        json!([])
    );
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id='seeker'", [])
        .unwrap();
    assert!(f.append(message, 1, "after ban").is_err());
    assert!(chat_stream::events(&f.state, &f.user, None, 0, 100).is_err());
}

#[test]
fn bounded_streams_and_pruned_replay_require_explicit_resynchronization() {
    let f = Fixture::new();
    let first = id(&f.start("limit", &"字".repeat(9999)));
    f.append(first, 1, "末").unwrap();
    assert!(f.append(first, 2, "overflow").is_err());
    assert!(f.append(first, 2, &"x".repeat(4001)).is_err());
    for n in 1..4 {
        f.start(&format!("active-{n}"), "pending");
    }
    assert!(
        chat_stream::begin(
            &f.state,
            &f.user,
            &f.room,
            StreamBeginInput {
                body: "fifth".into(),
                client_id: "fifth".into(),
                format: "markdown".into()
            }
        )
        .is_err()
    );
    chat_stream::cancel(
        &f.state,
        &f.user,
        &f.room,
        first,
        StreamEndInput { sequence: 2 },
    )
    .unwrap();
    assert_eq!(f.start("fifth", "allowed")["status"], "streaming");
    // Simulate the retention frontier without 10000 redundant test messages.
    f.state
        .conn()
        .unwrap()
        .execute("DELETE FROM chat_events WHERE id<=2", [])
        .unwrap();
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE chat_event_retention SET pruned_through=2 WHERE id=1",
            [],
        )
        .unwrap();
    let reset = chat_stream::events(&f.state, &f.user, Some(&f.room), 1, 100).unwrap();
    assert_eq!(reset["resync_required"], true);
    assert_eq!(reset["items"], json!([]));
    assert!(chat_stream::events(&f.state, &f.user, None, i64::MAX, 100).is_err());
}

#[tokio::test]
async fn sse_flushes_live_deltas_and_stops_after_credential_revocation() {
    let f = Fixture::new();
    let app = hireferry::app(f.state.clone());
    let response = app
        .oneshot(f.req(
            "GET",
            "/api/chat/events/stream?conversation_id=platform_seeker&after=0",
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(response.headers()["x-accel-buffering"], "no");
    let mut body = response.into_body();
    assert!(frame(&mut body).await.contains("event: cursor"));
    let message = id(&f.start("sse", "live"));
    let start = frame(&mut body).await;
    assert!(start.contains("event: message.started"));
    assert!(start.contains("live"));
    f.append(message, 1, " token").unwrap();
    // Cursor after the preceding batch is deliberately a separate frame.
    assert!(frame(&mut body).await.contains("event: cursor"));
    assert!(frame(&mut body).await.contains("event: message.delta"));
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE api_tokens SET revoked=1 WHERE id='token'", [])
        .unwrap();
    f.append(message, 2, " never delivered").unwrap();
    let next = frame(&mut body).await;
    let denied = if next.contains("event: cursor") {
        frame(&mut body).await
    } else {
        next
    };
    assert!(denied.contains("event: error"));
    assert!(!denied.contains("never delivered"));
}

#[test]
fn abandoned_streams_expire_and_free_the_active_stream_budget() {
    let f = Fixture::new();
    let mut ids = Vec::new();
    for index in 0..4 {
        ids.push(id(&f.start(&format!("idle-{index}"), "partial")));
    }
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE chat_messages SET stream_updated_at=?1 WHERE status='streaming'",
            [hireferry::now() - chat_stream::STREAM_IDLE_SECONDS - 1],
        )
        .unwrap();
    let fresh = f.start("fresh", "active");
    assert_eq!(fresh["status"], "streaming");
    let history = chat::messages(&f.state, &f.user, &f.room, -1, 100).unwrap();
    for old in &history["items"].as_array().unwrap()[..4] {
        assert_eq!(old["status"], "cancelled");
        assert_eq!(old["sequence"], 1);
    }
    assert!(f.append(ids[0], 1, "too late").is_err());
    let events = chat_stream::events(&f.state, &f.user, Some(&f.room), 0, 100).unwrap();
    assert_eq!(
        events["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == "message.cancelled")
            .count(),
        4
    );
}

#[tokio::test]
async fn sse_last_event_id_overrides_original_url_and_invalid_ids_fail_before_stream() {
    let f = Fixture::new();
    let message = id(&f.start("resume", "first"));
    f.append(message, 1, " second").unwrap();
    let mut request = f.req("GET", "/api/chat/events/stream?after=0", Body::empty());
    request
        .headers_mut()
        .insert("last-event-id", "1".parse().unwrap());
    let response = hireferry::app(f.state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let mut body = response.into_body();
    let replay = frame(&mut body).await;
    assert!(replay.contains("event: message.delta"));
    assert!(!replay.contains("event: message.started"));
    let mut request = f.req("GET", "/api/chat/events/stream", Body::empty());
    request
        .headers_mut()
        .insert("last-event-id", "invalid".parse().unwrap());
    assert_eq!(
        hireferry::app(f.state)
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn ndjson_accepts_split_utf8_and_flushes_ack_before_upload_eof() {
    let f = Fixture::new();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, Infallible>>(2);
    let stream = futures_util::stream::unfold(rx, |mut rx| async {
        rx.recv().await.map(|chunk| (chunk, rx))
    });
    let mut request = f.req(
        "POST",
        "/api/chat/conversations/platform_seeker/stream",
        Body::from_stream(stream),
    );
    request
        .headers_mut()
        .insert("content-type", "application/x-ndjson".parse().unwrap());
    let response = hireferry::app(f.state.clone())
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut response = response.into_body();
    let first = format!(
        "{}\n",
        json!({"type":"start","client_id":"utf8","body":"你好","format":"markdown"})
    )
    .into_bytes();
    let split = first.iter().position(|b| *b >= 128).unwrap() + 1;
    tx.send(Ok(Bytes::copy_from_slice(&first[..split])))
        .await
        .unwrap();
    tx.send(Ok(Bytes::copy_from_slice(&first[split..])))
        .await
        .unwrap();
    let ack: Value = serde_json::from_str(&frame(&mut response).await).unwrap();
    assert_eq!(ack["status"], "streaming");
    let message = ack["message_id"].as_i64().unwrap();
    assert_eq!(
        chat::messages(&f.state, &f.user, &f.room, -1, 10).unwrap()["items"][0]["body"],
        "你好"
    );
    tx.send(Ok(Bytes::from(format!(
        "{}\r\n",
        json!({"type":"delta","sequence":1,"delta":"，世界"})
    ))))
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&frame(&mut response).await).unwrap()["sequence"],
        1
    );
    tx.send(Ok(Bytes::from(format!(
        "{}\n",
        json!({"type":"finish","sequence":2})
    ))))
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&frame(&mut response).await).unwrap()["status"],
        "complete"
    );
    drop(tx);
    assert!(response.frame().await.is_none());
    assert_eq!(
        chat::messages(&f.state, &f.user, &f.room, -1, 10).unwrap()["items"][0]["id"],
        message
    );
}

#[tokio::test]
async fn interrupted_upload_is_resumable_and_revocation_blocks_later_frames() {
    let f = Fixture::new();
    let start = format!(
        "{}\n",
        json!({"type":"start","client_id":"interrupted","body":"partial"})
    );
    let mut request = f.req(
        "POST",
        "/api/chat/conversations/platform_seeker/stream",
        Body::from(start),
    );
    request
        .headers_mut()
        .insert("content-type", "application/x-ndjson".parse().unwrap());
    let response = hireferry::app(f.state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let mut body = response.into_body();
    let ack: Value = serde_json::from_str(&frame(&mut body).await).unwrap();
    let message = ack["message_id"].as_i64().unwrap();
    let failure: Value = serde_json::from_str(&frame(&mut body).await).unwrap();
    assert_eq!(failure["resumable"], true);
    f.append(message, 1, " resumed").unwrap();
    assert_eq!(
        chat_stream::finish(
            &f.state,
            &f.user,
            &f.room,
            message,
            StreamEndInput { sequence: 2 }
        )
        .unwrap()["status"],
        "complete"
    );
    assert_eq!(
        chat::messages(&f.state, &f.user, &f.room, -1, 10).unwrap()["items"][0]["body"],
        "partial resumed"
    );
    let chunks = vec![
        Ok::<_, Infallible>(Bytes::from(format!(
            "{}\n",
            json!({"type":"start","client_id":"revoked","body":"initial"})
        ))),
        Ok(Bytes::from(format!(
            "{}\n",
            json!({"type":"delta","sequence":1,"delta":"unauthorized"})
        ))),
    ];
    let mut request = f.req(
        "POST",
        "/api/chat/conversations/platform_seeker/stream",
        Body::from_stream(futures_util::stream::iter(chunks)),
    );
    request
        .headers_mut()
        .insert("content-type", "application/x-ndjson".parse().unwrap());
    let response = hireferry::app(f.state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let mut body = response.into_body();
    let ack: Value = serde_json::from_str(&frame(&mut body).await).unwrap();
    assert_eq!(ack["status"], "streaming");
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE api_tokens SET revoked=1 WHERE id='token'", [])
        .unwrap();
    let failure: Value = serde_json::from_str(&frame(&mut body).await).unwrap();
    assert_eq!(failure["type"], "error");
    assert_eq!(failure["status"], 401);
    let messages = chat::messages(&f.state, &f.user, &f.room, -1, 10).unwrap();
    assert!(!messages.to_string().contains("unauthorized"));
}

#[tokio::test]
async fn revoking_a_match_stops_live_sse_and_rejects_later_ndjson_frames() {
    let f = Fixture::new();
    let room = f.matched_room();
    let app = hireferry::app(f.state.clone());
    let events = app
        .clone()
        .oneshot(f.req(
            "GET",
            "/api/chat/events/stream?conversation_id=match_intro&after=0",
            Body::empty(),
        ))
        .await
        .unwrap();
    let mut events = events.into_body();
    assert!(frame(&mut events).await.contains("event: cursor"));
    let chunks = vec![
        Ok::<_, Infallible>(Bytes::from(format!(
            "{}\n",
            json!({"type":"start","client_id":"match-live","body":"visible before revocation"})
        ))),
        Ok(Bytes::from(format!(
            "{}\n",
            json!({"type":"delta","sequence":1,"delta":"forbidden after revocation"})
        ))),
    ];
    let mut upload = f.req(
        "POST",
        "/api/chat/conversations/match_intro/stream",
        Body::from_stream(futures_util::stream::iter(chunks)),
    );
    upload
        .headers_mut()
        .insert("content-type", "application/x-ndjson".parse().unwrap());
    let mut upload = app.oneshot(upload).await.unwrap().into_body();
    let ack: Value = serde_json::from_str(&frame(&mut upload).await).unwrap();
    assert_eq!(ack["status"], "streaming");
    assert!(frame(&mut events).await.contains("event: message.started"));
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE matches SET status='declined' WHERE id='intro'", [])
        .unwrap();
    f.state.chat_notify.notify_waiters();
    let failed: Value = serde_json::from_str(&frame(&mut upload).await).unwrap();
    assert_eq!(failed["type"], "error");
    assert_eq!(failed["status"], 404);
    let next = frame(&mut events).await;
    let denied = if next.contains("event: cursor") {
        frame(&mut events).await
    } else {
        next
    };
    assert!(denied.contains("event: error"));
    assert!(chat_stream::events(&f.state, &f.user, Some(&room), 0, 100).is_err());
    let all = chat_stream::events(&f.state, &f.user, None, 0, 100).unwrap();
    assert_eq!(all["items"], json!([]));
    let body: String = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT body FROM chat_messages WHERE id=?1",
            [ack["message_id"].as_i64().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(body, "visible before revocation");
}

#[test]
fn schema_14_upgrade_preserves_history_and_rolls_back_on_failure() {
    let f = Fixture::new();
    chat::send_message(
        &f.state,
        &f.user,
        &f.room,
        MessageInput {
            body: "legacy history".into(),
            client_id: "legacy".into(),
        },
    )
    .unwrap();
    let db = f.state.conn().unwrap();
    // Restore the previous real chat schema, retaining a historical row.
    db.execute_batch(
        "DROP TRIGGER email_message_insert; DROP TRIGGER email_message_complete;
        DROP TABLE chat_stream_chunks; DROP TABLE chat_events; DROP TABLE chat_event_retention;
        DROP INDEX chat_stream_expiry; DROP INDEX chat_stream_owners;
        ALTER TABLE chat_messages DROP COLUMN format;
        ALTER TABLE chat_messages DROP COLUMN status;
        ALTER TABLE chat_messages DROP COLUMN sequence;
        ALTER TABLE chat_messages DROP COLUMN stream_updated_at;
        PRAGMA user_version=13;
        CREATE TABLE chat_events(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    assert!(hireferry::db::migrate(&db).is_err());
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 13);
    let added:i64 = db.query_row("SELECT count(*) FROM pragma_table_info('chat_messages') WHERE name IN ('format','status','sequence','stream_updated_at')",[],|r|r.get(0)).unwrap();
    assert_eq!(
        added, 0,
        "failed migration must roll back every added column"
    );
    db.execute_batch("DROP TABLE chat_events;").unwrap();
    hireferry::db::migrate(&db).unwrap();
    hireferry::db::migrate(&db).unwrap();
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 16);
    let row: (String, String, String, i64) = db
        .query_row(
            "SELECT body,format,status,sequence FROM chat_messages",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "legacy history".into(),
            "markdown".into(),
            "complete".into(),
            0
        )
    );
}
