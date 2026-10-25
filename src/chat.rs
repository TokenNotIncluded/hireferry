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
pub(crate) const THREAD_FROM: &str = "FROM chat_conversations c
 LEFT JOIN users pu ON pu.id=c.user_id
 LEFT JOIN matches mt ON mt.id=c.match_id
 LEFT JOIN jobs j ON j.id=mt.job_id
 LEFT JOIN users sk ON sk.id=mt.seeker_id
 LEFT JOIN users er ON er.id=j.owner_id";
pub(crate) const VISIBLE: &str = "(?1=1 OR (c.kind='platform' AND pu.banned=0 AND c.user_id=?2)
 OR (c.kind='matched' AND mt.status IN ('introduced','interview','hired')
 AND j.status<>'deleted' AND sk.banned=0 AND er.banned=0
 AND (mt.seeker_id=?2 OR j.owner_id=?2)))";
// Resolve staff attribution alongside the message page. Disabled employees and
// rotated credentials retain their historical attribution without extra queries.
const MESSAGE_FIELDS: &str = "msg.*,COALESCE(sender.name,'') AS sender_name,
 COALESCE(staff_member.name,manager_user.name) AS staff_display_name";
const MESSAGE_JOINS: &str = "LEFT JOIN users sender ON sender.id=msg.sender_id
 LEFT JOIN team_ai_tokens staff_token ON staff_token.id=
    CASE WHEN substr(msg.sender_key,1,9)='staff_ai:' THEN substr(msg.sender_key,10) END
 LEFT JOIN team_members staff_member ON staff_member.id=
    CASE WHEN substr(msg.sender_key,1,6)='staff:' THEN substr(msg.sender_key,7)
         ELSE staff_token.member_id END
 LEFT JOIN users manager_user ON manager_user.id=
    CASE WHEN substr(msg.sender_key,1,8)='manager:' THEN substr(msg.sender_key,9) END";

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
 salary_currency TEXT NOT NULL DEFAULT 'CNY',
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
    let has_currency: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('mandates') WHERE name='salary_currency')",
        [],
        |row| row.get(0),
    )?;
    if !has_currency {
        db.execute_batch(
            "ALTER TABLE mandates ADD COLUMN salary_currency TEXT NOT NULL DEFAULT 'CNY';",
        )?;
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageInput {
    pub body: String,
    pub client_id: String,
}

/// Explicit wire format; legacy Rust callers keep using MessageInput.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormattedMessageInput {
    pub body: String,
    pub client_id: String,
    #[serde(default = "crate::chat_stream::default_format")]
    pub format: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MandateInput {
    pub target_industry: String,
    pub target_roles: String,
    pub salary_min_cents: Option<i64>,
    pub salary_max_cents: Option<i64>,
    pub salary_currency: Option<String>,
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
pub(crate) struct JobCardInput {
    pub job_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    message_id: i64,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .merge(crate::chat_stream::routes())
        .route("/api/chat/conversations", get(http_conversations))
        .route("/api/chat/conversations/{id}", get(http_conversation))
        .route("/api/chat/platform", post(http_platform))
        .route("/api/chat/conversations/{id}/job-card", post(http_job_card))
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
pub(crate) fn user_key(p: &Principal) -> AppResult<String> {
    if p.admin {
        // Managers keep their personal user identity; team employees use the
        // independent membership identity below. Neither shares the root cursor.
        if let Some(id) = &p.user_id {
            return Ok(format!("manager:{id}"));
        }
        // Each employee keeps their own read cursor and idempotency namespace.
        // A member's browser and WebMCP share it across that member's sessions.
        if let Some(identity) = p
            .actor
            .strip_prefix("staff:")
            .or_else(|| p.actor.strip_prefix("staff_webmcp:"))
        {
            let (id, _) = identity
                .split_once(':')
                .ok_or_else(|| AppError::unauthorized("Invalid staff identity"))?;
            return Ok(format!("staff:{id}"));
        }
        if p.actor.starts_with("staff_ai:") {
            return Ok(p.actor.clone());
        }
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
pub(crate) fn limit(value: i64) -> AppResult<i64> {
    if value < 1 {
        Err(AppError::bad_request("Limit must be positive"))
    } else {
        Ok(value.min(100))
    }
}
pub(crate) fn text(value: &str, max: usize, label: &str) -> AppResult<()> {
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
pub(crate) fn ensure_thread(db: &Connection, p: &Principal, id: &str) -> AppResult<()> {
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
    // The caller holds an IMMEDIATE transaction. Materialize only missing,
    // currently authorized rooms without loading every match into Rust.
    db.execute(
        "INSERT OR IGNORE INTO chat_conversations(id,kind,match_id,created_at,updated_at)
         SELECT 'match_' || mt.id,'matched',mt.id,?3,?3
         FROM matches mt
         JOIN jobs j ON j.id=mt.job_id
         JOIN users sk ON sk.id=mt.seeker_id
         JOIN users er ON er.id=j.owner_id
         WHERE (?1=1 OR (mt.status IN ('introduced','interview','hired')
           AND j.status<>'deleted' AND sk.banned=0 AND er.banned=0
           AND (mt.seeker_id=?2 OR j.owner_id=?2)))
         AND NOT EXISTS(SELECT 1 FROM chat_conversations c WHERE c.match_id=mt.id)",
        params![p.admin, uid(p), now()],
    )?;
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
pub(crate) fn writable(db: &Connection, id: &str) -> AppResult<bool> {
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

fn listed_participant(row: &rusqlite::Row<'_>, prefix: &str) -> rusqlite::Result<Value> {
    let id: String = row.get(format!("{prefix}_id").as_str())?;
    let avatar: String = row.get(format!("{prefix}_avatar").as_str())?;
    Ok(json!({
        "id":id,
        "name":row.get::<_,String>(format!("{prefix}_name").as_str())?,
        "role":row.get::<_,String>(format!("{prefix}_role").as_str())?,
        "avatar":if avatar.is_empty(){String::new()}else{format!("/api/avatars/{id}")}
    }))
}

fn listed_thread(row: &rusqlite::Row<'_>, admin: bool) -> rusqlite::Result<Value> {
    let kind: String = row.get("kind")?;
    let participants = if kind == "platform" {
        vec![
            listed_participant(row, "platform")?,
            json!({"id":"platform","name":"HireFerry · 聘渡","role":"platform","avatar":""}),
        ]
    } else {
        vec![
            listed_participant(row, "seeker")?,
            listed_participant(row, "employer")?,
        ]
    };
    let title = if kind == "platform" {
        if admin {
            participants[0]["name"].as_str().unwrap_or("").to_owned()
        } else {
            "聘渡 · 平台顾问".to_owned()
        }
    } else {
        row.get::<_, String>("job_title")?
    };
    let preview = row
        .get::<_, Option<String>>("last_message_body")?
        .unwrap_or_default();
    Ok(json!({
        "id":row.get::<_,String>("id")?,"kind":kind,"title":title,
        "user_id":row.get::<_,Option<String>>("user_id")?,
        "match_id":row.get::<_,Option<String>>("match_id")?,
        "job_id":row.get::<_,Option<String>>("job_id")?,
        "job_title":if kind=="matched"{Some(title.clone())}else{None},
        "participants":participants,
        "last_message_id":row.get::<_,Option<i64>>("last_message_id")?.unwrap_or(0),
        "last_message_preview":preview.chars().take(180).collect::<String>(),
        "last_message_at":row.get::<_,Option<i64>>("last_message_at")?,
        "unread_count":row.get::<_,i64>("unread_count")?,
        "writable":row.get::<_,bool>("writable")?,
        "created_at":row.get::<_,i64>("created_at")?,
        "updated_at":row.get::<_,i64>("updated_at")?
    }))
}

pub fn list_conversations(s: &AppState, p: &Principal) -> AppResult<Value> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_active(&tx, p)?;
    if !p.admin {
        ensure_thread(&tx, p, &format!("platform_{}", uid(p)))?;
    }
    ensure_matches(&tx, p)?;
    let expired = crate::chat_stream::expire_inactive(&tx)?;
    // Bound the authorized rooms before expanding message metadata. Materializing
    // this CTE prevents latest-message and unread lookups for discarded rooms.
    let sql = format!(
        "WITH recent AS MATERIALIZED (
           SELECT c.id {THREAD_FROM} WHERE {VISIBLE} ORDER BY c.updated_at DESC,c.id LIMIT ?3
         )
         SELECT c.id,c.kind,c.user_id,c.match_id,c.created_at,c.updated_at,
         pu.id AS platform_id,pu.name AS platform_name,pu.role AS platform_role,pu.avatar AS platform_avatar,
         sk.id AS seeker_id,sk.name AS seeker_name,sk.role AS seeker_role,sk.avatar AS seeker_avatar,
         er.id AS employer_id,er.name AS employer_name,er.role AS employer_role,er.avatar AS employer_avatar,
         j.id AS job_id,j.title AS job_title,
         last.id AS last_message_id,last.body AS last_message_body,last.created_at AS last_message_at,
         (SELECT COUNT(*) FROM chat_messages msg WHERE msg.conversation_id=c.id AND msg.sender_key<>?4
           AND msg.id>COALESCE((SELECT last_message_id FROM chat_reads WHERE conversation_id=c.id AND reader_key=?4),0)) AS unread_count,
         ((c.kind='platform' AND pu.banned=0) OR (c.kind='matched'
           AND mt.status IN ('introduced','interview','hired') AND j.status<>'deleted'
           AND sk.banned=0 AND er.banned=0)) AS writable
         {THREAD_FROM}
         JOIN recent ON recent.id=c.id
         LEFT JOIN chat_messages last ON last.id=(SELECT id FROM chat_messages WHERE conversation_id=c.id ORDER BY id DESC LIMIT 1)
         ORDER BY c.updated_at DESC,c.id"
    );
    let items = tx
        .prepare(&sql)?
        .query_map(
            params![
                p.admin,
                uid(p),
                s.cfg.limits.recent_list_limit,
                user_key(p)?
            ],
            |row| listed_thread(row, p.admin),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    access::ensure_active(&tx, p)?;
    let event_cursor = crate::chat_stream::cursor(&tx)?;
    tx.commit()?;
    drop(db);
    if expired {
        s.chat_notify.notify_waiters();
    }
    Ok(json!({"items":items,"poll_after_ms":POLL_MS,"event_cursor":event_cursor}))
}

/// Reads a known room independently of the bounded recent-conversations list.
/// Checks current membership before lazy creation, within the same transaction.
pub fn get_conversation(s: &AppState, p: &Principal, id: &str) -> AppResult<Value> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_thread(&tx, p, id)?;
    let result = thread_value(&tx, p, id)?;
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    Ok(result)
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

fn raw_message_row(r: &rusqlite::Row<'_>, admin_key: Option<&str>) -> rusqlite::Result<Value> {
    let kind: String = r.get("sender_kind")?;
    let body: String = r.get("body")?;
    let sender_name: String = r.get("sender_name")?;
    let via_ai: bool = r.get("via_ai")?;
    let mut value = json!({"id":r.get::<_,i64>("id")?,"conversation_id":r.get::<_,String>("conversation_id")?,"sender_kind":kind,"sender_id":r.get::<_,Option<String>>("sender_id")?,"sender_name":if kind=="platform"{"HireFerry · 聘渡".to_owned()}else{sender_name},"body":body,"format":r.get::<_,String>("format")?,"status":r.get::<_,String>("status")?,"sequence":r.get::<_,i64>("sequence")?,"client_id":r.get::<_,String>("client_id")?,"via_ai":via_ai,"created_at":r.get::<_,i64>("created_at")?});
    if let Some(viewer) = admin_key {
        let sender_key: String = r.get("sender_key")?;
        value["is_own"] = json!(kind == "platform" && sender_key == viewer);
        value["staff_name"] = if kind == "platform" {
            let name: Option<String> = r.get("staff_display_name")?;
            let fallback = if sender_key.starts_with("manager:") {
                "经理"
            } else if sender_key.starts_with("staff:") {
                "员工"
            } else if via_ai {
                "平台AI"
            } else {
                "超级管理员"
            };
            // A blank profile may historically have used the login email as
            // its display name. Chat attribution never exports that address.
            json!(
                name.as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty() && !name.contains('@'))
                    .unwrap_or(fallback)
            )
        } else {
            Value::Null
        };
    }
    Ok(value)
}
pub(crate) fn render_message(mut message: Value) -> Value {
    let body = message["body"].as_str().unwrap_or("");
    let escaped = || {
        body.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    message["body_html"] = Value::String(match message["format"].as_str().unwrap_or("markdown") {
        "plain" => format!("<p>{}</p>", escaped().replace('\n', "<br>")),
        "openui" => format!("<pre>{}</pre>", escaped()),
        _ => safety::markdown(body),
    });
    message
}
fn message_row(r: &rusqlite::Row<'_>, admin_key: Option<&str>) -> rusqlite::Result<Value> {
    raw_message_row(r, admin_key).map(render_message)
}
pub(crate) fn load_message(db: &Connection, p: &Principal, id: i64) -> AppResult<Value> {
    let key = p.admin.then(|| user_key(p)).transpose()?;
    Ok(db.query_row(
        &format!("SELECT {MESSAGE_FIELDS} FROM chat_messages msg {MESSAGE_JOINS} WHERE msg.id=?1"),
        [id],
        |r| message_row(r, key.as_deref()),
    )?)
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
    let expired = crate::chat_stream::expire_inactive(&tx)?;
    let backwards = after == -1;
    let (comparison, bound, ordering) = if backwards {
        ("<", before.unwrap_or(i64::MAX), "DESC")
    } else {
        (">", after, "ASC")
    };
    let sql = format!(
        "SELECT {MESSAGE_FIELDS} FROM chat_messages msg {MESSAGE_JOINS} WHERE msg.conversation_id=?1 AND msg.id {comparison} ?2 ORDER BY msg.id {ordering} LIMIT ?3"
    );
    let key = p.admin.then(|| user_key(p)).transpose()?;
    let mut items = tx
        .prepare(&sql)?
        .query_map(params![id, bound, count + 1], |r| {
            raw_message_row(r, key.as_deref())
        })?
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
    let event_cursor = crate::chat_stream::cursor(&tx)?;
    tx.commit()?;
    // The authorized snapshot owns its strings. Markdown parsing and sanitizing
    // must not hold the shared database mutex or its write transaction.
    drop(db);
    if expired {
        s.chat_notify.notify_waiters();
    }
    let items = items.into_iter().map(render_message).collect::<Vec<_>>();
    Ok(
        json!({"items":items,"next_cursor":cursor,"first_cursor":first,"has_more":!backwards&&extra,"has_older":older,"poll_after_ms":POLL_MS,"event_cursor":event_cursor}),
    )
}

pub fn send_message(
    s: &AppState,
    p: &Principal,
    id: &str,
    input: MessageInput,
) -> AppResult<Value> {
    send_formatted_message(
        s,
        p,
        id,
        FormattedMessageInput {
            body: input.body,
            client_id: input.client_id,
            format: crate::chat_stream::default_format(),
        },
    )
}

pub fn send_formatted_message(
    s: &AppState,
    p: &Principal,
    id: &str,
    input: FormattedMessageInput,
) -> AppResult<Value> {
    crate::chat_stream::validate_format(&input.format)?;
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
    let previous: Option<(i64,String,String,String)>=tx.query_row("SELECT id,body,format,status FROM chat_messages WHERE conversation_id=?1 AND sender_key=?2 AND client_id=?3",params![id,key,input.client_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let result = if let Some((message_id, body, format, status)) = previous {
        if body != input.body || format != input.format || status != "complete" {
            return Err(AppError::conflict(
                "This client ID already belongs to another message",
            ));
        }
        load_message(&tx, p, message_id)?
    } else {
        let kind = if p.admin { "platform" } else { "user" };
        let sender = if p.admin { None } else { p.user_id.as_deref() };
        tx.execute("INSERT INTO chat_messages(conversation_id,sender_kind,sender_id,sender_key,sender_actor,body,client_id,via_ai,created_at,format) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![id,kind,sender,key,p.actor,input.body,input.client_id,p.via_ai,now(),input.format])?;
        let message_id = tx.last_insert_rowid();
        crate::chat_stream::record_event(
            &tx,
            id,
            message_id,
            "message.created",
            0,
            Some(&input.body),
            None,
        )?;
        tx.execute(
            "UPDATE chat_conversations SET updated_at=?2 WHERE id=?1",
            params![id, now()],
        )?;
        crate::chat_ai::enqueue_tx(
            &tx,
            id,
            message_id,
            !p.admin && p.user_id.is_some(),
            &s.cfg.chat.auto_reply,
        )?;
        audit(
            &tx,
            &p.actor,
            "chat_message",
            id,
            &json!({"message_id":message_id,"via_ai":p.via_ai}).to_string(),
        )?;
        load_message(&tx, p, message_id)?
    };
    access::ensure_active(&tx, p)?;
    tx.commit()?;
    drop(db);
    s.chat_notify.notify_waiters();
    Ok(result)
}

/// Internal platform-AI sender used by the durable early-stage worker. It
/// intentionally accepts only platform conversations and never exposes a new
/// authorization path to HTTP, MCP, or browser callers.
pub(crate) fn send_system_ai(
    s: &AppState,
    id: &str,
    client_id: &str,
    body: &str,
    format: &str,
) -> AppResult<Value> {
    crate::chat_stream::validate_format(format)?;
    text(body, 10_000, "Message")?;
    text(client_id, 80, "Message client ID")?;
    if body.trim().is_empty() || client_id.trim().is_empty() {
        return Err(AppError::bad_request(
            "An AI message and client ID are required",
        ));
    }
    let system = Principal {
        user_id: None,
        admin: true,
        via_ai: true,
        actor: "system_ai".into(),
    };
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let active: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM chat_conversations c JOIN users u ON u.id=c.user_id WHERE c.id=?1 AND c.kind='platform' AND u.banned=0)",
        [id],
        |row| row.get(0),
    )?;
    if !active || !writable(&tx, id)? {
        return Err(AppError::not_found(
            "Platform conversation is no longer available",
        ));
    }
    let previous: Option<(i64, String, String, String)> = tx
        .query_row(
            "SELECT id,body,format,status FROM chat_messages WHERE conversation_id=?1 AND sender_key='system_ai' AND client_id=?2",
            params![id, client_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let result = if let Some((message_id, previous_body, previous_format, status)) = previous {
        if previous_body != body || previous_format != format || status != "complete" {
            return Err(AppError::conflict(
                "This AI reply client ID already belongs to another message",
            ));
        }
        load_message(&tx, &system, message_id)?
    } else {
        tx.execute(
            "INSERT INTO chat_messages(conversation_id,sender_kind,sender_id,sender_key,sender_actor,body,client_id,via_ai,created_at,format) VALUES(?1,'platform',NULL,'system_ai','system_ai',?2,?3,1,?4,?5)",
            params![id, body, client_id, now(), format],
        )?;
        let message_id = tx.last_insert_rowid();
        crate::chat_stream::record_event(
            &tx,
            id,
            message_id,
            "message.created",
            0,
            Some(body),
            None,
        )?;
        tx.execute(
            "UPDATE chat_conversations SET updated_at=?2 WHERE id=?1",
            params![id, now()],
        )?;
        audit(
            &tx,
            "system_ai",
            "chat_auto_reply",
            id,
            &json!({"message_id":message_id,"format":format}).to_string(),
        )?;
        load_message(&tx, &system, message_id)?
    };
    tx.commit()?;
    drop(db);
    s.chat_notify.notify_waiters();
    Ok(result)
}

fn openui_quote(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Send a bounded, clickable role card to a seeker from the staff chat. The
/// card never grants profile access by itself: an existing introduction opens
/// its room, otherwise the button prepares a reviewed request in the platform
/// conversation.
pub(crate) fn send_job_card(
    s: &AppState,
    p: &Principal,
    conversation: &str,
    input: JobCardInput,
) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    let (user_id, role): (String, String) = db
        .query_row(
            "SELECT c.user_id,u.role FROM chat_conversations c JOIN users u ON u.id=c.user_id WHERE c.id=?1 AND c.kind='platform' AND u.banned=0",
            [conversation],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("Platform conversation is no longer available"))?;
    if role != "seeker" {
        return Err(AppError::bad_request(
            "Job cards can only be recommended to job seekers",
        ));
    }
    let job = crate::models::load_job(&db, &input.job_id)?;
    if job.status != "published" || job.headcount <= 0 {
        return Err(AppError::bad_request(
            "Only open published roles can be sent as job cards",
        ));
    }
    let employer = crate::models::load_user(&db, &job.owner_id)?;
    if employer.banned {
        return Err(AppError::bad_request(
            "Only open published roles can be sent as job cards",
        ));
    }
    let match_id: Option<String> = db
        .query_row(
            "SELECT id FROM matches WHERE seeker_id=?1 AND job_id=?2 AND status IN ('introduced','interview','hired')",
            params![user_id, job.id],
            |row| row.get(0),
        )
        .optional()?;
    let origin = s.cfg.public_url.trim_end_matches('/');
    let detail_url = format!("{origin}/discover/jobs/{}", job.id);
    let intro = match_id.map(|id| format!("{origin}/app?tab=messages&conversation=match_{id}"));
    let title = openui_quote(&job.title);
    let employer_name = openui_quote(employer.public_employer_name().unwrap_or("公开招聘方"));
    let salary = crate::payments::minor_to_decimal(job.salary_cents, &job.currency)
        .ok_or_else(|| AppError::bad_request("Invalid job salary amount or currency"))?;
    let salary = format!("{} {salary}", job.currency);
    let excerpt = openui_quote(
        &job.jd_md
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(280)
            .collect::<String>(),
    );
    let action_text = openui_quote(&format!("我想了解这个岗位：{}", job.title));
    let action = if let Some(url) = intro {
        format!("Link(\"进入匹配会话\", \"{}\")", openui_quote(&url))
    } else {
        format!(
            "Button(\"发起对话\", @ToAssistant(\"{}\"), \"solid\", \"primary\", \"md\", \"message\")",
            action_text
        )
    };
    let body = format!(
        "root = Col([card])\ncard = Card([eyebrow, title, meta, excerpt, actions], \"outline\", 5, 4)\neyebrow = Badge(\"公开岗位 · 正在招聘\", \"success\")\ntitle = Title(\"{}\", \"lg\")\nmeta = Text(\"{} · {} / 月\", \"sm\", \"muted\", \"medium\")\nexcerpt = Markdown(\"{}\")\nactions = Row([details, contact], 3, \"start\", \"start\", true)\ndetails = Link(\"查看岗位详情\", \"{}\")\ncontact = {}",
        title,
        employer_name,
        salary,
        excerpt,
        openui_quote(&detail_url),
        action
    );
    drop(db);
    send_system_ai(
        s,
        conversation,
        &format!("job-card:{}:{}", job.id, uuid::Uuid::new_v4().simple()),
        &body,
        "openui",
    )
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
    // Every stored message already references a conversation. Creating empty
    // rooms cannot add any updates, and scanning all matches on each poll makes
    // a five-second read grow with the entire matching history.
    let sql = format!(
        "SELECT {MESSAGE_FIELDS} {THREAD_FROM} JOIN chat_messages msg ON msg.conversation_id=c.id {MESSAGE_JOINS} WHERE {VISIBLE} AND msg.id>?3 ORDER BY msg.id ASC LIMIT ?4"
    );
    let key = p.admin.then(|| user_key(p)).transpose()?;
    let mut items = tx
        .prepare(&sql)?
        .query_map(params![p.admin, uid(p), after, count + 1], |r| {
            message_row(r, key.as_deref())
        })?
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
    let currency: String = r.get("salary_currency")?;
    Ok(
        json!({"user_id":r.get::<_,String>("user_id")?,"user_name":r.get::<_,String>("user_name")?,"banned":r.get::<_,bool>("user_banned")?,"target_industry":r.get::<_,String>("target_industry")?,"target_roles":r.get::<_,String>("target_roles")?,"salary_min_cents":r.get::<_,Option<i64>>("salary_min_cents")?,"salary_max_cents":r.get::<_,Option<i64>>("salary_max_cents")?,"salary_currency":currency,"currency_decimals":crate::config::currency_decimals(&currency),"location":r.get::<_,String>("location")?,"preferences":r.get::<_,String>("preferences")?,"status":r.get::<_,String>("status")?,"allow_external":r.get::<_,bool>("allow_external")?,"consented_at":r.get::<_,Option<i64>>("consented_at")?,"consented_via_ai":r.get::<_,bool>("consented_via_ai")?,"via_ai":r.get::<_,bool>("via_ai")?,"created_at":r.get::<_,i64>("created_at")?,"updated_at":r.get::<_,i64>("updated_at")?}),
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
        return Err(AppError::bad_request(
            "Invalid salary range in minor currency units",
        ));
    }
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let user = target_seeker(&tx, p, target)?;
    let old = stored_mandate(&tx, &user)?;
    let salary_currency = input.salary_currency.as_deref().unwrap_or_else(|| {
        old.as_ref()
            .and_then(|v| v["salary_currency"].as_str())
            .unwrap_or(&s.cfg.pricing.currency)
    });
    if crate::config::currency_decimals(salary_currency).is_none()
        || (input.salary_currency.is_some() && !s.cfg.pricing.permits_currency(salary_currency))
    {
        return Err(AppError::bad_request("Choose a configured salary currency"));
    }
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
    tx.execute("INSERT INTO mandates(user_id,target_industry,target_roles,salary_min_cents,salary_max_cents,salary_currency,location,preferences,status,allow_external,consented_at,consented_via_ai,via_ai,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?14,?6,?7,?8,?9,?10,?11,?12,?13,?13) ON CONFLICT(user_id) DO UPDATE SET target_industry=excluded.target_industry,target_roles=excluded.target_roles,salary_min_cents=excluded.salary_min_cents,salary_max_cents=excluded.salary_max_cents,salary_currency=excluded.salary_currency,location=excluded.location,preferences=excluded.preferences,status=excluded.status,allow_external=excluded.allow_external,consented_at=excluded.consented_at,consented_via_ai=excluded.consented_via_ai,via_ai=excluded.via_ai,updated_at=excluded.updated_at",params![user,input.target_industry.trim(),input.target_roles.trim(),input.salary_min_cents,input.salary_max_cents,input.location.trim(),input.preferences,input.status,input.allow_external,consent,consent_ai,p.via_ai,now(),salary_currency])?;
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
    if !(1..=s.cfg.limits.list_max_page).contains(&page) {
        return Err(AppError::bad_request("Invalid page"));
    }
    let page_size = s.cfg.limits.list_page_size;
    let filter = "FROM mandates m JOIN users u ON u.id=m.user_id WHERE u.role='seeker' AND (?1 IS NULL OR m.status=?1) AND (u.name LIKE ?2 OR m.target_industry LIKE ?2 OR m.target_roles LIKE ?2 OR m.location LIKE ?2)";
    let search = format!("%{}%", q.trim());
    let total: i64 = db.query_row(
        &format!("SELECT count(*) {filter}"),
        params![status, search],
        |r| r.get(0),
    )?;
    let items=db.prepare(&format!("SELECT m.*,u.name AS user_name,u.banned AS user_banned {filter} ORDER BY m.updated_at DESC,m.user_id LIMIT ?3 OFFSET ?4"))?.query_map(params![status,search,page_size,(page-1)*page_size],mandate_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"items":items,"total":total,"page":page,"page_size":page_size}))
}

fn outreach_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,String>("id")?,"user_id":r.get::<_,String>("user_id")?,"banned":r.get::<_,bool>("user_banned")?,"channel":r.get::<_,String>("channel")?,"opportunity":r.get::<_,String>("opportunity")?,"source_url":r.get::<_,String>("source_url")?,"stage":r.get::<_,String>("stage")?,"note":r.get::<_,String>("note")?,"via_ai":r.get::<_,bool>("via_ai")?,"created_at":r.get::<_,i64>("created_at")?,"updated_at":r.get::<_,i64>("updated_at")?}),
    )
}
pub fn list_outreach(s: &AppState, p: &Principal, target: Option<&str>) -> AppResult<Value> {
    let db = s.conn()?;
    let user = target_seeker(&db, p, target)?;
    let items=db.prepare("SELECT o.*,u.banned AS user_banned FROM outreach o JOIN users u ON u.id=o.user_id WHERE o.user_id=?1 ORDER BY o.updated_at DESC,o.id LIMIT ?2")?.query_map(params![user,s.cfg.limits.recent_list_limit],outreach_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
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
    if let Some(id) = record_id
        && !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM outreach WHERE id=?1 AND user_id=?2)",
            params![id, user],
            |r| r.get::<_, bool>(0),
        )?
    {
        return Err(AppError::not_found("Progress record not found"));
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
async fn http_conversation(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(get_conversation(&s, &p, &id)?))
}
async fn http_platform(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(i): Json<PlatformInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(platform_conversation(&s, &p, i.user_id.as_deref())?))
}
async fn http_job_card(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(i): Json<JobCardInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(send_job_card(&s, &p, &id, i)?))
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
    Json(i): Json<FormattedMessageInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(send_formatted_message(&s, &p, &id, i)?))
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
