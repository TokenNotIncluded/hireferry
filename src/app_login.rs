//! Browser-to-WebView sign-in. The public request ID is never a credential.
//! The initiating cookie proves possession; explicit browser consent binds a
//! live member session, and redemption consumes the grant in one transaction.
use super::{
    cookie, csrf_check, hash_token, new_session, now, origin_check, random_token, rate_limit,
    require_session,
};
use crate::{AppError, AppResult, AppState};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, header},
    middleware,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};

const COOKIE: &str = "hf_app_login";
const TTL: i64 = 300;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/app", get(page))
        .route("/auth/app/authorize", get(authorize_page))
        .route("/api/auth/app/start", post(start))
        .route("/api/auth/app/context", get(context))
        .route("/api/auth/app/decide", post(decide))
        .route("/api/auth/app/redeem", post(redeem))
        .route("/api/auth/app/cancel", post(cancel))
        .layer(middleware::map_response(no_store))
}

async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

pub(super) fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch("SAVEPOINT app_login_v1;")?;
    let result = db.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_login_grants (
        request_id TEXT PRIMARY KEY,
        verifier_hash TEXT NOT NULL UNIQUE,
        status TEXT NOT NULL CHECK(status IN ('pending','approved','denied','consumed')),
        source_session_hash TEXT,
        issued_session_hash TEXT,
        user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS app_login_expiry ON app_login_grants(expires_at);",
    );
    match result {
        Ok(()) => {
            db.execute_batch("RELEASE app_login_v1;")?;
            Ok(())
        }
        Err(error) => {
            let _ = db.execute_batch("ROLLBACK TO app_login_v1; RELEASE app_login_v1;");
            Err(error.into())
        }
    }
}

fn valid_request(request: &str) -> AppResult<()> {
    if request.len() == 43
        && request
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Ok(())
    } else {
        Err(AppError::bad_request("Invalid app sign-in request"))
    }
}

pub(super) fn pending_request(db: &Connection, request: &str) -> AppResult<()> {
    valid_request(request)?;
    let found: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM app_login_grants WHERE request_id=?1 AND status='pending' AND expires_at>?2)", params![request,now()], |row| row.get(0))?;
    if found {
        Ok(())
    } else {
        Err(AppError::bad_request(
            "App sign-in expired or was already completed. Start again in the app.",
        ))
    }
}

pub(super) fn authorize_path(request: &str) -> String {
    // Only server-generated 43-character request IDs reach this function.
    format!("/auth/app/authorize?request={request}")
}

fn comparison_code(request: &str) -> String {
    let digest = hash_token(request).to_ascii_uppercase();
    format!("{}-{}", &digest[..4], &digest[4..8])
}

fn app_cookie(
    mut response: Response,
    state: &AppState,
    value: &str,
    age: i64,
) -> AppResult<Response> {
    let secure = if state.cfg.public_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{COOKIE}={value}; Path=/api/auth/app; HttpOnly; SameSite=Strict; Max-Age={age}{secure}"
        ))
        .map_err(AppError::internal)?,
    );
    Ok(response)
}

fn browser_only(headers: &HeaderMap) -> AppResult<()> {
    if headers.contains_key(header::AUTHORIZATION) {
        return Err(AppError::forbidden(
            "Use an interactive browser session for app sign-in",
        ));
    }
    Ok(())
}

fn ordinary_member(db: &Connection, uid: &str) -> AppResult<bool> {
    Ok(crate::team::human_member(db, uid)?.is_none()
        && crate::staff::active_role(db, uid)?.is_none())
}

