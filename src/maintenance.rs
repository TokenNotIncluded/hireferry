//! Bounded pruning of disposable state. Finance, audit and business history
//! are retained; housekeeping never rewrites the database with VACUUM.
use crate::{AppError, AppResult, AppState, config::Config, feedback, now, webhooks};
use rusqlite::{Connection, params};
use std::time::Duration;

pub(crate) const DEFAULT_BATCH_SIZE: i64 = 200;
const PHASES: [&str; 19] = [
    "cli_grants",
    "cli_tokens",
    "cli_refresh_tokens",
    "cli_authorizations",
    "sessions",
    "auth_challenges",
    "auth_limits",
    "api_tokens",
    "admin_api_tokens",
    "hg_nonces",
    "hg_limits",
    "hg_sessions",
    "hg_grants",
    "hg_requests",
    "admin_bootstrap",
    "staff_invitations",
    "checkout_orders",
    "feedback",
    "webhooks",
];

/// Installed with the schema migration, not on the request/cleanup path.
pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS api_tokens_expiry ON api_tokens(expires_at);
         CREATE INDEX IF NOT EXISTS cli_authorizations_expiry ON cli_authorizations(expires_at);
         CREATE INDEX IF NOT EXISTS auth_limits_expiry ON auth_limits(window_start);
         CREATE INDEX IF NOT EXISTS hg_grants_expiry ON hg_grants(expires_at);
         CREATE INDEX IF NOT EXISTS hg_grants_revoked ON hg_grants(revoked_at) WHERE revoked_at IS NOT NULL;
         CREATE INDEX IF NOT EXISTS webhook_endpoints_retention ON webhook_endpoints(revoked,updated_at);
         CREATE INDEX IF NOT EXISTS webhook_deliveries_event ON webhook_deliveries(event_id);
         CREATE INDEX IF NOT EXISTS staff_invitations_retention
             ON staff_invitations(coalesce(accepted_at,revoked_at,expires_at));
         CREATE INDEX IF NOT EXISTS orders_onboarding_expiry ON orders(checkout_expires_at)
             WHERE kind='onboarding' AND status='pending';",
    )?;
    Ok(())
}

