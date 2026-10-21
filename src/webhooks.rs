//! User-owned outbound HTTPS webhooks. Database triggers enqueue only resource
//! identifiers and statuses in the same transaction as the business operation.
//! Every attempt repeats authorization after DNS resolution and before sending.
use crate::{AppError, AppResult, AppState, auth, config::Webhooks, db::audit, now};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, patch, post},
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    future::Future,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    time::Duration,
};
use url::{Host, Url};

pub const EVENTS: [&str; 6] = [
    "message.created",
    "match.updated",
    "job.updated",
    "order.updated",
    "mandate.updated",
    "outreach.updated",
];

// Both enqueue-time and send-time checks use this exact predicate. A previously
// accessible private platform thread never becomes a matched thread's history.
fn visibility_sql(endpoint: &str, event: &str) -> String {
    format!(
        r#"(
 ({event}.event_type='webhook.test' AND {event}.resource_id={endpoint}.id)
 OR ({event}.event_type='order.updated' AND EXISTS(SELECT 1 FROM orders o WHERE o.id={event}.resource_id AND o.user_id={endpoint}.user_id))
 OR ({event}.event_type='mandate.updated' AND {event}.resource_id={endpoint}.user_id AND EXISTS(SELECT 1 FROM mandates m WHERE m.user_id={endpoint}.user_id))
 OR ({event}.event_type='outreach.updated' AND EXISTS(SELECT 1 FROM outreach o WHERE o.id={event}.resource_id AND o.user_id={endpoint}.user_id))
 OR ({event}.event_type='job.updated' AND EXISTS(SELECT 1 FROM jobs j JOIN users owner ON owner.id=j.owner_id WHERE j.id={event}.resource_id AND owner.banned=0 AND (j.owner_id={endpoint}.user_id OR (j.status<>'deleted' AND EXISTS(SELECT 1 FROM matches m JOIN users seeker ON seeker.id=m.seeker_id WHERE m.job_id=j.id AND m.seeker_id={endpoint}.user_id AND m.status IN ('introduced','interview','hired') AND seeker.banned=0)))))
 OR ({event}.event_type='match.updated' AND EXISTS(SELECT 1 FROM matches m JOIN jobs j ON j.id=m.job_id JOIN users sk ON sk.id=m.seeker_id JOIN users er ON er.id=j.owner_id WHERE m.id={event}.resource_id AND m.status IN ('introduced','interview','hired') AND j.status<>'deleted' AND sk.banned=0 AND er.banned=0 AND (m.seeker_id={endpoint}.user_id OR j.owner_id={endpoint}.user_id)))
 OR ({event}.event_type='message.created' AND EXISTS(SELECT 1 FROM chat_messages msg JOIN chat_conversations c ON c.id=msg.conversation_id LEFT JOIN matches m ON m.id=c.match_id LEFT JOIN jobs j ON j.id=m.job_id LEFT JOIN users sk ON sk.id=m.seeker_id LEFT JOIN users er ON er.id=j.owner_id WHERE CAST(msg.id AS TEXT)={event}.resource_id AND c.id={event}.scope_id AND ((c.kind='platform' AND c.user_id={endpoint}.user_id) OR (c.kind='matched' AND m.status IN ('introduced','interview','hired') AND j.status<>'deleted' AND sk.banned=0 AND er.banned=0 AND (m.seeker_id={endpoint}.user_id OR j.owner_id={endpoint}.user_id)))))
)"#
    )
}

pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(r#"
 CREATE TABLE IF NOT EXISTS webhook_runtime(
 id INTEGER PRIMARY KEY CHECK(id=1),enabled INTEGER NOT NULL DEFAULT 0,
 max_attempts INTEGER NOT NULL DEFAULT 5,max_pending INTEGER NOT NULL DEFAULT 1000);
 INSERT OR IGNORE INTO webhook_runtime(id) VALUES(1);
 CREATE TABLE IF NOT EXISTS webhook_endpoints(
 id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),name TEXT NOT NULL,url TEXT NOT NULL,
 events_json TEXT NOT NULL CHECK(json_valid(events_json)),signing_secret TEXT NOT NULL,
 enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)),
 dropped_events INTEGER NOT NULL DEFAULT 0,last_test_at INTEGER,last_retry_at INTEGER,
 created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
 CREATE INDEX IF NOT EXISTS webhook_endpoints_user ON webhook_endpoints(user_id,revoked);
 CREATE TABLE IF NOT EXISTS webhook_events(
 id TEXT PRIMARY KEY DEFAULT ('evt_'||lower(hex(randomblob(16)))),event_type TEXT NOT NULL,
 resource_id TEXT NOT NULL,scope_id TEXT NOT NULL DEFAULT '',data_json TEXT NOT NULL CHECK(json_valid(data_json)),
 body_json TEXT NOT NULL DEFAULT '{}',created_at INTEGER NOT NULL DEFAULT (unixepoch()));
 CREATE TABLE IF NOT EXISTS webhook_deliveries(
 id TEXT PRIMARY KEY DEFAULT ('whd_'||lower(hex(randomblob(16)))),endpoint_id TEXT NOT NULL REFERENCES webhook_endpoints(id),
 event_id TEXT NOT NULL REFERENCES webhook_events(id),status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','processing','delivered','failed','cancelled')),
 attempts INTEGER NOT NULL DEFAULT 0,max_attempts INTEGER NOT NULL DEFAULT 5,
 next_attempt_at INTEGER NOT NULL,last_attempt_at INTEGER,last_status_code INTEGER,last_error TEXT,
 claim_token TEXT,lease_until INTEGER,created_at INTEGER NOT NULL,delivered_at INTEGER,
 UNIQUE(endpoint_id,event_id));
 CREATE INDEX IF NOT EXISTS webhook_deliveries_due ON webhook_deliveries(status,next_attempt_at);
 CREATE INDEX IF NOT EXISTS webhook_deliveries_endpoint ON webhook_deliveries(endpoint_id,created_at);
 CREATE INDEX IF NOT EXISTS webhook_deliveries_retention ON webhook_deliveries(status,created_at);
 CREATE TRIGGER IF NOT EXISTS webhook_banned_user AFTER UPDATE OF banned ON users
 WHEN NEW.banned=1 AND OLD.banned<>1 BEGIN
 UPDATE webhook_endpoints SET enabled=0,updated_at=unixepoch() WHERE user_id=NEW.id;
 UPDATE webhook_deliveries SET status='cancelled',last_error='authorization_revoked',claim_token=NULL,lease_until=NULL
 WHERE status IN ('pending','processing') AND endpoint_id IN (SELECT id FROM webhook_endpoints WHERE user_id=NEW.id);
 END;
 "#)?;
    let visible = visibility_sql("e", "NEW");
    let eligible = format!(
        r#"e.enabled=1 AND e.revoked=0 AND EXISTS(SELECT 1 FROM users u WHERE u.id=e.user_id AND u.banned=0 AND u.role IN ('seeker','employer'))
 AND (NEW.event_type='webhook.test' OR EXISTS(SELECT 1 FROM json_each(e.events_json) WHERE value=NEW.event_type)) AND {visible}"#
    );
    db.execute_batch(&format!(r#"
 CREATE TRIGGER IF NOT EXISTS webhook_fanout AFTER INSERT ON webhook_events BEGIN
 UPDATE webhook_events SET body_json=json_object('id',NEW.id,'type',NEW.event_type,'created_at',NEW.created_at,'data',json(NEW.data_json)) WHERE id=NEW.id;
 UPDATE webhook_endpoints SET dropped_events=dropped_events+1 WHERE id IN(SELECT e.id FROM webhook_endpoints e WHERE {eligible}
 AND (SELECT count(*) FROM webhook_deliveries d WHERE d.endpoint_id=e.id AND d.status IN ('pending','processing')) >= (SELECT max_pending FROM webhook_runtime WHERE id=1));
 INSERT INTO webhook_deliveries(endpoint_id,event_id,max_attempts,next_attempt_at,created_at)
 SELECT e.id,NEW.id,(SELECT max_attempts FROM webhook_runtime WHERE id=1),NEW.created_at,NEW.created_at
 FROM webhook_endpoints e WHERE {eligible}
 AND (SELECT count(*) FROM webhook_deliveries d WHERE d.endpoint_id=e.id AND d.status IN ('pending','processing')) < (SELECT max_pending FROM webhook_runtime WHERE id=1);
 DELETE FROM webhook_events WHERE id=NEW.id AND NOT EXISTS(SELECT 1 FROM webhook_deliveries WHERE event_id=NEW.id);
 END;
 "#))?;
    capture(
        db,
        "chat_messages",
        "message.created",
        "CAST(NEW.id AS TEXT)",
        "NEW.conversation_id",
        "json_object('id',CAST(NEW.id AS TEXT),'conversation_id',NEW.conversation_id)",
        None,
    )?;
    capture(
        db,
        "matches",
        "match.updated",
        "NEW.id",
        "''",
        "json_object('id',NEW.id,'status',NEW.status)",
        Some("OLD.status IS NOT NEW.status OR OLD.note IS NOT NEW.note"),
    )?;
    // Schema v4 adds planned headcount. Replace the old v3 update predicate
    // when opening an existing database so free count changes also notify.
    db.execute_batch("DROP TRIGGER IF EXISTS webhook_capture_jobs_UPDATE;")?;
    capture(
        db,
        "jobs",
        "job.updated",
        "NEW.id",
        "''",
        "json_object('id',NEW.id,'status',NEW.status)",
        Some(
            "OLD.status IS NOT NEW.status OR OLD.version IS NOT NEW.version OR OLD.jd_md IS NOT NEW.jd_md OR OLD.headcount IS NOT NEW.headcount",
        ),
    )?;
    capture(
        db,
        "orders",
        "order.updated",
        "NEW.id",
        "''",
        "json_object('id',NEW.id,'status',NEW.status)",
        Some("OLD.status IS NOT NEW.status OR OLD.refunded_cents IS NOT NEW.refunded_cents"),
    )?;
    capture(
        db,
        "mandates",
        "mandate.updated",
        "NEW.user_id",
        "''",
        "json_object('id',NEW.user_id,'status',NEW.status)",
        Some("1"),
    )?;
    capture(
        db,
        "outreach",
        "outreach.updated",
        "NEW.id",
        "''",
        "json_object('id',NEW.id,'status',NEW.stage)",
        Some("1"),
    )?;
    Ok(())
}

fn capture(
    db: &Connection,
    table: &str,
    kind: &str,
    resource: &str,
    scope: &str,
    data: &str,
    update: Option<&str>,
) -> AppResult<()> {
    // All fragments are compile-time literals from migrate(), never user input.
    for (action, predicate) in [("INSERT", Some("1")), ("UPDATE", update)] {
        let Some(predicate) = predicate else {
            continue;
        };
        db.execute_batch(&format!(r#"CREATE TRIGGER IF NOT EXISTS webhook_capture_{table}_{action} AFTER {action} ON {table}
 WHEN ({predicate}) AND (SELECT enabled FROM webhook_runtime WHERE id=1)=1 AND EXISTS(SELECT 1 FROM webhook_endpoints WHERE enabled=1 AND revoked=0)
 BEGIN INSERT INTO webhook_events(event_type,resource_id,scope_id,data_json) VALUES('{kind}',{resource},{scope},{data}); END;"#))?;
    }
    Ok(())
}

pub fn configure(db: &Connection, limits: &Webhooks) -> AppResult<()> {
    db.execute(
        "UPDATE webhook_runtime SET enabled=?1,max_attempts=?2,max_pending=?3 WHERE id=1",
        params![
            limits.enabled,
            limits.max_attempts,
            limits.max_pending_per_endpoint
        ],
    )?;
    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/webhooks", get(list).post(create))
        .route("/api/webhooks/{id}", patch(update).delete(revoke))
        .route("/api/webhooks/{id}/deliveries", get(deliveries))
        .route("/api/webhooks/{id}/test", post(test_delivery))
        .route(
            "/api/webhooks/{id}/deliveries/{delivery_id}/retry",
            post(retry),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    pub name: String,
    pub url: String,
    pub events: Vec<String>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PatchInput {
    pub name: Option<String>,
    pub events: Option<Vec<String>>,
    pub enabled: Option<bool>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ListInput {
    limit: Option<i64>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

fn browser(s: &AppState, h: &HeaderMap, write: bool) -> AppResult<auth::Session> {
    if h.contains_key("authorization") || h.contains_key("x-hireferry-ai") {
        return Err(AppError::forbidden(
            "Manage webhook credentials in your signed-in browser",
        ));
    }
    let session = auth::require_session(s, h)?;
    if session.admin || session.bootstrap || session.user_id.is_none() {
        return Err(AppError::forbidden("A member account is required"));
    }
    if write {
        auth::csrf_check(s, h, &session)?;
    }
    Ok(session)
}
fn ensure_browser(db: &Connection, session: &auth::Session) -> AppResult<String> {
    db.query_row("SELECT u.id FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=?1 AND s.expires_at>?2 AND s.admin=0 AND s.bootstrap=0 AND u.banned=0 AND u.role IN ('seeker','employer')",params![session.token_hash,now()],|r|r.get(0)).optional()?.ok_or_else(||AppError::unauthorized("Choose an active account role and sign in again"))
}
fn enabled(s: &AppState) -> AppResult<()> {
    if !s.cfg.webhooks.enabled {
        return Err(AppError::unavailable(
            "Outbound webhooks are disabled by the administrator",
        ));
    }
    Ok(())
}
fn name(value: &str) -> AppResult<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 100 || value.chars().any(char::is_control) {
        return Err(AppError::bad_request(
            "Use a webhook name between 1 and 100 characters",
        ));
    }
    Ok(value.to_owned())
}
fn events(mut value: Vec<String>) -> AppResult<String> {
    if value.is_empty()
        || value.len() > EVENTS.len()
        || value.iter().any(|v| !EVENTS.contains(&v.as_str()))
    {
        return Err(AppError::bad_request(
            "Choose one or more supported webhook events",
        ));
    }
    value.sort();
    value.dedup();
    Ok(serde_json::to_string(&value)?)
}
fn endpoint_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let stored: String = r.get("events_json")?;
    Ok(
        json!({"id":r.get::<_,String>("id")?,"name":r.get::<_,String>("name")?,"url":r.get::<_,String>("url")?,"events":serde_json::from_str::<Value>(&stored).unwrap_or(json!([])),"enabled":r.get::<_,bool>("enabled")?,"dropped_events":r.get::<_,i64>("dropped_events")?,"created_at":r.get::<_,i64>("created_at")?,"updated_at":r.get::<_,i64>("updated_at")?}),
    )
}
fn owned(db: &Connection, uid: &str, id: &str) -> AppResult<Value> {
    db.query_row(
        "SELECT * FROM webhook_endpoints WHERE id=?1 AND user_id=?2 AND revoked=0",
        params![id, uid],
        endpoint_row,
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("Webhook not found"))
}
fn delivery_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,String>("id")?,"event_id":r.get::<_,String>("event_id")?,"event_type":r.get::<_,String>("event_type")?,"status":r.get::<_,String>("status")?,"attempts":r.get::<_,i64>("attempts")?,"max_attempts":r.get::<_,i64>("max_attempts")?,"next_attempt_at":r.get::<_,i64>("next_attempt_at")?,"last_attempt_at":r.get::<_,Option<i64>>("last_attempt_at")?,"last_status_code":r.get::<_,Option<i64>>("last_status_code")?,"last_error":r.get::<_,Option<String>>("last_error")?,"created_at":r.get::<_,i64>("created_at")?,"delivered_at":r.get::<_,Option<i64>>("delivered_at")?}),
    )
}
fn load_delivery(db: &Connection, id: &str, endpoint: &str) -> AppResult<Value> {
    db.query_row("SELECT d.*,w.event_type FROM webhook_deliveries d JOIN webhook_events w ON w.id=d.event_id WHERE d.id=?1 AND d.endpoint_id=?2",params![id,endpoint],delivery_row).optional()?.ok_or_else(||AppError::not_found("Delivery not found"))
}
async fn list(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let session = browser(&s, &h, false)?;
    let db = s.conn()?;
    let uid = ensure_browser(&db, &session)?;
    let items = db
        .prepare(
            "SELECT * FROM webhook_endpoints WHERE user_id=?1 AND revoked=0 ORDER BY created_at,id",
        )?
        .query_map([uid], endpoint_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Json(
        json!({"items":items,"enabled":s.cfg.webhooks.enabled,"limits":{"max_endpoints":s.cfg.webhooks.max_endpoints,"max_attempts":s.cfg.webhooks.max_attempts,"max_pending_per_endpoint":s.cfg.webhooks.max_pending_per_endpoint}}),
    ))
}
async fn create(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<CreateInput>,
) -> AppResult<Json<Value>> {
    let session = browser(&s, &h, true)?;
    enabled(&s)?;
    let name = name(&input.name)?;
    let url = validate_destination(&input.url)?.to_string();
    let events = events(input.events)?;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let secret = format!("hf_wh_{}", hex::encode(bytes));
    let id = uuid::Uuid::new_v4().to_string();
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let uid = ensure_browser(&tx, &session)?;
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM webhook_endpoints WHERE user_id=?1 AND revoked=0",
        [&uid],
        |r| r.get(0),
    )?;
    if count >= s.cfg.webhooks.max_endpoints {
        return Err(AppError::conflict("Webhook endpoint limit reached"));
    }
    tx.execute("INSERT INTO webhook_endpoints(id,user_id,name,url,events_json,signing_secret,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?7)",params![id,uid,name,url,events,secret,now()])?;
    audit(&tx, &uid, "webhook_create", &id, "")?;
    let endpoint = owned(&tx, &uid, &id)?;
    tx.commit()?;
    Ok(Json(json!({"endpoint":endpoint,"signing_secret":secret})))
}
async fn update(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<PatchInput>,
) -> AppResult<Json<Value>> {
    let session = browser(&s, &h, true)?;
    if input.enabled == Some(true) {
        enabled(&s)?;
    }
    let name = input.name.as_deref().map(name).transpose()?;
    let events = input.events.map(events).transpose()?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let uid = ensure_browser(&tx, &session)?;
    owned(&tx, &uid, &id)?;
    tx.execute("UPDATE webhook_endpoints SET name=COALESCE(?2,name),events_json=COALESCE(?3,events_json),enabled=COALESCE(?4,enabled),updated_at=?5 WHERE id=?1",params![id,name,events,input.enabled,now()])?;
    cancel_unavailable(&tx, Some(&id))?;
    audit(&tx, &uid, "webhook_update", &id, "")?;
    let endpoint = owned(&tx, &uid, &id)?;
    tx.commit()?;
    Ok(Json(json!({"endpoint":endpoint})))
}
async fn revoke(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let session = browser(&s, &h, true)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let uid = ensure_browser(&tx, &session)?;
    owned(&tx, &uid, &id)?;
    tx.execute("UPDATE webhook_endpoints SET enabled=0,revoked=1,signing_secret='',updated_at=?2 WHERE id=?1",params![id,now()])?;
    cancel_unavailable(&tx, Some(&id))?;
    audit(&tx, &uid, "webhook_revoke", &id, "")?;
    tx.commit()?;
    Ok(Json(json!({"revoked":true})))
}
async fn deliveries(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<ListInput>,
) -> AppResult<Json<Value>> {
    let session = browser(&s, &h, false)?;
    let db = s.conn()?;
    let uid = ensure_browser(&db, &session)?;
    owned(&db, &uid, &id)?;
    let count = q.limit.unwrap_or(50);
    if !(1..=100).contains(&count) {
        return Err(AppError::bad_request(
            "Delivery limit must be between 1 and 100",
        ));
    }
    let items=db.prepare("SELECT d.*,w.event_type FROM webhook_deliveries d JOIN webhook_events w ON w.id=d.event_id WHERE d.endpoint_id=?1 ORDER BY d.created_at DESC,d.id DESC LIMIT ?2")?.query_map(params![id,count],delivery_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Json(json!({"items":items})))
}
async fn test_delivery(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(_): Json<EmptyInput>,
) -> AppResult<Json<Value>> {
    let session = browser(&s, &h, true)?;
    enabled(&s)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let uid = ensure_browser(&tx, &session)?;
    let endpoint = owned(&tx, &uid, &id)?;
    if endpoint["enabled"] != true {
        return Err(AppError::conflict("Enable this webhook before testing"));
    }
    let last: Option<i64> = tx.query_row(
        "SELECT last_test_at FROM webhook_endpoints WHERE id=?1",
        [&id],
        |r| r.get(0),
    )?;
    if last.is_some_and(|t| now().saturating_sub(t) < 60) {
        return Err(AppError::conflict("Wait one minute between webhook tests"));
    }
    pending_room(&tx, &s.cfg.webhooks, &id)?;
    let event = format!("evt_{}", uuid::Uuid::new_v4().simple());
    tx.execute("INSERT INTO webhook_events(id,event_type,resource_id,data_json) VALUES(?1,'webhook.test',?2,'{\"test\":true}')",params![event,id])?;
    let delivery_id: String = tx.query_row(
        "SELECT id FROM webhook_deliveries WHERE event_id=?1 AND endpoint_id=?2",
        params![event, id],
        |r| r.get(0),
    )?;
    tx.execute(
        "UPDATE webhook_endpoints SET last_test_at=?2 WHERE id=?1",
        params![id, now()],
    )?;
    audit(&tx, &uid, "webhook_test", &id, "")?;
    let delivery = load_delivery(&tx, &delivery_id, &id)?;
    tx.commit()?;
    Ok(Json(json!({"delivery":delivery})))
}
async fn retry(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, delivery_id)): Path<(String, String)>,
    Json(_): Json<EmptyInput>,
) -> AppResult<Json<Value>> {
    let session = browser(&s, &h, true)?;
    enabled(&s)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let uid = ensure_browser(&tx, &session)?;
    owned(&tx, &uid, &id)?;
    let delivery = load_delivery(&tx, &delivery_id, &id)?;
    if delivery["status"] != "failed" {
        return Err(AppError::conflict("Only failed deliveries can be retried"));
    }
    if !authorized(&tx, &delivery_id)? {
        return Err(AppError::forbidden(
            "This event is no longer authorized for delivery",
        ));
    }
    let last: Option<i64> = tx.query_row(
        "SELECT last_retry_at FROM webhook_endpoints WHERE id=?1",
        [&id],
        |r| r.get(0),
    )?;
    if last.is_some_and(|t| now().saturating_sub(t) < 60) {
        return Err(AppError::conflict("Wait one minute between manual retries"));
    }
    pending_room(&tx, &s.cfg.webhooks, &id)?;
    tx.execute("UPDATE webhook_deliveries SET status='pending',attempts=0,max_attempts=?2,next_attempt_at=?3,claim_token=NULL,lease_until=NULL WHERE id=?1",params![delivery_id,s.cfg.webhooks.max_attempts,now()])?;
    tx.execute(
        "UPDATE webhook_endpoints SET last_retry_at=?2 WHERE id=?1",
        params![id, now()],
    )?;
    audit(&tx, &uid, "webhook_retry", &delivery_id, "")?;
    let delivery = load_delivery(&tx, &delivery_id, &id)?;
    tx.commit()?;
    Ok(Json(json!({"delivery":delivery})))
}
fn pending_room(db: &Connection, limits: &Webhooks, id: &str) -> AppResult<()> {
    let count:i64=db.query_row("SELECT count(*) FROM webhook_deliveries WHERE endpoint_id=?1 AND status IN ('pending','processing')",[id],|r|r.get(0))?;
    if count >= limits.max_pending_per_endpoint {
        return Err(AppError::conflict("Webhook delivery queue is full"));
    }
    Ok(())
}

