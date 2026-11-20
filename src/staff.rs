//! Human staff invitations. No super-admin credential is shared with employees.
//! Staff membership is orthogonal to a person's seeker/employer account role.
use crate::{AppError, AppResult, AppState, auth, db, now};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

const INVITE_SECONDS: i64 = 7 * 86_400;
const MAX_PENDING: i64 = 100;
const MAX_MEMBERS: i64 = 100;
const MAX_INVITES_PER_HOUR: i64 = 30;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/staff", get(list_staff))
        .route("/api/admin/staff/invitations", post(create_invitation))
        .route(
            "/api/admin/staff/invitations/{id}",
            axum::routing::delete(revoke_invitation),
        )
        .route("/api/admin/staff/members/{id}", patch(update_member))
        .route("/api/staff/invitations/{token}", get(invitation_details))
        .route(
            "/api/staff/invitations/{token}/accept",
            post(accept_invitation),
        )
}

pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS staff_members(
            user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
            role TEXT NOT NULL CHECK(role='manager'),
            active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0,1)),
            joined_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS staff_invitations(
            id TEXT PRIMARY KEY,
            token_hash TEXT NOT NULL UNIQUE,
            email TEXT NOT NULL COLLATE NOCASE,
            role TEXT NOT NULL CHECK(role='manager'),
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            sent_at INTEGER,
            accepted_by TEXT REFERENCES users(id),
            accepted_at INTEGER,
            revoked_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS staff_invitations_recent
            ON staff_invitations(created_at);
        CREATE INDEX IF NOT EXISTS staff_invitations_email
            ON staff_invitations(email, created_at);
        CREATE INDEX IF NOT EXISTS staff_invitations_pending
            ON staff_invitations(expires_at)
            WHERE accepted_at IS NULL AND revoked_at IS NULL;",
    )?;
    Ok(())
}

/// Re-checked under the business connection/transaction on every operation.
pub fn active_role(db: &Connection, user_id: &str) -> AppResult<Option<String>> {
    Ok(db
        .query_row(
            "SELECT m.role FROM staff_members m JOIN users u ON u.id=m.user_id
         WHERE m.user_id=?1 AND m.active=1 AND u.banned=0",
            [user_id],
            |row| row.get(0),
        )
        .optional()?)
}

fn owner(s: &AppState, h: &HeaderMap, write: bool) -> AppResult<()> {
    // Intentionally rejects staff cookies, CLI/AI bearer tokens and WebMCP.
    if h.contains_key("authorization") || h.contains_key("x-hireferry-ai") {
        return Err(AppError::forbidden(
            "A super-administrator passkey session is required",
        ));
    }
    let session = auth::require_admin(s, h)?;
    if write {
        auth::csrf_check(s, h, &session)?;
    }
    Ok(())
}

fn token_hash(token: &str) -> AppResult<String> {
    if token.len() != 43
        || !token
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(AppError::not_found("Invitation unavailable"));
    }
    Ok(auth::hash_token(token))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteInput {
    email: String,
    role: String,
    #[serde(default = "send_by_default")]
    send_email: bool,
}
fn send_by_default() -> bool {
    true
}

fn create_record(db: &mut Connection, email: &str) -> AppResult<(String, String, i64)> {
    let time = now();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM staff_members m
         JOIN users u ON u.id=m.user_id
         WHERE u.email=?1 COLLATE NOCASE AND m.active=1)",
        [email],
        |r| r.get(0),
    )?;
    if exists {
        return Err(AppError::conflict("This person is already a staff member"));
    }
    let active: i64 = tx.query_row(
        "SELECT count(*) FROM staff_invitations
         WHERE accepted_at IS NULL AND revoked_at IS NULL AND expires_at>?1",
        [time],
        |r| r.get(0),
    )?;
    let total: i64 = tx.query_row(
        "SELECT count(*) FROM staff_invitations WHERE created_at>?1",
        [time - 3600],
        |r| r.get(0),
    )?;
    if active >= MAX_PENDING || total >= MAX_INVITES_PER_HOUR {
        return Err(AppError {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: "Staff invitation limit reached; try again later".into(),
        });
    }
    let recent: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM staff_invitations
         WHERE email=?1 COLLATE NOCASE AND created_at>?2)",
        params![email, time - 60],
        |r| r.get(0),
    )?;
    if recent {
        return Err(AppError {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: "Please wait before inviting this email again".into(),
        });
    }
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let raw = URL_SAFE_NO_PAD.encode(bytes);
    let id = Uuid::new_v4().to_string();
    let expires_at = time + INVITE_SECONDS;
    tx.execute(
        "INSERT INTO staff_invitations(id,token_hash,email,role,created_at,expires_at)
         VALUES(?1,?2,?3,'manager',?4,?5)",
        params![id, auth::hash_token(&raw), email, time, expires_at],
    )?;
    db::audit(&tx, "admin", "staff.invite_created", &id, "manager")?;
    tx.commit()?;
    Ok((id, raw, expires_at))
}

