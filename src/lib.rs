pub mod access;
pub mod admin;
pub mod admin_ai;
pub mod api;
mod assets;
pub mod auth;
pub mod chat;
pub mod chat_capabilities;
pub mod chat_stream;
pub mod cli_auth;
pub mod cli_distribution;
pub mod commerce;
pub mod config;
pub mod db;
pub mod error;
pub mod feedback;
pub mod humangrant;
pub mod humangrant_access;
pub mod legal;
pub mod maintenance;
pub mod mcp;
pub mod models;
pub mod payments;
pub mod promos;
pub mod relation_views;
pub mod relations;
pub mod safety;
mod seo;
pub mod staff;
pub mod team;
pub mod webhooks;
pub mod workbench;

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
pub use error::{AppError, AppResult};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<config::Config>,
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub relations_cache: Arc<Mutex<relations::Cache>>,
    pub client: reqwest::Client,
    pub auth: Arc<auth::AuthService>,
    /// Database cursors survive restarts; notifications only wake live readers.
    pub chat_notify: Arc<tokio::sync::Notify>,
    // Scoped per request; never placed in the shared application router state.
    humangrant: Option<humangrant_access::Execution>,
}
impl AppState {
    pub fn new(mut cfg: config::Config) -> AppResult<Self> {
        cfg.validate()?;
        // Browsers and signing clients canonicalize host case and default ports.
        // Use the same origin for signatures, consent checks and generated URLs.
        cfg.public_url = url::Url::parse(&cfg.public_url)
            .map_err(AppError::bad_request)?
            .origin()
            .ascii_serialization();
        let db = db::open(&cfg.database)?;
        webhooks::configure(&db, &cfg.webhooks)?;
        let auth = auth::AuthService::new(&cfg)?;
        Ok(Self {
            cfg: Arc::new(cfg),
            db: Arc::new(Mutex::new(db)),
            relations_cache: Arc::new(Mutex::new(relations::Cache::default())),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .map_err(AppError::internal)?,
            auth: Arc::new(auth),
            chat_notify: Arc::new(tokio::sync::Notify::new()),
            humangrant: None,
        })
    }
    pub fn conn(&self) -> AppResult<MutexGuard<'_, rusqlite::Connection>> {
        let db = self
            .db
            .lock()
            .map_err(|_| AppError::internal("Database mutex poisoned"))?;
        if let Some(execution) = &self.humangrant {
            if !self.cfg.humangrant.enabled {
                return Err(AppError::forbidden("HumanGrant is disabled"));
            }
            if let Some(tool) = &execution.tool {
                humangrant_access::authorize_tool(
                    &db,
                    &execution.grant,
                    tool,
                    &execution.arguments,
                )?;
            } else {
                humangrant::validate_grant(&db, &execution.grant)?;
            }
        }
        // Each business operation holds this guard through its database work.
        // This orders revocation against access within this shared AppState.
        // The SQLite deployment uses one service instance per database; this
        // Rust mutex does not coordinate independent processes/connections.
        Ok(db)
    }
    pub(crate) fn for_humangrant(
        &self,
        grant: humangrant::GrantContext,
        tool: Option<&str>,
        arguments: serde_json::Value,
    ) -> Self {
        let mut scoped = self.clone();
        scoped.humangrant = Some(humangrant_access::Execution {
            grant,
            tool: tool.map(str::to_owned),
            arguments,
        });
        scoped
    }
}
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
pub fn app(state: AppState) -> Router {
    Router::new()
        .merge(auth::routes())
        .merge(humangrant::routes())
        .merge(cli_auth::routes())
        .merge(cli_distribution::routes())
        .merge(api::routes())
        .merge(feedback::routes())
        .merge(legal::routes())
        .merge(admin::routes())
        .merge(staff::routes())
        .merge(admin_ai::routes())
        .merge(team::routes())
        .merge(chat::routes())
        .merge(chat_capabilities::routes())
        .merge(payments::routes())
        .merge(promos::routes())
        .merge(mcp::routes())
        .merge(webhooks::routes())
        .merge(workbench::routes())
        .merge(relations::routes())
        .merge(relation_views::routes())
        .route(
            "/healthz",
            axum::routing::get(|| async { StatusCode::NO_CONTENT }),
        )
        .fallback_service(
            axum::routing::get(assets::serve)
                .with_state(state.clone())
                .layer(tower_http::compression::CompressionLayer::new()),
        )
        .layer(DefaultBodyLimit::max(3 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), security))
        .with_state(state)
}
async fn security(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if let Some(origin) = req.headers().get("origin") {
        if origin.to_str().ok() != Some(state.cfg.public_url.trim_end_matches('/')) {
            return AppError::forbidden("Origin is not allowed").into_response();
        }
    }
    let indexable = seo::PUBLIC_PAGES.contains(&req.uri().path());
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    if !indexable {
        h.insert("x-robots-tag", HeaderValue::from_static("noindex, follow"));
    }
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
    h.insert("content-security-policy",HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' https://api.github.com; font-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
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
        cli_auth::check_business_token(&db, &state.cfg, token)?;
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