fn eligibility_sql() -> String {
    format!(
        "e.enabled=1 AND e.revoked=0 AND u.banned=0 AND u.role IN ('seeker','employer') AND (w.event_type='webhook.test' OR EXISTS(SELECT 1 FROM json_each(e.events_json) WHERE value=w.event_type)) AND {}",
        visibility_sql("e", "w")
    )
}
fn authorized(db: &Connection, id: &str) -> AppResult<bool> {
    Ok(db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM webhook_deliveries d JOIN webhook_endpoints e ON e.id=d.endpoint_id JOIN users u ON u.id=e.user_id JOIN webhook_events w ON w.id=d.event_id WHERE d.id=?1 AND {})",eligibility_sql()),[id],|r|r.get(0))?)
}
fn cancel_unavailable(db: &Connection, endpoint: Option<&str>) -> AppResult<()> {
    db.execute(&format!("UPDATE webhook_deliveries SET status='cancelled',last_error='authorization_revoked',claim_token=NULL,lease_until=NULL WHERE id IN(SELECT d.id FROM webhook_deliveries d JOIN webhook_endpoints e ON e.id=d.endpoint_id JOIN users u ON u.id=e.user_id JOIN webhook_events w ON w.id=d.event_id WHERE d.status IN ('pending','processing') AND (?1 IS NULL OR d.endpoint_id=?1) AND NOT({}))",eligibility_sql()),[endpoint])?;
    Ok(())
}

