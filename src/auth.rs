//! Cookie authentication and the administrator's passkey-only sign-in.
//!
//! Challenges stay on the server, expire, are browser-bound, and can be consumed
//! once. The command-line bootstrap password cannot grant console access.

use crate::{AppError, AppResult, AppState, config::Config};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::header::ContentType,
    transport::smtp::authentication::Credentials,
};
use rand::{Rng, RngCore, rngs::OsRng};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;
use uuid::Uuid;
use webauthn_rs::prelude::*;

const SESSION_COOKIE: &str = "hf_session";
const EMAIL_COOKIE: &str = "hf_email_challenge";
const GOOGLE_COOKIE: &str = "hf_google_challenge";
const PASSKEY_COOKIE: &str = "hf_passkey_challenge";
const CHALLENGE_TTL: i64 = 300;
const EMAIL_TTL: i64 = 600;
const RECENT_AUTH_SECONDS: i64 = 600;
const ADMIN_UUID: &str = "fe9ea666-4f30-437c-a7e2-d87b4ee80e78";

pub struct AuthService {
    webauthn: Webauthn,
    mailer: Option<AsyncSmtpTransport<Tokio1Executor>>,
}

impl AuthService {
    pub fn new(config: &Config) -> AppResult<Self> {
        let origin = Url::parse(&config.public_url).map_err(AppError::internal)?;
        let webauthn = WebauthnBuilder::new(&config.webauthn_rp_id, &origin)
            .map_err(AppError::internal)?
            .rp_name("HireFerry · 聘渡")
            .build()
            .map_err(AppError::internal)?;
        let mailer = if config.smtp.enabled {
            let mut builder = if config.smtp.starttls {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.smtp.host)
            } else {
                AsyncSmtpTransport::<Tokio1Executor>::relay(&config.smtp.host)
            }
            .map_err(AppError::internal)?
            .port(config.smtp.port)
            .timeout(Some(Duration::from_secs(15)));
            if !config.smtp.username.is_empty() {
                builder = builder.credentials(Credentials::new(
                    config.smtp.username.clone(),
                    config.smtp.password.clone(),
                ));
            }
            Some(builder.build())
        } else {
            None
        };
        Ok(Self { webauthn, mailer })
    }

    pub(crate) fn smtp_enabled(&self) -> bool {
        self.mailer.is_some()
    }

    /// Only the passkey-authenticated owner may request this mail via staff routes.
    pub(crate) async fn send_staff_invitation(
        &self,
        config: &Config,
        email: &str,
        link: &str,
    ) -> AppResult<()> {
        let mailer = self
            .mailer
            .as_ref()
            .ok_or_else(|| AppError::unavailable("SMTP is not configured"))?;
        let mut builder = Message::builder()
            .from(config.smtp.from.parse().map_err(AppError::internal)?)
            .to(email.parse().map_err(AppError::internal)?)
            .subject("聘渡 HireFerry · 团队经理邀请 / Manager invitation")
            .header(ContentType::TEXT_PLAIN);
        if !config.smtp.reply_to.is_empty() {
            builder = builder.reply_to(config.smtp.reply_to.parse().map_err(AppError::internal)?);
        }
        let message = builder.body(format!(
            "你收到了一份聘渡团队经理邀请。请使用收件邮箱登录并在 7 天内接受：\n{link}\n\n             You are invited to join the HireFerry team as a manager. Sign in with this email and accept within 7 days:\n{link}\n\n             如果你不认识邀请人，请忽略此邮件。不要转发邀请链接。\n             If unexpected, ignore this email. Do not forward the invitation link."
        )).map_err(AppError::internal)?;
        mailer
            .send(message)
            .await
            .map_err(|_| AppError::unavailable("Could not deliver the staff invitation email"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Session {
    #[serde(skip)]
    pub token_hash: String,
    pub user_id: Option<String>,
    pub admin: bool,
    pub bootstrap: bool,
    pub csrf: String,
    pub expires_at: i64,
    pub recent_auth_at: i64,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/session", get(get_session))
        .route("/api/auth/email/start", post(email_start))
        .route("/api/auth/email/verify", post(email_verify))
        .route("/api/auth/google/start", get(google_start))
        .route("/auth/google/callback", get(google_callback))
        .route("/api/auth/logout", post(logout))
        .route("/api/admin/auth/status", get(admin_status))
        .route("/api/admin/auth/bootstrap", post(admin_bootstrap))
        .route("/api/admin/passkeys", get(passkeys_list))
        .route("/api/admin/passkeys/{id}", delete(passkey_delete))
        .route(
            "/api/admin/passkeys/register/start",
            post(passkey_register_start),
        )
        .route(
            "/api/admin/passkeys/register/finish",
            post(passkey_register_finish),
        )
        .route("/api/admin/passkeys/login/start", post(passkey_login_start))
        .route(
            "/api/admin/passkeys/login/finish",
            post(passkey_login_finish),
        )
}

pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            token_hash TEXT PRIMARY KEY,
            user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
            admin INTEGER NOT NULL DEFAULT 0 CHECK(admin IN (0,1)),
            bootstrap INTEGER NOT NULL DEFAULT 0 CHECK(bootstrap IN (0,1)),
            csrf TEXT NOT NULL, expires_at INTEGER NOT NULL,
            recent_auth_at INTEGER NOT NULL,
            CHECK(NOT(admin=1 AND bootstrap=1)),
            CHECK(NOT(user_id IS NOT NULL AND (admin=1 OR bootstrap=1)))
        );
        CREATE INDEX IF NOT EXISTS sessions_expiry ON sessions(expires_at);
        CREATE INDEX IF NOT EXISTS sessions_user ON sessions(user_id);
        CREATE TABLE IF NOT EXISTS auth_challenges (
            id TEXT PRIMARY KEY, kind TEXT NOT NULL, subject TEXT NOT NULL,
            secret_hash TEXT NOT NULL DEFAULT '', payload TEXT NOT NULL DEFAULT '{}',
            expires_at INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS auth_challenges_expiry ON auth_challenges(expires_at);
        CREATE INDEX IF NOT EXISTS auth_challenges_email ON auth_challenges(subject) WHERE kind='email';
        CREATE INDEX IF NOT EXISTS api_tokens_user ON api_tokens(user_id);
        CREATE TABLE IF NOT EXISTS auth_limits (
            key TEXT PRIMARY KEY, window_start INTEGER NOT NULL, count INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS admin_bootstrap (
            singleton INTEGER PRIMARY KEY CHECK(singleton=1),
            secret_hash TEXT NOT NULL, expires_at INTEGER NOT NULL,
            used INTEGER NOT NULL DEFAULT 0 CHECK(used IN (0,1))
        );
        CREATE TABLE IF NOT EXISTS admin_passkeys (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, credential_json TEXT NOT NULL,
            created_at INTEGER NOT NULL, last_used_at INTEGER,
            counter INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS account_access_clock (
            id INTEGER PRIMARY KEY CHECK(id=1), generation INTEGER NOT NULL DEFAULT 0
        );
        INSERT OR IGNORE INTO account_access_clock(id) VALUES(1);
        CREATE TABLE IF NOT EXISTS account_moderation (
            user_id TEXT PRIMARY KEY REFERENCES users(id),
            banned INTEGER NOT NULL CHECK(banned IN (0,1)),
            reason TEXT NOT NULL, changed_by TEXT NOT NULL,
            changed_at INTEGER NOT NULL, generation INTEGER NOT NULL
        );
        CREATE TRIGGER IF NOT EXISTS account_access_changed AFTER UPDATE OF banned ON users
        WHEN OLD.banned IS NOT NEW.banned BEGIN
            UPDATE account_access_clock SET generation=generation+1 WHERE id=1;
            INSERT INTO account_moderation(user_id,banned,reason,changed_by,changed_at,generation)
            VALUES(NEW.id,NEW.banned,CASE WHEN NEW.banned=1 THEN 'Account suspended' ELSE 'Access restored' END,
                'system',unixepoch(),(SELECT generation FROM account_access_clock WHERE id=1))
            ON CONFLICT(user_id) DO UPDATE SET banned=excluded.banned,reason=excluded.reason,
                changed_by=excluded.changed_by,changed_at=excluded.changed_at,generation=excluded.generation;
            DELETE FROM sessions WHERE NEW.banned=1 AND user_id=NEW.id;
            UPDATE api_tokens SET revoked=1 WHERE NEW.banned=1 AND user_id=NEW.id;
            DELETE FROM auth_challenges WHERE NEW.banned=1 AND kind='email' AND subject=NEW.email COLLATE NOCASE;
        END;",
    )?;
    Ok(())
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn equal_hash(a: &str, b: &str) -> bool {
    if a.len() != 64 || b.len() != 64 {
        return false;
    }
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

fn cookie(headers: &HeaderMap, key: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            (name == key && value.len() <= 128).then(|| value.to_owned())
        })
}

fn cookie_value(config: &Config, name: &str, value: &str, max_age: i64) -> String {
    let secure = if config.public_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
}

fn with_cookie(
    mut response: Response,
    config: &Config,
    name: &str,
    value: &str,
    ttl: i64,
) -> AppResult<Response> {
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie_value(config, name, value, ttl))
            .map_err(AppError::internal)?,
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// Require the configured origin rather than trusting a Host or forwarding header.
pub fn origin_check(state: &AppState, headers: &HeaderMap) -> AppResult<()> {
    let allowed = Url::parse(&state.cfg.public_url)
        .map_err(AppError::internal)?
        .origin()
        .ascii_serialization();
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if origin != Some(allowed.as_str()) {
        return Err(AppError::forbidden("Request origin is not allowed"));
    }
    Ok(())
}

pub fn session(state: &AppState, headers: &HeaderMap) -> AppResult<Option<Session>> {
    let Some(raw) = cookie(headers, SESSION_COOKIE) else {
        return Ok(None);
    };
    let db = state
        .db
        .lock()
        .map_err(|_| AppError::internal("Database lock failed"))?;
    let found = db
        .query_row(
            "SELECT s.token_hash,s.user_id,s.admin,s.bootstrap,s.csrf,s.expires_at,s.recent_auth_at
         FROM sessions s LEFT JOIN users u ON s.user_id=u.id
         WHERE s.token_hash=?1 AND s.expires_at>?2
         AND (s.user_id IS NULL OR (u.id IS NOT NULL AND u.banned=0))",
            params![hash_token(&raw), now()],
            |r| {
                Ok(Session {
                    token_hash: r.get(0)?,
                    user_id: r.get(1)?,
                    admin: r.get(2)?,
                    bootstrap: r.get(3)?,
                    csrf: r.get(4)?,
                    expires_at: r.get(5)?,
                    recent_auth_at: r.get(6)?,
                })
            },
        )
        .optional()?;
    Ok(found)
}

pub fn require_session(state: &AppState, headers: &HeaderMap) -> AppResult<Session> {
    session(state, headers)?.ok_or_else(|| AppError::unauthorized("Please sign in"))
}

pub fn require_user(state: &AppState, headers: &HeaderMap) -> AppResult<String> {
    let s = require_session(state, headers)?;
    if s.admin || s.bootstrap {
        return Err(AppError::forbidden("A member account is required"));
    }
    s.user_id
        .ok_or_else(|| AppError::unauthorized("Please sign in"))
}

pub fn require_admin(state: &AppState, headers: &HeaderMap) -> AppResult<Session> {
    let s = require_session(state, headers)?;
    if !s.admin || s.bootstrap {
        return Err(AppError::forbidden("Passkey sign-in is required"));
    }
    Ok(s)
}

pub fn csrf_check(state: &AppState, headers: &HeaderMap, session: &Session) -> AppResult<()> {
    origin_check(state, headers)?;
    let supplied = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !equal_hash(&hash_token(supplied), &hash_token(&session.csrf)) {
        return Err(AppError::forbidden(
            "Request verification failed; reload and try again",
        ));
    }
    Ok(())
}

fn require_recent(s: &Session) -> AppResult<()> {
    if now().saturating_sub(s.recent_auth_at) > RECENT_AUTH_SECONDS {
        return Err(AppError::forbidden(
            "Sign in with a passkey again before changing passkeys",
        ));
    }
    Ok(())
}

// Recheck under the write transaction. A shell reset or another device can
// revoke a session after the request's initial authentication check.
fn require_active_session(db: &Connection, s: &Session) -> AppResult<()> {
    let active:bool=db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=?1 AND expires_at>?2 AND admin=?3 AND bootstrap=?4)",
        params![s.token_hash,now(),s.admin,s.bootstrap],|r|r.get(0))?;
    if !active {
        return Err(AppError::unauthorized(
            "This session expired or was revoked",
        ));
    }
    Ok(())
}

fn rate_limit(db: &Connection, key: &str, limit: i64, window: i64) -> AppResult<()> {
    let t = now();
    db.execute(
        "DELETE FROM auth_limits WHERE rowid IN
         (SELECT rowid FROM auth_limits WHERE window_start<?1 ORDER BY window_start LIMIT ?2)",
        params![t - 86_400, crate::maintenance::DEFAULT_BATCH_SIZE],
    )?;
    db.execute(
        "INSERT INTO auth_limits(key,window_start,count) VALUES(?1,?2,1)
         ON CONFLICT(key) DO UPDATE SET
         count=CASE WHEN window_start<=?3 THEN 1 ELSE count+1 END,
         window_start=CASE WHEN window_start<=?3 THEN ?2 ELSE window_start END",
        params![key, t, t - window],
    )?;
    let count: i64 = db.query_row("SELECT count FROM auth_limits WHERE key=?1", [key], |r| {
        r.get(0)
    })?;
    if count > limit {
        return Err(AppError {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: "Too many attempts; please try again later".into(),
        });
    }
    Ok(())
}

fn cleanup(db: &Connection) -> AppResult<()> {
    for table in ["sessions", "auth_challenges"] {
        db.execute(
            &format!(
                "DELETE FROM {table} WHERE rowid IN
                (SELECT rowid FROM {table} WHERE expires_at<=?1 ORDER BY expires_at LIMIT ?2)"
            ),
            params![now(), crate::maintenance::DEFAULT_BATCH_SIZE],
        )?;
    }
    Ok(())
}

fn new_session(
    db: &Connection,
    config: &Config,
    user_id: Option<&str>,
    admin: bool,
    bootstrap: bool,
) -> AppResult<(String, String)> {
    if let Some(user_id) = user_id {
        let active: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND banned=0)",
            [user_id],
            |r| r.get(0),
        )?;
        if !active {
            return Err(AppError::forbidden("This account is suspended"));
        }
    }
    cleanup(db)?;
    let raw = random_token();
    let csrf = random_token();
    let ttl = if bootstrap {
        CHALLENGE_TTL
    } else {
        (config.session_hours as i64).saturating_mul(3600)
    };
    db.execute(
        "INSERT INTO sessions(token_hash,user_id,admin,bootstrap,csrf,expires_at,recent_auth_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            hash_token(&raw),
            user_id,
            admin,
            bootstrap,
            csrf,
            now() + ttl,
            now()
        ],
    )?;
    Ok((raw, csrf))
}

