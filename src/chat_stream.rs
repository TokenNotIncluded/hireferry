//! Durable, resumable message streams shared by HTTP, MCP and the browser.
//! Event IDs are independent of message IDs. A reconnect can replay deltas;
//! a pruned cursor explicitly requests a fresh message snapshot.
use std::{convert::Infallible, time::Duration};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures_util::StreamExt;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    AppError, AppResult, AppState,
    access::{self, Principal},
    chat,
    db::audit,
    now,
};

pub const MAX_MESSAGE_CHARS: usize = 10_000;
pub const MAX_DELTA_CHARS: usize = 4_000;
pub const MAX_STREAM_CHUNKS: i64 = 4096;
pub const MAX_ACTIVE_STREAMS: i64 = 4;
pub const EVENT_RETENTION: i64 = 10_000;
pub const STREAM_IDLE_SECONDS: i64 = 900;
const HEARTBEAT: Duration = Duration::from_secs(15);
const MAX_LINE_BYTES: usize = 65_536;
const MAX_UPLOAD_BYTES: usize = 1_048_576;

pub fn default_format() -> String {
    "markdown".into()
}

pub(crate) fn validate_format(format: &str) -> AppResult<()> {
    if !matches!(format, "plain" | "markdown" | "openui") {
        return Err(AppError::bad_request(
            "Format must be plain, markdown or openui",
        ));
    }
    Ok(())
}