/// Conservative public-address allowlist. IPv4 translation, private networks,
/// documentation ranges, link-local metadata and IPv6 transition ranges fail.
pub fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, d] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 168 && b == 63 && c == 129 && d == 16)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 192 && b == 0 && (c == 0 || c == 2))
                || (a == 192 && b == 88 && c == 99)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

/// Syntax and literal-address validation also runs on create. DNS is resolved
/// afresh for every delivery, and the verified answers are pinned to the client.
pub fn validate_destination(value: &str) -> AppResult<Url> {
    let invalid = || {
        AppError::bad_request(
            "Use a public HTTPS URL on port 443 without credentials or a fragment",
        )
    };
    if value.len() > 2048
        || value
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return Err(invalid());
    }
    let url = Url::parse(value).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    match url.host().ok_or_else(invalid)? {
        Host::Ipv4(ip) if !public_address(IpAddr::V4(ip)) => return Err(invalid()),
        Host::Ipv6(ip) if !public_address(IpAddr::V6(ip)) => return Err(invalid()),
        Host::Domain(host) => {
            let host = host.to_ascii_lowercase();
            if !host.contains('.')
                || host.ends_with('.')
                || [
                    "localhost",
                    "local",
                    "internal",
                    "home",
                    "lan",
                    "test",
                    "invalid",
                    "onion",
                ]
                .iter()
                .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
            {
                return Err(invalid());
            }
        }
        _ => {}
    }
    Ok(url)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryFailure {
    DestinationBlocked,
    DnsFailed,
    Timeout,
    Network,
    ResponseTooLarge,
    HttpStatus(u16),
}
impl DeliveryFailure {
    fn code(self) -> &'static str {
        match self {
            Self::DestinationBlocked => "destination_blocked",
            Self::DnsFailed => "dns_failed",
            Self::Timeout => "timeout",
            Self::Network => "network_error",
            Self::ResponseTooLarge => "response_too_large",
            Self::HttpStatus(_) => "http_error",
        }
    }
    fn retryable(self) -> bool {
        match self {
            Self::DestinationBlocked | Self::ResponseTooLarge => false,
            Self::HttpStatus(s) => s == 408 || s == 429 || s >= 500,
            _ => true,
        }
    }
}

