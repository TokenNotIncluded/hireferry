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
    if version > 15 {
        return Err(AppError::conflict(
            "Database schema is newer than this application",
        ));
    }
    db.execute_batch(r#"
 CREATE TABLE IF NOT EXISTS users(
 id TEXT PRIMARY KEY,email TEXT NOT NULL UNIQUE COLLATE NOCASE,google_sub TEXT UNIQUE,name TEXT NOT NULL DEFAULT '',role TEXT NOT NULL DEFAULT 'unassigned' CHECK(role IN ('unassigned','seeker','employer')),
 employer_paid INTEGER NOT NULL DEFAULT 0,avatar TEXT NOT NULL DEFAULT '',public_email TEXT NOT NULL DEFAULT '',resume_md TEXT NOT NULL DEFAULT '',visible INTEGER NOT NULL DEFAULT 0,banned INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
 CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,owner_id TEXT NOT NULL REFERENCES users(id),title TEXT NOT NULL,jd_md TEXT NOT NULL,salary_cents INTEGER NOT NULL CHECK(salary_cents>0),headcount INTEGER NOT NULL DEFAULT 1 CHECK(headcount>=0 AND headcount<=1000000),status TEXT NOT NULL DEFAULT 'draft' CHECK(status IN ('draft','published','closed','deleted')),delete_count INTEGER NOT NULL DEFAULT 0 CHECK(delete_count>=0 AND delete_count<=2),deleted_from_status TEXT CHECK(deleted_from_status IN ('draft','published','closed')),deleted_headcount INTEGER CHECK(deleted_headcount>=0 AND deleted_headcount<=1000000),edits_used INTEGER NOT NULL DEFAULT 0,version INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
 CREATE INDEX IF NOT EXISTS jobs_owner ON jobs(owner_id,created_at);
 CREATE TABLE IF NOT EXISTS orders(id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),kind TEXT NOT NULL CHECK(kind IN ('onboarding','publish','edit')),job_id TEXT REFERENCES jobs(id),amount_cents INTEGER NOT NULL CHECK(amount_cents>=0),currency TEXT NOT NULL DEFAULT 'CNY',provider TEXT NOT NULL,live_mode INTEGER NOT NULL DEFAULT 0,status TEXT NOT NULL DEFAULT 'pending',snapshot_json TEXT NOT NULL DEFAULT '{}',provider_session TEXT,checkout_url TEXT,checkout_started_at INTEGER,created_at INTEGER NOT NULL,paid_at INTEGER);
 CREATE UNIQUE INDEX IF NOT EXISTS one_job_pending ON orders(job_id) WHERE status='pending';
 CREATE UNIQUE INDEX IF NOT EXISTS one_onboarding_pending ON orders(user_id) WHERE kind='onboarding' AND status='pending';
 CREATE INDEX IF NOT EXISTS orders_user ON orders(user_id,created_at);
 CREATE TABLE IF NOT EXISTS payment_events(provider TEXT NOT NULL,event_id TEXT NOT NULL,order_id TEXT NOT NULL REFERENCES orders(id),created_at INTEGER NOT NULL,PRIMARY KEY(provider,event_id));
 CREATE TABLE IF NOT EXISTS ledger(id INTEGER PRIMARY KEY,order_id TEXT NOT NULL UNIQUE REFERENCES orders(id),provider TEXT NOT NULL,payment_id TEXT NOT NULL,amount_cents INTEGER NOT NULL,currency TEXT NOT NULL,live_mode INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,UNIQUE(provider,payment_id));
 CREATE TABLE IF NOT EXISTS api_tokens(id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),name TEXT NOT NULL,token_hash TEXT NOT NULL UNIQUE,created_at INTEGER NOT NULL,expires_at INTEGER NOT NULL,last_used_at INTEGER,revoked INTEGER NOT NULL DEFAULT 0);
 CREATE TABLE IF NOT EXISTS audit(id INTEGER PRIMARY KEY,actor TEXT NOT NULL,action TEXT NOT NULL,target TEXT NOT NULL,detail TEXT NOT NULL DEFAULT '',created_at INTEGER NOT NULL);
 CREATE TABLE IF NOT EXISTS matches(id TEXT PRIMARY KEY,seeker_id TEXT NOT NULL REFERENCES users(id),job_id TEXT NOT NULL REFERENCES jobs(id),status TEXT NOT NULL DEFAULT 'introduced',note TEXT NOT NULL DEFAULT '',created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,UNIQUE(seeker_id,job_id));
 "#)?;
    // Upgrade before webhook trigger creation: the job update predicate reads
    // headcount. Existing live/draft adverts keep one planned opening; archives
    // must never acquire openings simply because the new column defaults to one.
    let job_columns = {
        let mut query = db.prepare("PRAGMA table_info(jobs)")?;
        query
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let added_headcount = !job_columns.iter().any(|column| column == "headcount");
    if !job_columns.iter().any(|column| column == "currency") {
        db.execute_batch("ALTER TABLE jobs ADD COLUMN currency TEXT NOT NULL DEFAULT 'CNY';")?;
    }
    if !job_columns
        .iter()
        .any(|column| column == "ai_min_salary_cents")
    {
        db.execute_batch("ALTER TABLE jobs ADD COLUMN ai_min_salary_cents INTEGER NOT NULL DEFAULT 300000 CHECK(ai_min_salary_cents>=0);")?;
    }
    if added_headcount {
        db.execute_batch("ALTER TABLE jobs ADD COLUMN headcount INTEGER NOT NULL DEFAULT 1 CHECK(headcount>=0 AND headcount<=1000000);")?;
    }
    let added_lifecycle = !job_columns.iter().any(|column| column == "delete_count");
    if added_lifecycle {
        db.execute_batch("ALTER TABLE jobs ADD COLUMN delete_count INTEGER NOT NULL DEFAULT 0 CHECK(delete_count>=0 AND delete_count<=2);")?;
    }
    if !job_columns
        .iter()
        .any(|column| column == "deleted_from_status")
    {
        db.execute_batch("ALTER TABLE jobs ADD COLUMN deleted_from_status TEXT CHECK(deleted_from_status IN ('draft','published','closed'));")?;
    }
    if !job_columns
        .iter()
        .any(|column| column == "deleted_headcount")
    {
        db.execute_batch("ALTER TABLE jobs ADD COLUMN deleted_headcount INTEGER CHECK(deleted_headcount>=0 AND deleted_headcount<=1000000);")?;
    }
    if version < 4 || added_headcount {
        db.execute(
            "UPDATE jobs SET headcount=0 WHERE status IN ('closed','deleted') AND headcount<>0",
            [],
        )?;
    }
    if version < 4 || added_lifecycle {
        // Older deleted adverts were administrator removals, not recoverable
        // trash. Migration must never bring their removed content back.
        db.execute(
            "UPDATE jobs SET delete_count=2,deleted_from_status=NULL,deleted_headcount=NULL,headcount=0,title='',jd_md='' WHERE status='deleted'",
            [],
        )?;
        db.execute(
            "UPDATE orders SET snapshot_json='{}',checkout_url=NULL,status=CASE WHEN status='pending' THEN 'cancelled' ELSE status END WHERE job_id IN (SELECT id FROM jobs WHERE status='deleted' AND delete_count=2)",
            [],
        )?;
    }
    crate::auth::migrate(db)?;
    crate::admin_ai::migrate(db)?;
    crate::chat::migrate(db)?;
    crate::commerce::migrate(db)?;
    crate::webhooks::migrate(db)?;
    crate::workbench::migrate(db)?;
    crate::humangrant::migrate(db)?;
    // HumanGrant owns version 7. Its idempotent migration also preserves the
    // upstream suspension backfill when reopening an existing database.
    if version < 7 {
        db.pragma_update(None, "user_version", 7)?;
    }
    // Version 8 adds native-client grants and rotating credential families.
    // The schema and version marker commit together; a failed upgrade rolls back.
    if version < 8 {
        let tx = db.unchecked_transaction()?;
        crate::cli_auth::migrate(&tx)?;
        tx.pragma_update(None, "user_version", 8)?;
        tx.commit()?;
    }
    // Version 9 makes promotion reservations part of payment fulfillment.
    // Older binaries must refuse this database rather than ignore their expiry.
    if version < 9 {
        let tx = db.unchecked_transaction()?;
        crate::promos::migrate(&tx)?;
        tx.pragma_update(None, "user_version", 9)?;
        tx.commit()?;
    }
    // Version 10 adds short-lived reports without modifying finance tables.
    if version < 10 {
        let tx = db.unchecked_transaction()?;
        crate::feedback::migrate(&tx)?;
        tx.pragma_update(None, "user_version", 10)?;
        tx.commit()?;
    }
    // Schema 11: isolated human staff roles and one-time invitation hashes.
    if version < 11 {
        let tx = db.unchecked_transaction()?;
        crate::staff::migrate(&tx)?;
        tx.pragma_update(None, "user_version", 11)?;
        tx.commit()?;
    }
    // Schema 12: persistent abuse controls and indexed, bounded housekeeping.
    if version < 12 {
        let tx = db.unchecked_transaction()?;
        crate::feedback::migrate(&tx)?;
        crate::maintenance::migrate(&tx)?;
        tx.pragma_update(None, "user_version", 12)?;
        tx.commit()?;
    }
    // Schema 13 adds team employees independently of manager and root identities.
    // Commit all team tables, revocation triggers and the version atomically.
    if version < 13 {
        let tx = db.unchecked_transaction()?;
        crate::team::migrate(&tx)?;
        tx.pragma_update(None, "user_version", 13)?;
        tx.commit()?;
    }
    // Schema 14: typed messages and a durable stream event journal. Older
    // binaries must not ignore active streams or their idempotency records.
    if version < 14 {
        let tx = db.unchecked_transaction()?;
        crate::chat_stream::migrate(&tx)?;
        tx.pragma_update(None, "user_version", 14)?;
        tx.commit()?;
    }
    if version < 15 {
        let tx = db.unchecked_transaction()?;
        crate::notifications::migrate(&tx)?;
        tx.pragma_update(None, "user_version", 15)?;
        tx.commit()?;
    }
    // Additive graph indexes/revision and presentation state, like the workbench
    // indexes above, do not change the authoritative business record format.
    crate::relations::migrate(db)?;
    crate::relation_views::migrate(db)?;
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