#[derive(Debug)]
struct InvitationRecord {
    id: String,
    email: String,
    role: String,
    expires_at: i64,
    accepted_at: Option<i64>,
    revoked_at: Option<i64>,
}

fn accept_record(db: &mut Connection, token: &str, user_id: &str) -> AppResult<()> {
    let digest = token_hash(token)?;
    let time = now();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let requested: Option<InvitationRecord> = tx
        .query_row(
            "SELECT id,email,role,expires_at,accepted_at,revoked_at
         FROM staff_invitations WHERE token_hash=?1",
            [&digest],
            |r| {
                Ok(InvitationRecord {
                    id: r.get(0)?,
                    email: r.get(1)?,
                    role: r.get(2)?,
                    expires_at: r.get(3)?,
                    accepted_at: r.get(4)?,
                    revoked_at: r.get(5)?,
                })
            },
        )
        .optional()?;
    let InvitationRecord {
        id,
        email,
        role,
        expires_at: expires,
        accepted_at: accepted,
        revoked_at: revoked,
    } = requested.ok_or_else(|| AppError::not_found("Invitation unavailable"))?;
    if role != "manager" || expires <= time || accepted.is_some() || revoked.is_some() {
        return Err(AppError::conflict(
            "This invitation was used, revoked or expired",
        ));
    }
    // The session identifies a verified login email. A copied link alone
    // does not grant staff access to the wrong account.
    let actual: Option<(String, bool)> = tx
        .query_row(
            "SELECT email,banned FROM users WHERE id=?1",
            [user_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (actual_email, banned) =
        actual.ok_or_else(|| AppError::unauthorized("Sign in required"))?;
    if banned || !actual_email.eq_ignore_ascii_case(&email) {
        return Err(AppError::forbidden(
            "Sign in using the invited email address",
        ));
    }
    let members: i64 = tx.query_row(
        "SELECT count(*) FROM staff_members WHERE active=1",
        [],
        |r| r.get(0),
    )?;
    let already_member: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM staff_members WHERE user_id=?1 AND active=1)",
        [user_id],
        |r| r.get(0),
    )?;
    if already_member {
        return Err(AppError::conflict("Already a staff member"));
    }
    if members >= MAX_MEMBERS {
        return Err(AppError::conflict("Staff membership limit reached"));
    }
    let updated = tx.execute(
        "UPDATE staff_invitations SET accepted_by=?1,accepted_at=?2
         WHERE id=?3 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at>?2",
        params![user_id, time, id],
    )?;
    if updated != 1 {
        return Err(AppError::conflict("This invitation is no longer available"));
    }
    tx.execute(
        "INSERT INTO staff_members(user_id,role,active,joined_at,updated_at)
         VALUES(?1,'manager',1,?2,?2)
         ON CONFLICT(user_id) DO UPDATE SET role='manager',active=1,updated_at=excluded.updated_at",
        params![user_id, time],
    )?;
    tx.execute(
        "UPDATE staff_invitations SET revoked_at=?1
         WHERE email=?2 COLLATE NOCASE AND id<>?3
         AND accepted_at IS NULL AND revoked_at IS NULL",
        params![time, email, id],
    )?;
    db::audit(
        &tx,
        &format!("staff:{user_id}"),
        "staff.invite_accepted",
        &id,
        "manager",
    )?;
    tx.commit()?;
    Ok(())
}