pub(crate) fn migrate(db: &Connection) -> AppResult<()> {
    let columns = db
        .prepare("PRAGMA table_info(chat_messages)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (name, declaration) in [
        (
            "format",
            "TEXT NOT NULL DEFAULT 'markdown' CHECK(format IN ('plain','markdown','openui'))",
        ),
        (
            "status",
            "TEXT NOT NULL DEFAULT 'complete' CHECK(status IN ('streaming','complete','cancelled'))",
        ),
        ("sequence", "INTEGER NOT NULL DEFAULT 0 CHECK(sequence>=0)"),
        ("stream_updated_at", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !columns.iter().any(|column| column == name) {
            db.execute_batch(&format!(
                "ALTER TABLE chat_messages ADD COLUMN {name} {declaration};"
            ))?;
        }
    }
    db.execute_batch(
        r#"
      CREATE TABLE IF NOT EXISTS chat_events(
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        conversation_id TEXT NOT NULL REFERENCES chat_conversations(id),
        message_id INTEGER NOT NULL REFERENCES chat_messages(id),
        event_type TEXT NOT NULL,
        sequence INTEGER NOT NULL,
        body TEXT, delta TEXT, created_at INTEGER NOT NULL);
      CREATE INDEX IF NOT EXISTS chat_events_conversation ON chat_events(conversation_id,id);
      CREATE TABLE IF NOT EXISTS chat_event_retention(
        id INTEGER PRIMARY KEY CHECK(id=1), pruned_through INTEGER NOT NULL DEFAULT 0);
      INSERT OR IGNORE INTO chat_event_retention(id) VALUES(1);
      CREATE TABLE IF NOT EXISTS chat_stream_chunks(
        message_id INTEGER NOT NULL REFERENCES chat_messages(id),
        sequence INTEGER NOT NULL, delta TEXT NOT NULL,
        PRIMARY KEY(message_id,sequence));
      CREATE INDEX IF NOT EXISTS chat_stream_owners ON chat_messages(sender_actor,status);
      CREATE INDEX IF NOT EXISTS chat_stream_expiry ON chat_messages(status,stream_updated_at);
    "#,
    )?;
    Ok(())
}

/// High water is captured alongside REST snapshots, before releasing the DB guard.
pub(crate) fn cursor(db: &Connection) -> AppResult<i64> {
    Ok(db.query_row("SELECT MAX(COALESCE((SELECT MAX(id) FROM chat_events),0),pruned_through) FROM chat_event_retention WHERE id=1", [], |r| r.get(0))?)
}

pub(crate) fn record_event(
    db: &Connection,
    conversation: &str,
    message: i64,
    kind: &str,
    sequence: i64,
    body: Option<&str>,
    delta: Option<&str>,
) -> AppResult<()> {
    db.execute("INSERT INTO chat_events(conversation_id,message_id,event_type,sequence,body,delta,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![conversation,message,kind,sequence,body,delta,now()])?;
    let latest = db.last_insert_rowid();
    let cutoff = latest.saturating_sub(EVENT_RETENTION);
    if cutoff > 0 {
        db.execute("DELETE FROM chat_events WHERE id<=?1", [cutoff])?;
        db.execute(
            "UPDATE chat_event_retention SET pruned_through=MAX(pruned_through,?1) WHERE id=1",
            [cutoff],
        )?;
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamBeginInput {
    pub body: String,
    pub client_id: String,
    #[serde(default = "default_format")]
    pub format: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamAppendInput {
    pub sequence: i64,
    pub delta: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamEndInput {
    pub sequence: i64,
}

fn client_id(value: &str) -> AppResult<()> {
    chat::text(value, 80, "Message client ID")?;
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(AppError::bad_request("A message client ID is required"));
    }
    Ok(())
}

fn write_room(db: &Connection, p: &Principal, conversation: &str) -> AppResult<()> {
    chat::ensure_thread(db, p, conversation)?;
    if !chat::writable(db, conversation)? {
        return Err(AppError::forbidden(
            "This conversation is archived and read-only",
        ));
    }
    Ok(())
}

/// Expire a bounded batch without depending on the original sender's credentials.
/// Called from message reads, event reads and stream operations. A disconnected
/// sender cannot reserve an active stream slot forever.
pub(crate) fn expire_inactive(db: &Connection) -> AppResult<bool> {
    let stale = db.prepare("SELECT id,conversation_id,body,sequence FROM chat_messages WHERE status='streaming' AND stream_updated_at<=?1 ORDER BY stream_updated_at LIMIT 100")?
        .query_map([now()-STREAM_IDLE_SECONDS], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (message, conversation, body, sequence) in &stale {
        db.execute(
            "UPDATE chat_messages SET status='cancelled',sequence=sequence+1 WHERE id=?1",
            [message],
        )?;
        record_event(
            db,
            conversation,
            *message,
            "message.cancelled",
            sequence + 1,
            Some(body),
            None,
        )?;
        db.execute(
            "UPDATE chat_conversations SET updated_at=?2 WHERE id=?1",
            params![conversation, now()],
        )?;
        audit(
            db,
            "system",
            "chat_stream_expired",
            conversation,
            &json!({"message_id":message}).to_string(),
        )?;
    }
    Ok(!stale.is_empty())
}

fn receipt(db: &Connection, conversation: &str, message: i64) -> AppResult<Value> {
    let (sequence, status): (i64, String) = db.query_row(
        "SELECT sequence,status FROM chat_messages WHERE id=?1",
        [message],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(
        json!({"id":message,"conversation_id":conversation,"sequence":sequence,"status":status,"event_cursor":cursor(db)?}),
    )
}

fn owned_stream(
    db: &Connection,
    p: &Principal,
    conversation: &str,
    message: i64,
) -> AppResult<(String, String, i64)> {
    write_room(db, p, conversation)?;
    expire_inactive(db)?;
    db.query_row("SELECT body,status,sequence FROM chat_messages WHERE id=?1 AND conversation_id=?2 AND sender_key=?3 AND sender_actor=?4 AND EXISTS(SELECT 1 FROM chat_stream_chunks WHERE message_id=?1 AND sequence=0)", params![message,conversation,chat::user_key(p)?,p.actor], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
        .optional()?.ok_or_else(|| AppError::not_found("Message stream not found"))
}

pub fn begin(
    s: &AppState,
    p: &Principal,
    conversation: &str,
    input: StreamBeginInput,
) -> AppResult<Value> {
    validate_format(&input.format)?;
    chat::text(&input.body, MAX_MESSAGE_CHARS, "Message")?;
    client_id(&input.client_id)?;
    if input.body.is_empty() {
        return Err(AppError::bad_request(
            "Start with the first nonempty text chunk",
        ));
    }
    let key = chat::user_key(p)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    write_room(&tx, p, conversation)?;
    expire_inactive(&tx)?;
    let previous: Option<(i64, String, String, Option<String>)> = tx.query_row(
        "SELECT msg.id,msg.format,msg.sender_actor,chunk.delta FROM chat_messages msg LEFT JOIN chat_stream_chunks chunk ON chunk.message_id=msg.id AND chunk.sequence=0 WHERE msg.conversation_id=?1 AND msg.sender_key=?2 AND msg.client_id=?3",
        params![conversation,key,input.client_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let message = if let Some((message, format, actor, first)) = previous {
        if format != input.format
            || actor != p.actor
            || first.as_deref() != Some(input.body.as_str())
        {
            return Err(AppError::conflict(
                "This client ID already belongs to another message",
            ));
        }
        message
    } else {
        let active: i64 = tx.query_row(
            "SELECT count(*) FROM chat_messages WHERE sender_actor=?1 AND status='streaming' AND stream_updated_at>?2",
            params![p.actor, now()-STREAM_IDLE_SECONDS],
            |r| r.get(0),
        )?;
        if active >= MAX_ACTIVE_STREAMS {
            return Err(AppError::conflict(
                "Finish or cancel an existing message stream before starting another",
            ));
        }
        let kind = if p.admin { "platform" } else { "user" };
        let sender = if p.admin { None } else { p.user_id.as_deref() };
        tx.execute("INSERT INTO chat_messages(conversation_id,sender_kind,sender_id,sender_key,sender_actor,body,client_id,via_ai,created_at,format,status,stream_updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'streaming',?9)", params![conversation,kind,sender,key,p.actor,input.body,input.client_id,p.via_ai,now(),input.format])?;
        let message = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO chat_stream_chunks(message_id,sequence,delta) VALUES(?1,0,?2)",
            params![message, input.body],
        )?;
        record_event(
            &tx,
            conversation,
            message,
            "message.started",
            0,
            Some(&input.body),
            None,
        )?;
        tx.execute(
            "UPDATE chat_conversations SET updated_at=?2 WHERE id=?1",
            params![conversation, now()],
        )?;
        audit(
            &tx,
            &p.actor,
            "chat_stream_begin",
            conversation,
            &json!({"message_id":message,"format":input.format,"via_ai":p.via_ai}).to_string(),
        )?;
        message
    };
    let mut result = chat::load_message(&tx, p, message)?;
    result["event_cursor"] = json!(cursor(&tx)?);
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    drop(db);
    s.chat_notify.notify_waiters();
    Ok(result)
}

pub fn append(
    s: &AppState,
    p: &Principal,
    conversation: &str,
    message: i64,
    input: StreamAppendInput,
) -> AppResult<Value> {
    chat::text(&input.delta, MAX_DELTA_CHARS, "Message delta")?;
    if input.delta.is_empty() || !(1..=MAX_STREAM_CHUNKS).contains(&input.sequence) {
        return Err(AppError::bad_request(
            "A nonempty delta and sequence in 1..4096 are required",
        ));
    }
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (body, status, sequence) = owned_stream(&tx, p, conversation, message)?;
    let prior: Option<String> = tx
        .query_row(
            "SELECT delta FROM chat_stream_chunks WHERE message_id=?1 AND sequence=?2",
            params![message, input.sequence],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(prior) = prior {
        if prior != input.delta {
            return Err(AppError::conflict(
                "This sequence already contains a different delta",
            ));
        }
    } else {
        if status != "streaming" || input.sequence != sequence + 1 {
            return Err(AppError::conflict(
                "Message stream is closed or the sequence is out of order",
            ));
        }
        if body.chars().count() + input.delta.chars().count() > MAX_MESSAGE_CHARS {
            return Err(AppError::bad_request("Message exceeds 10000 characters"));
        }
        tx.execute(
            "UPDATE chat_messages SET body=body||?2,sequence=?3,stream_updated_at=?4 WHERE id=?1",
            params![message, input.delta, input.sequence, now()],
        )?;
        tx.execute(
            "INSERT INTO chat_stream_chunks(message_id,sequence,delta) VALUES(?1,?2,?3)",
            params![message, input.sequence, input.delta],
        )?;
        record_event(
            &tx,
            conversation,
            message,
            "message.delta",
            input.sequence,
            None,
            Some(&input.delta),
        )?;
        tx.execute(
            "UPDATE chat_conversations SET updated_at=?2 WHERE id=?1",
            params![conversation, now()],
        )?;
    }
    let result = receipt(&tx, conversation, message)?;
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    drop(db);
    s.chat_notify.notify_waiters();
    Ok(result)
}

pub fn finish(
    s: &AppState,
    p: &Principal,
    conversation: &str,
    message: i64,
    input: StreamEndInput,
) -> AppResult<Value> {
    end(
        s,
        p,
        conversation,
        message,
        input,
        "complete",
        "message.completed",
    )
}
pub fn cancel(
    s: &AppState,
    p: &Principal,
    conversation: &str,
    message: i64,
    input: StreamEndInput,
) -> AppResult<Value> {
    end(
        s,
        p,
        conversation,
        message,
        input,
        "cancelled",
        "message.cancelled",
    )
}
fn end(
    s: &AppState,
    p: &Principal,
    conversation: &str,
    message: i64,
    input: StreamEndInput,
    target: &str,
    event: &str,
) -> AppResult<Value> {
    if !(1..=MAX_STREAM_CHUNKS + 1).contains(&input.sequence) {
        return Err(AppError::bad_request("Invalid message sequence"));
    }
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (body, status, sequence) = owned_stream(&tx, p, conversation, message)?;
    if status != target || input.sequence != sequence {
        if status != "streaming" || input.sequence != sequence + 1 {
            return Err(AppError::conflict(
                "Message stream is closed or the sequence is out of order",
            ));
        }
        if target == "complete" && body.trim().is_empty() {
            return Err(AppError::bad_request(
                "A completed message must contain text",
            ));
        }
        tx.execute(
            "UPDATE chat_messages SET status=?2,sequence=?3 WHERE id=?1",
            params![message, target, input.sequence],
        )?;
        record_event(
            &tx,
            conversation,
            message,
            event,
            input.sequence,
            Some(&body),
            None,
        )?;
        tx.execute(
            "UPDATE chat_conversations SET updated_at=?2 WHERE id=?1",
            params![conversation, now()],
        )?;
        audit(
            &tx,
            &p.actor,
            event,
            conversation,
            &json!({"message_id":message,"sequence":input.sequence}).to_string(),
        )?;
    }
    let result = receipt(&tx, conversation, message)?;
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    drop(db);
    s.chat_notify.notify_waiters();
    Ok(result)
}

pub fn events(
    s: &AppState,
    p: &Principal,
    conversation: Option<&str>,
    after: i64,
    count: i64,
) -> AppResult<Value> {
    if after < 0 {
        return Err(AppError::bad_request("Invalid event cursor"));
    }
    let count = chat::limit(count)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_active(&tx, p)?;
    if let Some(conversation) = conversation {
        chat::ensure_thread(&tx, p, conversation)?;
    }
    let expired = expire_inactive(&tx)?;
    let latest = cursor(&tx)?;
    let pruned: i64 = tx.query_row(
        "SELECT pruned_through FROM chat_event_retention WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    if after > latest {
        return Err(AppError::bad_request("Event cursor is ahead of the server"));
    }
    if after < pruned {
        access::ensure_active(&tx, p)?;
        tx.commit()?;
        drop(db);
        if expired {
            s.chat_notify.notify_waiters();
        }
        return Ok(
            json!({"items":[],"next_cursor":latest,"has_more":false,"resync_required":true}),
        );
    }
    let sql = format!(
        "SELECT e.id,e.message_id,e.conversation_id,e.event_type,e.sequence,e.body,e.delta,e.created_at {} JOIN chat_events e ON e.conversation_id=c.id WHERE {} AND e.id>?3 AND (?5 IS NULL OR c.id=?5) ORDER BY e.id LIMIT ?4",
        chat::THREAD_FROM,
        chat::VISIBLE
    );
    let mut items = tx.prepare(&sql)?.query_map(params![p.admin,p.user_id.as_deref().unwrap_or(""),after,count+1,conversation], |r| {
        Ok(json!({"id":r.get::<_,i64>(0)?,"message_id":r.get::<_,i64>(1)?,"conversation_id":r.get::<_,String>(2)?,"type":r.get::<_,String>(3)?,"sequence":r.get::<_,i64>(4)?,"body":r.get::<_,Option<String>>(5)?,"delta":r.get::<_,Option<String>>(6)?,"created_at":r.get::<_,i64>(7)?}))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    let more = items.len() > count as usize;
    items.truncate(count as usize);
    for item in &mut items {
        if let Some(body) = item["body"].as_str() {
            let mut message = chat::load_message(&tx, p, item["message_id"].as_i64().unwrap())?;
            message["body"] = json!(body);
            message["sequence"] = item["sequence"].clone();
            message["status"] = json!(match item["type"].as_str().unwrap_or("") {
                "message.started" => "streaming",
                "message.cancelled" => "cancelled",
                _ => "complete",
            });
            item["message"] = chat::render_message(message);
        }
        item.as_object_mut().unwrap().remove("body");
        if item["delta"].is_null() {
            item.as_object_mut().unwrap().remove("delta");
        }
    }
    let next = if more {
        items.last().and_then(|i| i["id"].as_i64()).unwrap_or(after)
    } else {
        latest
    };
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    drop(db);
    if expired {
        s.chat_notify.notify_waiters();
    }
    Ok(json!({"items":items,"next_cursor":next,"has_more":more,"resync_required":false}))
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct EventQuery {
    conversation_id: Option<String>,
    after: Option<i64>,
    limit: Option<i64>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/chat/conversations/{id}/streams", post(http_begin))
        .route(
            "/api/chat/conversations/{id}/streams/{message}/append",
            post(http_append),
        )
        .route(
            "/api/chat/conversations/{id}/streams/{message}/finish",
            post(http_finish),
        )
        .route(
            "/api/chat/conversations/{id}/streams/{message}/cancel",
            post(http_cancel),
        )
        .route("/api/chat/conversations/{id}/stream", post(http_upload))
        .route("/api/chat/events", get(http_events))
        .route("/api/chat/events/stream", get(http_stream))
}

async fn http_begin(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(i): Json<StreamBeginInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(begin(&s, &p, &id, i)?))
}
async fn http_append(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, message)): Path<(String, i64)>,
    Json(i): Json<StreamAppendInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(append(&s, &p, &id, message, i)?))
}
async fn http_finish(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, message)): Path<(String, i64)>,
    Json(i): Json<StreamEndInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(finish(&s, &p, &id, message, i)?))
}
async fn http_cancel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, message)): Path<(String, i64)>,
    Json(i): Json<StreamEndInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(cancel(&s, &p, &id, message, i)?))
}
async fn http_events(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<EventQuery>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(events(
        &s,
        &p,
        q.conversation_id.as_deref(),
        q.after.unwrap_or(0),
        q.limit.unwrap_or(100),
    )?))
}

fn event_cursor(h: &HeaderMap, query: Option<i64>) -> AppResult<Option<i64>> {
    let header = h
        .get("last-event-id")
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.parse::<i64>().ok())
                .filter(|value| *value >= 0)
                .ok_or_else(|| AppError::bad_request("Invalid Last-Event-ID"))
        })
        .transpose()?;
    if query.is_some_and(|value| value < 0) {
        return Err(AppError::bad_request("Invalid event cursor"));
    }
    // EventSource keeps the URL's initial cursor across reconnects. Its last
    // successfully delivered event header must take precedence over that URL.
    Ok(header.or(query))
}

