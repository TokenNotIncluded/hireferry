//! Administrator automation credentials. Issuance and revocation are only
//! available to an interactive, Passkey-authenticated administrator session.
use crate::{AppError, AppResult, AppState, auth, db::audit, now, safety};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get},
};
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};

const TOKEN_TTL: i64 = 30 * 24 * 3600;
const RECENT_AUTH_SECONDS: i64 = 600;
const MAX_ACTIVE_TOKENS: i64 = 10;

pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS admin_api_tokens(
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            token_hash TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            last_used_at INTEGER,
            revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1))
        );
        CREATE INDEX IF NOT EXISTS admin_tokens_expiry ON admin_api_tokens(expires_at);",
    )?;
    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/tokens", get(list).post(create))
        .route("/api/admin/tokens/{id}", delete(revoke))
}

/// Resolve only the administrator namespace. A member token is never promoted.
pub fn verify_db(db: &Connection, raw: &str) -> AppResult<Option<String>> {
    let Some(secret) = raw.strip_prefix("hf_admin_") else {
        return Ok(None);
    };
    if secret.len() != 64 || !secret.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let t = now();
    let id: Option<String> = db
        .query_row(
            "SELECT id FROM admin_api_tokens WHERE token_hash=?1 AND revoked=0 AND expires_at>?2",
            params![auth::hash_token(raw), t],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = id {
        let updated = db.execute(
            "UPDATE admin_api_tokens SET last_used_at=?2 WHERE id=?1 AND revoked=0 AND expires_at>?2",
            params![id, t],
        )?;
        Ok((updated == 1).then_some(id))
    } else {
        Ok(None)
    }
}

pub fn verify(s: &AppState, raw: &str) -> AppResult<Option<String>> {
    verify_db(&*s.conn()?, raw)
}

/// Recheck inside the caller's read/write transaction, after initial request auth.
pub fn ensure_token(db: &Connection, id: &str) -> AppResult<()> {
    let active: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM admin_api_tokens WHERE id=?1 AND revoked=0 AND expires_at>?2)",
        params![id, now()],
        |r| r.get(0),
    )?;
    if !active {
        return Err(AppError::unauthorized(
            "Administrator token expired or revoked",
        ));
    }
    Ok(())
}

fn cookie_guard(s: &AppState, headers: &HeaderMap, write: bool) -> AppResult<auth::Session> {
    if headers.contains_key("authorization") {
        return Err(AppError::forbidden(
            "Manage administrator tokens from a Passkey-authenticated browser session",
        ));
    }
    let session = auth::require_admin(s, headers)?;
    if write {
        auth::csrf_check(s, headers, &session)?;
    }
    Ok(session)
}

fn ensure_cookie(db: &Connection, session: &auth::Session, recent: bool) -> AppResult<()> {
    let authenticated_at: Option<i64> = db
        .query_row(
            "SELECT recent_auth_at FROM sessions WHERE token_hash=?1 AND expires_at>?2 AND admin=1 AND bootstrap=0 AND user_id IS NULL",
            params![session.token_hash, now()],
            |r| r.get(0),
        )
        .optional()?;
    let authenticated_at = authenticated_at.ok_or_else(|| {
        AppError::unauthorized("This administrator session expired or was revoked")
    })?;
    if recent && now().saturating_sub(authenticated_at) > RECENT_AUTH_SECONDS {
        return Err(AppError::forbidden(
            "Sign in with a passkey again before creating an administrator token",
        ));
    }
    Ok(())
}

async fn list(State(s): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let session = cookie_guard(&s, &headers, false)?;
    let db = s.conn()?;
    ensure_cookie(&db, &session, false)?;
    let items = db
        .prepare("SELECT id,name,created_at,expires_at,last_used_at FROM admin_api_tokens WHERE revoked=0 ORDER BY created_at DESC,id LIMIT 200")?
        .query_map([], |r| Ok(json!({
            "id": r.get::<_, String>(0)?,
            "name": r.get::<_, String>(1)?,
            "created_at": r.get::<_, i64>(2)?,
            "expires_at": r.get::<_, i64>(3)?,
            "last_used_at": r.get::<_, Option<i64>>(4)?
        })))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Json(json!({"items":items})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenInput {
    name: String,
}

async fn create(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<TokenInput>,
) -> AppResult<Json<Value>> {
    let session = cookie_guard(&s, &headers, true)?;
    safety::bounded_text(&input.name, 80, "Token name")?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request("Token name is required"));
    }
    let mut secret = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let raw = format!("hf_admin_{}", hex::encode(secret));
    let id = uuid::Uuid::new_v4().to_string();
    let t = now();
    let expires_at = t + TOKEN_TTL;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_cookie(&tx, &session, true)?;
    let active: i64 = tx.query_row(
        "SELECT count(*) FROM admin_api_tokens WHERE revoked=0 AND expires_at>?1",
        [t],
        |r| r.get(0),
    )?;
    if active >= MAX_ACTIVE_TOKENS {
        return Err(AppError::conflict("At most 10 active administrator tokens"));
    }
    tx.execute(
        "INSERT INTO admin_api_tokens(id,name,token_hash,created_at,expires_at) VALUES(?1,?2,?3,?4,?5)",
        params![id, name, auth::hash_token(&raw), t, expires_at],
    )?;
    audit(
        &tx,
        &format!("admin_session:{}", session.token_hash),
        "admin_token_create",
        &id,
        "",
    )?;
    tx.commit()?;
    Ok(Json(json!({"id":id,"token":raw,"expires_at":expires_at})))
}

async fn revoke(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let session = cookie_guard(&s, &headers, true)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_cookie(&tx, &session, false)?;
    if tx.execute("UPDATE admin_api_tokens SET revoked=1 WHERE id=?1", [&id])? != 1 {
        return Err(AppError::not_found("Administrator token not found"));
    }
    audit(
        &tx,
        &format!("admin_session:{}", session.token_hash),
        "admin_token_revoke",
        &id,
        "",
    )?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}
