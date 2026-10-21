//! Private platform conversations, explicitly introduced matches, and seeker mandates.
//! Every business operation repeats authorization against the current database state.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, patch, post},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    AppError, AppResult, AppState,
    access::{self, Principal},
    db::audit,
    now, safety,
};

const POLL_MS: i64 = 5000;
const THREAD_FROM: &str = "FROM chat_conversations c
 LEFT JOIN users pu ON pu.id=c.user_id
 LEFT JOIN matches mt ON mt.id=c.match_id
 LEFT JOIN jobs j ON j.id=mt.job_id
 LEFT JOIN users sk ON sk.id=mt.seeker_id
 LEFT JOIN users er ON er.id=j.owner_id";
const VISIBLE: &str = "(?1=1 OR (c.kind='platform' AND pu.banned=0 AND c.user_id=?2)
 OR (c.kind='matched' AND mt.status IN ('introduced','interview','hired')
 AND j.status<>'deleted' AND sk.banned=0 AND er.banned=0
 AND (mt.seeker_id=?2 OR j.owner_id=?2)))";

pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(r#"
 CREATE TABLE IF NOT EXISTS chat_conversations(
 id TEXT PRIMARY KEY,kind TEXT NOT NULL CHECK(kind IN ('platform','matched')),
 user_id TEXT UNIQUE REFERENCES users(id),match_id TEXT UNIQUE REFERENCES matches(id),
 created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,
 CHECK((kind='platform' AND user_id IS NOT NULL AND match_id IS NULL)
 OR (kind='matched' AND match_id IS NOT NULL AND user_id IS NULL)));
 CREATE TABLE IF NOT EXISTS chat_messages(
 id INTEGER PRIMARY KEY AUTOINCREMENT,conversation_id TEXT NOT NULL REFERENCES chat_conversations(id),
 sender_kind TEXT NOT NULL CHECK(sender_kind IN ('user','platform')),
 sender_id TEXT REFERENCES users(id),sender_key TEXT NOT NULL,sender_actor TEXT NOT NULL,
 body TEXT NOT NULL CHECK(length(body) BETWEEN 1 AND 10000),client_id TEXT NOT NULL CHECK(length(client_id) BETWEEN 1 AND 80),
 via_ai INTEGER NOT NULL DEFAULT 0 CHECK(via_ai IN (0,1)),created_at INTEGER NOT NULL,
 UNIQUE(conversation_id,sender_key,client_id),
 CHECK((sender_kind='user' AND sender_id IS NOT NULL) OR (sender_kind='platform' AND sender_id IS NULL)));
 CREATE INDEX IF NOT EXISTS chat_messages_thread ON chat_messages(conversation_id,id);
 CREATE TABLE IF NOT EXISTS chat_reads(
 conversation_id TEXT NOT NULL REFERENCES chat_conversations(id),reader_key TEXT NOT NULL,
 last_message_id INTEGER NOT NULL DEFAULT 0 CHECK(last_message_id>=0),updated_at INTEGER NOT NULL,
 PRIMARY KEY(conversation_id,reader_key));
 CREATE TABLE IF NOT EXISTS mandates(
 user_id TEXT PRIMARY KEY REFERENCES users(id),target_industry TEXT NOT NULL,target_roles TEXT NOT NULL,
 salary_min_cents INTEGER CHECK(salary_min_cents>=0),salary_max_cents INTEGER CHECK(salary_max_cents>=0),
 location TEXT NOT NULL,preferences TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','paused','closed')),
 allow_external INTEGER NOT NULL DEFAULT 0 CHECK(allow_external IN (0,1)),consented_at INTEGER,
 consented_via_ai INTEGER NOT NULL DEFAULT 0 CHECK(consented_via_ai IN (0,1)),
 via_ai INTEGER NOT NULL DEFAULT 0 CHECK(via_ai IN (0,1)),created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,
 CHECK(salary_min_cents IS NULL OR salary_max_cents IS NULL OR salary_min_cents<=salary_max_cents),
 CHECK(allow_external=0 OR consented_at IS NOT NULL));
 CREATE INDEX IF NOT EXISTS mandates_status ON mandates(status,updated_at);
 CREATE TABLE IF NOT EXISTS outreach(
 id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),channel TEXT NOT NULL,opportunity TEXT NOT NULL,
 source_url TEXT NOT NULL,stage TEXT NOT NULL CHECK(stage IN ('identified','contacted','replied','interview','offer','closed')),
 note TEXT NOT NULL,via_ai INTEGER NOT NULL DEFAULT 0 CHECK(via_ai IN (0,1)),
 created_by TEXT NOT NULL,updated_by TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
 CREATE INDEX IF NOT EXISTS outreach_user ON outreach(user_id,updated_at);
 "#)?;
    Ok(())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageInput {
    pub body: String,
    pub client_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MandateInput {
    pub target_industry: String,
    pub target_roles: String,
    pub salary_min_cents: Option<i64>,
    pub salary_max_cents: Option<i64>,
    pub location: String,
    pub preferences: String,
    pub status: String,
    pub allow_external: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutreachInput {
    pub channel: String,
    pub opportunity: String,
    #[serde(default)]
    pub source_url: String,
    pub stage: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchInput {
    pub q: Option<String>,
    pub status: Option<String>,
    pub page: Option<i64>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PageInput {
    after: Option<i64>,
    before: Option<i64>,
    limit: Option<i64>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PlatformInput {
    user_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    message_id: i64,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/chat/conversations", get(http_conversations))
        .route("/api/chat/platform", post(http_platform))
        .route(
            "/api/chat/conversations/{id}/messages",
            get(http_messages).post(http_send),
        )
        .route("/api/chat/conversations/{id}/read", post(http_read))
        .route("/api/chat/updates", get(http_updates))
        .route("/api/mandate", get(http_mandate).put(http_save_mandate))
        .route("/api/outreach", get(http_outreach))
        .route("/api/admin/mandates", get(http_mandates))
        .route(
            "/api/admin/mandates/{user_id}",
            get(http_admin_mandate).patch(http_admin_save_mandate),
        )
        .route(
            "/api/admin/outreach/{user_id}",
            get(http_admin_outreach).post(http_new_outreach),
        )
        .route(
            "/api/admin/outreach/records/{id}",
            patch(http_update_outreach),
        )
}

fn missing_thread() -> AppError {
    AppError::not_found("Conversation not found")
}
fn user_key(p: &Principal) -> AppResult<String> {
    if p.admin {
        Ok("admin".into())
    } else {
        Ok(format!(
            "user_{}",
            p.user_id
                .as_deref()
                .ok_or_else(|| AppError::unauthorized("Sign in required"))?
        ))
    }
}
fn uid(p: &Principal) -> &str {
    p.user_id.as_deref().unwrap_or("")
}
fn limit(value: i64) -> AppResult<i64> {
    if value < 1 {
        Err(AppError::bad_request("Limit must be positive"))
    } else {
        Ok(value.min(100))
    }
}
fn text(value: &str, max: usize, label: &str) -> AppResult<()> {
    if value.chars().count() > max
        || value
            .chars()
            .any(|c| c == '\0' || (c.is_control() && !matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(AppError::bad_request(format!(
            "{label} is too long or contains invalid characters"
        )));
    }
    Ok(())
}
fn require_admin(db: &Connection, p: &Principal) -> AppResult<()> {
    access::ensure_active(db, p)?;
    if !p.admin {
        return Err(AppError::forbidden("Administrator access required"));
    }
    Ok(())
}

/// Validates existence and current membership before creating any conversation row.
fn ensure_thread(db: &Connection, p: &Principal, id: &str) -> AppResult<()> {
    access::ensure_active(db, p)?;
    if let Some(user) = id.strip_prefix("platform_") {
        if user.is_empty() || (!p.admin && uid(p) != user) {
            return Err(missing_thread());
        }
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND (banned=0 OR ?2=1))",
            params![user, p.admin],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(missing_thread());
        }
        db.execute("INSERT OR IGNORE INTO chat_conversations(id,kind,user_id,created_at,updated_at) VALUES(?1,'platform',?2,?3,?3)", params![id,user,now()])?;
    } else if let Some(match_id) = id.strip_prefix("match_") {
        let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM matches mt JOIN jobs j ON j.id=mt.job_id JOIN users sk ON sk.id=mt.seeker_id JOIN users er ON er.id=j.owner_id WHERE mt.id=?1 AND (?2=1 OR (mt.status IN ('introduced','interview','hired') AND j.status<>'deleted' AND sk.banned=0 AND er.banned=0 AND (mt.seeker_id=?3 OR j.owner_id=?3))))", params![match_id,p.admin,uid(p)], |r| r.get(0))?;
        if !exists {
            return Err(missing_thread());
        }
        db.execute("INSERT OR IGNORE INTO chat_conversations(id,kind,match_id,created_at,updated_at) VALUES(?1,'matched',?2,?3,?3)", params![id,match_id,now()])?;
    } else {
        return Err(missing_thread());
    }
    Ok(())
}

fn ensure_matches(db: &Connection, p: &Principal) -> AppResult<()> {
    access::ensure_active(db, p)?;
    let ids = db.prepare("SELECT mt.id FROM matches mt JOIN jobs j ON j.id=mt.job_id JOIN users sk ON sk.id=mt.seeker_id JOIN users er ON er.id=j.owner_id WHERE ?1=1 OR (mt.status IN ('introduced','interview','hired') AND j.status<>'deleted' AND sk.banned=0 AND er.banned=0 AND (mt.seeker_id=?2 OR j.owner_id=?2))")?
        .query_map(params![p.admin,uid(p)], |r| r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for id in ids {
        ensure_thread(db, p, &format!("match_{id}"))?;
    }
    Ok(())
}

fn participant(db: &Connection, user: &str) -> AppResult<Value> {
    Ok(db.query_row("SELECT id,name,role,avatar FROM users WHERE id=?1", [user], |r| {
        let avatar: String = r.get(3)?;
        Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"role":r.get::<_,String>(2)?,"avatar":if avatar.is_empty(){String::new()}else{format!("/api/avatars/{user}")}}))
    })?)
}
fn unread(db: &Connection, p: &Principal, id: &str) -> AppResult<i64> {
    let key = user_key(p)?;
    Ok(db.query_row("SELECT COUNT(*) FROM chat_messages WHERE conversation_id=?1 AND sender_key<>?2 AND id>COALESCE((SELECT last_message_id FROM chat_reads WHERE conversation_id=?1 AND reader_key=?2),0)", params![id,key], |r| r.get(0))?)
}
fn writable(db: &Connection, id: &str) -> AppResult<bool> {
    Ok(db.query_row(&format!("SELECT EXISTS(SELECT 1 {THREAD_FROM} WHERE c.id=?1 AND ((c.kind='platform' AND pu.banned=0) OR (c.kind='matched' AND mt.status IN ('introduced','interview','hired') AND j.status<>'deleted' AND sk.banned=0 AND er.banned=0)))"),[id],|r|r.get(0))?)
}
fn thread_value(db: &Connection, p: &Principal, id: &str) -> AppResult<Value> {
    let (kind, user, matched, created, updated): (
        String,
        Option<String>,
        Option<String>,
        i64,
        i64,
    ) = db.query_row(
        "SELECT kind,user_id,match_id,created_at,updated_at FROM chat_conversations WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    let (title, job_id, participants) = if let Some(ref user) = user {
        let person = participant(db, user)?;
        let title = if p.admin {
            person["name"].as_str().unwrap_or("").to_owned()
        } else {
            "聘渡 · 平台顾问".into()
        };
        (
            title,
            None,
            vec![
                person,
                json!({"id":"platform","name":"HireFerry · 聘渡","role":"platform","avatar":""}),
            ],
        )
    } else {
        let (seeker, employer, job, title): (String,String,String,String) = db.query_row("SELECT mt.seeker_id,j.owner_id,j.id,j.title FROM matches mt JOIN jobs j ON j.id=mt.job_id WHERE mt.id=?1", [&matched], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        (
            title,
            Some(job),
            vec![participant(db, &seeker)?, participant(db, &employer)?],
        )
    };
    let last: Option<(i64,String,i64)> = db.query_row("SELECT id,body,created_at FROM chat_messages WHERE conversation_id=?1 ORDER BY id DESC LIMIT 1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    Ok(
        json!({"id":id,"kind":kind,"title":title,"user_id":user,"match_id":matched,"job_id":job_id,"job_title":if kind=="matched"{Some(title.clone())}else{None},"participants":participants,
        "last_message_id":last.as_ref().map(|v|v.0).unwrap_or(0),"last_message_preview":last.as_ref().map(|v|v.1.chars().take(180).collect::<String>()).unwrap_or_default(),"last_message_at":last.as_ref().map(|v|v.2),"unread_count":unread(db,p,id)?,"writable":writable(db,id)?,"created_at":created,"updated_at":updated}),
    )
}

pub fn list_conversations(s: &AppState, p: &Principal) -> AppResult<Value> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_active(&tx, p)?;
    if !p.admin {
        ensure_thread(&tx, p, &format!("platform_{}", uid(p)))?;
    }
    ensure_matches(&tx, p)?;
    let sql = format!(
        "SELECT c.id {THREAD_FROM} WHERE {VISIBLE} ORDER BY c.updated_at DESC,c.id LIMIT 200"
    );
    let ids = tx
        .prepare(&sql)?
        .query_map(params![p.admin, uid(p)], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let items = ids
        .iter()
        .map(|id| thread_value(&tx, p, id))
        .collect::<AppResult<Vec<_>>>()?;
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    Ok(json!({"items":items,"poll_after_ms":POLL_MS}))
}
pub fn platform_conversation(
    s: &AppState,
    p: &Principal,
    target_user_id: Option<&str>,
) -> AppResult<Value> {
    let user = target_user_id
        .or(p.user_id.as_deref())
        .ok_or_else(|| AppError::bad_request("Choose a user"))?;
    let id = format!("platform_{user}");
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_thread(&tx, p, &id)?;
    let result = thread_value(&tx, p, &id)?;
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    Ok(result)
}

fn message_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let kind: String = r.get("sender_kind")?;
    let body: String = r.get("body")?;
    let sender_name: String = r.get("sender_name")?;
    Ok(
        json!({"id":r.get::<_,i64>("id")?,"conversation_id":r.get::<_,String>("conversation_id")?,"sender_kind":kind,"sender_id":r.get::<_,Option<String>>("sender_id")?,"sender_name":if kind=="platform"{"HireFerry · 聘渡".to_owned()}else{sender_name},"body":body,"body_html":safety::markdown(&body),"client_id":r.get::<_,String>("client_id")?,"via_ai":r.get::<_,bool>("via_ai")?,"created_at":r.get::<_,i64>("created_at")?}),
    )
}
fn load_message(db: &Connection, id: i64) -> AppResult<Value> {
    Ok(db.query_row("SELECT msg.*,COALESCE(sender.name,'') AS sender_name FROM chat_messages msg LEFT JOIN users sender ON sender.id=msg.sender_id WHERE msg.id=?1",[id],message_row)?)
}

/// after=-1 selects the latest page; after=0 starts at the beginning of history.
pub fn messages(s: &AppState, p: &Principal, id: &str, after: i64, count: i64) -> AppResult<Value> {
    message_page(s, p, id, after, None, count)
}
pub fn messages_before(
    s: &AppState,
    p: &Principal,
    id: &str,
    before: i64,
    count: i64,
) -> AppResult<Value> {
    message_page(s, p, id, -1, Some(before), count)
}
fn message_page(
    s: &AppState,
    p: &Principal,
    id: &str,
    after: i64,
    before: Option<i64>,
    count: i64,
) -> AppResult<Value> {
    let count = limit(count)?;
    if after < -1 || before.is_some_and(|n| n < 1) || (before.is_some() && after != -1) {
        return Err(AppError::bad_request("Invalid message cursor"));
    }
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_thread(&tx, p, id)?;
    let backwards = after == -1;
    let (comparison, bound, ordering) = if backwards {
        ("<", before.unwrap_or(i64::MAX), "DESC")
    } else {
        (">", after, "ASC")
    };
    let sql = format!(
        "SELECT msg.*,COALESCE(sender.name,'') AS sender_name FROM chat_messages msg LEFT JOIN users sender ON sender.id=msg.sender_id WHERE msg.conversation_id=?1 AND msg.id {comparison} ?2 ORDER BY msg.id {ordering} LIMIT ?3"
    );
    let mut items = tx
        .prepare(&sql)?
        .query_map(params![id, bound, count + 1], message_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let extra = items.len() > count as usize;
    items.truncate(count as usize);
    if backwards {
        items.reverse();
    }
    let first = items.first().and_then(|v| v["id"].as_i64()).unwrap_or(0);
    let cursor = items
        .last()
        .and_then(|v| v["id"].as_i64())
        .unwrap_or(after.max(0));
    let older = if backwards {
        extra
    } else if first > 0 {
        tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM chat_messages WHERE conversation_id=?1 AND id<?2)",
            params![id, first],
            |r| r.get::<_, bool>(0),
        )?
    } else {
        false
    };
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    Ok(
        json!({"items":items,"next_cursor":cursor,"first_cursor":first,"has_more":!backwards&&extra,"has_older":older,"poll_after_ms":POLL_MS}),
    )
}

pub fn send_message(
    s: &AppState,
    p: &Principal,
    id: &str,
    input: MessageInput,
) -> AppResult<Value> {
    text(&input.body, 10000, "Message")?;
    text(&input.client_id, 80, "Message client ID")?;
    if input.body.trim().is_empty()
        || input.client_id.trim().is_empty()
        || input.client_id.chars().any(char::is_control)
    {
        return Err(AppError::bad_request(
            "A message and a client ID are required",
        ));
    }
    let key = user_key(p)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_thread(&tx, p, id)?;
    if !writable(&tx, id)? {
        return Err(AppError::forbidden(
            "This conversation is archived and read-only",
        ));
    }
    let previous: Option<(i64,String)>=tx.query_row("SELECT id,body FROM chat_messages WHERE conversation_id=?1 AND sender_key=?2 AND client_id=?3",params![id,key,input.client_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let result = if let Some((message_id, body)) = previous {
        if body != input.body {
            return Err(AppError::conflict(
                "This client ID already belongs to another message",
            ));
        }
        load_message(&tx, message_id)?
    } else {
        let kind = if p.admin { "platform" } else { "user" };
        let sender = if p.admin { None } else { p.user_id.as_deref() };
        tx.execute("INSERT INTO chat_messages(conversation_id,sender_kind,sender_id,sender_key,sender_actor,body,client_id,via_ai,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id,kind,sender,key,p.actor,input.body,input.client_id,p.via_ai,now()])?;
        let message_id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE chat_conversations SET updated_at=?2 WHERE id=?1",
            params![id, now()],
        )?;
        audit(
            &tx,
            &p.actor,
            "chat_message",
            id,
            &json!({"message_id":message_id,"via_ai":p.via_ai}).to_string(),
        )?;
        load_message(&tx, message_id)?
    };
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    Ok(result)
}

pub fn mark_read(s: &AppState, p: &Principal, id: &str, message_id: i64) -> AppResult<Value> {
    if message_id < 0 {
        return Err(AppError::bad_request("Invalid message ID"));
    }
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_thread(&tx, p, id)?;
    if message_id > 0
        && !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM chat_messages WHERE id=?1 AND conversation_id=?2)",
            params![message_id, id],
            |r| r.get::<_, bool>(0),
        )?
    {
        return Err(AppError::not_found("Message not found"));
    }
    let key = user_key(p)?;
    tx.execute("INSERT INTO chat_reads(conversation_id,reader_key,last_message_id,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(conversation_id,reader_key) DO UPDATE SET last_message_id=MAX(chat_reads.last_message_id,excluded.last_message_id),updated_at=excluded.updated_at",params![id,key,message_id,now()])?;
    let stored: i64 = tx.query_row(
        "SELECT last_message_id FROM chat_reads WHERE conversation_id=?1 AND reader_key=?2",
        params![id, key],
        |r| r.get(0),
    )?;
    let result = json!({"message_id":stored,"unread_count":unread(&tx,p,id)?});
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    Ok(result)
}

pub fn updates(s: &AppState, p: &Principal, after: i64, count: i64) -> AppResult<Value> {
    if after < 0 {
        return Err(AppError::bad_request("Invalid update cursor"));
    }
    let count = limit(count)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_active(&tx, p)?;
    ensure_matches(&tx, p)?;
    let sql = format!(
        "SELECT msg.*,COALESCE(sender.name,'') AS sender_name {THREAD_FROM} JOIN chat_messages msg ON msg.conversation_id=c.id LEFT JOIN users sender ON sender.id=msg.sender_id WHERE {VISIBLE} AND msg.id>?3 ORDER BY msg.id ASC LIMIT ?4"
    );
    let mut items = tx
        .prepare(&sql)?
        .query_map(params![p.admin, uid(p), after, count + 1], message_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let more = items.len() > count as usize;
    items.truncate(count as usize);
    let cursor = items.last().and_then(|v| v["id"].as_i64()).unwrap_or(after);
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    Ok(json!({"items":items,"next_cursor":cursor,"has_more":more,"poll_after_ms":POLL_MS}))
}

fn target_seeker(db: &Connection, p: &Principal, target: Option<&str>) -> AppResult<String> {
    access::ensure_active(db, p)?;
    let user = target
        .or(p.user_id.as_deref())
        .ok_or_else(|| AppError::bad_request("Choose a job seeker"))?;
    if !p.admin && uid(p) != user {
        return Err(AppError::not_found("Mandate not found"));
    }
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND role='seeker' AND (banned=0 OR ?2=1))",
        params![user, p.admin],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(AppError::not_found("Mandate not found"));
    }
    Ok(user.to_owned())
}
fn mandate_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"user_id":r.get::<_,String>("user_id")?,"user_name":r.get::<_,String>("user_name")?,"banned":r.get::<_,bool>("user_banned")?,"target_industry":r.get::<_,String>("target_industry")?,"target_roles":r.get::<_,String>("target_roles")?,"salary_min_cents":r.get::<_,Option<i64>>("salary_min_cents")?,"salary_max_cents":r.get::<_,Option<i64>>("salary_max_cents")?,"location":r.get::<_,String>("location")?,"preferences":r.get::<_,String>("preferences")?,"status":r.get::<_,String>("status")?,"allow_external":r.get::<_,bool>("allow_external")?,"consented_at":r.get::<_,Option<i64>>("consented_at")?,"consented_via_ai":r.get::<_,bool>("consented_via_ai")?,"via_ai":r.get::<_,bool>("via_ai")?,"created_at":r.get::<_,i64>("created_at")?,"updated_at":r.get::<_,i64>("updated_at")?}),
    )
}
fn stored_mandate(db: &Connection, user: &str) -> AppResult<Option<Value>> {
    Ok(db.query_row("SELECT m.*,u.name AS user_name,u.banned AS user_banned FROM mandates m JOIN users u ON u.id=m.user_id WHERE m.user_id=?1",[user],mandate_row).optional()?)
}
pub fn get_mandate(s: &AppState, p: &Principal, user: Option<&str>) -> AppResult<Value> {
    let db = s.conn()?;
    let user = target_seeker(&db, p, user)?;
    Ok(json!({"mandate":stored_mandate(&db,&user)?}))
}