async fn create_invitation(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<InviteInput>,
) -> AppResult<Json<Value>> {
    owner(&s, &h, true)?;
    if input.role != "manager" {
        return Err(AppError::bad_request(
            "Only the manager role is available in this release",
        ));
    }
    let email = auth::normalized_email(&input.email)?;
    if input.send_email && !s.auth.smtp_enabled() {
        return Err(AppError::unavailable(
            "SMTP is not configured; choose a copyable invitation link",
        ));
    }
    let (id, token, expires_at) = {
        let mut conn = s.conn()?;
        create_record(&mut conn, &email)?
    };
    let url = format!(
        "{}/staff/invite/{}",
        s.cfg.public_url.trim_end_matches('/'),
        token
    );
    if input.send_email {
        if let Err(error) = s.auth.send_staff_invitation(&s.cfg, &email, &url).await {
            // A failed/unknown SMTP delivery cannot leave an untracked live invite.
            let conn = s.conn()?;
            conn.execute(
                "UPDATE staff_invitations SET revoked_at=?2
                 WHERE id=?1 AND accepted_at IS NULL AND revoked_at IS NULL",
                params![id, now()],
            )?;
            return Err(error);
        }
        let conn = s.conn()?;
        conn.execute(
            "UPDATE staff_invitations SET sent_at=?2 WHERE id=?1",
            params![id, now()],
        )?;
    }
    Ok(Json(json!({
        "id":id,"email":email,"role":"manager","expires_at":expires_at,
        "sent":input.send_email,"invite_url":url
    })))
}