pub fn validate_addresses(addresses: &[SocketAddr]) -> Result<(), DeliveryFailure> {
    if addresses.is_empty()
        || addresses.len() > 64
        || addresses
            .iter()
            .any(|v| v.port() != 443 || !public_address(v.ip()))
    {
        Err(DeliveryFailure::DestinationBlocked)
    } else {
        Ok(())
    }
}

pub fn signature(secret: &str, timestamp: i64, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts arbitrary key lengths");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    format!("v1={}", hex::encode(mac.finalize().into_bytes()))
}
/// Receiver-side example: use the raw bytes, reject old timestamps, and perform
/// constant-time MAC verification. Receivers must also deduplicate delivery IDs.
pub fn verify_signature(
    secret: &str,
    timestamp: i64,
    body: &[u8],
    supplied: &str,
    at: i64,
) -> bool {
    if timestamp.abs_diff(at) > 300 {
        return false;
    }
    let Some(encoded) = supplied.strip_prefix("v1=") else {
        return false;
    };
    let Ok(mac_bytes) = hex::decode(encoded) else {
        return false;
    };
    if mac_bytes.len() != 32 {
        return false;
    }
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.verify_slice(&mac_bytes).is_ok()
}

pub struct OutboundRequest {
    pub url: Url,
    pub addresses: Vec<SocketAddr>,
    pub body: Vec<u8>,
    pub event_id: String,
    pub delivery_id: String,
    pub event_type: String,
    pub timestamp: i64,
    pub signature: String,
}
pub type TransportFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
/// An injectable I/O boundary lets tests exercise the real outbox worker without
/// weakening destination policy or sending requests to external services.
pub trait Transport: Send + Sync {
    fn resolve<'a>(
        &'a self,
        url: &'a Url,
        limits: &'a Webhooks,
    ) -> TransportFuture<'a, Result<Vec<SocketAddr>, DeliveryFailure>>;
    fn post<'a>(
        &'a self,
        request: OutboundRequest,
        limits: &'a Webhooks,
    ) -> TransportFuture<'a, Result<u16, DeliveryFailure>>;
}
pub struct HttpsTransport;
impl Transport for HttpsTransport {
    fn resolve<'a>(
        &'a self,
        url: &'a Url,
        limits: &'a Webhooks,
    ) -> TransportFuture<'a, Result<Vec<SocketAddr>, DeliveryFailure>> {
        Box::pin(async move {
            let addresses = match url.host().ok_or(DeliveryFailure::DestinationBlocked)? {
                Host::Ipv4(ip) => vec![SocketAddr::new(IpAddr::V4(ip), 443)],
                Host::Ipv6(ip) => vec![SocketAddr::new(IpAddr::V6(ip), 443)],
                Host::Domain(domain) => {
                    let resolved = tokio::time::timeout(
                        Duration::from_secs(limits.request_timeout_secs),
                        tokio::net::lookup_host((domain, 443)),
                    )
                    .await
                    .map_err(|_| DeliveryFailure::Timeout)?
                    .map_err(|_| DeliveryFailure::DnsFailed)?;
                    let mut addresses = Vec::new();
                    for address in resolved {
                        if !addresses.contains(&address) {
                            addresses.push(address);
                        }
                        if addresses.len() > 64 {
                            return Err(DeliveryFailure::DestinationBlocked);
                        }
                    }
                    addresses
                }
            };
            validate_addresses(&addresses)?;
            Ok(addresses)
        })
    }
    fn post<'a>(
        &'a self,
        request: OutboundRequest,
        limits: &'a Webhooks,
    ) -> TransportFuture<'a, Result<u16, DeliveryFailure>> {
        Box::pin(async move {
            validate_destination(request.url.as_str())
                .map_err(|_| DeliveryFailure::DestinationBlocked)?;
            validate_addresses(&request.addresses)?;
            if request.body.len() > 8192 {
                return Err(DeliveryFailure::ResponseTooLarge);
            }
            let host = request
                .url
                .host_str()
                .ok_or(DeliveryFailure::DestinationBlocked)?;
            let client = reqwest::Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(host, &request.addresses)
                .timeout(Duration::from_secs(limits.request_timeout_secs))
                .connect_timeout(Duration::from_secs(limits.request_timeout_secs))
                .pool_max_idle_per_host(0)
                .build()
                .map_err(|_| DeliveryFailure::Network)?;
            let map_error = |e: reqwest::Error| {
                if e.is_timeout() {
                    DeliveryFailure::Timeout
                } else {
                    DeliveryFailure::Network
                }
            };
            let mut response = client
                .post(request.url)
                .header("content-type", "application/json")
                .header("user-agent", "HireFerry-Webhooks/1")
                .header("x-hireferry-event", request.event_type)
                .header("x-hireferry-delivery", request.delivery_id)
                .header("x-hireferry-timestamp", request.timestamp.to_string())
                .header("x-hireferry-signature", request.signature)
                .body(request.body)
                .send()
                .await
                .map_err(map_error)?;
            let status = response.status().as_u16();
            if response
                .content_length()
                .is_some_and(|n| n > limits.max_response_bytes as u64)
            {
                return Err(DeliveryFailure::ResponseTooLarge);
            }
            let mut bytes = 0usize;
            while let Some(chunk) = response.chunk().await.map_err(map_error)? {
                bytes = bytes.saturating_add(chunk.len());
                if bytes > limits.max_response_bytes {
                    return Err(DeliveryFailure::ResponseTooLarge);
                }
            }
            // No response body, headers, certificate details or URL reach logs/DB.
            Ok(status)
        })
    }
}