pub fn save_mandate(
    s: &AppState,
    p: &Principal,
    target: Option<&str>,
    input: MandateInput,
) -> AppResult<Value> {
    text(&input.target_industry, 500, "Target industry")?;
    text(&input.target_roles, 1000, "Target roles")?;
    text(&input.location, 500, "Location")?;
    text(&input.preferences, 5000, "Preferences")?;
    if input.target_industry.trim().is_empty() && input.target_roles.trim().is_empty() {
        return Err(AppError::bad_request(
            "Describe your target industry or roles",
        ));
    }
    if !matches!(input.status.as_str(), "active" | "paused" | "closed") {
        return Err(AppError::bad_request("Invalid mandate status"));
    }
    if input
        .salary_min_cents
        .into_iter()
        .chain(input.salary_max_cents)
        .any(|n| !(0..=100_000_000).contains(&n))
        || matches!((input.salary_min_cents,input.salary_max_cents),(Some(a),Some(b)) if a>b)
    {
        return Err(AppError::bad_request("Invalid salary range in CNY cents"));
    }
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let user = target_seeker(&tx, p, target)?;
    let old = stored_mandate(&tx, &user)?;
    if input.status == "active"
        && tx.query_row("SELECT banned FROM users WHERE id=?1", [&user], |r| {
            r.get::<_, bool>(0)
        })?
    {
        return Err(AppError::forbidden(
            "A suspended account cannot activate a mandate",
        ));
    }
    let previously_allowed = old.as_ref().is_some_and(|v| v["allow_external"] == true);
    if p.admin {
        let previous = old
            .as_ref()
            .ok_or_else(|| AppError::forbidden("Only the seeker can submit a new mandate"))?;
        if input.allow_external && !previously_allowed {
            return Err(AppError::forbidden(
                "Only the seeker can authorize external outreach",
            ));
        }
        if input.status == "active" && previous["status"] != "active" {
            return Err(AppError::forbidden("Only the seeker can resume a mandate"));
        }
    }
    let consent = if input.allow_external {
        Some(if previously_allowed {
            old.as_ref()
                .and_then(|v| v["consented_at"].as_i64())
                .unwrap_or_else(now)
        } else {
            now()
        })
    } else {
        None
    };
    let consent_ai = if input.allow_external {
        if previously_allowed {
            old.as_ref().is_some_and(|v| v["consented_via_ai"] == true)
        } else {
            p.via_ai
        }
    } else {
        false
    };
    tx.execute("INSERT INTO mandates(user_id,target_industry,target_roles,salary_min_cents,salary_max_cents,location,preferences,status,allow_external,consented_at,consented_via_ai,via_ai,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13) ON CONFLICT(user_id) DO UPDATE SET target_industry=excluded.target_industry,target_roles=excluded.target_roles,salary_min_cents=excluded.salary_min_cents,salary_max_cents=excluded.salary_max_cents,location=excluded.location,preferences=excluded.preferences,status=excluded.status,allow_external=excluded.allow_external,consented_at=excluded.consented_at,consented_via_ai=excluded.consented_via_ai,via_ai=excluded.via_ai,updated_at=excluded.updated_at",params![user,input.target_industry.trim(),input.target_roles.trim(),input.salary_min_cents,input.salary_max_cents,input.location.trim(),input.preferences,input.status,input.allow_external,consent,consent_ai,p.via_ai,now()])?;
    audit(&tx,&p.actor,"mandate_update",&user,&json!({"status":input.status,"allow_external":input.allow_external,"previous_allow_external":previously_allowed,"via_ai":p.via_ai}).to_string())?;
    let result = json!({"mandate":stored_mandate(&tx,&user)?});
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    Ok(result)
}
pub fn list_mandates(s: &AppState, p: &Principal, query: SearchInput) -> AppResult<Value> {
    let db = s.conn()?;
    require_admin(&db, p)?;
    let q = query.q.unwrap_or_default();
    text(&q, 200, "Search")?;
    let status = query.status.filter(|v| !v.is_empty());
    if status
        .as_deref()
        .is_some_and(|v| !matches!(v, "active" | "paused" | "closed"))
    {
        return Err(AppError::bad_request("Invalid mandate status"));
    }
    let page = query.page.unwrap_or(1);
    if !(1..=1_000_000).contains(&page) {
        return Err(AppError::bad_request("Invalid page"));
    }
    let filter = "FROM mandates m JOIN users u ON u.id=m.user_id WHERE u.role='seeker' AND (?1 IS NULL OR m.status=?1) AND (u.name LIKE ?2 OR m.target_industry LIKE ?2 OR m.target_roles LIKE ?2 OR m.location LIKE ?2)";
    let search = format!("%{}%", q.trim());
    let total: i64 = db.query_row(
        &format!("SELECT count(*) {filter}"),
        params![status, search],
        |r| r.get(0),
    )?;
    let items=db.prepare(&format!("SELECT m.*,u.name AS user_name,u.banned AS user_banned {filter} ORDER BY m.updated_at DESC,m.user_id LIMIT 50 OFFSET ?3"))?.query_map(params![status,search,(page-1)*50],mandate_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"items":items,"total":total,"page":page}))
}

