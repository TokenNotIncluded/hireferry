//! Configurable platform announcements and durable read confirmations.
//!
//! An announcement is a small, operator-managed piece of product copy. Its
//! version changes whenever the content or delivery policy changes, so a read
//! confirmation always belongs to the exact announcement the user saw.

use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    routing::{get, post},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppError, AppResult, AppState, access, auth, db::audit, now, safety};

pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS platform_announcements(
    id INTEGER PRIMARY KEY CHECK(id=1),
    enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
    title TEXT NOT NULL DEFAULT '平台公告',
    title_en TEXT NOT NULL DEFAULT 'Platform announcement',
    body_md TEXT NOT NULL DEFAULT '',
    body_md_en TEXT NOT NULL DEFAULT '',
    require_ack INTEGER NOT NULL DEFAULT 0 CHECK(require_ack IN (0,1)),
    version INTEGER NOT NULL DEFAULT 1 CHECK(version>0),
    updated_at INTEGER NOT NULL,
    updated_by TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS platform_announcement_reads(
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    announcement_version INTEGER NOT NULL CHECK(announcement_version>0),
    read_at INTEGER NOT NULL,
    PRIMARY KEY(user_id,announcement_version)
);
CREATE INDEX IF NOT EXISTS platform_announcement_reads_user
    ON platform_announcement_reads(user_id,announcement_version);
INSERT OR IGNORE INTO platform_announcements
    (id,enabled,title,title_en,body_md,body_md_en,require_ack,version,updated_at,updated_by)
    VALUES (1,0,'平台公告','Platform announcement',
            '欢迎来到聘渡。平台公告会在这里更新重要信息。',
            'Welcome to HireFerry. Important platform updates will appear here.',
            0,1,0,'system');
"#,
    )?;
    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/announcement", get(user_announcement))
        .route("/api/announcement/ack", post(acknowledge))
        .route(
            "/api/admin/announcement",
            get(admin_announcement).put(update_announcement),
        )
}

#[derive(Clone, Debug)]
struct Record {
    enabled: bool,
    title: String,
    title_en: String,
    body_md: String,
    body_md_en: String,
    require_ack: bool,
    version: i64,
    updated_at: i64,
    updated_by: String,
}

impl Record {
    fn value(&self, include_private: bool) -> Value {
        let mut value = json!({
            "enabled": self.enabled,
            "title": self.title,
            "title_en": self.title_en,
            "body_md": self.body_md,
            "body_md_en": self.body_md_en,
            "body_html": safety::markdown(&self.body_md),
            "body_html_en": safety::markdown(&self.body_md_en),
            "require_ack": self.require_ack,
            "version": self.version,
        });
        if include_private {
            value["updated_at"] = json!(self.updated_at);
            value["updated_by"] = json!(self.updated_by);
        }
        value
    }
}

fn read_record(db: &Connection) -> AppResult<Record> {
    db.query_row(
        "SELECT enabled,title,title_en,body_md,body_md_en,require_ack,version,updated_at,updated_by FROM platform_announcements WHERE id=1",
        [],
        |row| {
            Ok(Record {
                enabled: row.get::<_, i64>(0)? != 0,
                title: row.get(1)?,
                title_en: row.get(2)?,
                body_md: row.get(3)?,
                body_md_en: row.get(4)?,
                require_ack: row.get::<_, i64>(5)? != 0,
                version: row.get(6)?,
                updated_at: row.get(7)?,
                updated_by: row.get(8)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| AppError::internal("Platform announcement is missing"))
}

fn validate_text(value: &str, max: usize, label: &str, required: bool) -> AppResult<()> {
    safety::bounded_text(value, max, label)?;
    if required && value.trim().is_empty() {
        return Err(AppError::bad_request(format!("{label} cannot be empty")));
    }
    if value
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(AppError::bad_request(format!(
            "{label} contains invalid control characters"
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateInput {
    pub enabled: bool,
    pub title: String,
    pub title_en: String,
    pub body_md: String,
    pub body_md_en: String,
    pub require_ack: bool,
    pub expected_version: i64,
}

fn validate_update(input: &UpdateInput) -> AppResult<()> {
    validate_text(&input.title, 160, "Announcement title", true)?;
    validate_text(&input.title_en, 160, "English announcement title", true)?;
    validate_text(&input.body_md, 8000, "Announcement body", true)?;
    validate_text(&input.body_md_en, 8000, "English announcement body", true)?;
    if input.expected_version < 1 {
        return Err(AppError::bad_request("Announcement version is invalid"));
    }
    Ok(())
}

fn admin_data(s: &AppState, p: &access::Principal) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_operator(&db, p)?;
    Ok(json!({"announcement": read_record(&db)?.value(true)}))
}

async fn admin_announcement(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(admin_data(&s, &p)?))
}

async fn update_announcement(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<UpdateInput>,
) -> AppResult<Json<Value>> {
    validate_update(&input)?;
    let p = access::principal(&s, &h, true)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_operator(&tx, &p)?;
    let current = read_record(&tx)?;
    if current.version != input.expected_version {
        return Err(AppError::conflict(
            "Announcement changed; refresh and try again",
        ));
    }
    let version = current.version.saturating_add(1);
    let updated_at = now();
    tx.execute(
        "UPDATE platform_announcements SET enabled=?1,title=?2,title_en=?3,body_md=?4,body_md_en=?5,require_ack=?6,version=?7,updated_at=?8,updated_by=?9 WHERE id=1",
        params![
            input.enabled,
            input.title.trim(),
            input.title_en.trim(),
            input.body_md.trim(),
            input.body_md_en.trim(),
            input.require_ack,
            version,
            updated_at,
            p.actor,
        ],
    )?;
    audit(
        &tx,
        &p.actor,
        "platform_announcement_update",
        "platform_announcement",
        &json!({
            "from_version": current.version,
            "to_version": version,
            "enabled": input.enabled,
            "require_ack": input.require_ack,
        })
        .to_string(),
    )?;
    let result = json!({"announcement": read_record(&tx)?.value(true)});
    tx.commit()?;
    Ok(Json(result))
}

async fn user_announcement(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let uid = auth::require_user(&s, &h)?;
    let db = s.conn()?;
    let announcement = read_record(&db)?;
    if !announcement.enabled {
        return Ok(Json(json!({"announcement": null})));
    }
    let acknowledged: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM platform_announcement_reads WHERE user_id=?1 AND announcement_version=?2)",
        params![uid, announcement.version],
        |row| row.get(0),
    )?;
    Ok(Json(json!({
        "announcement": announcement.value(false),
        "acknowledged": acknowledged,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AckInput {
    version: i64,
}

async fn acknowledge(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<AckInput>,
) -> AppResult<Json<Value>> {
    let uid = auth::require_user(&s, &h)?;
    let session = auth::require_session(&s, &h)?;
    auth::csrf_check(&s, &h, &session)?;
    if input.version < 1 {
        return Err(AppError::bad_request("Announcement version is invalid"));
    }
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let announcement = read_record(&tx)?;
    if !announcement.enabled {
        return Err(AppError::not_found("There is no active announcement"));
    }
    if announcement.version != input.version {
        return Err(AppError::conflict(
            "Announcement changed; reload before confirming",
        ));
    }
    tx.execute(
        "INSERT INTO platform_announcement_reads(user_id,announcement_version,read_at) VALUES(?1,?2,?3) ON CONFLICT(user_id,announcement_version) DO UPDATE SET read_at=excluded.read_at",
        params![uid, input.version, now()],
    )?;
    tx.commit()?;
    Ok(Json(
        json!({"acknowledged": true, "version": input.version}),
    ))
}