fn session_response(
    state: &AppState,
    raw: &str,
    payload: Value,
    bootstrap: bool,
) -> AppResult<Response> {
    let ttl = if bootstrap {
        CHALLENGE_TTL
    } else {
        (state.cfg.session_hours as i64).saturating_mul(3600)
    };
    with_cookie(
        Json(payload).into_response(),
        &state.cfg,
        SESSION_COOKIE,
        raw,
        ttl,
    )
}

async fn get_session(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    let Some(s) = session(&state, &headers)? else {
        return Ok(Json(
            json!({"user":null,"admin":false,"bootstrap":false,"staff_role":null,"csrf":null}),
        )
        .into_response());
    };
    let (user, staff_role) = if let Some(id) = &s.user_id {
        let db = state.conn()?;
        (
            serde_json::to_value(crate::models::load_user(&db, id)?)?,
            crate::staff::active_role(&db, id)?,
        )
    } else {
        (Value::Null, None)
    };
    Ok(
        Json(json!({"user":user,"admin":s.admin,"bootstrap":s.bootstrap,
                    "staff_role":staff_role,"csrf":s.csrf}))
        .into_response(),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailStart {
    email: String,
}

pub(crate) fn normalized_email(value: &str) -> AppResult<String> {
    if value.contains(['\r', '\n']) {
        return Err(AppError::bad_request("Enter a valid email address"));
    }
    let value = value.trim().to_ascii_lowercase();
    if value.len() > 254 || value.contains(['\r', '\n', ' ', '\t']) || !value.contains('@') {
        return Err(AppError::bad_request("Enter a valid email address"));
    }
    let parsed: lettre::Address = value
        .parse()
        .map_err(|_| AppError::bad_request("Enter a valid email address"))?;
    if !parsed.domain().contains('.') && parsed.domain() != "localhost" {
        return Err(AppError::bad_request("Enter a valid email address"));
    }
    Ok(value)
}

async fn email_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EmailStart>,
) -> AppResult<Response> {
    origin_check(&state, &headers)?;
    let email = normalized_email(&body.email)?;
    if !state.cfg.development && state.auth.mailer.is_none() {
        return Err(AppError::unavailable("Email sign-in is not configured"));
    }
    let id = random_token();
    let code = format!("{:08}", OsRng.gen_range(0..100_000_000_u32));
    {
        let db = state
            .db
            .lock()
            .map_err(|_| AppError::internal("Database lock failed"))?;
        cleanup(&db)?;
        rate_limit(&db, "email-global", 100, 300)?;
        rate_limit(&db, &format!("email-short:{}", hash_token(&email)), 1, 60)?;
        rate_limit(&db, &format!("email-long:{}", hash_token(&email)), 10, 3600)?;
        // A newer request invalidates all older codes for this address.
        db.execute(
            "DELETE FROM auth_challenges WHERE kind='email' AND subject=?1",
            [&email],
        )?;
        let blocked: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE email=?1 AND banned=1)",
            [&email],
            |row| row.get(0),
        )?;
        if blocked {
            // Keep the public response neutral: this request must neither send
            // mail nor disclose whether this address belongs to a blocked user.
            return with_cookie(
                Json(json!({"sent":true})).into_response(),
                &state.cfg,
                EMAIL_COOKIE,
                "",
                0,
            );
        }
        db.execute(
            "INSERT INTO auth_challenges(id,kind,subject,secret_hash,expires_at) VALUES(?1,'email',?2,?3,?4)",
            params![hash_token(&id),email,hash_token(&format!("{id}:{code}")),now()+EMAIL_TTL],
        )?;
    }
    if let Some(mailer) = &state.auth.mailer {
        let mut message = Message::builder()
            .from(state.cfg.smtp.from.parse().map_err(AppError::internal)?)
            .to(email.parse().map_err(AppError::internal)?)
            .subject("聘渡 HireFerry · 登录验证码 / Sign-in code")
            .header(ContentType::TEXT_PLAIN);
        if !state.cfg.smtp.reply_to.is_empty() {
            message = message.reply_to(
                state
                    .cfg
                    .smtp
                    .reply_to
                    .parse()
                    .map_err(AppError::internal)?,
            );
        }
        let message = message
            .body(format!("你的聘渡登录验证码：{code}\n\nYour HireFerry sign-in code: {code}\n\n10分钟内有效。请勿转发。\nValid for 10 minutes. Do not share this code.\n\n如果不是你本人操作，请忽略这封邮件。\nIf you did not request this, ignore this email."))
            .map_err(AppError::internal)?;
        if mailer.send(message).await.is_err() {
            let db = state
                .db
                .lock()
                .map_err(|_| AppError::internal("Database lock failed"))?;
            db.execute("DELETE FROM auth_challenges WHERE id=?1", [hash_token(&id)])?;
            return Err(AppError::unavailable(
                "We could not send the email; try again later",
            ));
        }
    }
    let payload = if state.cfg.development && state.auth.mailer.is_none() {
        json!({"sent":true,"dev_code":code})
    } else {
        json!({"sent":true})
    };
    with_cookie(
        Json(payload).into_response(),
        &state.cfg,
        EMAIL_COOKIE,
        &id,
        EMAIL_TTL,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailVerify {
    email: String,
    code: String,
}

async fn email_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EmailVerify>,
) -> AppResult<Response> {
    origin_check(&state, &headers)?;
    let email = normalized_email(&body.email)?;
    let id = cookie(&headers, EMAIL_COOKIE)
        .ok_or_else(|| AppError::unauthorized("Request a new sign-in code"))?;
    if body.code.len() != 8 || !body.code.bytes().all(|c| c.is_ascii_digit()) {
        return Err(AppError::unauthorized("The code is invalid or expired"));
    }
    let (raw, csrf, user) = {
        let mut db = state
            .db
            .lock()
            .map_err(|_| AppError::internal("Database lock failed"))?;
        rate_limit(
            &db,
            &format!("email-verify:{}", hash_token(&email)),
            30,
            300,
        )?;
        let row = db.query_row(
            "SELECT secret_hash,attempts FROM auth_challenges WHERE id=?1 AND kind='email' AND subject=?2 AND expires_at>?3",
            params![hash_token(&id),email,now()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)),
        ).optional()?;
        let (expected, attempts) =
            row.ok_or_else(|| AppError::unauthorized("The code is invalid or expired"))?;
        if attempts >= 5 {
            return Err(AppError::unauthorized("Request a new sign-in code"));
        }
        // This update is intentionally committed even when verification fails.
        db.execute(
            "UPDATE auth_challenges SET attempts=attempts+1 WHERE id=?1",
            [hash_token(&id)],
        )?;
        if !equal_hash(&expected, &hash_token(&format!("{id}:{}", body.code))) {
            return Err(AppError::unauthorized("The code is invalid or expired"));
        }
        let tx = db.transaction()?;
        tx.execute("DELETE FROM auth_challenges WHERE id=?1", [hash_token(&id)])?;
        let existing = tx
            .query_row(
                "SELECT id,banned FROM users WHERE email=?1",
                [&email],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
            )
            .optional()?;
        let user_id = match existing {
            Some((_, true)) => {
                tx.commit()?;
                return Err(AppError::forbidden("This account is suspended"));
            }
            Some((id, false)) => id,
            None => {
                let uid = Uuid::new_v4().to_string();
                tx.execute("INSERT INTO users(id,email,name,role,created_at,updated_at) VALUES(?1,?2,'','unassigned',?3,?3)",params![uid,email,now()])?;
                uid
            }
        };
        let user = crate::models::load_user(&tx, &user_id)?;
        if let Some(old) = cookie(&headers, SESSION_COOKIE) {
            tx.execute(
                "DELETE FROM sessions WHERE token_hash=?1",
                [hash_token(&old)],
            )?;
        }
        let (raw, csrf) = new_session(&tx, &state.cfg, Some(&user_id), false, false)?;
        tx.commit()?;
        (raw, csrf, user)
    };
    let response = session_response(&state, &raw, json!({"user":user,"csrf":csrf}), false)?;
    with_cookie(response, &state.cfg, EMAIL_COOKIE, "", 0)
}