fn outreach_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,String>("id")?,"user_id":r.get::<_,String>("user_id")?,"banned":r.get::<_,bool>("user_banned")?,"channel":r.get::<_,String>("channel")?,"opportunity":r.get::<_,String>("opportunity")?,"source_url":r.get::<_,String>("source_url")?,"stage":r.get::<_,String>("stage")?,"note":r.get::<_,String>("note")?,"via_ai":r.get::<_,bool>("via_ai")?,"created_at":r.get::<_,i64>("created_at")?,"updated_at":r.get::<_,i64>("updated_at")?}),
    )
}
pub fn list_outreach(s: &AppState, p: &Principal, target: Option<&str>) -> AppResult<Value> {
    let db = s.conn()?;
    let user = target_seeker(&db, p, target)?;
    let items=db.prepare("SELECT o.*,u.banned AS user_banned FROM outreach o JOIN users u ON u.id=o.user_id WHERE o.user_id=?1 ORDER BY o.updated_at DESC,o.id LIMIT 200")?.query_map([user],outreach_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"items":items}))
}
pub fn save_outreach(
    s: &AppState,
    p: &Principal,
    target: &str,
    record_id: Option<&str>,
    input: OutreachInput,
) -> AppResult<Value> {
    text(&input.channel, 120, "Channel")?;
    text(&input.opportunity, 500, "Opportunity")?;
    text(&input.source_url, 2048, "Source URL")?;
    text(&input.note, 5000, "Progress note")?;
    if input.channel.trim().is_empty() || input.opportunity.trim().is_empty() {
        return Err(AppError::bad_request(
            "Channel and opportunity are required",
        ));
    }
    if !matches!(
        input.stage.as_str(),
        "identified" | "contacted" | "replied" | "interview" | "offer" | "closed"
    ) {
        return Err(AppError::bad_request("Invalid outreach stage"));
    }
    if !input.source_url.is_empty() {
        let url = url::Url::parse(&input.source_url)
            .map_err(|_| AppError::bad_request("Use a valid HTTP or HTTPS source URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(AppError::bad_request(
                "Use an HTTP or HTTPS source URL without credentials",
            ));
        }
    }
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_admin(&tx, p)?;
    let user = target_seeker(&tx, p, Some(target))?;
    if let Some(id) = record_id {
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM outreach WHERE id=?1 AND user_id=?2)",
            params![id, user],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(AppError::not_found("Progress record not found"));
        }
    }
    let allowed: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM mandates m JOIN users u ON u.id=m.user_id WHERE m.user_id=?1 AND m.status='active' AND m.allow_external=1 AND m.consented_at IS NOT NULL AND u.banned=0)",[&user],|r|r.get(0))?;
    if !allowed && !(record_id.is_some() && input.stage == "closed") {
        return Err(AppError::forbidden(
            "External outreach requires an active mandate and the seeker's consent",
        ));
    }
    let id = record_id
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if record_id.is_some() {
        tx.execute("UPDATE outreach SET channel=?2,opportunity=?3,source_url=?4,stage=?5,note=?6,via_ai=?7,updated_by=?8,updated_at=?9 WHERE id=?1 AND user_id=?10",params![id,input.channel.trim(),input.opportunity.trim(),input.source_url,input.stage,input.note,p.via_ai,p.actor,now(),user])?;
    } else {
        tx.execute("INSERT INTO outreach(id,user_id,channel,opportunity,source_url,stage,note,via_ai,created_by,updated_by,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9,?10,?10)",params![id,user,input.channel.trim(),input.opportunity.trim(),input.source_url,input.stage,input.note,p.via_ai,p.actor,now()])?;
    }
    audit(
        &tx,
        &p.actor,
        "outreach_update",
        &id,
        &json!({"user_id":user,"stage":input.stage,"via_ai":p.via_ai}).to_string(),
    )?;
    let result=tx.query_row("SELECT o.*,u.banned AS user_banned FROM outreach o JOIN users u ON u.id=o.user_id WHERE o.id=?1",[&id],outreach_row)?;
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    Ok(result)
}

