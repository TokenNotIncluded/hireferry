//! Bounded plain-text reports. Retention never evicts an unexpired report to
//! make room, and persistent abuse controls survive cleanup and restarts.
use crate::{AppError, AppResult, AppState, access, auth, cli_auth, config::Feedback, db, now};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get as route_get, post},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const MAX_MESSAGE_CHARS: usize = 2000;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/feedback", post(submit))
        .route("/api/admin/feedback", route_get(http_list))
        .route(
            "/api/admin/feedback/{id}",
            route_get(http_get).patch(http_update),
        )
        .layer(DefaultBodyLimit::max(16 * 1024))
}

pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS feedback (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            actor_key TEXT NOT NULL,
            actor_kind TEXT NOT NULL CHECK(actor_kind IN ('guest','member','admin')),
            user_id TEXT,
            source TEXT NOT NULL CHECK(source IN ('web','cli')),
            category TEXT NOT NULL CHECK(category IN ('bug','idea','other')),
            message TEXT NOT NULL CHECK(length(message) BETWEEN 1 AND 8000),
            page TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open','resolved')),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS feedback_actor_time ON feedback(actor_key,created_at);
        CREATE INDEX IF NOT EXISTS feedback_status_id ON feedback(status,id);
        CREATE INDEX IF NOT EXISTS feedback_created ON feedback(status,created_at);
        CREATE INDEX IF NOT EXISTS feedback_resolved ON feedback(status,updated_at);
        CREATE TABLE IF NOT EXISTS feedback_limits (
            actor_key TEXT PRIMARY KEY,
            window_start INTEGER NOT NULL,
            submissions INTEGER NOT NULL CHECK(submissions>=0)
        );
        CREATE INDEX IF NOT EXISTS feedback_limits_expiry ON feedback_limits(window_start);
        CREATE TABLE IF NOT EXISTS feedback_dedup (
            actor_key TEXT NOT NULL,
            fingerprint TEXT NOT NULL,
            expires_at INTEGER NOT NULL,
            PRIMARY KEY(actor_key,fingerprint)
        );
        CREATE INDEX IF NOT EXISTS feedback_dedup_expiry ON feedback_dedup(expires_at);",
    )?;
    Ok(())
}

/// One bounded pass. Reports expire from submission or the last actual status
/// change. Reopening restores the open window; repeating a status never renews it.
pub fn cleanup_batch(db: &Connection, limits: &Feedback, at: i64, batch: i64) -> AppResult<usize> {
    let batch = batch.clamp(1, 1000);
    let removed = db.execute(
        "DELETE FROM feedback WHERE id IN (
            SELECT id FROM feedback WHERE (status='open' AND created_at<?1 AND updated_at<?1)
                OR (status='resolved' AND updated_at<?2) ORDER BY id LIMIT ?3
        )",
        params![
            at.saturating_sub(limits.retention_days * 86_400),
            at.saturating_sub(limits.resolved_retention_days * 86_400),
            batch
        ],
    )?;
    // Only counters from completed windows can be removed. Deleting feedback
    // never resets an actor's current quota or duplicate protection.
    let counters = db.execute(
        "DELETE FROM feedback_limits WHERE actor_key IN (
            SELECT actor_key FROM feedback_limits WHERE window_start<?1
            ORDER BY window_start LIMIT ?2)",
        params![at - 86_400, batch],
    )?;
    let duplicates = db.execute(
        "DELETE FROM feedback_dedup WHERE rowid IN (
            SELECT rowid FROM feedback_dedup WHERE expires_at<=?1
            ORDER BY expires_at LIMIT ?2)",
        params![at, batch],
    )?;
    // The worker uses this maximum to schedule another small pass if any
    // table filled its batch, even when the other tables had no expired rows.
    Ok(removed.max(counters).max(duplicates))
}