#[derive(Deserialize, Serialize)]
struct GooglePending {
    verifier: String,
    #[serde(default)]
    access_generation: i64,
}

fn google_access_unchanged(db: &Connection, user_id: &str, generation: i64) -> AppResult<()> {
    let allowed: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM users u LEFT JOIN account_moderation a ON a.user_id=u.id WHERE u.id=?1 AND u.banned=0 AND COALESCE(a.generation,0)<=?2)",
        params![user_id, generation],
        |row| row.get(0),
    )?;
    if !allowed {
        return Err(AppError::unauthorized(
            "Account access changed; start Google sign-in again",
        ));
    }
    Ok(())
}

async fn google_start(State(state): State<AppState>) -> AppResult<Response> {
    if !state.cfg.google.enabled {
        return Err(AppError::unavailable("Google sign-in is not configured"));
    }
    let id = random_token();
    let verifier = random_token();
    let pkce = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    {
        let db = state
            .db
            .lock()
            .map_err(|_| AppError::internal("Database lock failed"))?;
        cleanup(&db)?;
        rate_limit(&db, "google-start", 300, 300)?;
        let access_generation: i64 = db.query_row(
            "SELECT generation FROM account_access_clock WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        db.execute("INSERT INTO auth_challenges(id,kind,subject,payload,expires_at) VALUES(?1,'google','',?2,?3)",
            params![hash_token(&id),serde_json::to_string(&GooglePending{verifier,access_generation})?,now()+CHALLENGE_TTL])?;
    }
    let callback = format!(
        "{}/auth/google/callback",
        state.cfg.public_url.trim_end_matches('/')
    );
    let mut url =
        Url::parse("https://accounts.google.com/o/oauth2/v2/auth").map_err(AppError::internal)?;
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", state.cfg.google.client_id.as_str()),
        ("redirect_uri", callback.as_str()),
        ("scope", "openid email profile"),
        ("state", id.as_str()),
        ("code_challenge", pkce.as_str()),
        ("code_challenge_method", "S256"),
        ("prompt", "select_account"),
    ]);
    with_cookie(
        Redirect::to(url.as_str()).into_response(),
        &state.cfg,
        GOOGLE_COOKIE,
        &id,
        CHALLENGE_TTL,
    )
}