async fn page() -> Html<&'static str> {
    Html(include_str!("../static/app-login.html"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestId {
    request: String,
}

async fn authorize_page(
    State(state): State<AppState>,
    Query(input): Query<RequestId>,
) -> AppResult<Html<&'static str>> {
    {
        let db = state.conn()?;
        pending_request(&db, &input.request)?;
    }
    Ok(page().await)
}

async fn start(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    browser_only(&headers)?;
    origin_check(&state, &headers)?;
    let old = cookie(&headers, COOKIE);
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM app_login_grants WHERE expires_at<=?1", [now()])?;
    let existing: Option<(String, i64)> = if let Some(value) = &old {
        tx.query_row("SELECT request_id,expires_at FROM app_login_grants WHERE verifier_hash=?1 AND status IN ('pending','approved') AND expires_at>?2", params![hash_token(value),now()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?
    } else {
        None
    };
    let (request, verifier, expires) = if let Some((request, expires)) = existing {
        (request, old.unwrap(), expires)
    } else {
        rate_limit(&tx, "app-login-start", 100, TTL)?;
        let request = random_token();
        let verifier = random_token();
        let expires = now() + TTL;
        tx.execute("INSERT INTO app_login_grants(request_id,verifier_hash,status,created_at,expires_at) VALUES(?1,?2,'pending',?3,?4)", params![request,hash_token(&verifier),now(),expires])?;
        (request, verifier, expires)
    };
    tx.commit()?;
    let url = format!("{}{}", state.cfg.public_url, authorize_path(&request));
    app_cookie(Json(json!({"request":request,"verification_uri":url,"comparison_code":comparison_code(&request),"expires_at":expires,"interval":3})).into_response(), &state, &verifier, expires-now())
}

async fn context(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<RequestId>,
) -> AppResult<Json<Value>> {
    browser_only(&headers)?;
    let session = super::session(&state, &headers)?;
    let db = state.conn()?;
    pending_request(&db, &input.request)?;
    let operator = if let Some(s) = &session {
        s.admin
            || s.bootstrap
            || match s.user_id.as_deref() {
                Some(uid) => !ordinary_member(&db, uid)?,
                None => true,
            }
    } else {
        false
    };
    let user = match &session {
        Some(s) if !operator && s.user_id.is_some() => {
            let id = s.user_id.as_ref().unwrap();
            let (email, name): (String, String) = db.query_row(
                "SELECT email,name FROM users WHERE id=?1 AND banned=0",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            json!({"email":email,"name":name})
        }
        _ => Value::Null,
    };
    Ok(Json(
        json!({"request":input.request,"comparison_code":comparison_code(&input.request),"site":state.cfg.public_url,"user":user,"can_approve":!user.is_null(),"admin_session":operator,"csrf":session.map(|s| s.csrf),"google_enabled":state.cfg.google.enabled,"email_enabled":state.cfg.smtp.enabled || state.cfg.development}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    request: String,
    approve: bool,
}

async fn decide(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Decision>,
) -> AppResult<Json<Value>> {
    browser_only(&headers)?;
    let session = require_session(&state, &headers)?;
    csrf_check(&state, &headers, &session)?;
    let uid = session.user_id.as_deref().filter(|_| !session.admin && !session.bootstrap)
        .ok_or_else(|| AppError::forbidden("App sign-in requires a member account. Administrator sessions cannot be transferred."))?;
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    pending_request(&tx, &input.request)?;
    if !ordinary_member(&tx, uid)? {
        return Err(AppError::forbidden(
            "Staff and managers must sign in directly in the app",
        ));
    }
    let live: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=?1 AND s.user_id=?2 AND s.expires_at>?3 AND s.admin=0 AND s.bootstrap=0 AND u.banned=0)", params![session.token_hash,uid,now()], |r| r.get(0))?;
    if !live {
        return Err(AppError::unauthorized(
            "Sign in again before approving the app",
        ));
    }
    tx.execute("UPDATE app_login_grants SET status=?2,source_session_hash=?3,user_id=?4 WHERE request_id=?1 AND status='pending'", params![input.request,if input.approve {"approved"} else {"denied"},session.token_hash,uid])?;
    crate::db::audit(
        &tx,
        uid,
        if input.approve {
            "app_login_approved"
        } else {
            "app_login_denied"
        },
        "native_app",
        "",
    )?;
    tx.commit()?;
    Ok(Json(json!({"approved":input.approve})))
}

async fn redeem(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RequestId>,
) -> AppResult<Response> {
    browser_only(&headers)?;
    origin_check(&state, &headers)?;
    valid_request(&input.request)?;
    let verifier = cookie(&headers, COOKIE)
        .ok_or_else(|| AppError::unauthorized("Start sign-in in this app again"))?;
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let grant: Option<(String, Option<String>, Option<String>)> = tx.query_row("SELECT status,source_session_hash,user_id FROM app_login_grants WHERE request_id=?1 AND verifier_hash=?2 AND expires_at>?3", params![input.request,hash_token(&verifier),now()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let (status, source, uid) = grant.ok_or_else(|| {
        AppError::unauthorized("App sign-in expired or does not belong to this app")
    })?;
    if status == "pending" {
        return Ok(Json(json!({"status":"pending","interval":3})).into_response());
    }
    if status == "denied" {
        return app_cookie(
            Json(json!({"status":"denied"})).into_response(),
            &state,
            "",
            0,
        );
    }
    if status != "approved" {
        return Err(AppError::conflict("This app sign-in was already used"));
    }
    if !ordinary_member(&tx, uid.as_deref().unwrap_or(""))? {
        return Err(AppError::forbidden(
            "Staff and managers must sign in directly in the app",
        ));
    }
    let source_times: Option<(i64,i64)> = tx.query_row("SELECT s.expires_at,s.recent_auth_at FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=?1 AND s.user_id=?2 AND s.expires_at>?3 AND s.admin=0 AND s.bootstrap=0 AND u.banned=0", params![source,uid,now()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    let (source_expires, source_recent) = source_times.ok_or_else(|| {
        AppError::unauthorized("The approving browser session was revoked. Start again.")
    })?;
    let (raw, _) = new_session(&tx, &state.cfg, uid.as_deref(), false, false)?;
    // Consent is not a fresh authentication event and cannot extend the
    // lifetime of the source session by repeatedly hopping between clients.
    tx.execute(
        "UPDATE sessions SET expires_at=MIN(expires_at,?2),recent_auth_at=?3 WHERE token_hash=?1",
        params![hash_token(&raw), source_expires, source_recent],
    )?;
    let expires: i64 = tx.query_row(
        "SELECT expires_at FROM sessions WHERE token_hash=?1",
        [hash_token(&raw)],
        |r| r.get(0),
    )?;
    // Replace only a session already owned by the initiating WebView. The
    // independent approving browser remains signed in and can still log out.
    if let Some(old) = cookie(&headers, super::SESSION_COOKIE) {
        tx.execute(
            "DELETE FROM sessions WHERE token_hash=?1 AND token_hash<>?2",
            params![hash_token(&old), source],
        )?;
    }
    tx.execute(
        "UPDATE app_login_grants SET status='consumed',issued_session_hash=?2 WHERE request_id=?1 AND status='approved'",
        params![input.request,hash_token(&raw)],
    )?;
    crate::db::audit(
        &tx,
        uid.as_deref().unwrap_or(""),
        "app_login_completed",
        "native_app",
        "",
    )?;
    tx.commit()?;
    let response = super::with_cookie(
        Json(json!({"status":"complete","redirect":"/app"})).into_response(),
        &state.cfg,
        super::SESSION_COOKIE,
        &raw,
        expires - now(),
    )?;
    app_cookie(response, &state, "", 0)
}

async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RequestId>,
) -> AppResult<Response> {
    browser_only(&headers)?;
    origin_check(&state, &headers)?;
    valid_request(&input.request)?;
    let verifier = cookie(&headers, COOKIE).map(|v| hash_token(&v));
    let session_hash = cookie(&headers, super::SESSION_COOKIE).map(|v| hash_token(&v));
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let issued:Option<Option<String>> = tx.query_row("SELECT issued_session_hash FROM app_login_grants WHERE request_id=?1 AND (verifier_hash=?2 OR issued_session_hash=?3)",params![input.request,verifier,session_hash],|r|r.get(0)).optional()?;
    let issued =
        issued.ok_or_else(|| AppError::unauthorized("App sign-in is not active in this app"))?;
    if let Some(hash) = &issued {
        // Cancellation can race a poll response. Revoke the exact newly issued
        // app session even if its Set-Cookie response has not arrived yet.
        tx.execute("DELETE FROM sessions WHERE token_hash=?1", [hash])?;
    }
    tx.execute(
        "DELETE FROM app_login_grants WHERE request_id=?1",
        [&input.request],
    )?;
    tx.commit()?;
    let response = app_cookie(
        Json(json!({"cancelled":true})).into_response(),
        &state,
        "",
        0,
    )?;
    if issued.is_some() && issued == session_hash {
        super::with_cookie(response, &state.cfg, super::SESSION_COOKIE, "", 0)
    } else {
        Ok(response)
    }
}