struct Claim {
    id: String,
    token: String,
    url: String,
    secret: String,
    body: String,
    event_id: String,
    event_type: String,
    attempts: i64,
    max_attempts: i64,
}

pub fn cleanup(db: &Connection, limits: &Webhooks, at: i64) -> AppResult<()> {
    db.execute("UPDATE webhook_deliveries SET status='cancelled',last_error='worker_interrupted',claim_token=NULL,lease_until=NULL WHERE status IN ('pending','processing') AND created_at<?1",[at-limits.failed_retention_days*86400])?;
    db.execute("DELETE FROM webhook_deliveries WHERE (status IN ('delivered','cancelled') AND created_at<?1) OR (status='failed' AND created_at<?2)",params![at-limits.delivered_retention_days*86400,at-limits.failed_retention_days*86400])?;
    db.execute("DELETE FROM webhook_events WHERE NOT EXISTS(SELECT 1 FROM webhook_deliveries d WHERE d.event_id=webhook_events.id)",[])?;
    db.execute("DELETE FROM webhook_endpoints WHERE revoked=1 AND updated_at<?1 AND NOT EXISTS(SELECT 1 FROM webhook_deliveries d WHERE d.endpoint_id=webhook_endpoints.id)",[at-limits.failed_retention_days*86400])?;
    Ok(())
}
fn claim(s: &AppState, at: i64) -> AppResult<Option<Claim>> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    cancel_unavailable(&tx, None)?;
    tx.execute("UPDATE webhook_deliveries SET status='failed',last_error='worker_interrupted',claim_token=NULL,lease_until=NULL WHERE status='processing' AND lease_until<=?1 AND attempts>=max_attempts",[at])?;
    let id:Option<String>=tx.query_row("SELECT id FROM webhook_deliveries WHERE ((status='pending' AND next_attempt_at<=?1) OR (status='processing' AND lease_until<=?1)) AND attempts<max_attempts ORDER BY next_attempt_at,created_at,id LIMIT 1",[at],|r|r.get(0)).optional()?;
    let Some(id) = id else {
        tx.commit()?;
        return Ok(None);
    };
    let token = uuid::Uuid::new_v4().to_string();
    let lease = at + (s.cfg.webhooks.request_timeout_secs * 2 + 30) as i64;
    tx.execute("UPDATE webhook_deliveries SET status='processing',attempts=attempts+1,last_attempt_at=?2,claim_token=?3,lease_until=?4 WHERE id=?1",params![id,at,token,lease])?;
    let result=tx.query_row("SELECT d.id,d.attempts,d.max_attempts,e.url,e.signing_secret,w.id,w.event_type,w.body_json FROM webhook_deliveries d JOIN webhook_endpoints e ON e.id=d.endpoint_id JOIN webhook_events w ON w.id=d.event_id WHERE d.id=?1",[&id],|r|Ok(Claim{id:r.get(0)?,attempts:r.get(1)?,max_attempts:r.get(2)?,url:r.get(3)?,secret:r.get(4)?,event_id:r.get(5)?,event_type:r.get(6)?,body:r.get(7)?,token}))?;
    tx.commit()?;
    Ok(Some(result))
}
fn still_authorized(s: &AppState, item: &Claim) -> AppResult<bool> {
    let db = s.conn()?;
    let owned:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM webhook_deliveries WHERE id=?1 AND status='processing' AND claim_token=?2)",params![item.id,item.token],|r|r.get(0))?;
    let permitted = owned && authorized(&db, &item.id)?;
    if !permitted {
        db.execute("UPDATE webhook_deliveries SET status='cancelled',last_error='authorization_revoked',claim_token=NULL,lease_until=NULL WHERE id=?1 AND claim_token=?2 AND status='processing'",params![item.id,item.token])?;
    }
    Ok(permitted)
}
fn complete(
    s: &AppState,
    item: &Claim,
    result: Result<u16, DeliveryFailure>,
    at: i64,
) -> AppResult<()> {
    let db = s.conn()?;
    let (status, error, http, next, delivered) = match result {
        Ok(code) if (200..300).contains(&code) => ("delivered", None, Some(code), at, Some(at)),
        result => {
            let failure = match result {
                Ok(code) => DeliveryFailure::HttpStatus(code),
                Err(failure) => failure,
            };
            let http = if let DeliveryFailure::HttpStatus(code) = failure {
                Some(code)
            } else {
                None
            };
            let retry = failure.retryable() && item.attempts < item.max_attempts;
            let delay = 30_i64.saturating_mul(1_i64 << ((item.attempts - 1).clamp(0, 4) as u32));
            (
                if retry { "pending" } else { "failed" },
                Some(failure.code()),
                http,
                at + delay,
                None,
            )
        }
    };
    db.execute("UPDATE webhook_deliveries SET status=?3,last_error=?4,last_status_code=?5,next_attempt_at=?6,delivered_at=?7,claim_token=NULL,lease_until=NULL WHERE id=?1 AND claim_token=?2 AND status='processing'",params![item.id,item.token,status,error,http,next,delivered])?;
    Ok(())
}

