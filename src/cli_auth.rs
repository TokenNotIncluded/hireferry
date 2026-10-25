//! Native-client login uses separate identity, member-MCP and administrator
//! grants. Credential families make refresh rotation and logout atomic.
use crate::{AppError, AppResult, AppState, access, auth, config::Config, db::audit, models, now};
use axum::{
    Form, Json, Router,
    extract::{Path, Query, State, rejection::FormRejection},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware,
    response::{Html, IntoResponse, Response},
    routing::{delete, get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{Rng, RngCore, rngs::OsRng};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

// Preserve the existing administrator step-up requirement, independent of
// configurable client session lifetimes.
const RECENT_AUTH_SECONDS: i64 = 600;
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXYZ23456789";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/oauth/device/code", post(device_code))
        .route("/oauth/device", get(device_page))
        .route("/oauth/authorize", get(authorize_page))
        .route("/oauth/connections", get(connections_page))
        .route("/api/cli/authorization", get(details).post(decide))
        .route("/api/cli/connections", get(connections))
        .route("/api/cli/connections/{id}", delete(disconnect))
        .route("/oauth/token", post(token))
        .route("/oauth/revoke", post(revoke))
        .route("/oauth/userinfo", get(userinfo))
        .route("/.well-known/oauth-authorization-server", get(metadata))
        .layer(middleware::map_response(no_store))
}
async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert("pragma", HeaderValue::from_static("no-cache"));
    response
}

/// Database version 8. Called inside the migration transaction, once.
pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        "CREATE TABLE cli_authorizations(
            id TEXT PRIMARY KEY,
            user_id TEXT REFERENCES users(id),
            admin INTEGER NOT NULL CHECK(admin IN (0,1)),
            client_id TEXT NOT NULL,
            scope TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)),
            current_access_hash TEXT,
            CHECK((admin=1 AND user_id IS NULL) OR (admin=0 AND user_id IS NOT NULL))
        );
        CREATE INDEX cli_authorizations_user ON cli_authorizations(user_id,admin,expires_at);
        CREATE TABLE cli_grants(
            secret_hash TEXT PRIMARY KEY,
            flow TEXT NOT NULL CHECK(flow IN ('device','oauth')),
            user_code_hash TEXT UNIQUE,
            client_id TEXT NOT NULL,
            scope TEXT NOT NULL,
            redirect_uri TEXT NOT NULL DEFAULT '',
            code_challenge TEXT NOT NULL DEFAULT '',
            session_hash TEXT,
            user_id TEXT REFERENCES users(id),
            admin INTEGER NOT NULL DEFAULT 0 CHECK(admin IN (0,1)),
            status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','approved','denied','consumed')),
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            last_poll_at INTEGER NOT NULL DEFAULT 0,
            poll_interval INTEGER NOT NULL DEFAULT 5,
            issued_family_id TEXT,
            CHECK(NOT(admin=1 AND user_id IS NOT NULL))
        );
        CREATE INDEX cli_grants_expiry ON cli_grants(expires_at);
        CREATE TABLE cli_tokens(
            token_hash TEXT PRIMARY KEY,
            token_id TEXT,
            family_id TEXT NOT NULL REFERENCES cli_authorizations(id) ON DELETE CASCADE,
            admin INTEGER NOT NULL CHECK(admin IN (0,1)),
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1))
        );
        CREATE INDEX cli_tokens_family ON cli_tokens(family_id);
        CREATE TABLE cli_refresh_tokens(
            token_hash TEXT PRIMARY KEY,
            family_id TEXT NOT NULL REFERENCES cli_authorizations(id) ON DELETE CASCADE,
            created_at INTEGER NOT NULL,
            used_at INTEGER
        );
        CREATE INDEX cli_refresh_family ON cli_refresh_tokens(family_id);
        CREATE TRIGGER cli_account_suspended AFTER UPDATE OF banned ON users
        WHEN OLD.banned IS NOT NEW.banned AND NEW.banned=1 BEGIN
            UPDATE cli_authorizations SET revoked=1 WHERE user_id=NEW.id;
            UPDATE cli_tokens SET revoked=1 WHERE family_id IN
                (SELECT id FROM cli_authorizations WHERE user_id=NEW.id);
        END;",
    )?;
    Ok(())
}

struct OAuthError {
    status: StatusCode,
    code: &'static str,
    description: String,
}
impl OAuthError {
    fn new(code: &'static str, description: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            description: description.into(),
        }
    }
    fn invalid_grant() -> Self {
        Self::new(
            "invalid_grant",
            "The authorization is invalid, expired, revoked or already used",
        )
    }
}
impl From<AppError> for OAuthError {
    fn from(error: AppError) -> Self {
        Self {
            status: error.status,
            code: if error.status.is_server_error() {
                "server_error"
            } else {
                "access_denied"
            },
            description: error.message,
        }
    }
}
impl From<rusqlite::Error> for OAuthError {
    fn from(error: rusqlite::Error) -> Self {
        AppError::from(error).into()
    }
}
impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"error":self.code,"error_description":self.description})),
        )
            .into_response()
    }
}

