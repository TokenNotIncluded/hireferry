//! Periodic pruning for disposable state only. Accounting, audit trails,
//! private conversations, applications and recruitment history are excluded.
use crate::{AppResult, AppState, config::Config, feedback, now, webhooks};
use rusqlite::Connection;
use std::time::Duration;

pub fn prune(db: &Connection, cfg: &Config, at: i64) -> AppResult<()> {
    let tx = db.unchecked_transaction()?;
    let expired = at - cfg.maintenance.expired_credentials_grace_days * 86_400;
    let grants = at - cfg.maintenance.human_grant_history_days * 86_400;

    // A CLI family's refresh hashes must remain until the entire family has
    // expired; deleting individual used refresh hashes earlier breaks reuse
    // detection. ON DELETE CASCADE removes them together.
    tx.execute("DELETE FROM cli_grants WHERE expires_at<?1", [at - 86_400])?;
    tx.execute(
        "DELETE FROM cli_authorizations WHERE expires_at<?1",
        [expired],
    )?;
    tx.execute("DELETE FROM sessions WHERE expires_at<?1", [at - 86_400])?;
    tx.execute(
        "DELETE FROM auth_challenges WHERE expires_at<?1",
        [at - 86_400],
    )?;
    tx.execute(
        "DELETE FROM auth_limits WHERE window_start<?1",
        [at - 86_400],
    )?;
    // A CLI refresh family may remain valid much longer than its current
    // access token. Keep that family's backing token until the family ends,
    // otherwise housekeeping would silently break a valid refresh grant.
    tx.execute(
        "DELETE FROM api_tokens WHERE expires_at<?1 AND NOT EXISTS (
            SELECT 1 FROM cli_tokens t JOIN cli_authorizations f ON f.id=t.family_id
            WHERE t.token_id=api_tokens.id AND t.admin=0
              AND f.revoked=0 AND f.expires_at>?2
        )",
        rusqlite::params![expired, at],
    )?;
    tx.execute(
        "DELETE FROM admin_api_tokens WHERE expires_at<?1 AND NOT EXISTS (
            SELECT 1 FROM cli_tokens t JOIN cli_authorizations f ON f.id=t.family_id
            WHERE t.token_id=admin_api_tokens.id AND t.admin=1
              AND f.revoked=0 AND f.expires_at>?2
        )",
        rusqlite::params![expired, at],
    )?;
    tx.execute("DELETE FROM hg_nonces WHERE expires_at<?1", [at])?;
    tx.execute("DELETE FROM hg_limits WHERE created_at<?1", [at - 86_400])?;

    // HumanGrant's foreign-key chain is session -> grant -> request. Only
    // expired or long-revoked grants are historical and safe to retire.
    tx.execute("DELETE FROM hg_sessions WHERE grant_id IN
        (SELECT id FROM hg_grants WHERE expires_at<?1 OR (revoked_at IS NOT NULL AND revoked_at<?1))", [grants])?;
    tx.execute(
        "DELETE FROM hg_grants WHERE expires_at<?1 OR (revoked_at IS NOT NULL AND revoked_at<?1)",
        [grants],
    )?;
    tx.execute(
        "DELETE FROM hg_requests WHERE expires_at<?1 AND NOT EXISTS
        (SELECT 1 FROM hg_grants WHERE request_id=hg_requests.id)",
        [grants],
    )?;

    feedback::cleanup(&tx, &cfg.feedback, at)?;
    webhooks::cleanup(&tx, &cfg.webhooks, at)?;
    // Never call VACUUM while serving requests. It requires an exclusive
    // rewrite and can need a second database-sized allocation on disk.
    tx.commit()?;
    Ok(())
}

pub fn spawn_worker(s: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_secs(s.cfg.maintenance.interval_minutes * 60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if s.conn().and_then(|db| prune(&db, &s.cfg, now())).is_err() {
                tracing::warn!("Periodic retention cleanup failed");
            }
        }
    })
}