/// Process at most one durable delivery. A fake transport is sufficient to test
/// retries/restarts and the exact authorization boundary without any network I/O.
pub async fn run_once_with<T: Transport>(s: &AppState, transport: &T, at: i64) -> AppResult<bool> {
    if !s.cfg.webhooks.enabled {
        return Ok(false);
    }
    let Some(item) = claim(s, at)? else {
        return Ok(false);
    };
    let url = match validate_destination(&item.url) {
        Ok(url) => url,
        Err(_) => {
            complete(s, &item, Err(DeliveryFailure::DestinationBlocked), at)?;
            return Ok(true);
        }
    };
    let addresses = match transport
        .resolve(&url, &s.cfg.webhooks)
        .await
        .and_then(|addresses| {
            validate_addresses(&addresses)?;
            Ok(addresses)
        }) {
        Ok(addresses) => addresses,
        Err(failure) => {
            complete(s, &item, Err(failure), at)?;
            return Ok(true);
        }
    };
    // DNS can be slow: an endpoint, user or match may have been revoked while it
    // was resolving. Recheck immediately before starting the actual HTTP request.
    if !still_authorized(s, &item)? {
        return Ok(true);
    }
    let request = OutboundRequest {
        url,
        addresses,
        body: item.body.as_bytes().to_vec(),
        event_id: item.event_id.clone(),
        delivery_id: item.id.clone(),
        event_type: item.event_type.clone(),
        timestamp: at,
        signature: signature(&item.secret, at, item.body.as_bytes()),
    };
    let result = tokio::time::timeout(
        Duration::from_secs(s.cfg.webhooks.request_timeout_secs),
        transport.post(request, &s.cfg.webhooks),
    )
    .await
    .unwrap_or(Err(DeliveryFailure::Timeout));
    complete(s, &item, result, at)?;
    Ok(true)
}
pub fn spawn_worker(s: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if !s.cfg.webhooks.enabled {
            return;
        }
        let transport = HttpsTransport;
        let mut last_cleanup = 0;
        loop {
            let at = now();
            if at - last_cleanup >= 60 {
                let cleaned = s.conn().and_then(|db| cleanup(&db, &s.cfg.webhooks, at));
                if cleaned.is_err() {
                    tracing::warn!("Webhook queue maintenance failed");
                }
                last_cleanup = at;
            }
            match run_once_with(&s, &transport, at).await {
                Ok(true) => tokio::time::sleep(Duration::from_millis(100)).await,
                Ok(false) => {
                    tokio::time::sleep(Duration::from_secs(s.cfg.webhooks.poll_interval_secs)).await
                }
                Err(_) => {
                    tracing::warn!("Webhook worker operation failed");
                    tokio::time::sleep(Duration::from_secs(s.cfg.webhooks.poll_interval_secs))
                        .await;
                }
            }
        }
    })
}