#[derive(Deserialize)]
struct GoogleCallback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct GoogleToken {
    access_token: String,
    token_type: String,
}

#[derive(Deserialize)]
struct GoogleUser {
    sub: String,
    email: String,
    email_verified: bool,
    #[serde(default)]
    name: String,
}

async fn google_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<GoogleCallback>,
) -> AppResult<Response> {
    if !state.cfg.google.enabled {
        return Err(AppError::unavailable("Google sign-in is not configured"));
    }
    let id = cookie(&headers, GOOGLE_COOKIE)
        .ok_or_else(|| AppError::unauthorized("Start Google sign-in again"))?;
    let returned = input.state.as_deref().unwrap_or("");
    if !equal_hash(&hash_token(&id), &hash_token(returned)) {
        return Err(AppError::unauthorized("Google sign-in verification failed"));
    }
    let pending = {
        let db = state
            .db
            .lock()
            .map_err(|_| AppError::internal("Database lock failed"))?;
        consume_challenge(&db, &id, "google", None)?
    };
    if input.error.is_some() {
        return Ok(Redirect::to("/login?error=google_cancelled").into_response());
    }
    let code = input
        .code
        .filter(|c| !c.is_empty() && c.len() < 4096)
        .ok_or_else(|| AppError::unauthorized("Google sign-in was not completed"))?;
    let pending: GooglePending = serde_json::from_str(&pending)?;
    let callback = format!(
        "{}/auth/google/callback",
        state.cfg.public_url.trim_end_matches('/')
    );
    let token = state
        .client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", state.cfg.google.client_id.as_str()),
            ("client_secret", state.cfg.google.client_secret.as_str()),
            ("code", code.as_str()),
            ("redirect_uri", callback.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", pending.verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|_| AppError::unavailable("Google sign-in is temporarily unavailable"))?
        .error_for_status()
        .map_err(|_| AppError::unauthorized("Google sign-in was not accepted"))?
        .json::<GoogleToken>()
        .await
        .map_err(|_| AppError::unauthorized("Invalid Google sign-in response"))?;
    if !token.token_type.eq_ignore_ascii_case("bearer") {
        return Err(AppError::unauthorized("Invalid Google token type"));
    }
    // Identity is fetched with the access token from our own authenticated code
    // exchange. We never accept identity claims, emails or ID tokens from JS.
    let identity = state
        .client
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(&token.access_token)
        .send()
        .await
        .map_err(|_| AppError::unavailable("Google sign-in is temporarily unavailable"))?
        .error_for_status()
        .map_err(|_| AppError::unauthorized("Google identity could not be verified"))?
        .json::<GoogleUser>()
        .await
        .map_err(|_| AppError::unauthorized("Invalid Google identity response"))?;
    if !identity.email_verified || identity.sub.is_empty() || identity.sub.len() > 255 {
        return Err(AppError::unauthorized(
            "A verified Google email address is required",
        ));
    }
    let email = normalized_email(&identity.email)?;
    let raw = {
        let mut db = state
            .db
            .lock()
            .map_err(|_| AppError::internal("Database lock failed"))?;
        let tx = db.transaction()?;
        let found = tx
            .query_row(
                "SELECT id,banned FROM users WHERE google_sub=?1",
                [&identity.sub],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
            )
            .optional()?;
        let uid = match found {
            Some((_, true)) => return Err(AppError::forbidden("This account is suspended")),
            Some((uid, false)) => uid,
            None => {
                let email_exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM users WHERE email=?1)",
                    [&email],
                    |r| r.get(0),
                )?;
                // Prevent a Google account's changing/recycled email from taking
                // over an already registered email identity.
                if email_exists {
                    return Err(AppError::conflict(
                        "This email already has an account. Use email sign-in.",
                    ));
                }
                let uid = Uuid::new_v4().to_string();
                let display = identity.name.chars().take(80).collect::<String>();
                tx.execute("INSERT INTO users(id,email,google_sub,name,role,created_at,updated_at) VALUES(?1,?2,?3,?4,'unassigned',?5,?5)",
                    params![uid,email,identity.sub,display,now()])?;
                uid
            }
        };
        google_access_unchanged(&tx, &uid, pending.access_generation)?;
        if let Some(old) = cookie(&headers, SESSION_COOKIE) {
            tx.execute(
                "DELETE FROM sessions WHERE token_hash=?1",
                [hash_token(&old)],
            )?;
        }
        let (raw, _) = new_session(&tx, &state.cfg, Some(&uid), false, false)?;
        tx.commit()?;
        raw
    };
    let response = with_cookie(
        Redirect::to("/app").into_response(),
        &state.cfg,
        SESSION_COOKIE,
        &raw,
        (state.cfg.session_hours as i64) * 3600,
    )?;
    with_cookie(response, &state.cfg, GOOGLE_COOKIE, "", 0)
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    let s = require_session(&state, &headers)?;
    csrf_check(&state, &headers, &s)?;
    let db = state
        .db
        .lock()
        .map_err(|_| AppError::internal("Database lock failed"))?;
    db.execute("DELETE FROM sessions WHERE token_hash=?1", [&s.token_hash])?;
    with_cookie(
        Json(json!({"ok":true})).into_response(),
        &state.cfg,
        SESSION_COOKIE,
        "",
        0,
    )
}