async fn list_staff(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    owner(&s, &h, false)?;
    let conn = s.conn()?;
    let members = conn
        .prepare(
            "SELECT m.user_id,u.email,u.name,m.role,m.active,m.joined_at,m.updated_at
         FROM staff_members m JOIN users u ON u.id=m.user_id ORDER BY m.joined_at DESC LIMIT 101",
        )?
        .query_map([], |r| {
            Ok(json!({
                "user_id":r.get::<_,String>(0)?,"email":r.get::<_,String>(1)?,
                "name":r.get::<_,String>(2)?,"role":r.get::<_,String>(3)?,
                "active":r.get::<_,bool>(4)?,"joined_at":r.get::<_,i64>(5)?,
                "updated_at":r.get::<_,i64>(6)?
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let invitations = conn
        .prepare(
            "SELECT id,email,role,created_at,expires_at,sent_at,accepted_at,revoked_at
         FROM staff_invitations ORDER BY created_at DESC LIMIT 100",
        )?
        .query_map([], |r| {
            Ok(json!({
                "id":r.get::<_,String>(0)?,"email":r.get::<_,String>(1)?,
                "role":r.get::<_,String>(2)?,"created_at":r.get::<_,i64>(3)?,
                "expires_at":r.get::<_,i64>(4)?,"sent_at":r.get::<_,Option<i64>>(5)?,
                "accepted_at":r.get::<_,Option<i64>>(6)?,"revoked_at":r.get::<_,Option<i64>>(7)?
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Json(json!({"members":members,"invitations":invitations})))
}

async fn invitation_details(
    State(s): State<AppState>,
    Path(token): Path<String>,
) -> AppResult<Json<Value>> {
    let digest = token_hash(&token)?;
    let conn = s.conn()?;
    let details = conn
        .query_row(
            "SELECT email,role,expires_at FROM staff_invitations
         WHERE token_hash=?1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at>?2",
            params![digest, now()],
            |r| {
                Ok(json!({"email":r.get::<_,String>(0)?,
            "role":r.get::<_,String>(1)?,"expires_at":r.get::<_,i64>(2)?}))
            },
        )
        .optional()?;
    Ok(Json(details.ok_or_else(|| {
        AppError::not_found("Invitation unavailable")
    })?))
}

async fn accept_invitation(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(token): Path<String>,
) -> AppResult<Json<Value>> {
    let session = auth::require_session(&s, &h)?;
    if session.admin || session.bootstrap {
        return Err(AppError::forbidden(
            "Sign in with the invited person's account",
        ));
    }
    auth::csrf_check(&s, &h, &session)?;
    let user_id = session
        .user_id
        .ok_or_else(|| AppError::unauthorized("Sign in required"))?;
    let mut conn = s.conn()?;
    accept_record(&mut conn, &token, &user_id)?;
    Ok(Json(json!({"ok":true,"role":"manager"})))
}

async fn revoke_invitation(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    owner(&s, &h, true)?;
    let conn = s.conn()?;
    let changed = conn.execute(
        "UPDATE staff_invitations SET revoked_at=?2 WHERE id=?1
         AND accepted_at IS NULL AND revoked_at IS NULL",
        params![id, now()],
    )?;
    if changed == 0 {
        return Err(AppError::not_found("Pending invitation not found"));
    }
    db::audit(&conn, "admin", "staff.invite_revoked", &id, "")?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberPatch {
    active: bool,
}

async fn update_member(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<MemberPatch>,
) -> AppResult<Json<Value>> {
    owner(&s, &h, true)?;
    let conn = s.conn()?;
    let changed = conn.execute(
        "UPDATE staff_members SET active=?2,updated_at=?3 WHERE user_id=?1",
        params![id, input.active, now()],
    )?;
    if changed == 0 {
        return Err(AppError::not_found("Staff member not found"));
    }
    db::audit(
        &conn,
        "admin",
        if input.active {
            "staff.member_enabled"
        } else {
            "staff.member_disabled"
        },
        &id,
        "",
    )?;
    Ok(Json(json!({"ok":true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE users(id TEXT PRIMARY KEY,email TEXT UNIQUE,banned INTEGER NOT NULL DEFAULT 0,name TEXT NOT NULL DEFAULT '');
             CREATE TABLE audit(id INTEGER PRIMARY KEY,actor TEXT NOT NULL,action TEXT NOT NULL,target TEXT NOT NULL,detail TEXT NOT NULL,created_at INTEGER NOT NULL);
             INSERT INTO users(id,email) VALUES('one','one@example.org'),('two','two@example.org');",
        ).unwrap();
        migrate(&db).unwrap();
        db
    }
    #[test]
    fn invitation_is_single_use_and_requires_matching_verified_email() {
        let mut db = setup();
        let (_id, token, _) = create_record(&mut db, "one@example.org").unwrap();
        assert!(accept_record(&mut db, &token, "two").is_err());
        assert_eq!(active_role(&db, "two").unwrap(), None);
        accept_record(&mut db, &token, "one").unwrap();
        assert_eq!(active_role(&db, "one").unwrap().as_deref(), Some("manager"));
        assert!(accept_record(&mut db, &token, "one").is_err());
        db.execute("UPDATE staff_members SET active=0 WHERE user_id='one'", [])
            .unwrap();
        assert_eq!(active_role(&db, "one").unwrap(), None);
    }
    #[test]
    fn expired_revoked_and_malformed_tokens_are_rejected() {
        let mut db = setup();
        let (id, token, _) = create_record(&mut db, "one@example.org").unwrap();
        db.execute(
            "UPDATE staff_invitations SET expires_at=0 WHERE id=?1",
            [&id],
        )
        .unwrap();
        assert!(accept_record(&mut db, &token, "one").is_err());
        let (_id, another, _) = create_record(&mut db, "two@example.org").unwrap();
        db.execute(
            "UPDATE staff_invitations SET revoked_at=?1 WHERE token_hash=?2",
            params![now(), auth::hash_token(&another)],
        )
        .unwrap();
        assert!(accept_record(&mut db, &another, "two").is_err());
        assert!(accept_record(&mut db, "bad", "one").is_err());
    }
    #[test]
    fn invitation_rate_limit_and_banned_member_are_enforced() {
        let mut db = setup();
        let (_id, token, _) = create_record(&mut db, "one@example.org").unwrap();
        assert!(create_record(&mut db, "one@example.org").is_err());
        db.execute("UPDATE users SET banned=1 WHERE id='one'", [])
            .unwrap();
        assert!(accept_record(&mut db, &token, "one").is_err());
        assert_eq!(active_role(&db, "one").unwrap(), None);
    }
}