pub fn cleanup(db: &Connection, limits: &Feedback, at: i64) -> AppResult<()> {
    cleanup_batch(db, limits, at, 200).map(|_| ())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitInput {
    category: String,
    message: String,
    #[serde(default)]
    page: String,
}

fn validate(input: &SubmitInput) -> AppResult<&str> {
    if !matches!(input.category.as_str(), "bug" | "idea" | "other") {
        return Err(AppError::bad_request("Choose bug, idea or other"));
    }
    let message = input.message.trim();
    if !(2..=MAX_MESSAGE_CHARS).contains(&message.chars().count())
        || message.len() > 8000
        || message
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(AppError::bad_request(
            "Feedback must be 2–2000 characters of plain text",
        ));
    }
    if !input.page.is_empty()
        && (input.page.len() > 160
            || !input.page.starts_with('/')
            || input.page.starts_with("//")
            || input
                .page
                .chars()
                .any(|c| matches!(c, '?' | '#' | '\\') || c.is_control()))
    {
        return Err(AppError::bad_request("Only a local page path is allowed"));
    }
    Ok(message)
}

async fn submit(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<SubmitInput>,
) -> AppResult<(StatusCode, Json<Value>)> {
    if !s.cfg.feedback.enabled {
        return Err(AppError::unavailable("Feedback is temporarily unavailable"));
    }
    let message = validate(&input)?;
    // Guests can report login failures. A browser session requires CSRF; a
    // bearer must be a genuine CLI identity, never an ordinary MCP token.
    let session = if headers.get("authorization").is_none() {
        auth::origin_check(&s, &headers)?;
        let session = auth::session(&s, &headers)?;
        if let Some(session) = &session {
            if session.bootstrap {
                return Err(AppError::forbidden("Finish administrator enrollment first"));
            }
            auth::csrf_check(&s, &headers, session)?;
        }
        session
    } else {
        None
    };
    let mut db = s.conn()?;
    if let Some(session) = &session {
        ensure_browser_session(&db, session)?;
    }
    let (actor_key, actor_kind, user_id, source) = if let Some(value) = headers.get("authorization")
    {
        let raw = value
            .to_str()
            .ok()
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| AppError::unauthorized("Invalid bearer token"))?;
        let (admin, uid) = cli_auth::feedback_identity(&db, &s.cfg, raw)?;
        if admin {
            ("admin".to_owned(), "admin", None, "cli")
        } else {
            let uid = uid.ok_or_else(|| AppError::unauthorized("Account unavailable"))?;
            (format!("member:{uid}"), "member", Some(uid), "cli")
        }
    } else if let Some(session) = session {
        if session.admin {
            ("admin".to_owned(), "admin", None, "web")
        } else {
            let uid = session
                .user_id
                .ok_or_else(|| AppError::unauthorized("Account unavailable"))?;
            // Recheck under the write lock so a concurrent suspension cannot
            // publish with a browser identity checked before that suspension.
            access::ensure_active(
                &db,
                &access::Principal {
                    user_id: Some(uid.clone()),
                    admin: false,
                    via_ai: false,
                    actor: uid.clone(),
                },
            )?;
            (format!("member:{uid}"), "member", Some(uid), "web")
        }
    } else {
        ("guest".to_owned(), "guest", None, "web")
    };
    let at = now();
    cleanup_batch(&db, &s.cfg.feedback, at, s.cfg.maintenance.batch_size)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Page and source are deliberately excluded: retrying the same report
    // through another page or CLI must not create another row for this actor.
    let fingerprint = hex::encode(Sha256::digest(serde_json::to_vec(&(
        &input.category,
        message,
    ))?));
    let duplicate: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM feedback_dedup
            WHERE actor_key=?1 AND fingerprint=?2 AND expires_at>?3)
         OR EXISTS(SELECT 1 FROM feedback WHERE actor_key=?1 AND category=?4
            AND message=?5 AND created_at>?6)",
        params![
            actor_key,
            fingerprint,
            at,
            input.category,
            message,
            at - s.cfg.feedback.duplicate_window_seconds
        ],
        |row| row.get(0),
    )?;
    if duplicate {
        // In particular, do not disclose a different anonymous guest's ID.
        return Err(AppError::conflict("This feedback was already received"));
    }
    let window_start = at.div_euclid(3600) * 3600;
    let recent = tx
        .query_row(
            "SELECT submissions FROM feedback_limits WHERE actor_key=?1 AND window_start=?2",
            params![actor_key, window_start],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(tx.query_row(
            // Includes pre-upgrade reports when an actor first acquires a counter.
            "SELECT count(*) FROM feedback WHERE actor_key=?1 AND created_at>=?2",
            params![actor_key, window_start],
            |row| row.get(0),
        )?);
    let allowance = if actor_kind == "guest" {
        s.cfg.feedback.max_guest_per_hour
    } else {
        s.cfg.feedback.max_per_account_per_hour
    };
    if recent >= allowance {
        return Err(AppError {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: "Feedback limit reached; please try again later".into(),
        });
    }
    let count: i64 = tx.query_row("SELECT count(*) FROM feedback", [], |row| row.get(0))?;
    if count >= s.cfg.feedback.max_records {
        return Err(AppError::unavailable(
            "Feedback inbox is full. Please try again later.",
        ));
    }
    tx.execute(
        "INSERT INTO feedback(actor_key,actor_kind,user_id,source,category,message,page,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8)",
        params![actor_key, actor_kind, user_id, source, input.category, message, input.page, at],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO feedback_limits(actor_key,window_start,submissions) VALUES(?1,?2,?3)
         ON CONFLICT(actor_key) DO UPDATE SET window_start=excluded.window_start,submissions=excluded.submissions",
        params![actor_key, window_start, recent + 1],
    )?;
    tx.execute(
        "INSERT INTO feedback_dedup(actor_key,fingerprint,expires_at) VALUES(?1,?2,?3)
         ON CONFLICT(actor_key,fingerprint) DO UPDATE SET expires_at=excluded.expires_at",
        params![
            actor_key,
            fingerprint,
            at + s.cfg.feedback.duplicate_window_seconds
        ],
    )?;
    db::audit(
        &tx,
        &actor_key,
        "feedback_submitted",
        &id.to_string(),
        &json!({"source":source,"category":input.category}).to_string(),
    )?;
    tx.commit()?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":id,"retention_days":s.cfg.feedback.retention_days})),
    ))
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ListInput {
    pub limit: Option<i64>,
    pub before_id: Option<i64>,
    pub status: Option<String>,
    pub category: Option<String>,
    pub source: Option<String>,
}

