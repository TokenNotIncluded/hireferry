//! Member-to-member invitations.
//!
//! Every account gets one stable, non-secret invite code. Sharing the link is
//! optional; accepting it only records who introduced a newly created account.
//! There is no reward or automatic outreach attached to a referral.

use crate::{AppError, AppResult, now};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use uuid::Uuid;

const CODE_PREFIX: &str = "hf_";

/// Create the referral tables and backfill one stable code for older accounts.
pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS referral_codes(
            code TEXT PRIMARY KEY,
            user_id TEXT NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS user_referrals(
            id TEXT PRIMARY KEY,
            inviter_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            invitee_id TEXT NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
            code TEXT NOT NULL REFERENCES referral_codes(code),
            created_at INTEGER NOT NULL,
            CHECK(inviter_id<>invitee_id)
        );
        CREATE INDEX IF NOT EXISTS user_referrals_inviter ON user_referrals(inviter_id,created_at);
        CREATE INDEX IF NOT EXISTS user_referrals_code ON user_referrals(code);
        INSERT OR IGNORE INTO referral_codes(code,user_id,created_at)
            SELECT 'hf_'||substr(replace(id,'-',''),1,32),id,created_at FROM users;
    ",
    )?;
    Ok(())
}

/// Keep codes URL-safe and reject arbitrary values before they reach SQL.
pub fn normalize_code(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if !(3..=64).contains(&value.len())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    let value = value.to_ascii_lowercase();
    value.starts_with(CODE_PREFIX).then_some(value)
}

/// Ensure a code exists for a user created by a test, an old import, or a new
/// authentication flow. Deterministic UUID-derived codes make retries safe.
pub fn ensure_code(db: &Connection, user_id: &str, created_at: i64) -> AppResult<String> {
    if let Some(code) = db
        .query_row(
            "SELECT code FROM referral_codes WHERE user_id=?1",
            [user_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(code);
    }
    let base = format!("{CODE_PREFIX}{}", user_id.replace('-', ""));
    let code = base.chars().take(35).collect::<String>();
    db.execute(
        "INSERT OR IGNORE INTO referral_codes(code,user_id,created_at) VALUES(?1,?2,?3)",
        params![code, user_id, created_at],
    )?;
    db.query_row(
        "SELECT code FROM referral_codes WHERE user_id=?1",
        [user_id],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .ok_or_else(|| AppError::conflict("Could not create a member invitation code"))
}

/// Record the inviter only for a newly created account. Invalid, missing, self,
/// and suspended inviter codes are harmless no-ops.
pub fn record_signup(
    db: &Connection,
    invitee_id: &str,
    code: Option<&str>,
    created_at: i64,
) -> AppResult<Option<String>> {
    let Some(code) = normalize_code(code) else {
        return Ok(None);
    };
    let inviter = db
        .query_row(
            "SELECT c.user_id FROM referral_codes c JOIN users u ON u.id=c.user_id
             WHERE c.code=?1 AND u.banned=0",
            [&code],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    let Some(inviter_id) = inviter else {
        return Ok(None);
    };
    if inviter_id == invitee_id {
        return Ok(None);
    }
    let inserted = db.execute(
        "INSERT OR IGNORE INTO user_referrals(id,inviter_id,invitee_id,code,created_at)
         VALUES(?1,?2,?3,?4,?5)",
        params![
            Uuid::new_v4().to_string(),
            inviter_id,
            invitee_id,
            code,
            created_at
        ],
    )?;
    let accepted = db.query_row(
        "SELECT inviter_id FROM user_referrals WHERE invitee_id=?1",
        [invitee_id],
        |r| r.get::<_, String>(0),
    )?;
    if accepted == inviter_id {
        if inserted == 0 {
            return Ok(Some(inviter_id));
        }
        crate::db::audit(db, "system", "referral_signup", invitee_id, &inviter_id)?;
        Ok(Some(inviter_id))
    } else {
        Ok(None)
    }
}

/// Profile-safe referral details. The code is intended to be shared publicly;
/// it grants no account access and does not reveal invited users' emails.
pub fn profile_data(db: &Connection, user_id: &str, public_url: &str) -> AppResult<Value> {
    let code = ensure_code(db, user_id, now())?;
    let invited_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM user_referrals WHERE inviter_id=?1",
        [user_id],
        |r| r.get(0),
    )?;
    let url = format!("{}/login?ref={code}", public_url.trim_end_matches('/'));
    Ok(json!({"code":code,"url":url,"invited_count":invited_count}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE users(id TEXT PRIMARY KEY,email TEXT NOT NULL,banned INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL);
             CREATE TABLE audit(id INTEGER PRIMARY KEY,actor TEXT NOT NULL,action TEXT NOT NULL,target TEXT NOT NULL,detail TEXT NOT NULL DEFAULT '',created_at INTEGER NOT NULL);",
        )
        .unwrap();
        db
    }

    #[test]
    fn stable_codes_and_signup_record_are_idempotent() {
        let db = db();
        db.execute(
            "INSERT INTO users(id,email,created_at) VALUES('a','a@example.com',1),('b','b@example.com',2)",
            [],
        )
        .unwrap();
        migrate(&db).unwrap();
        let code = ensure_code(&db, "a", 1).unwrap();
        assert_eq!(code, ensure_code(&db, "a", 99).unwrap());
        assert_eq!(
            record_signup(&db, "b", Some(&code), 3).unwrap(),
            Some("a".into())
        );
        assert_eq!(
            record_signup(&db, "b", Some(&code), 4).unwrap(),
            Some("a".into())
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM user_referrals", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn invalid_and_self_codes_do_not_create_referrals() {
        let db = db();
        db.execute(
            "INSERT INTO users(id,email,created_at) VALUES('a','a@example.com',1)",
            [],
        )
        .unwrap();
        migrate(&db).unwrap();
        let code = ensure_code(&db, "a", 1).unwrap();
        assert_eq!(normalize_code(Some("bad")), None);
        assert_eq!(record_signup(&db, "a", Some(&code), 2).unwrap(), None);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM user_referrals", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
