use crate::error::{AppError, AppResult};
use rusqlite::Connection;

pub fn open(path: &str) -> AppResult<Connection> {
    if let Some(parent) = std::path::Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        let new = !parent.exists();
        std::fs::create_dir_all(parent).map_err(AppError::internal)?;
        #[cfg(unix)]
        if new {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .map_err(AppError::internal)?;
        }
    }
    let db = Connection::open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path != ":memory:" {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(AppError::internal)?;
        }
    }
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    migrate(&db)?;
    Ok(db)
}
pub fn migrate(db: &Connection) -> AppResult<()> {
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > 2 {
        return Err(AppError::conflict(
            "Database schema is newer than this application",
        ));
    }
    db.execute_batch(r#"
 CREATE TABLE IF NOT EXISTS users(
 id TEXT PRIMARY KEY,email TEXT NOT NULL UNIQUE COLLATE NOCASE,google_sub TEXT UNIQUE,name TEXT NOT NULL DEFAULT '',role TEXT NOT NULL DEFAULT 'unassigned' CHECK(role IN ('unassigned','seeker','employer')),
 employer_paid INTEGER NOT NULL DEFAULT 0,avatar TEXT NOT NULL DEFAULT '',public_email TEXT NOT NULL DEFAULT '',resume_md TEXT NOT NULL DEFAULT '',visible INTEGER NOT NULL DEFAULT 0,banned INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
 CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,owner_id TEXT NOT NULL REFERENCES users(id),title TEXT NOT NULL,jd_md TEXT NOT NULL,salary_cents INTEGER NOT NULL CHECK(salary_cents>0),status TEXT NOT NULL DEFAULT 'draft' CHECK(status IN ('draft','published','closed','deleted')),edits_used INTEGER NOT NULL DEFAULT 0,version INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
 CREATE INDEX IF NOT EXISTS jobs_owner ON jobs(owner_id,created_at);
 CREATE TABLE IF NOT EXISTS orders(id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),kind TEXT NOT NULL CHECK(kind IN ('onboarding','publish','edit')),job_id TEXT REFERENCES jobs(id),amount_cents INTEGER NOT NULL CHECK(amount_cents>0),currency TEXT NOT NULL DEFAULT 'CNY',provider TEXT NOT NULL,live_mode INTEGER NOT NULL DEFAULT 0,status TEXT NOT NULL DEFAULT 'pending',snapshot_json TEXT NOT NULL DEFAULT '{}',provider_session TEXT,checkout_url TEXT,checkout_started_at INTEGER,created_at INTEGER NOT NULL,paid_at INTEGER);
 CREATE UNIQUE INDEX IF NOT EXISTS one_job_pending ON orders(job_id) WHERE status='pending';
 CREATE UNIQUE INDEX IF NOT EXISTS one_onboarding_pending ON orders(user_id) WHERE kind='onboarding' AND status='pending';
 CREATE INDEX IF NOT EXISTS orders_user ON orders(user_id,created_at);
 CREATE TABLE IF NOT EXISTS payment_events(provider TEXT NOT NULL,event_id TEXT NOT NULL,order_id TEXT NOT NULL REFERENCES orders(id),created_at INTEGER NOT NULL,PRIMARY KEY(provider,event_id));
 CREATE TABLE IF NOT EXISTS ledger(id INTEGER PRIMARY KEY,order_id TEXT NOT NULL UNIQUE REFERENCES orders(id),provider TEXT NOT NULL,payment_id TEXT NOT NULL,amount_cents INTEGER NOT NULL,currency TEXT NOT NULL,live_mode INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,UNIQUE(provider,payment_id));
 CREATE TABLE IF NOT EXISTS api_tokens(id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),name TEXT NOT NULL,token_hash TEXT NOT NULL UNIQUE,created_at INTEGER NOT NULL,expires_at INTEGER NOT NULL,last_used_at INTEGER,revoked INTEGER NOT NULL DEFAULT 0);
 CREATE TABLE IF NOT EXISTS audit(id INTEGER PRIMARY KEY,actor TEXT NOT NULL,action TEXT NOT NULL,target TEXT NOT NULL,detail TEXT NOT NULL DEFAULT '',created_at INTEGER NOT NULL);
 CREATE TABLE IF NOT EXISTS matches(id TEXT PRIMARY KEY,seeker_id TEXT NOT NULL REFERENCES users(id),job_id TEXT NOT NULL REFERENCES jobs(id),status TEXT NOT NULL DEFAULT 'introduced',note TEXT NOT NULL DEFAULT '',created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,UNIQUE(seeker_id,job_id));
 "#)?;
    crate::auth::migrate(db)?;
    crate::admin_ai::migrate(db)?;
    crate::chat::migrate(db)?;
    db.pragma_update(None, "user_version", 2)?;
    Ok(())
}
pub fn audit(
    db: &Connection,
    actor: &str,
    action: &str,
    target: &str,
    detail: &str,
) -> AppResult<()> {
    db.execute(
        "INSERT INTO audit(actor,action,target,detail,created_at) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![actor, action, target, detail, crate::now()],
    )?;
    Ok(())
}