fn record(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id":row.get::<_,i64>(0)?, "actor_kind":row.get::<_,String>(1)?,
        "user_id":row.get::<_,Option<String>>(2)?, "source":row.get::<_,String>(3)?,
        "category":row.get::<_,String>(4)?, "message":row.get::<_,String>(5)?,
        "page":row.get::<_,String>(6)?, "status":row.get::<_,String>(7)?,
        "created_at":row.get::<_,i64>(8)?, "updated_at":row.get::<_,i64>(9)?
    }))
}

pub fn list(s: &AppState, p: &access::Principal, input: ListInput) -> AppResult<Value> {
    let limit = input.limit.unwrap_or(25);
    if !(1..=100).contains(&limit)
        || input.before_id.is_some_and(|id| id <= 0)
        || input
            .status
            .as_deref()
            .is_some_and(|v| !matches!(v, "open" | "resolved"))
        || input
            .category
            .as_deref()
            .is_some_and(|v| !matches!(v, "bug" | "idea" | "other"))
        || input
            .source
            .as_deref()
            .is_some_and(|v| !matches!(v, "web" | "cli"))
    {
        return Err(AppError::bad_request(
            "Invalid feedback filters or page size",
        ));
    }
    let db = s.conn()?;
    ensure_admin(&db, p)?;
    let mut query = db.prepare(
        "SELECT id,actor_kind,user_id,source,category,message,page,status,created_at,updated_at
         FROM feedback WHERE (?1 IS NULL OR id<?1) AND (?2 IS NULL OR status=?2)
             AND (?3 IS NULL OR category=?3) AND (?4 IS NULL OR source=?4)
         ORDER BY id DESC LIMIT ?5",
    )?;
    let mut items = query
        .query_map(
            params![
                input.before_id,
                input.status,
                input.category,
                input.source,
                limit + 1
            ],
            record,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_before_id = if has_more {
        items.last().map(|v| v["id"].clone())
    } else {
        None
    };
    Ok(json!({"items":items,"next_before_id":next_before_id,
        "retention_days":s.cfg.feedback.retention_days,
        "resolved_retention_days":s.cfg.feedback.resolved_retention_days}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdInput {
    pub id: i64,
}

pub fn get(s: &AppState, p: &access::Principal, id: i64) -> AppResult<Value> {
    if id <= 0 {
        return Err(AppError::bad_request("Invalid feedback ID"));
    }
    let db = s.conn()?;
    ensure_admin(&db, p)?;
    db.query_row(
        "SELECT id,actor_kind,user_id,source,category,message,page,status,created_at,updated_at
         FROM feedback WHERE id=?1",
        [id],
        record,
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("Feedback not found or already expired"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateInput {
    pub status: String,
}

pub fn update(
    s: &AppState,
    p: &access::Principal,
    id: i64,
    input: UpdateInput,
) -> AppResult<Value> {
    if id <= 0 || !matches!(input.status.as_str(), "open" | "resolved") {
        return Err(AppError::bad_request("Invalid feedback ID or status"));
    }
    let mut db = s.conn()?;
    ensure_admin(&db, p)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let previous: String = tx
        .query_row("SELECT status FROM feedback WHERE id=?1", [id], |row| {
            row.get(0)
        })
        .optional()?
        .ok_or_else(|| AppError::not_found("Feedback not found or already expired"))?;
    if previous != input.status {
        tx.execute(
            "UPDATE feedback SET status=?2,updated_at=?3 WHERE id=?1",
            params![id, input.status, now()],
        )?;
        db::audit(
            &tx,
            &p.actor,
            "feedback_status_changed",
            &id.to_string(),
            &json!({"from":previous,"to":input.status}).to_string(),
        )?;
    }
    tx.commit()?;
    Ok(json!({"id":id,"status":input.status}))
}

fn ensure_browser_session(db: &Connection, session: &auth::Session) -> AppResult<()> {
    let active: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=?1 AND user_id IS ?2
            AND admin=?3 AND bootstrap=0 AND expires_at>?4)",
        params![session.token_hash, session.user_id, session.admin, now()],
        |r| r.get(0),
    )?;
    if !active {
        return Err(AppError::unauthorized("Session expired or revoked"));
    }
    Ok(())
}

fn ensure_admin(db: &Connection, p: &access::Principal) -> AppResult<()> {
    access::ensure_admin(db, p)?;
    if let Some(hash) = p.actor.strip_prefix("admin_session:") {
        let active: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=?1 AND admin=1
                AND bootstrap=0 AND user_id IS NULL AND expires_at>?2)",
            params![hash, now()],
            |r| r.get(0),
        )?;
        if !active {
            return Err(AppError::unauthorized(
                "Administrator session expired or revoked",
            ));
        }
    }
    Ok(())
}

fn http_principal(s: &AppState, h: &HeaderMap, write: bool) -> AppResult<access::Principal> {
    let mut principal = access::principal(s, h, write)?;
    if h.get("authorization").is_none() && principal.admin && !principal.via_ai {
        let session = auth::require_admin(s, h)?;
        principal.actor = format!("admin_session:{}", session.token_hash);
    }
    Ok(principal)
}

async fn http_list(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(input): Query<ListInput>,
) -> AppResult<Json<Value>> {
    let p = http_principal(&s, &h, false)?;
    Ok(Json(list(&s, &p, input)?))
}

async fn http_get(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let p = http_principal(&s, &h, false)?;
    Ok(Json(get(&s, &p, id)?))
}

async fn http_update(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<UpdateInput>,
) -> AppResult<Json<Value>> {
    let p = http_principal(&s, &h, true)?;
    Ok(Json(update(&s, &p, id, input)?))
}