/// Only the local operator can create the first sign-in secret. The presence of
/// any passkey permanently disables password bootstrap until a deliberate local
/// recovery procedure resets the database. No password recovery HTTP route exists.
pub fn generate_bootstrap(db: &Connection) -> AppResult<String> {
    let count: i64 = db.query_row("SELECT COUNT(*) FROM admin_passkeys", [], |r| r.get(0))?;
    if count != 0 {
        return Err(AppError::conflict(
            "Administrator passkeys already exist. Password bootstrap is disabled.",
        ));
    }
    let raw = random_token();
    db.execute("INSERT INTO admin_bootstrap(singleton,secret_hash,expires_at,used) VALUES(1,?1,?2,0)
        ON CONFLICT(singleton) DO UPDATE SET secret_hash=excluded.secret_hash,expires_at=excluded.expires_at,used=0",
        params![hash_token(&raw),now()+600])?;
    db.execute("DELETE FROM sessions WHERE bootstrap=1", [])?;
    Ok(raw)
}

async fn admin_status(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let db = state
        .db
        .lock()
        .map_err(|_| AppError::internal("Database lock failed"))?;
    let initialized: bool =
        db.query_row("SELECT EXISTS(SELECT 1 FROM admin_passkeys)", [], |r| {
            r.get(0)
        })?;
    let available: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM admin_bootstrap WHERE used=0 AND expires_at>?1)",
        [now()],
        |r| r.get(0),
    )?;
    Ok(Json(
        json!({"initialized":initialized,"bootstrap_available":!initialized&&available}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bootstrap {
    password: String,
}

async fn admin_bootstrap(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Bootstrap>,
) -> AppResult<Response> {
    origin_check(&state, &headers)?;
    if body.password.len() > 128 {
        return Err(AppError::unauthorized(
            "The one-time password is invalid or expired",
        ));
    }
    let (raw, csrf) = {
        let mut db = state
            .db
            .lock()
            .map_err(|_| AppError::internal("Database lock failed"))?;
        rate_limit(&db, "admin-bootstrap", 10, 300)?;
        let tx = db.transaction()?;
        let initialized: bool =
            tx.query_row("SELECT EXISTS(SELECT 1 FROM admin_passkeys)", [], |r| {
                r.get(0)
            })?;
        if initialized {
            return Err(AppError::forbidden("Use a passkey to sign in"));
        }
        let secret = tx.query_row("SELECT secret_hash FROM admin_bootstrap WHERE singleton=1 AND used=0 AND expires_at>?1",[now()],|r|r.get::<_,String>(0)).optional()?;
        if !secret
            .as_deref()
            .is_some_and(|v| equal_hash(v, &hash_token(&body.password)))
        {
            return Err(AppError::unauthorized(
                "The one-time password is invalid or expired",
            ));
        }
        tx.execute("UPDATE admin_bootstrap SET used=1 WHERE singleton=1", [])?;
        tx.execute("DELETE FROM sessions WHERE bootstrap=1", [])?;
        crate::db::audit(&tx, "bootstrap", "bootstrap_consumed", "admin", "")?;
        let result = new_session(&tx, &state.cfg, None, false, true)?;
        tx.commit()?;
        result
    };
    session_response(
        &state,
        &raw,
        json!({"bootstrap":true,"admin":false,"csrf":csrf}),
        true,
    )
}

fn consume_challenge(
    db: &Connection,
    raw: &str,
    kind: &str,
    subject: Option<&str>,
) -> AppResult<String> {
    let row = db
        .query_row(
            "SELECT subject,payload FROM auth_challenges WHERE id=?1 AND kind=?2 AND expires_at>?3",
            params![hash_token(raw), kind, now()],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;
    let (stored, payload) = row
        .ok_or_else(|| AppError::unauthorized("This sign-in request expired. Please try again."))?;
    if subject.is_some_and(|s| s != stored) {
        return Err(AppError::unauthorized(
            "This sign-in request belongs to another session",
        ));
    }
    db.execute("DELETE FROM auth_challenges WHERE id=?1", [hash_token(raw)])?;
    Ok(payload)
}

fn load_passkeys(db: &Connection) -> AppResult<Vec<Passkey>> {
    let mut stmt =
        db.prepare("SELECT credential_json FROM admin_passkeys ORDER BY created_at,id")?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|s| serde_json::from_str(&s).map_err(AppError::from))
        .collect()
}

fn passkey_id(passkey: &Passkey) -> String {
    URL_SAFE_NO_PAD.encode(passkey.cred_id().as_ref())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasskeyName {
    name: String,
}

#[derive(Deserialize, Serialize)]
struct RegistrationPending {
    state: PasskeyRegistration,
    name: String,
    bootstrap: bool,
}

async fn passkey_register_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PasskeyName>,
) -> AppResult<Response> {
    let s = require_session(&state, &headers)?;
    csrf_check(&state, &headers, &s)?;
    if !s.admin && !s.bootstrap {
        return Err(AppError::forbidden("Administrator sign-in required"));
    }
    require_recent(&s)?;
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(AppError::bad_request(
            "Name this passkey using 1 to 80 characters",
        ));
    }
    let id = random_token();
    let options = {
        let db = state
            .db
            .lock()
            .map_err(|_| AppError::internal("Database lock failed"))?;
        let keys = load_passkeys(&db)?;
        if s.bootstrap && !keys.is_empty() {
            return Err(AppError::forbidden("Use your existing passkey"));
        }
        if keys.len() >= 20 {
            return Err(AppError::conflict("Up to 20 passkeys can be registered"));
        }
        let exclude = keys.iter().map(|k| k.cred_id().clone()).collect();
        let (options, pending) = state
            .auth
            .webauthn
            .start_passkey_registration(
                Uuid::parse_str(ADMIN_UUID).map_err(AppError::internal)?,
                "hireferry-admin",
                "HireFerry administrator",
                Some(exclude),
            )
            .map_err(|_| AppError::bad_request("Could not start passkey registration"))?;
        let payload = serde_json::to_string(&RegistrationPending {
            state: pending,
            name: name.to_owned(),
            bootstrap: s.bootstrap,
        })?;
        db.execute(
            "DELETE FROM auth_challenges WHERE kind='passkey-register' AND subject=?1",
            [&s.token_hash],
        )?;
        db.execute("INSERT INTO auth_challenges(id,kind,subject,payload,expires_at) VALUES(?1,'passkey-register',?2,?3,?4)",params![hash_token(&id),s.token_hash,payload,now()+CHALLENGE_TTL])?;
        options
    };
    with_cookie(
        Json(json!({"options":options})).into_response(),
        &state.cfg,
        PASSKEY_COOKIE,
        &id,
        CHALLENGE_TTL,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterFinish {
    credential: RegisterPublicKeyCredential,
}

async fn passkey_register_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RegisterFinish>,
) -> AppResult<Response> {
    let s = require_session(&state, &headers)?;
    csrf_check(&state, &headers, &s)?;
    if !s.admin && !s.bootstrap {
        return Err(AppError::forbidden("Administrator sign-in required"));
    }
    require_recent(&s)?;
    let id = cookie(&headers, PASSKEY_COOKIE)
        .ok_or_else(|| AppError::unauthorized("Start passkey registration again"))?;
    let (raw, csrf) = {
        let mut db = state
            .db
            .lock()
            .map_err(|_| AppError::internal("Database lock failed"))?;
        // Consume outside the later transaction: failed authenticator data may
        // not replay the same challenge.
        let data = consume_challenge(&db, &id, "passkey-register", Some(&s.token_hash))?;
        let pending: RegistrationPending = serde_json::from_str(&data)?;
        if pending.bootstrap != s.bootstrap {
            return Err(AppError::unauthorized("Passkey session changed"));
        }
        let key = state
            .auth
            .webauthn
            .finish_passkey_registration(&input.credential, &pending.state)
            .map_err(|_| AppError::unauthorized("Passkey registration could not be verified"))?;
        let tx = db.transaction()?;
        require_active_session(&tx, &s)?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM admin_passkeys", [], |r| r.get(0))?;
        if (s.bootstrap && count != 0) || count >= 20 {
            return Err(AppError::conflict("Passkey list changed; try again"));
        }
        let duplicate: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM admin_passkeys WHERE id=?1)",
            [passkey_id(&key)],
            |r| r.get(0),
        )?;
        if duplicate {
            return Err(AppError::conflict("This passkey is already registered"));
        }
        tx.execute(
            "INSERT INTO admin_passkeys(id,name,credential_json,created_at) VALUES(?1,?2,?3,?4)",
            params![
                passkey_id(&key),
                pending.name,
                serde_json::to_string(&key)?,
                now()
            ],
        )?;
        crate::db::audit(
            &tx,
            if s.bootstrap { "bootstrap" } else { "admin" },
            "passkey_added",
            &passkey_id(&key),
            "",
        )?;
        tx.execute(
            "DELETE FROM sessions WHERE bootstrap=1 OR token_hash=?1",
            [&s.token_hash],
        )?;
        tx.execute("DELETE FROM admin_bootstrap", [])?;
        let result = new_session(&tx, &state.cfg, None, true, false)?;
        tx.commit()?;
        result
    };
    let response = session_response(
        &state,
        &raw,
        json!({"ok":true,"admin":true,"bootstrap":false,"csrf":csrf}),
        false,
    )?;
    with_cookie(response, &state.cfg, PASSKEY_COOKIE, "", 0)
}

async fn passkey_login_start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Response> {
    origin_check(&state, &headers)?;
    let id = random_token();
    let options = {
        let db = state
            .db
            .lock()
            .map_err(|_| AppError::internal("Database lock failed"))?;
        cleanup(&db)?;
        rate_limit(&db, "passkey-login", 100, 300)?;
        let keys = load_passkeys(&db)?;
        if keys.is_empty() {
            return Err(AppError::conflict(
                "Complete first-time administrator setup",
            ));
        }
        let (options, pending) = state
            .auth
            .webauthn
            .start_passkey_authentication(&keys)
            .map_err(|_| AppError::bad_request("Could not start passkey sign-in"))?;
        db.execute("INSERT INTO auth_challenges(id,kind,subject,payload,expires_at) VALUES(?1,'passkey-login','',?2,?3)",
            params![hash_token(&id),serde_json::to_string(&pending)?,now()+CHALLENGE_TTL])?;
        options
    };
    with_cookie(
        Json(json!({"options":options})).into_response(),
        &state.cfg,
        PASSKEY_COOKIE,
        &id,
        CHALLENGE_TTL,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginFinish {
    credential: PublicKeyCredential,
}

async fn passkey_login_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<LoginFinish>,
) -> AppResult<Response> {
    origin_check(&state, &headers)?;
    let id = cookie(&headers, PASSKEY_COOKIE)
        .ok_or_else(|| AppError::unauthorized("Start passkey sign-in again"))?;
    let (raw, csrf) = {
        let mut db = state
            .db
            .lock()
            .map_err(|_| AppError::internal("Database lock failed"))?;
        let data = consume_challenge(&db, &id, "passkey-login", None)?;
        let pending: PasskeyAuthentication = serde_json::from_str(&data)?;
        let result = state
            .auth
            .webauthn
            .finish_passkey_authentication(&input.credential, &pending)
            .map_err(|_| AppError::unauthorized("Passkey sign-in could not be verified"))?;
        if !result.user_verified() {
            return Err(AppError::unauthorized(
                "Passkey user verification is required",
            ));
        }
        let credential_id = URL_SAFE_NO_PAD.encode(result.cred_id().as_ref());
        let tx = db.transaction()?;
        // Reload the current credential, not the challenge's old copy: removal
        // or a concurrent successful authentication must take effect immediately.
        let stored = tx
            .query_row(
                "SELECT credential_json,counter FROM admin_passkeys WHERE id=?1",
                [&credential_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)),
            )
            .optional()?
            .ok_or_else(|| AppError::unauthorized("This passkey is no longer registered"))?;
        if (stored.1 != 0 || result.counter() != 0) && result.counter() <= stored.1 {
            return Err(AppError::unauthorized(
                "Passkey counter did not advance; use another passkey",
            ));
        }
        let mut key: Passkey = serde_json::from_str(&stored.0)?;
        key.update_credential(&result)
            .ok_or_else(|| AppError::unauthorized("Passkey credential mismatch"))?;
        tx.execute(
            "UPDATE admin_passkeys SET credential_json=?1,counter=?2,last_used_at=?3 WHERE id=?4",
            params![
                serde_json::to_string(&key)?,
                result.counter(),
                now(),
                credential_id
            ],
        )?;
        crate::db::audit(&tx, "admin", "passkey_sign_in", &credential_id, "")?;
        if let Some(old) = cookie(&headers, SESSION_COOKIE) {
            tx.execute(
                "DELETE FROM sessions WHERE token_hash=?1",
                [hash_token(&old)],
            )?;
        }
        let values = new_session(&tx, &state.cfg, None, true, false)?;
        tx.commit()?;
        values
    };
    let response = session_response(
        &state,
        &raw,
        json!({"ok":true,"admin":true,"bootstrap":false,"csrf":csrf}),
        false,
    )?;
    with_cookie(response, &state.cfg, PASSKEY_COOKIE, "", 0)
}

async fn passkeys_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    require_admin(&state, &headers)?;
    let db = state
        .db
        .lock()
        .map_err(|_| AppError::internal("Database lock failed"))?;
    let mut statement = db.prepare(
        "SELECT id,name,created_at,last_used_at FROM admin_passkeys ORDER BY created_at",
    )?;
    let keys = statement
        .query_map([], |r| {
            Ok(
                json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,
        "created_at":r.get::<_,i64>(2)?,"last_used_at":r.get::<_,Option<i64>>(3)?}),
            )
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(json!({"passkeys":keys})))
}

