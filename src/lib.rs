pub mod access;
pub mod admin;
pub mod admin_ai;
pub mod api;
pub mod auth;
pub mod chat;
pub mod commerce;
pub mod config;
pub mod db;
pub mod error;
pub mod mcp;
pub mod models;
pub mod payments;
pub mod safety;

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
pub use error::{AppError, AppResult};
use include_dir::{Dir, include_dir};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<config::Config>,
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub client: reqwest::Client,
    pub auth: Arc<auth::AuthService>,
}
impl AppState {
    pub fn new(cfg: config::Config) -> AppResult<Self> {
        cfg.validate()?;
        let db = db::open(&cfg.database)?;
        let auth = auth::AuthService::new(&cfg)?;
        Ok(Self {
            cfg: Arc::new(cfg),
            db: Arc::new(Mutex::new(db)),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .map_err(AppError::internal)?,
            auth: Arc::new(auth),
        })
    }
    pub fn conn(&self) -> AppResult<MutexGuard<'_, rusqlite::Connection>> {
        self.db
            .lock()
            .map_err(|_| AppError::internal("Database mutex poisoned"))
    }
}
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
static ASSETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/static");
pub fn app(state: AppState) -> Router {
    Router::new()
        .merge(auth::routes())
        .merge(api::routes())
        .merge(admin::routes())
        .merge(admin_ai::routes())
        .merge(chat::routes())
        .merge(payments::routes())
        .merge(mcp::routes())
        .route(
            "/healthz",
            axum::routing::get(|| async { StatusCode::NO_CONTENT }),
        )
        .fallback(static_asset)
        .layer(DefaultBodyLimit::max(3 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), security))
        .with_state(state)
}
async fn static_asset(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = path.strip_prefix("static/").unwrap_or(path);
    if path.starts_with("api/") || path == "mcp" {
        return AppError::not_found("Route not found").into_response();
    }
    let name = if path.is_empty() || !path.rsplit('/').next().unwrap_or("").contains('.') {
        "index.html"
    } else {
        path
    };
    match ASSETS.get_file(name) {
        Some(file) => {
            let mime = if name.ends_with(".css") {
                "text/css; charset=utf-8"
            } else if name.ends_with(".js") {
                "text/javascript; charset=utf-8"
            } else if name.ends_with(".svg") {
                "image/svg+xml"
            } else if name.ends_with(".png") {
                "image/png"
            } else if name.ends_with(".woff2") {
                "font/woff2"
            } else {
                "text/html; charset=utf-8"
            };
            (
                [("content-type", mime), ("cache-control", "no-cache")],
                file.contents(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn security(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if let Some(origin) = req.headers().get("origin") {
        if origin.to_str().ok() != Some(state.cfg.public_url.trim_end_matches('/')) {
            return AppError::forbidden("Origin is not allowed").into_response();
        }
    }
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
    );
    h.insert("content-security-policy",HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    if !state.cfg.development {
        h.insert(
            "strict-transport-security",
            HeaderValue::from_static("max-age=31536000"),
        );
    }
    if !h.contains_key("cache-control") {
        h.insert("cache-control", HeaderValue::from_static("no-store"));
    }
    res
}

pub fn user_id(state: &AppState, headers: &HeaderMap, write: bool) -> AppResult<String> {
    if let Some(value) = headers.get("authorization") {
        let token = value
            .to_str()
            .ok()
            .and_then(|s| s.strip_prefix("Bearer "))
            .ok_or_else(|| AppError::unauthorized("Invalid bearer token"))?;
        use rusqlite::OptionalExtension;
        use sha2::Digest;
        let hash = hex::encode(sha2::Sha256::digest(token.as_bytes()));
        let db = state.conn()?;
        let id:Option<String>=db.query_row("SELECT t.user_id FROM api_tokens t JOIN users u ON u.id=t.user_id WHERE t.token_hash=?1 AND t.revoked=0 AND t.expires_at>?2 AND u.banned=0",rusqlite::params![hash,now()],|r|r.get(0)).optional()?;
        let id = id.ok_or_else(|| AppError::unauthorized("Token expired or revoked"))?;
        // Check on every use: closing the last qualifying job immediately
        // disables an employer's existing AI tokens, including REST requests.
        access::require_user_ai(&db, &id)?;
        db.execute(
            "UPDATE api_tokens SET last_used_at=?2 WHERE token_hash=?1",
            rusqlite::params![hash, now()],
        )?;
        Ok(id)
    } else {
        let id = auth::require_user(state, headers)?;
        if headers.get("x-hireferry-ai").is_some_and(|v| v == "webmcp") {
            access::require_user_ai(&*state.conn()?, &id)?;
        }
        if write {
            let session = auth::session(state, headers)?
                .ok_or_else(|| AppError::unauthorized("Sign in required"))?;
            auth::csrf_check(state, headers, &session)?;
        }
        Ok(id)
    }
}