async fn http_conversations(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(list_conversations(&s, &p)?))
}
async fn http_platform(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(i): Json<PlatformInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(platform_conversation(&s, &p, i.user_id.as_deref())?))
}
async fn http_messages(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<PageInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(message_page(
        &s,
        &p,
        &id,
        q.after.unwrap_or(-1),
        q.before,
        q.limit.unwrap_or(100),
    )?))
}
async fn http_send(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(i): Json<MessageInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(send_message(&s, &p, &id, i)?))
}
async fn http_read(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(i): Json<ReadInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(mark_read(&s, &p, &id, i.message_id)?))
}
async fn http_updates(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<PageInput>,
) -> AppResult<Json<Value>> {
    if q.before.is_some() {
        return Err(AppError::bad_request("Updates use an after cursor"));
    }
    let p = access::principal(&s, &h, false)?;
    Ok(Json(updates(
        &s,
        &p,
        q.after.unwrap_or(0),
        q.limit.unwrap_or(100),
    )?))
}
async fn http_mandate(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(get_mandate(&s, &p, None)?))
}
async fn http_save_mandate(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(i): Json<MandateInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(save_mandate(&s, &p, None, i)?))
}
async fn http_outreach(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(list_outreach(&s, &p, None)?))
}
async fn http_mandates(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<SearchInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(list_mandates(&s, &p, q)?))
}
async fn http_admin_mandate(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(user): Path<String>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    require_admin(&*s.conn()?, &p)?;
    Ok(Json(get_mandate(&s, &p, Some(&user))?))
}
async fn http_admin_save_mandate(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(user): Path<String>,
    Json(i): Json<MandateInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    require_admin(&*s.conn()?, &p)?;
    Ok(Json(save_mandate(&s, &p, Some(&user), i)?))
}
async fn http_admin_outreach(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(user): Path<String>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    require_admin(&*s.conn()?, &p)?;
    Ok(Json(list_outreach(&s, &p, Some(&user))?))
}
async fn http_new_outreach(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(user): Path<String>,
    Json(i): Json<OutreachInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(save_outreach(&s, &p, &user, None, i)?))
}
async fn http_update_outreach(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(i): Json<OutreachInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    let user = {
        let db = s.conn()?;
        require_admin(&db, &p)?;
        db.query_row("SELECT user_id FROM outreach WHERE id=?1", [&id], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .ok_or_else(|| AppError::not_found("Progress record not found"))?
    };
    Ok(Json(save_outreach(&s, &p, &user, Some(&id), i)?))
}