async fn passkey_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let s = require_admin(&state, &headers)?;
    csrf_check(&state, &headers, &s)?;
    require_recent(&s)?;
    let mut db = state
        .db
        .lock()
        .map_err(|_| AppError::internal("Database lock failed"))?;
    let tx = db.transaction()?;
    require_active_session(&tx, &s)?;
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM admin_passkeys", [], |r| r.get(0))?;
    if count <= 1 {
        return Err(AppError::conflict(
            "Keep at least one administrator passkey",
        ));
    }
    let removed = tx.execute("DELETE FROM admin_passkeys WHERE id=?1", [&id])?;
    if removed == 0 {
        return Err(AppError::not_found("Passkey not found"));
    }
    // Invalidate pending authentication challenges and other admin sessions so
    // a removed device cannot continue through a previously issued session.
    tx.execute("DELETE FROM auth_challenges WHERE kind='passkey-login'", [])?;
    tx.execute(
        "DELETE FROM sessions WHERE admin=1 AND token_hash!=?1",
        [&s.token_hash],
    )?;
    crate::db::audit(&tx, "admin", "passkey_removed", &id, "")?;
    tx.commit()?;
    Ok(Json(json!({"ok":true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE users(id TEXT PRIMARY KEY,email TEXT NOT NULL DEFAULT '',banned INTEGER NOT NULL DEFAULT 0); CREATE TABLE api_tokens(id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),revoked INTEGER NOT NULL DEFAULT 0);")
            .unwrap();
        migrate(&db).unwrap();
        db
    }

    #[test]
    fn bootstrap_is_hashed_expires_and_cannot_be_recreated_after_passkey() {
        let db = db();
        let secret = generate_bootstrap(&db).unwrap();
        assert!(secret.len() >= 40);
        let (stored, expires): (String, i64) = db
            .query_row(
                "SELECT secret_hash,expires_at FROM admin_bootstrap",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_ne!(secret, stored);
        assert!(equal_hash(&stored, &hash_token(&secret)));
        assert!(expires <= now() + 600);
        db.execute("INSERT INTO admin_passkeys(id,name,credential_json,created_at) VALUES('key','test','{}',0)",[]).unwrap();
        assert!(generate_bootstrap(&db).is_err());
    }

    #[test]
    fn challenges_are_single_use_and_bound_to_session() {
        let db = db();
        let raw = random_token();
        db.execute("INSERT INTO auth_challenges(id,kind,subject,payload,expires_at) VALUES(?1,'test','session-a','value',?2)",params![hash_token(&raw),now()+60]).unwrap();
        assert!(consume_challenge(&db, &raw, "test", Some("session-b")).is_err());
        assert_eq!(
            consume_challenge(&db, &raw, "test", Some("session-a")).unwrap(),
            "value"
        );
        assert!(consume_challenge(&db, &raw, "test", Some("session-a")).is_err());
    }

    #[test]
    fn expired_challenges_and_wrong_kind_fail() {
        let db = db();
        let raw = random_token();
        db.execute(
            "INSERT INTO auth_challenges(id,kind,subject,expires_at) VALUES(?1,'email','',?2)",
            params![hash_token(&raw), now() - 1],
        )
        .unwrap();
        assert!(consume_challenge(&db, &raw, "email", None).is_err());
        assert!(consume_challenge(&db, &raw, "google", None).is_err());
    }

    #[test]
    fn rate_limit_persists_across_calls_and_resets_at_window() {
        let db = db();
        assert!(rate_limit(&db, "test", 2, 60).is_ok());
        assert!(rate_limit(&db, "test", 2, 60).is_ok());
        assert!(rate_limit(&db, "test", 2, 60).is_err());
        db.execute("UPDATE auth_limits SET window_start=?1", [now() - 61])
            .unwrap();
        assert!(rate_limit(&db, "test", 2, 60).is_ok());
    }

    #[test]
    fn email_validation_rejects_header_injection_and_display_names() {
        assert_eq!(
            normalized_email(" USER@Example.com ").unwrap(),
            "user@example.com"
        );
        for bad in [
            "user@example.com\r\nBcc: other@example.com",
            "Alice <a@example.com>",
            "abc",
            "a@",
            "a@localhost\n",
        ] {
            assert!(normalized_email(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn cookie_parser_matches_complete_name() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other_hf_session=bad; hf_session=good; extra=x"),
        );
        assert_eq!(cookie(&headers, SESSION_COOKIE).as_deref(), Some("good"));
    }

    #[test]
    fn suspension_invalidates_old_google_flow_without_blocking_other_accounts() {
        let db = db();
        db.execute("INSERT INTO users(id,email) VALUES('first','first@example.invalid'),('second','second@example.invalid')", []).unwrap();
        assert!(google_access_unchanged(&db, "first", 0).is_ok());
        db.execute("UPDATE users SET banned=1 WHERE id='first'", [])
            .unwrap();
        assert!(google_access_unchanged(&db, "first", 0).is_err());
        assert!(new_session(&db, &Config::default(), Some("first"), false, false).is_err());
        let started_while_suspended: i64 = db
            .query_row(
                "SELECT generation FROM account_access_clock WHERE id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        db.execute("UPDATE users SET banned=0 WHERE id='first'", [])
            .unwrap();
        assert!(google_access_unchanged(&db, "first", 0).is_err());
        assert!(google_access_unchanged(&db, "first", started_while_suspended).is_err());
        assert!(google_access_unchanged(&db, "second", 0).is_ok());
        let fresh: i64 = db
            .query_row(
                "SELECT generation FROM account_access_clock WHERE id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(google_access_unchanged(&db, "first", fresh).is_ok());
        assert!(new_session(&db, &Config::default(), Some("first"), false, false).is_ok());
    }
}
