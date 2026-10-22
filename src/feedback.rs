//! Brief user reports with strict input bounds, rate limits and finite storage.
//! No IP address, user-agent, URL query or authentication secret is stored.
use crate::{AppError, AppResult, AppState, access, auth, cli_auth, config::Feedback, now};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch, post},
};
use rusqlite::{Connection, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_MESSAGE_CHARS: usize = 2000;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/feedback", post(submit))
        .route("/api/admin/feedback", get(http_list))
        .route("/api/admin/feedback/{id}", patch(http_update))
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
        CREATE INDEX IF NOT EXISTS feedback_status_id ON feedback(status,id);",
    )?;
    Ok(())
}

/// Both age and absolute row count are enforced. Called on every write/read,
/// and by the hourly housekeeping worker even when nobody sends feedback.
pub fn cleanup(db: &Connection, limits: &Feedback, at: i64) -> AppResult<()> {
    db.execute(
        "DELETE FROM feedback WHERE created_at<?1",
        [at.saturating_sub(limits.retention_days * 86_400)],
    )?;
    db.execute(
        "DELETE FROM feedback WHERE id IN
         (SELECT id FROM feedback ORDER BY id DESC LIMIT -1 OFFSET ?1)",
        [limits.max_records],
    )?;
    Ok(())
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
    if message.chars().count() < 2
        || message.chars().count() > MAX_MESSAGE_CHARS
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
                .bytes()
                .any(|c| matches!(c, b'?' | b'#' | b'\\') || c < 32))
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
    // Guest reports work on login/error pages; a valid browser session needs
    // its CSRF token. Bearer tokens must be genuine CLI identity grants.
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
            (format!("member:{uid}"), "member", Some(uid), "web")
        }
    } else {
        ("guest".to_owned(), "guest", None, "web")
    };
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let at = now();
    cleanup(&tx, &s.cfg.feedback, at)?;
    let recent: i64 = tx.query_row(
        "SELECT count(*) FROM feedback WHERE actor_key=?1 AND created_at>?2",
        params![actor_key, at - 3600],
        |row| row.get(0),
    )?;
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
    tx.execute(
        "INSERT INTO feedback(actor_key,actor_kind,user_id,source,category,message,page,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8)",
        params![actor_key, actor_kind, user_id, source, input.category, message, input.page, at],
    )?;
    let id = tx.last_insert_rowid();
    cleanup(&tx, &s.cfg.feedback, at)?;
    tx.commit()?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "retention_days": s.cfg.feedback.retention_days})),
    ))
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ListInput {
    pub limit: Option<i64>,
    pub before_id: Option<i64>,
    pub status: Option<String>,
}

pub fn list(s: &AppState, p: &access::Principal, input: ListInput) -> AppResult<Value> {
    let limit = input.limit.unwrap_or(25);
    if !(1..=100).contains(&limit)
        || input.before_id.is_some_and(|id| id <= 0)
        || input
            .status
            .as_deref()
            .is_some_and(|v| !matches!(v, "open" | "resolved"))
    {
        return Err(AppError::bad_request(
            "Invalid feedback filters or page size",
        ));
    }
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    cleanup(&db, &s.cfg.feedback, now())?;
    let mut query = db.prepare(
        "SELECT id,actor_kind,user_id,source,category,message,page,status,created_at,updated_at
         FROM feedback WHERE (?1 IS NULL OR id<?1) AND (?2 IS NULL OR status=?2)
         ORDER BY id DESC LIMIT ?3",
    )?;
    let mut items = query
        .query_map(params![input.before_id, input.status, limit + 1], |row| {
            Ok(json!({
                "id":row.get::<_,i64>(0)?, "actor_kind":row.get::<_,String>(1)?,
                "user_id":row.get::<_,Option<String>>(2)?, "source":row.get::<_,String>(3)?,
                "category":row.get::<_,String>(4)?, "message":row.get::<_,String>(5)?,
                "page":row.get::<_,String>(6)?, "status":row.get::<_,String>(7)?,
                "created_at":row.get::<_,i64>(8)?, "updated_at":row.get::<_,i64>(9)?
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_before_id = if has_more {
        items.last().map(|v| v["id"].clone())
    } else {
        None
    };
    Ok(
        json!({"items":items,"next_before_id":next_before_id,"retention_days":s.cfg.feedback.retention_days}),
    )
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
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    cleanup(&db, &s.cfg.feedback, now())?;
    let changed = db.execute(
        "UPDATE feedback SET status=?2,updated_at=?3 WHERE id=?1",
        params![id, input.status, now()],
    )?;
    if changed == 0 {
        return Err(AppError::not_found("Feedback not found or already expired"));
    }
    Ok(json!({"id":id,"status":input.status}))
}

async fn http_list(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(input): Query<ListInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(list(&s, &p, input)?))
}

async fn http_update(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<UpdateInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(update(&s, &p, id, input)?))
}