fn sse_event(kind: &str, value: &Value, id: Option<i64>) -> Event {
    let mut event = Event::default().event(kind).data(value.to_string());
    if let Some(id) = id {
        event = event.id(id.to_string());
    }
    event
}

async fn http_stream(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<EventQuery>,
) -> AppResult<Response> {
    let p = access::principal(&s, &h, false)?;
    let after = event_cursor(&h, q.after)?.unwrap_or(cursor(&*s.conn()?)?);
    // Authenticate and validate membership/cursor before committing HTTP 200.
    events(
        &s,
        &p,
        q.conversation_id.as_deref(),
        after,
        q.limit.unwrap_or(100),
    )?;
    let output = async_stream::stream! {
        let mut after = after;
        loop {
            // Register before querying, so a commit between the query and wait
            // cannot lose its notification. No database guard crosses await.
            let notified = s.chat_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let page = access::principal(&s, &h, false).and_then(|p| events(&s, &p, q.conversation_id.as_deref(), after, q.limit.unwrap_or(100)));
            let page = match page {
                Ok(page) => page,
                Err(error) => {
                    yield Ok::<_, Infallible>(sse_event("error", &json!({"error":error.message,"status":error.status.as_u16()}), None));
                    break;
                }
            };
            if page["resync_required"] == true {
                yield Ok(sse_event("resync", &json!({"next_cursor":page["next_cursor"],"reason":"cursor_expired"}), None));
                break;
            }
            let mut denied = false;
            for item in page["items"].as_array().unwrap() {
                // A slow reader must not keep receiving a previously buffered
                // batch after its session, token or room access is revoked.
                let allowed = access::principal(&s, &h, false).and_then(|p| {
                    let db = s.conn()?;
                    chat::ensure_thread(&db, &p, item["conversation_id"].as_str().unwrap())
                });
                if let Err(error) = allowed {
                    yield Ok(sse_event("error", &json!({"error":error.message,"status":error.status.as_u16()}), None));
                    denied = true;
                    break;
                }
                after = item["id"].as_i64().unwrap();
                yield Ok(sse_event(item["type"].as_str().unwrap(), item, Some(after)));
            }
            if denied { break; }
            after = page["next_cursor"].as_i64().unwrap();
            yield Ok(sse_event("cursor", &json!({"next_cursor":after}), Some(after)));
            if page["has_more"] == true { continue; }
            tokio::select! {
                _ = &mut notified => {},
                _ = tokio::time::sleep(HEARTBEAT) => {
                    // Next loop repeats full credential and membership checks.
                }
            }
        }
    };
    let mut response = Sse::new(output).into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", "no".parse().unwrap());
    response
        .headers_mut()
        .insert("cache-control", "no-store, no-transform".parse().unwrap());
    Ok(response)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum UploadFrame {
    Start {
        body: String,
        client_id: String,
        #[serde(default = "default_format")]
        format: String,
    },
    Delta {
        sequence: i64,
        delta: String,
    },
    Finish {
        sequence: i64,
    },
    Cancel {
        sequence: i64,
    },
}

fn upload_frame(
    s: &AppState,
    h: &HeaderMap,
    conversation: &str,
    message: &mut Option<i64>,
    line: &[u8],
) -> AppResult<Value> {
    let p = access::principal(s, h, true)?;
    let frame: UploadFrame = serde_json::from_slice(line)?;
    let value = match (frame, *message) {
        (
            UploadFrame::Start {
                body,
                client_id,
                format,
            },
            None,
        ) => {
            let result = begin(
                s,
                &p,
                conversation,
                StreamBeginInput {
                    body,
                    client_id,
                    format,
                },
            )?;
            *message = result["id"].as_i64();
            result
        }
        (UploadFrame::Delta { sequence, delta }, Some(message)) => append(
            s,
            &p,
            conversation,
            message,
            StreamAppendInput { sequence, delta },
        )?,
        (UploadFrame::Finish { sequence }, Some(message)) => {
            finish(s, &p, conversation, message, StreamEndInput { sequence })?
        }
        (UploadFrame::Cancel { sequence }, Some(message)) => {
            cancel(s, &p, conversation, message, StreamEndInput { sequence })?
        }
        _ => {
            return Err(AppError::bad_request(
                "Send one start frame before deltas and a finish or cancel frame",
            ));
        }
    };
    Ok(
        json!({"type":"ack","message_id":value["id"],"sequence":value["sequence"],"status":value["status"]}),
    )
}
fn ndjson(value: Value) -> Bytes {
    Bytes::from(format!("{value}\n"))
}

async fn http_upload(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(conversation): Path<String>,
    body: Body,
) -> AppResult<Response> {
    let content_type = h
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if content_type != "application/x-ndjson" {
        return Err(AppError::bad_request(
            "Use Content-Type: application/x-ndjson",
        ));
    }
    let p = access::principal(&s, &h, true)?;
    write_room(&*s.conn()?, &p, &conversation)?;
    let output = async_stream::stream! {
        let mut input = body.into_data_stream();
        let mut pending = Vec::<u8>::new();
        let mut received = 0usize;
        let mut message = None;
        let started = tokio::time::Instant::now();
        let mut terminal = false;
        'upload: loop {
            if started.elapsed() > Duration::from_secs(900) {
                yield Ok::<_, Infallible>(ndjson(json!({"type":"error","error":"Upload duration exceeded","message_id":message,"resumable":true})));
                break;
            }
            let next = tokio::time::timeout(Duration::from_secs(30), input.next()).await;
            let eof = match next {
                Ok(Some(Ok(chunk))) => {
                    received = received.saturating_add(chunk.len());
                    if received > MAX_UPLOAD_BYTES {
                        yield Ok(ndjson(json!({"type":"error","error":"Upload exceeds 1 MiB","message_id":message,"resumable":true})));
                        break;
                    }
                    pending.extend_from_slice(&chunk);
                    false
                },
                Ok(None) => true,
                _ => {
                    yield Ok(ndjson(json!({"type":"error","error":"Upload interrupted or idle timeout","message_id":message,"resumable":true})));
                    break;
                }
            };
            loop {
                let boundary = pending.iter().position(|b| *b == b'\n');
                let end = match boundary { Some(end) => end, None if eof && !pending.is_empty() => pending.len(), _ => break };
                if end > MAX_LINE_BYTES {
                    yield Ok(ndjson(json!({"type":"error","error":"NDJSON line exceeds 65536 bytes","message_id":message,"resumable":true})));
                    break 'upload;
                }
                let mut line = pending.drain(..end).collect::<Vec<_>>();
                if boundary.is_some() { pending.remove(0); }
                if line.last() == Some(&b'\r') { line.pop(); }
                if line.iter().all(u8::is_ascii_whitespace) { continue; }
                if terminal {
                    yield Ok(ndjson(json!({"type":"error","error":"Frames after completion are not allowed","message_id":message})));
                    break 'upload;
                }
                match upload_frame(&s, &h, &conversation, &mut message, &line) {
                    Ok(ack) => {
                        terminal = ack["status"] != "streaming";
                        yield Ok(ndjson(ack));
                    },
                    Err(error) => {
                        yield Ok(ndjson(json!({"type":"error","error":error.message,"status":error.status.as_u16(),"message_id":message,"resumable":true})));
                        break 'upload;
                    }
                }
            }
            if pending.len() > MAX_LINE_BYTES {
                yield Ok(ndjson(json!({"type":"error","error":"NDJSON line exceeds 65536 bytes","message_id":message,"resumable":true})));
                break;
            }
            if eof {
                if !terminal {
                    yield Ok(ndjson(json!({"type":"error","error":"Missing finish or cancel frame","message_id":message,"resumable":message.is_some()})));
                }
                break;
            }
        }
    };
    Ok((
        [
            ("content-type", "application/x-ndjson"),
            ("cache-control", "no-store, no-transform"),
            ("x-accel-buffering", "no"),
        ],
        Body::from_stream(output),
    )
        .into_response())
}