fn prune_phase(db: &Connection, cfg: &Config, at: i64, phase: &str) -> AppResult<usize> {
    let batch = cfg.maintenance.batch_size;
    if !(1..=1000).contains(&batch) {
        return Err(AppError::bad_request("Invalid housekeeping batch size"));
    }
    let expired = at.saturating_sub(cfg.maintenance.expired_credentials_grace_days * 86_400);
    let history = at.saturating_sub(cfg.maintenance.human_grant_history_days * 86_400);
    let ephemeral = at.saturating_sub(86_400);
    let changed = match phase {
        "cli_grants" => db.execute(
            "DELETE FROM cli_grants WHERE rowid IN
             (SELECT rowid FROM cli_grants WHERE expires_at<?1 ORDER BY expires_at LIMIT ?2)",
            params![ephemeral, batch],
        )?,
        // Retire children only after the whole family's absolute expiry and
        // grace period. Never discard a live family's used refresh hashes.
        "cli_tokens" | "cli_refresh_tokens" => db.execute(
            &format!(
                "DELETE FROM {phase} WHERE rowid IN
                 (SELECT t.rowid FROM cli_authorizations f JOIN {phase} t ON t.family_id=f.id
                  WHERE f.expires_at<?1 ORDER BY f.expires_at LIMIT ?2)"
            ),
            params![expired, batch],
        )?,
        "cli_authorizations" => db.execute(
            "DELETE FROM cli_authorizations WHERE rowid IN
             (SELECT f.rowid FROM cli_authorizations f WHERE f.expires_at<?1
              AND NOT EXISTS(SELECT 1 FROM cli_tokens WHERE family_id=f.id)
              AND NOT EXISTS(SELECT 1 FROM cli_refresh_tokens WHERE family_id=f.id)
              ORDER BY f.expires_at LIMIT ?2)",
            params![expired, batch],
        )?,
        "sessions" | "auth_challenges" => db.execute(
            &format!(
                "DELETE FROM {phase} WHERE rowid IN
                 (SELECT rowid FROM {phase} WHERE expires_at<?1 ORDER BY expires_at LIMIT ?2)"
            ),
            params![ephemeral, batch],
        )?,
        "auth_limits" => db.execute(
            "DELETE FROM auth_limits WHERE rowid IN
             (SELECT rowid FROM auth_limits WHERE window_start<?1 ORDER BY window_start LIMIT ?2)",
            params![ephemeral, batch],
        )?,
        // Expired access credentials may still back a valid refresh family.
        // The access and refresh lifetimes must not be confused here.
        "api_tokens" | "admin_api_tokens" => db.execute(
            &format!(
                "DELETE FROM {phase} WHERE rowid IN
                 (SELECT a.rowid FROM {phase} a WHERE a.expires_at<?1 AND NOT EXISTS
                  (SELECT 1 FROM cli_tokens t JOIN cli_authorizations f ON f.id=t.family_id
                   WHERE t.token_id=a.id AND t.admin=?3 AND f.revoked=0 AND f.expires_at>?4)
                  ORDER BY a.expires_at LIMIT ?2)"
            ),
            params![expired, batch, phase == "admin_api_tokens", at],
        )?,
        "hg_nonces" => db.execute(
            "DELETE FROM hg_nonces WHERE rowid IN
             (SELECT rowid FROM hg_nonces WHERE expires_at<=?1 ORDER BY expires_at LIMIT ?2)",
            params![at, batch],
        )?,
        "hg_limits" => db.execute(
            "DELETE FROM hg_limits WHERE rowid IN
             (SELECT rowid FROM hg_limits WHERE created_at<=?1 ORDER BY created_at LIMIT ?2)",
            params![at.saturating_sub(3600), batch],
        )?,
        "hg_sessions" => db.execute(
            "DELETE FROM hg_sessions WHERE rowid IN
             (SELECT s.rowid FROM hg_sessions s JOIN hg_grants g ON g.id=s.grant_id
              WHERE g.expires_at<?1 OR (g.revoked_at IS NOT NULL AND g.revoked_at<?1) LIMIT ?2)",
            params![history, batch],
        )?,
        "hg_grants" => db.execute(
            "DELETE FROM hg_grants WHERE rowid IN
             (SELECT g.rowid FROM hg_grants g
              WHERE (g.expires_at<?1 OR (g.revoked_at IS NOT NULL AND g.revoked_at<?1))
              AND NOT EXISTS(SELECT 1 FROM hg_sessions WHERE grant_id=g.id) LIMIT ?2)",
            params![history, batch],
        )?,
        "hg_requests" => db.execute(
            "DELETE FROM hg_requests WHERE rowid IN
             (SELECT r.rowid FROM hg_requests r WHERE r.expires_at<?1
              AND NOT EXISTS(SELECT 1 FROM hg_grants WHERE request_id=r.id)
              AND NOT EXISTS(SELECT 1 FROM hg_sessions WHERE request_id=r.id)
              ORDER BY r.expires_at LIMIT ?2)",
            params![history, batch],
        )?,
        "admin_bootstrap" => {
            db.execute("DELETE FROM admin_bootstrap WHERE expires_at<=?1", [at])?
        }
        "staff_invitations" => db.execute(
            "DELETE FROM staff_invitations WHERE rowid IN
             (SELECT rowid FROM staff_invitations
              WHERE coalesce(accepted_at,revoked_at,expires_at)<?1
              AND (accepted_at IS NOT NULL OR revoked_at IS NOT NULL OR expires_at<=?2)
              ORDER BY coalesce(accepted_at,revoked_at,expires_at) LIMIT ?3)",
            params![
                at.saturating_sub(cfg.maintenance.staff_invitation_history_days * 86_400),
                at,
                batch
            ],
        )?,
        // Expire only the checkout/reservation state. Keep the orders, gateway
        // identifiers and ledger forever: verified late receipts still settle.
        "checkout_orders" => {
            let onboarding = db.execute(
                "UPDATE orders SET status='expired',checkout_url=NULL WHERE rowid IN
                 (SELECT rowid FROM orders WHERE kind='onboarding' AND status='pending'
                  AND checkout_expires_at<=?1 ORDER BY checkout_expires_at LIMIT ?2)",
                params![at, batch],
            )?;
            let reservations = db.execute(
                "UPDATE orders SET status='expired',checkout_url=NULL WHERE rowid IN
                 (SELECT o.rowid FROM promo_redemptions r JOIN orders o ON o.id=r.order_id
                  WHERE r.status='reserved' AND r.reserved_until<=?1 AND o.status='pending'
                  ORDER BY r.reserved_until LIMIT ?2)",
                params![at, batch - onboarding as i64],
            )?;
            onboarding + reservations
        }
        "feedback" => feedback::cleanup_batch(db, &cfg.feedback, at, batch)?,
        "webhooks" => webhooks::cleanup_batch(db, &cfg.webhooks, at, batch)?,
        _ => unreachable!("maintenance phases are internal constants"),
    };
    if changed > 0 {
        tracing::info!(
            phase,
            changed,
            batch_size = batch,
            "Retention batch completed"
        );
    }
    Ok(changed)
}

/// One finite pass, useful for an explicit maintenance run and deterministic
/// tests. Each statement is atomic; an unrelated table cannot roll it back.
pub fn prune(db: &Connection, cfg: &Config, at: i64) -> AppResult<()> {
    for phase in PHASES {
        prune_phase(db, cfg, at, phase)?;
    }
    Ok(())
}

/// Run one bounded pass with a fresh mutex guard for every phase. SQLite work
/// stays off the async runtime and other requests can run between batches.
/// The return value requests another pass when a batch may have more to drain.
pub async fn run_once(s: &AppState, at: i64) -> AppResult<bool> {
    let mut backlog = false;
    let mut failure = None;
    for phase in PHASES {
        let state = s.clone();
        let result = tokio::task::spawn_blocking(move || {
            let db = state.conn()?;
            prune_phase(&db, &state.cfg, at, phase)
        })
        .await
        .map_err(AppError::internal)
        .and_then(|result| result);
        match result {
            Ok(count) => backlog |= count >= s.cfg.maintenance.batch_size as usize,
            Err(error) => {
                tracing::warn!(phase, "Retention batch failed");
                failure = Some(error);
            }
        }
        tokio::task::yield_now().await;
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(backlog),
    }
}

pub fn spawn_worker(s: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            // The first pass runs immediately, including when all related
            // product features are disabled and no browser has visited.
            let delay = match run_once(&s, now()).await {
                Ok(true) => Duration::from_secs(1),
                Ok(false) => Duration::from_secs(s.cfg.maintenance.interval_minutes * 60),
                Err(_) => Duration::from_secs(60),
            };
            tokio::time::sleep(delay).await;
        }
    })
}