#[derive(Clone, Copy)]
struct Scopes {
    mcp: bool,
    admin: bool,
}
impl Scopes {
    fn parse(value: &str) -> Result<Self, OAuthError> {
        let mut scope = Self {
            mcp: false,
            admin: false,
        };
        for item in value.split_ascii_whitespace() {
            match item {
                "identity" => {}
                "mcp" => scope.mcp = true,
                "admin" => scope.admin = true,
                _ => return Err(OAuthError::new("invalid_scope", "Unknown permission scope")),
            }
        }
        if scope.mcp && scope.admin {
            return Err(OAuthError::new(
                "invalid_scope",
                "Request member mcp or administrator admin scope, not both",
            ));
        }
        Ok(scope)
    }
    fn validate(self, cfg: &Config) -> Result<Self, OAuthError> {
        let allowed = |name: &str| cfg.cli_auth.allowed_scopes.iter().any(|s| s == name);
        if !allowed("identity")
            || (self.mcp && !allowed("mcp"))
            || (self.admin && !allowed("admin"))
        {
            return Err(OAuthError::new(
                "invalid_scope",
                "The requested permission is disabled on this server",
            ));
        }
        Ok(self)
    }
    fn text(self) -> &'static str {
        if self.admin {
            "identity admin"
        } else if self.mcp {
            "identity mcp"
        } else {
            "identity"
        }
    }
    fn subset_of(self, old: Self) -> bool {
        (!self.mcp || old.mcp) && (!self.admin || old.admin)
    }
}
fn requested_scope(cfg: &Config, value: Option<&str>) -> Result<Scopes, OAuthError> {
    Scopes::parse(value.unwrap_or("identity"))?.validate(cfg)
}
fn scope_for_browser(cfg: &Config, value: Option<&str>) -> AppResult<Scopes> {
    requested_scope(cfg, value).map_err(|e| AppError::bad_request(e.description))
}
fn enabled(state: &AppState) -> AppResult<()> {
    if state.cfg.cli_auth.enabled {
        Ok(())
    } else {
        Err(AppError::unavailable("Command-line sign-in is disabled"))
    }
}
fn issuer(state: &AppState) -> &str {
    state.cfg.public_url.trim_end_matches('/')
}
fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
fn validate_client(cfg: &Config, client_id: &str) -> Result<(), OAuthError> {
    if client_id == cfg.cli_auth.client_id {
        Ok(())
    } else {
        Err(OAuthError::new("invalid_client", "Unknown public client"))
    }
}
fn rate_limit(db: &Connection, key: &str, limit: u64, window: u64) -> AppResult<()> {
    let time = now();
    db.execute(
        "DELETE FROM auth_limits WHERE rowid IN
         (SELECT rowid FROM auth_limits WHERE window_start<?1 ORDER BY window_start LIMIT ?2)",
        params![time - 86400, crate::maintenance::DEFAULT_BATCH_SIZE],
    )?;
    db.execute(
        "INSERT INTO auth_limits(key,window_start,count) VALUES(?1,?2,1)
        ON CONFLICT(key) DO UPDATE SET count=CASE WHEN window_start<=?3 THEN 1 ELSE count+1 END,
        window_start=CASE WHEN window_start<=?3 THEN ?2 ELSE window_start END",
        params![key, time, time - window as i64],
    )?;
    let count: i64 = db.query_row("SELECT count FROM auth_limits WHERE key=?1", [key], |r| {
        r.get(0)
    })?;
    if count > limit as i64 {
        return Err(AppError {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: "Too many authorization attempts; try again later".into(),
        });
    }
    Ok(())
}
fn cleanup(db: &Connection) -> AppResult<()> {
    db.execute(
        "DELETE FROM cli_grants WHERE rowid IN
         (SELECT rowid FROM cli_grants WHERE expires_at<?1 ORDER BY expires_at LIMIT ?2)",
        params![now() - 86400, crate::maintenance::DEFAULT_BATCH_SIZE],
    )?;
    // The maintenance worker removes expired families and their children in
    // separate batches after the configured grace period. An unbounded CASCADE
    // here would lock every request behind a large rotation history.
    Ok(())
}
fn grant_capacity(db: &Connection, cfg: &Config) -> AppResult<()> {
    let count: i64 = db.query_row(
        "SELECT count(*) FROM cli_grants WHERE status IN ('pending','approved') AND expires_at>?1",
        [now()],
        |r| r.get(0),
    )?;
    if count >= cfg.cli_auth.max_pending_grants as i64 {
        Err(AppError::unavailable(
            "Too many pending sign-in requests; try again later",
        ))
    } else {
        Ok(())
    }
}

#[derive(Deserialize)]
struct DeviceRequest {
    client_id: String,
    scope: Option<String>,
}
async fn device_code(
    State(state): State<AppState>,
    form: Result<Form<DeviceRequest>, FormRejection>,
) -> Result<Json<Value>, OAuthError> {
    enabled(&state)?;
    let Form(input) = form
        .map_err(|_| OAuthError::new("invalid_request", "Send client_id and scope as form data"))?;
    validate_client(&state.cfg, &input.client_id)?;
    let scope = requested_scope(&state.cfg, input.scope.as_deref())?;
    let secret = random_secret();
    let code: String = (0..8)
        .map(|_| USER_CODE_ALPHABET[OsRng.gen_range(0..USER_CODE_ALPHABET.len())] as char)
        .collect();
    let display_code = format!("{}-{}", &code[..4], &code[4..]);
    let time = now();
    let expires_in = state.cfg.cli_auth.grant_minutes as i64 * 60;
    let interval = state.cfg.cli_auth.poll_seconds as i64;
    let db = state.conn()?;
    cleanup(&db)?;
    rate_limit(
        &db,
        "cli-device-start",
        state.cfg.cli_auth.rate_limit_requests,
        state.cfg.cli_auth.rate_limit_window_seconds,
    )?;
    grant_capacity(&db, &state.cfg)?;
    db.execute("INSERT INTO cli_grants(secret_hash,flow,user_code_hash,client_id,scope,created_at,expires_at,poll_interval)
        VALUES(?1,'device',?2,?3,?4,?5,?6,?7)",params![auth::hash_token(&secret),auth::hash_token(&code),input.client_id,scope.text(),time,time+expires_in,interval])?;
    let verification_uri = format!("{}/oauth/device", issuer(&state));
    Ok(Json(
        json!({"device_code":secret,"user_code":display_code,"verification_uri":verification_uri,
        "verification_uri_complete":format!("{verification_uri}?user_code={display_code}"),"expires_in":expires_in,"interval":interval}),
    ))
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorizationInput {
    #[serde(default)]
    flow: String,
    user_code: Option<String>,
    client_id: Option<String>,
    response_type: Option<String>,
    redirect_uri: Option<String>,
    scope: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    approve: Option<bool>,
}
struct BrowserRequest<'a> {
    redirect_uri: &'a str,
    state: &'a str,
    challenge: &'a str,
    scope: Scopes,
}
impl AuthorizationInput {
    fn browser(&self, cfg: &Config) -> AppResult<BrowserRequest<'_>> {
        if !matches!(self.flow.as_str(), "" | "oauth")
            || self.user_code.is_some()
            || self.client_id.as_deref() != Some(cfg.cli_auth.client_id.as_str())
            || self.response_type.as_deref() != Some("code")
            || self.code_challenge_method.as_deref() != Some("S256")
        {
            return Err(AppError::bad_request(
                "Use the configured public client's authorization code flow with S256 PKCE",
            ));
        }
        let scope = scope_for_browser(cfg, self.scope.as_deref())?;
        let redirect_uri = self
            .redirect_uri
            .as_deref()
            .ok_or_else(|| AppError::bad_request("redirect_uri is required"))?;
        validate_redirect(redirect_uri)?;
        let state = self
            .state
            .as_deref()
            .filter(|s| (16..=512).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_graphic()))
            .ok_or_else(|| AppError::bad_request("A random state value is required"))?;
        let challenge = self
            .code_challenge
            .as_deref()
            .filter(|s| {
                s.len() == 43
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            })
            .ok_or_else(|| AppError::bad_request("A valid S256 code challenge is required"))?;
        Ok(BrowserRequest {
            redirect_uri,
            state,
            challenge,
            scope,
        })
    }
    fn device(&self) -> AppResult<String> {
        if !matches!(self.flow.as_str(), "" | "device")
            || self.client_id.is_some()
            || self.response_type.is_some()
            || self.redirect_uri.is_some()
            || self.scope.is_some()
            || self.state.is_some()
            || self.code_challenge.is_some()
            || self.code_challenge_method.is_some()
        {
            return Err(AppError::bad_request("Invalid device confirmation request"));
        }
        normalize_code(self.user_code.as_deref().unwrap_or(""))
    }
}
fn validate_redirect(value: &str) -> AppResult<()> {
    let port = value
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| value.strip_prefix("http://[::1]:"))
        .and_then(|s| s.strip_suffix("/callback"))
        .and_then(|s| {
            s.parse::<u16>()
                .ok()
                .filter(|p| *p != 0 && p.to_string() == s)
        });
    if port.is_some() {
        Ok(())
    } else {
        Err(AppError::bad_request(
            "redirect_uri must use an explicit loopback IP, port and /callback path",
        ))
    }
}
fn normalize_code(value: &str) -> AppResult<String> {
    let value = value.trim().to_ascii_uppercase();
    let code = if value.len() == 9 && value.as_bytes()[4] == b'-' {
        format!("{}{}", &value[..4], &value[5..])
    } else {
        value
    };
    if code.len() != 8 || !code.bytes().all(|c| USER_CODE_ALPHABET.contains(&c)) {
        return Err(AppError::bad_request(
            "The device code is invalid or expired",
        ));
    }
    Ok(code)
}
fn browser_session(state: &AppState, headers: &HeaderMap, write: bool) -> AppResult<auth::Session> {
    if headers.contains_key("authorization") {
        return Err(AppError::forbidden(
            "Approve sign-in from an interactive browser session",
        ));
    }
    let session = auth::require_session(state, headers)?;
    if session.bootstrap {
        return Err(AppError::forbidden(
            "Complete Passkey enrollment before authorizing a client",
        ));
    }
    if write {
        auth::csrf_check(state, headers, &session)?;
    }
    Ok(session)
}
fn check_identity(db: &Connection, uid: Option<&str>, admin: bool) -> AppResult<()> {
    if admin {
        if uid.is_none() {
            return Ok(());
        }
    } else if let Some(uid) = uid {
        let active: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND banned=0)",
            [uid],
            |r| r.get(0),
        )?;
        if active {
            return Ok(());
        }
    }
    Err(AppError::unauthorized("Account unavailable"))
}
fn check_scope(
    db: &Connection,
    cfg: &Config,
    uid: Option<&str>,
    admin: bool,
    scope: Scopes,
) -> AppResult<()> {
    check_identity(db, uid, admin)?;
    scope
        .validate(cfg)
        .map_err(|e| AppError::forbidden(e.description))?;
    if scope.admin && !admin {
        return Err(AppError::forbidden(
            "Administrator scope requires an administrator Passkey session",
        ));
    }
    if scope.mcp {
        if admin {
            return Err(AppError::forbidden(
                "Member mcp scope requires a member account; request admin separately",
            ));
        }
        access::require_user_ai(
            db,
            uid.ok_or_else(|| AppError::unauthorized("A member account is required"))?,
        )?;
    }
    Ok(())
}
fn check_session(
    db: &Connection,
    cfg: &Config,
    hash: &str,
    uid: Option<&str>,
    admin: bool,
    scope: Scopes,
) -> AppResult<()> {
    let found:Option<(Option<String>,bool,i64)>=db.query_row(
        "SELECT user_id,admin,recent_auth_at FROM sessions WHERE token_hash=?1 AND bootstrap=0 AND expires_at>?2",
        params![hash,now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional()?;
    let Some((actual_uid, actual_admin, recent)) = found else {
        return Err(AppError::unauthorized(
            "This browser session expired or was revoked",
        ));
    };
    if actual_uid.as_deref() != uid || actual_admin != admin {
        return Err(AppError::forbidden("The signed-in account changed"));
    }
    check_scope(db, cfg, uid, admin, scope)?;
    if scope.admin && now().saturating_sub(recent) > RECENT_AUTH_SECONDS {
        return Err(AppError::forbidden(
            "Sign in with a Passkey again before granting administrator access",
        ));
    }
    Ok(())
}
fn identity(db: &Connection, uid: Option<&str>) -> AppResult<Value> {
    match uid {
        Some(uid) => {
            let u = models::load_user(db, uid)?;
            Ok(json!({"id":u.id,"name":u.name,"email":u.email,"role":u.role}))
        }
        None => Ok(Value::Null),
    }
}
fn ai_eligibility(db: &Connection, uid: Option<&str>, admin: bool) -> AppResult<Value> {
    if admin {
        Ok(json!({"eligible":true,"reason":"administrator","role":"admin"}))
    } else {
        access::eligibility(
            db,
            uid.ok_or_else(|| AppError::unauthorized("Account unavailable"))?,
        )
    }
}
async fn device_page(State(state): State<AppState>) -> AppResult<Html<&'static str>> {
    enabled(&state)?;
    Ok(Html(include_str!("../static/cli-auth.html")))
}
#[derive(Deserialize)]
struct AuthorizeQuery {
    client_id: Option<String>,
    response_type: Option<String>,
    redirect_uri: Option<String>,
    scope: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
}
async fn authorize_page(
    State(state): State<AppState>,
    Query(query): Query<AuthorizeQuery>,
) -> AppResult<Html<&'static str>> {
    enabled(&state)?;
    let input = AuthorizationInput {
        client_id: query.client_id,
        response_type: query.response_type,
        redirect_uri: query.redirect_uri,
        scope: query.scope,
        state: query.state,
        code_challenge: query.code_challenge,
        code_challenge_method: query.code_challenge_method,
        ..Default::default()
    };
    input.browser(&state.cfg)?;
    Ok(Html(include_str!("../static/cli-auth.html")))
}
async fn details(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<AuthorizationInput>,
) -> AppResult<Json<Value>> {
    enabled(&state)?;
    let session = browser_session(&state, &headers, false)?;
    let db = state.conn()?;
    rate_limit(
        &db,
        &format!("cli-code-check:{}", session.token_hash),
        state.cfg.cli_auth.max_user_code_attempts,
        state.cfg.cli_auth.rate_limit_window_seconds,
    )?;
    let mut result = json!({"client_id":state.cfg.cli_auth.client_id,"client_name":"HireFerry CLI (hfy)",
        "admin":session.admin,"user":identity(&db,session.user_id.as_deref())?,"user_code":null,"redirect_uri":null,"expires_at":null,
        "token_expires_in":state.cfg.cli_auth.access_token_minutes*60,"refresh_expires_in":state.cfg.cli_auth.refresh_token_days*86400});
    let scope = if input.user_code.is_some() {
        let code = input.device()?;
        let row:Option<(i64,String)>=db.query_row("SELECT expires_at,scope FROM cli_grants WHERE user_code_hash=?1 AND flow='device' AND status='pending' AND expires_at>?2 AND client_id=?3",
            params![auth::hash_token(&code),now(),state.cfg.cli_auth.client_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (expires, scope) = row.ok_or_else(|| {
            AppError::bad_request("The device code is invalid, expired or already confirmed")
        })?;
        result["flow"] = json!("device");
        result["expires_at"] = json!(expires);
        result["user_code"] = json!(format!("{}-{}", &code[..4], &code[4..]));
        scope_for_browser(&state.cfg, Some(&scope))?
    } else {
        let request = input.browser(&state.cfg)?;
        result["flow"] = json!("oauth");
        result["redirect_uri"] = json!(request.redirect_uri);
        request.scope
    };
    check_session(
        &db,
        &state.cfg,
        &session.token_hash,
        session.user_id.as_deref(),
        session.admin,
        Scopes {
            mcp: false,
            admin: false,
        },
    )?;
    // An active account may inspect and decline a request even when it cannot
    // approve the requested permission. Keep session/database failures as real
    // errors; only a permission denial becomes a recoverable consent state.
    let approval_error = match check_session(
        &db,
        &state.cfg,
        &session.token_hash,
        session.user_id.as_deref(),
        session.admin,
        scope,
    ) {
        Ok(()) => None,
        Err(error) if error.status == StatusCode::FORBIDDEN => Some(error.message),
        Err(error) => return Err(error),
    };
    result["can_approve"] = json!(approval_error.is_none());
    result["requires_recent_auth"] =
        json!(approval_error.is_some() && scope.admin && session.admin);
    result["approval_error"] = json!(approval_error);
    result["scope"] = json!(scope.text());
    result["ai_eligibility"] = ai_eligibility(&db, session.user_id.as_deref(), session.admin)?;
    Ok(Json(result))
}
async fn decide(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AuthorizationInput>,
) -> AppResult<Json<Value>> {
    enabled(&state)?;
    let session = browser_session(&state, &headers, true)?;
    let approve = input
        .approve
        .ok_or_else(|| AppError::bad_request("An explicit approve decision is required"))?;
    let mut db = state.conn()?;
    rate_limit(
        &db,
        &format!("cli-consent:{}", session.token_hash),
        state.cfg.cli_auth.max_user_code_attempts,
        state.cfg.cli_auth.rate_limit_window_seconds,
    )?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    cleanup(&tx)?;
    let result = if input.user_code.is_some() {
        let code = input.device()?;
        let scope:Option<String>=tx.query_row("SELECT scope FROM cli_grants WHERE user_code_hash=?1 AND flow='device' AND status='pending' AND expires_at>?2 AND client_id=?3",params![auth::hash_token(&code),now(),state.cfg.cli_auth.client_id],|r|r.get(0)).optional()?;
        let scope = scope.ok_or_else(|| {
            AppError::bad_request("The device code is invalid, expired or already confirmed")
        })?;
        let scope = scope_for_browser(&state.cfg, Some(&scope))?;
        check_session(
            &tx,
            &state.cfg,
            &session.token_hash,
            session.user_id.as_deref(),
            session.admin,
            if approve {
                scope
            } else {
                Scopes::parse("identity").map_err(|e| AppError::bad_request(e.description))?
            },
        )?;
        tx.execute("UPDATE cli_grants SET status=?2,session_hash=?3,user_id=?4,admin=?5 WHERE user_code_hash=?1 AND status='pending'",
            params![auth::hash_token(&code),if approve{"approved"}else{"denied"},session.token_hash,session.user_id,session.admin])?;
        json!({"approved":approve})
    } else {
        let request = input.browser(&state.cfg)?;
        check_session(
            &tx,
            &state.cfg,
            &session.token_hash,
            session.user_id.as_deref(),
            session.admin,
            if approve {
                request.scope
            } else {
                Scopes::parse("identity").map_err(|e| AppError::bad_request(e.description))?
            },
        )?;
        let mut redirect = url::Url::parse(request.redirect_uri).map_err(AppError::bad_request)?;
        if approve {
            grant_capacity(&tx, &state.cfg)?;
            let secret = random_secret();
            tx.execute("INSERT INTO cli_grants(secret_hash,flow,client_id,scope,redirect_uri,code_challenge,session_hash,user_id,admin,status,created_at,expires_at)
                VALUES(?1,'oauth',?2,?3,?4,?5,?6,?7,?8,'approved',?9,?10)",params![auth::hash_token(&secret),state.cfg.cli_auth.client_id,request.scope.text(),request.redirect_uri,request.challenge,session.token_hash,session.user_id,session.admin,now(),now()+state.cfg.cli_auth.code_seconds as i64])?;
            redirect.query_pairs_mut().append_pair("code", &secret);
        } else {
            redirect
                .query_pairs_mut()
                .append_pair("error", "access_denied");
        }
        redirect
            .query_pairs_mut()
            .append_pair("state", request.state)
            .append_pair("iss", issuer(&state));
        json!({"redirect_uri":redirect.as_str()})
    };
    audit(
        &tx,
        session.user_id.as_deref().unwrap_or("admin"),
        if approve { "cli_authorize" } else { "cli_deny" },
        &state.cfg.cli_auth.client_id,
        "",
    )?;
    tx.commit()?;
    Ok(Json(result))
}

struct Family {
    id: String,
    user_id: Option<String>,
    admin: bool,
    client_id: String,
    scope: String,
    expires_at: i64,
    revoked: bool,
    current_access_hash: Option<String>,
}
fn family(db: &Connection, id: &str) -> AppResult<Option<Family>> {
    Ok(db.query_row("SELECT id,user_id,admin,client_id,scope,expires_at,revoked,current_access_hash FROM cli_authorizations WHERE id=?1",[id],|r|Ok(Family{
        id:r.get(0)?,user_id:r.get(1)?,admin:r.get(2)?,client_id:r.get(3)?,scope:r.get(4)?,expires_at:r.get(5)?,revoked:r.get(6)?,current_access_hash:r.get(7)?,
    })).optional()?)
}
fn revoke_access(db: &Connection, id: &str) -> AppResult<()> {
    db.execute("UPDATE api_tokens SET revoked=1 WHERE id IN (SELECT token_id FROM cli_tokens WHERE family_id=?1 AND admin=0)",[id])?;
    db.execute("UPDATE admin_api_tokens SET revoked=1 WHERE id IN (SELECT token_id FROM cli_tokens WHERE family_id=?1 AND admin=1)",[id])?;
    db.execute("UPDATE cli_tokens SET revoked=1 WHERE family_id=?1", [id])?;
    Ok(())
}
fn revoke_family(db: &Connection, id: &str) -> AppResult<()> {
    db.execute("UPDATE cli_authorizations SET revoked=1 WHERE id=?1", [id])?;
    revoke_access(db, id)
}
/// Existing website token revocation must also invalidate refresh credentials.
/// A manual token has no family and keeps its previous behavior.
pub fn revoke_token_family_by_id(db: &Connection, id: &str, admin: bool) -> AppResult<()> {
    let id: Option<String> = db
        .query_row(
            "SELECT family_id FROM cli_tokens WHERE token_id=?1 AND admin=?2",
            params![id, admin],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = id {
        revoke_family(db, &id)?;
    }
    Ok(())
}
#[derive(Debug)]
struct AccessRecord {
    family_id: String,
    token_id: Option<String>,
    admin: bool,
    expires_at: i64,
    revoked: bool,
}

fn access_row(db: &Connection, hash: &str) -> AppResult<Option<AccessRecord>> {
    Ok(db.query_row("SELECT family_id,token_id,admin,expires_at,revoked FROM cli_tokens WHERE token_hash=?1",[hash],|r|Ok(AccessRecord {
        family_id: r.get(0)?, token_id: r.get(1)?, admin: r.get(2)?, expires_at: r.get(3)?, revoked: r.get(4)?,
    })).optional()?)
}

/// Identity-only CLI grants may submit feedback but do not gain MCP/business
/// privileges. Resolve and recheck the family on every request.
pub(crate) fn feedback_identity(
    db: &Connection,
    cfg: &Config,
    raw: &str,
) -> AppResult<(bool, Option<String>)> {
    let AccessRecord {
        family_id,
        token_id,
        admin,
        expires_at: expires,
        revoked,
    } = access_row(db, &auth::hash_token(raw))?
        .ok_or_else(|| AppError::unauthorized("A valid CLI connection is required"))?;
    let family = family(db, &family_id)?
        .ok_or_else(|| AppError::unauthorized("Client authorization expired or revoked"))?;
    if revoked
        || expires <= now()
        || family.revoked
        || family.expires_at <= now()
        || family.client_id != cfg.cli_auth.client_id
        || !underlying_active(db, token_id.as_deref(), admin, true)?
        || family.admin != admin
    {
        return Err(AppError::unauthorized(
            "Client authorization expired or revoked",
        ));
    }
    Scopes::parse(&family.scope)
        .map_err(|_| AppError::unauthorized("Invalid CLI authorization scope"))?
        .validate(cfg)
        .map_err(|_| AppError::forbidden("CLI authorization is not enabled"))?;
    check_identity(db, family.user_id.as_deref(), family.admin)?;
    Ok((admin, family.user_id))
}
fn underlying_active(
    db: &Connection,
    id: Option<&str>,
    admin: bool,
    require_unexpired: bool,
) -> AppResult<bool> {
    let Some(id) = id else {
        return Ok(true);
    };
    let table = if admin {
        "admin_api_tokens"
    } else {
        "api_tokens"
    };
    Ok(db.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id=?1 AND revoked=0 AND (?2=0 OR expires_at>?3))"),
        params![id, require_unexpired, now()],
        |r| r.get(0),
    )?)
}
fn current_access_active(db: &Connection, family: &Family) -> AppResult<bool> {
    let Some(hash) = family.current_access_hash.as_deref() else {
        return Ok(false);
    };
    let Some(AccessRecord {
        token_id: id,
        admin,
        revoked,
        ..
    }) = access_row(db, hash)?
    else {
        return Ok(false);
    };
    // An expired access token can be refreshed, but a missing or explicitly
    // revoked one cannot resurrect its authorization family.
    Ok(!revoked && underlying_active(db, id.as_deref(), admin, false)?)
}
/// Recheck CLI family and configured scope before existing business token code.
/// This never turns an identity credential into a member or administrator token.
pub fn check_business_token(db: &Connection, cfg: &Config, raw: &str) -> AppResult<()> {
    let Some(AccessRecord {
        family_id,
        token_id: id,
        admin,
        expires_at: expires,
        revoked,
    }) = access_row(db, &auth::hash_token(raw))?
    else {
        return Ok(());
    };
    let family = family(db, &family_id)?
        .ok_or_else(|| AppError::unauthorized("Client authorization expired or revoked"))?;
    if revoked
        || expires <= now()
        || family.revoked
        || family.expires_at <= now()
        || family.client_id != cfg.cli_auth.client_id
        || !underlying_active(db, id.as_deref(), admin, true)?
    {
        return Err(AppError::unauthorized(
            "Client authorization expired or revoked",
        ));
    }
    let scope = scope_for_browser(cfg, Some(&family.scope))?;
    if !(scope.mcp || scope.admin) {
        return Err(AppError::forbidden(
            "This credential grants identity access only; authorize the required permission separately",
        ));
    }
    check_scope(db, cfg, family.user_id.as_deref(), family.admin, scope)
}

struct TokenPair {
    access: String,
    refresh: String,
    access_expires: i64,
    refresh_expires: i64,
    scope: String,
}
impl TokenPair {
    fn response(self) -> Json<Value> {
        Json(
            json!({"access_token":self.access,"token_type":"Bearer","expires_in":(self.access_expires-now()).max(0),
        "refresh_token":self.refresh,"refresh_expires_in":(self.refresh_expires-now()).max(0),"scope":self.scope}),
        )
    }
}
fn issue_pair(
    db: &Connection,
    cfg: &Config,
    family: &Family,
    scope: Scopes,
) -> AppResult<TokenPair> {
    let time = now();
    let expires = (time + cfg.cli_auth.access_token_minutes as i64 * 60).min(family.expires_at);
    let id = uuid::Uuid::new_v4().to_string();
    let access = if scope.mcp || scope.admin {
        let mut secret = [0u8; 32];
        OsRng.fill_bytes(&mut secret);
        let raw = format!(
            "{}{}",
            if scope.admin { "hf_admin_" } else { "hf_" },
            hex::encode(secret)
        );
        let active: i64 = if scope.admin {
            db.query_row(
                "SELECT count(*) FROM admin_api_tokens WHERE revoked=0 AND expires_at>?1",
                [time],
                |r| r.get(0),
            )?
        } else {
            db.query_row(
                "SELECT count(*) FROM api_tokens WHERE user_id=?1 AND revoked=0 AND expires_at>?2",
                params![family.user_id, time],
                |r| r.get(0),
            )?
        };
        if active >= cfg.cli_auth.max_active_tokens as i64 {
            return Err(AppError::conflict(
                "Too many active business tokens; revoke an old connection in the website",
            ));
        }
        if scope.admin {
            db.execute("INSERT INTO admin_api_tokens(id,name,token_hash,created_at,expires_at) VALUES(?1,'hfy CLI',?2,?3,?4)",params![id,auth::hash_token(&raw),time,expires])?;
        } else {
            db.execute("INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES(?1,?2,'hfy CLI',?3,?4,?5)",params![id,family.user_id,auth::hash_token(&raw),time,expires])?;
        }
        raw
    } else {
        format!("hf_cli_{}", random_secret())
    };
    let token_id = if scope.mcp || scope.admin {
        Some(id)
    } else {
        None
    };
    let access_hash = auth::hash_token(&access);
    db.execute("INSERT INTO cli_tokens(token_hash,token_id,family_id,admin,created_at,expires_at) VALUES(?1,?2,?3,?4,?5,?6)",params![access_hash,token_id,family.id,family.admin,time,expires])?;
    let refresh = format!("hf_refresh_{}", random_secret());
    db.execute(
        "INSERT INTO cli_refresh_tokens(token_hash,family_id,created_at) VALUES(?1,?2,?3)",
        params![auth::hash_token(&refresh), family.id, time],
    )?;
    db.execute(
        "UPDATE cli_authorizations SET current_access_hash=?2,scope=?3 WHERE id=?1",
        params![family.id, access_hash, scope.text()],
    )?;
    Ok(TokenPair {
        access,
        refresh,
        access_expires: expires,
        refresh_expires: family.expires_at,
        scope: scope.text().into(),
    })
}
fn create_family(
    db: &Connection,
    state: &AppState,
    grant: &Grant,
    scope: Scopes,
) -> AppResult<(String, TokenPair)> {
    let count:i64=db.query_row("SELECT count(*) FROM cli_authorizations WHERE admin=?1 AND user_id IS ?2 AND revoked=0 AND expires_at>?3",params![grant.admin,grant.user_id,now()],|r|r.get(0))?;
    if count >= state.cfg.cli_auth.max_active_tokens as i64 {
        return Err(AppError::conflict(
            "Too many active command-line connections; revoke an old device in the website",
        ));
    }
    let family = Family {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: grant.user_id.clone(),
        admin: grant.admin,
        client_id: grant.client_id.clone(),
        scope: scope.text().into(),
        expires_at: now() + state.cfg.cli_auth.refresh_token_days as i64 * 86400,
        revoked: false,
        current_access_hash: None,
    };
    db.execute("INSERT INTO cli_authorizations(id,user_id,admin,client_id,scope,created_at,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![family.id,family.user_id,family.admin,family.client_id,family.scope,now(),family.expires_at])?;
    let pair = issue_pair(db, &state.cfg, &family, scope)?;
    audit(
        db,
        grant.user_id.as_deref().unwrap_or("admin"),
        "cli_connection_create",
        &family.id,
        scope.text(),
    )?;
    Ok((family.id, pair))
}

#[derive(Deserialize)]
struct TokenRequest {
    client_id: String,
    grant_type: String,
    device_code: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
    scope: Option<String>,
}
struct Grant {
    flow: String,
    client_id: String,
    scope: String,
    redirect_uri: String,
    challenge: String,
    session_hash: Option<String>,
    user_id: Option<String>,
    admin: bool,
    status: String,
    expires_at: i64,
    last_poll_at: i64,
    poll_interval: i64,
    issued_family_id: Option<String>,
}
async fn token(
    State(state): State<AppState>,
    form: Result<Form<TokenRequest>, FormRejection>,
) -> Result<Json<Value>, OAuthError> {
    enabled(&state)?;
    let Form(input) = form.map_err(|_| {
        OAuthError::new(
            "invalid_request",
            "Send the required token parameters as form data",
        )
    })?;
    validate_client(&state.cfg, &input.client_id)?;
    if input.grant_type == "refresh_token" {
        return refresh(&state, input).map(TokenPair::response);
    }
    let device = input.grant_type == "urn:ietf:params:oauth:grant-type:device_code";
    if !device && input.grant_type != "authorization_code" {
        return Err(OAuthError::new(
            "unsupported_grant_type",
            "Use device_code, authorization_code or refresh_token",
        ));
    }
    let secret = if device {
        input.device_code.as_deref()
    } else {
        input.code.as_deref()
    }
    .filter(|v| {
        v.len() == 43
            && v.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    })
    .ok_or_else(OAuthError::invalid_grant)?;
    let hash = auth::hash_token(secret);
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let grant:Option<Grant>=tx.query_row("SELECT flow,client_id,scope,redirect_uri,code_challenge,session_hash,user_id,admin,status,expires_at,last_poll_at,poll_interval,issued_family_id FROM cli_grants WHERE secret_hash=?1",[&hash],|r|Ok(Grant{
        flow:r.get(0)?,client_id:r.get(1)?,scope:r.get(2)?,redirect_uri:r.get(3)?,challenge:r.get(4)?,session_hash:r.get(5)?,user_id:r.get(6)?,admin:r.get(7)?,status:r.get(8)?,expires_at:r.get(9)?,last_poll_at:r.get(10)?,poll_interval:r.get(11)?,issued_family_id:r.get(12)?,
    })).optional()?;
    let grant = grant.ok_or_else(OAuthError::invalid_grant)?;
    if grant.client_id != input.client_id || grant.flow != if device { "device" } else { "oauth" } {
        return Err(OAuthError::invalid_grant());
    }
    if grant.expires_at <= now() {
        return Err(if device {
            OAuthError::new("expired_token", "The device authorization expired")
        } else {
            OAuthError::invalid_grant()
        });
    }
    let scope = requested_scope(&state.cfg, Some(&grant.scope))?;
    if !device {
        let verifier = input
            .code_verifier
            .as_deref()
            .filter(|v| {
                (43..=128).contains(&v.len())
                    && v.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-._~".contains(&c))
            })
            .ok_or_else(OAuthError::invalid_grant)?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        if input.redirect_uri.as_deref() != Some(grant.redirect_uri.as_str())
            || !bool::from(challenge.as_bytes().ct_eq(grant.challenge.as_bytes()))
        {
            return Err(OAuthError::invalid_grant());
        }
    }
    if grant.status == "consumed" {
        if !device {
            if let Some(id) = &grant.issued_family_id {
                revoke_family(&tx, id)?;
            }
            tx.commit()?;
        }
        return Err(OAuthError::invalid_grant());
    }
    if grant.status == "denied" {
        return Err(OAuthError::new(
            "access_denied",
            "The user declined the authorization",
        ));
    }
    if device {
        if grant.last_poll_at != 0 && now() - grant.last_poll_at < grant.poll_interval {
            tx.execute("UPDATE cli_grants SET last_poll_at=?2,poll_interval=poll_interval+5 WHERE secret_hash=?1",params![hash,now()])?;
            tx.commit()?;
            return Err(OAuthError::new(
                "slow_down",
                "Increase the polling interval by five seconds",
            ));
        }
        tx.execute(
            "UPDATE cli_grants SET last_poll_at=?2 WHERE secret_hash=?1",
            params![hash, now()],
        )?;
    }
    if grant.status == "pending" {
        tx.commit()?;
        return Err(OAuthError::new(
            "authorization_pending",
            "Waiting for the user's decision",
        ));
    }
    check_session(
        &tx,
        &state.cfg,
        grant
            .session_hash
            .as_deref()
            .ok_or_else(OAuthError::invalid_grant)?,
        grant.user_id.as_deref(),
        grant.admin,
        scope,
    )?;
    let (family_id, pair) = create_family(&tx, &state, &grant, scope)?;
    tx.execute("UPDATE cli_grants SET status='consumed',issued_family_id=?2 WHERE secret_hash=?1 AND status='approved'",params![hash,family_id])?;
    tx.commit()?;
    Ok(pair.response())
}
fn refresh(state: &AppState, input: TokenRequest) -> Result<TokenPair, OAuthError> {
    let raw = input
        .refresh_token
        .as_deref()
        .filter(|v| v.starts_with("hf_refresh_") && v.len() == 54)
        .ok_or_else(OAuthError::invalid_grant)?;
    let hash = auth::hash_token(raw);
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let record: Option<(String, Option<i64>)> = tx
        .query_row(
            "SELECT family_id,used_at FROM cli_refresh_tokens WHERE token_hash=?1",
            [&hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (id, used_at) = record.ok_or_else(OAuthError::invalid_grant)?;
    let family = family(&tx, &id)?.ok_or_else(OAuthError::invalid_grant)?;
    if family.client_id != input.client_id || family.revoked || family.expires_at <= now() {
        return Err(OAuthError::invalid_grant());
    }
    if used_at.is_some() {
        revoke_family(&tx, &id)?;
        audit(&tx, "cli", "cli_refresh_reuse", &id, "")?;
        tx.commit()?;
        return Err(OAuthError::invalid_grant());
    }
    if !current_access_active(&tx, &family)? {
        revoke_family(&tx, &id)?;
        tx.commit()?;
        return Err(OAuthError::invalid_grant());
    }
    let previous = Scopes::parse(&family.scope)?;
    let scope = if let Some(value) = input.scope.as_deref().filter(|s| !s.is_empty()) {
        requested_scope(&state.cfg, Some(value))?
    } else {
        previous.validate(&state.cfg)?
    };
    if !scope.subset_of(previous) {
        return Err(OAuthError::new(
            "invalid_scope",
            "Refresh cannot add permission; authorize the additional permission in the browser",
        ));
    }
    if let Err(error) = check_scope(
        &tx,
        &state.cfg,
        family.user_id.as_deref(),
        family.admin,
        scope,
    ) {
        revoke_family(&tx, &id)?;
        tx.commit()?;
        return Err(error.into());
    }
    revoke_access(&tx, &id)?;
    tx.execute(
        "UPDATE cli_refresh_tokens SET used_at=?2 WHERE token_hash=?1 AND used_at IS NULL",
        params![hash, now()],
    )?;
    let pair = issue_pair(&tx, &state.cfg, &family, scope)?;
    audit(
        &tx,
        family.user_id.as_deref().unwrap_or("admin"),
        "cli_refresh",
        &id,
        scope.text(),
    )?;
    tx.commit()?;
    Ok(pair)
}

#[derive(Deserialize)]
struct RevokeRequest {
    client_id: String,
    token: String,
}
async fn revoke(
    State(state): State<AppState>,
    form: Result<Form<RevokeRequest>, FormRejection>,
) -> Result<StatusCode, OAuthError> {
    let Form(input) = form
        .map_err(|_| OAuthError::new("invalid_request", "Send client_id and token as form data"))?;
    // Old clients must still be able to revoke after the configured ID changes.
    if input.client_id.is_empty() || input.client_id.len() > 80 || input.token.len() > 256 {
        return Err(OAuthError::new(
            "invalid_request",
            "Invalid revocation request",
        ));
    }
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let id:Option<String>=tx.query_row("SELECT f.id FROM cli_authorizations f WHERE f.client_id=?2 AND f.id IN
        (SELECT family_id FROM cli_tokens WHERE token_hash=?1 UNION SELECT family_id FROM cli_refresh_tokens WHERE token_hash=?1)",params![auth::hash_token(&input.token),input.client_id],|r|r.get(0)).optional()?;
    if let Some(id) = id {
        revoke_family(&tx, &id)?;
        audit(&tx, "cli", "cli_connection_revoke", &id, "")?;
    }
    tx.commit()?;
    Ok(StatusCode::OK)
}
async fn userinfo(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let raw = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::unauthorized("A client bearer token is required"))?;
    let db = state.conn()?;
    let AccessRecord {
        family_id: id,
        token_id,
        admin,
        expires_at: expires,
        revoked,
    } = access_row(&db, &auth::hash_token(raw))?
        .ok_or_else(|| AppError::unauthorized("Client token expired or revoked"))?;
    let family = family(&db, &id)?
        .ok_or_else(|| AppError::unauthorized("Client authorization expired or revoked"))?;
    if revoked
        || expires <= now()
        || family.revoked
        || family.expires_at <= now()
        || !underlying_active(&db, token_id.as_deref(), admin, true)?
    {
        return Err(AppError::unauthorized(
            "Client authorization expired or revoked",
        ));
    }
    check_identity(&db, family.user_id.as_deref(), family.admin)?;
    let scope = Scopes::parse(&family.scope)
        .map_err(|_| AppError::unauthorized("Invalid client permission"))?;
    let eligible = ai_eligibility(&db, family.user_id.as_deref(), family.admin)?;
    let allowed =
        family.client_id == state.cfg.cli_auth.client_id && scope.validate(&state.cfg).is_ok();
    Ok(Json(
        json!({"admin":family.admin,"user":identity(&db,family.user_id.as_deref())?,"scope":family.scope,"expires_at":expires,
        "ai_eligibility":eligible,"capabilities":{"identity":true,"mcp":allowed&&scope.mcp&&eligible["eligible"]==true,"admin":allowed&&scope.admin}}),
    ))
}
async fn connections_page() -> Html<&'static str> {
    Html(include_str!("../static/cli-connections.html"))
}
async fn connections(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let session = browser_session(&state, &headers, false)?;
    let db = state.conn()?;
    check_session(
        &db,
        &state.cfg,
        &session.token_hash,
        session.user_id.as_deref(),
        session.admin,
        Scopes {
            mcp: false,
            admin: false,
        },
    )?;
    let items=db.prepare("SELECT id,client_id,scope,created_at,expires_at,revoked FROM cli_authorizations WHERE admin=?1 AND user_id IS ?2 ORDER BY created_at DESC,id DESC LIMIT 200")?
        .query_map(params![session.admin,session.user_id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"client_id":r.get::<_,String>(1)?,"scope":r.get::<_,String>(2)?,"created_at":r.get::<_,i64>(3)?,"expires_at":r.get::<_,i64>(4)?,"revoked":r.get::<_,bool>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Json(json!({"items":items})))
}
async fn disconnect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let session = browser_session(&state, &headers, true)?;
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    check_session(
        &tx,
        &state.cfg,
        &session.token_hash,
        session.user_id.as_deref(),
        session.admin,
        Scopes {
            mcp: false,
            admin: false,
        },
    )?;
    let owns:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM cli_authorizations WHERE id=?1 AND admin=?2 AND user_id IS ?3)",params![id,session.admin,session.user_id],|r|r.get(0))?;
    if !owns {
        return Err(AppError::not_found("Client connection not found"));
    }
    revoke_family(&tx, &id)?;
    audit(
        &tx,
        session.user_id.as_deref().unwrap_or("admin"),
        "cli_connection_revoke",
        &id,
        "",
    )?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}
async fn metadata(State(state): State<AppState>) -> AppResult<Json<Value>> {
    enabled(&state)?;
    let base = issuer(&state);
    Ok(Json(
        json!({"issuer":base,"authorization_endpoint":format!("{base}/oauth/authorize"),"device_authorization_endpoint":format!("{base}/oauth/device/code"),
        "token_endpoint":format!("{base}/oauth/token"),"revocation_endpoint":format!("{base}/oauth/revoke"),"userinfo_endpoint":format!("{base}/oauth/userinfo"),
        "grant_types_supported":["authorization_code","urn:ietf:params:oauth:grant-type:device_code","refresh_token"],"response_types_supported":["code"],
        "code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"],"revocation_endpoint_auth_methods_supported":["none"],
        "scopes_supported":state.cfg.cli_auth.allowed_scopes,"authorization_response_iss_parameter_supported":true,"client_id":state.cfg.cli_auth.client_id}),
    ))
}
