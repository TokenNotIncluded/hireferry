//! Platform employees are separate from both customer accounts and the root
//! Passkey identity. Only the root browser can grant or change team access.
use crate::{AppError, AppResult, AppState, auth, db::audit, now, safety};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, patch, post},
};
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};

const RECENT_AUTH_SECONDS: i64 = 600;
const INVITE_PREFIX: &str = "hf_invite_";
const AI_PREFIX: &str = "hf_staff_";

pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS team_members(
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL CHECK(kind IN ('human','ai')),
            user_id TEXT UNIQUE REFERENCES users(id),
            name TEXT NOT NULL,
            active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0,1)),
            created_by TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            CHECK((kind='human' AND user_id IS NOT NULL) OR (kind='ai' AND user_id IS NULL))
        );
        CREATE INDEX IF NOT EXISTS team_members_active ON team_members(active,created_at);
        CREATE TABLE IF NOT EXISTS team_invitations(
            id TEXT PRIMARY KEY,
            email TEXT NOT NULL COLLATE NOCASE,
            name TEXT NOT NULL DEFAULT '',
            token_hash TEXT NOT NULL UNIQUE,
            status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','accepted','revoked','expired')),
            created_by TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            accepted_by TEXT REFERENCES users(id),
            accepted_at INTEGER,
            delivery_status TEXT NOT NULL DEFAULT 'not_sent' CHECK(delivery_status IN ('not_sent','sending','sent','failed')),
            last_sent_at INTEGER
        );
        CREATE UNIQUE INDEX IF NOT EXISTS team_pending_email ON team_invitations(email) WHERE status='pending';
        CREATE INDEX IF NOT EXISTS team_invitations_expiry ON team_invitations(status,expires_at);
        CREATE TABLE IF NOT EXISTS team_ai_tokens(
            id TEXT PRIMARY KEY,
            member_id TEXT NOT NULL REFERENCES team_members(id),
            token_hash TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            last_used_at INTEGER,
            revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1))
        );
        CREATE INDEX IF NOT EXISTS team_tokens_member ON team_ai_tokens(member_id,revoked,expires_at);
        CREATE TRIGGER IF NOT EXISTS team_member_disabled AFTER UPDATE OF active ON team_members
        WHEN NEW.active=0 BEGIN
            UPDATE team_ai_tokens SET revoked=1 WHERE member_id=NEW.id;
        END;
        CREATE TRIGGER IF NOT EXISTS team_account_suspended AFTER UPDATE OF banned ON users
        WHEN NEW.banned=1 BEGIN
            UPDATE team_members SET active=0,updated_at=unixepoch() WHERE user_id=NEW.id;
            UPDATE team_invitations SET status='revoked',updated_at=unixepoch()
                WHERE email=NEW.email COLLATE NOCASE AND status='pending';
        END;",
    )?;
    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/team", get(list))
        .route("/api/admin/team/invitations", post(invite))
        .route(
            "/api/admin/team/invitations/{id}",
            delete(revoke_invitation),
        )
        .route("/api/admin/team/invitations/{id}/resend", post(resend))
        .route("/api/admin/team/members/{id}", patch(update_member))
        .route("/api/admin/team/ai", post(create_ai))
        .route("/api/admin/team/members/{id}/token", post(rotate_ai_token))
        .route("/api/team/invitations/preview", post(preview))
        .route("/api/team/invitations/accept", post(accept))
}

pub fn human_member(db: &Connection, uid: &str) -> AppResult<Option<Value>> {
    Ok(db.query_row(
        "SELECT m.id,m.name FROM team_members m JOIN users u ON u.id=m.user_id
         WHERE m.user_id=?1 AND m.kind='human' AND m.active=1 AND u.banned=0",
        [uid],
        |row| Ok(json!({"id":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"kind":"human","role":"staff","active":true})),
    ).optional()?)
}

pub fn ensure_member(db: &Connection, id: &str) -> AppResult<()> {
    let active: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM team_members m JOIN users u ON u.id=m.user_id
         WHERE m.id=?1 AND m.kind='human' AND m.active=1 AND u.banned=0)",
        [id],
        |row| row.get(0),
    )?;
    if !active {
        return Err(AppError::forbidden("Employee access has been disabled"));
    }
    Ok(())
}

/// Customer moderation must not become a second way to administer the team.
/// Invited accounts are protected too: suspending one revokes its invitation.
pub fn ensure_account_access_change(
    db: &Connection,
    principal: &crate::access::Principal,
    user_id: &str,
) -> AppResult<()> {
    if principal.admin && !principal.via_ai && principal.actor == "admin" {
        return Ok(());
    }
    let protected: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM team_members WHERE user_id=?1)
         OR EXISTS(SELECT 1 FROM team_invitations i JOIN users u ON i.email=u.email COLLATE NOCASE
                   WHERE u.id=?1 AND i.status='pending' AND i.expires_at>?2)
         OR EXISTS(SELECT 1 FROM staff_members WHERE user_id=?1)
         OR EXISTS(SELECT 1 FROM staff_invitations i JOIN users u ON i.email=u.email COLLATE NOCASE
                   WHERE u.id=?1 AND i.accepted_at IS NULL AND i.revoked_at IS NULL AND i.expires_at>?2)",
        params![user_id, now()],
        |row| row.get(0),
    )?;
    if protected {
        return Err(AppError::forbidden(
            "Only the super administrator can change an employee account's access",
        ));
    }
    Ok(())
}

fn valid_token(raw: &str, prefix: &str) -> bool {
    raw.strip_prefix(prefix)
        .is_some_and(|secret| secret.len() == 64 && secret.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn new_token(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("{prefix}{}", hex::encode(bytes))
}

pub fn verify_ai_token(db: &Connection, raw: &str) -> AppResult<Option<String>> {
    if !valid_token(raw, AI_PREFIX) {
        return Ok(None);
    }
    let id: Option<String> = db
        .query_row(
            "SELECT t.id FROM team_ai_tokens t JOIN team_members m ON m.id=t.member_id
         WHERE t.token_hash=?1 AND t.revoked=0 AND t.expires_at>?2 AND m.kind='ai' AND m.active=1",
            params![auth::hash_token(raw), now()],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = &id {
        db.execute(
            "UPDATE team_ai_tokens SET last_used_at=?2 WHERE id=?1",
            params![id, now()],
        )?;
    }
    Ok(id)
}

pub fn ensure_ai_token(db: &Connection, id: &str) -> AppResult<()> {
    let active: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM team_ai_tokens t JOIN team_members m ON m.id=t.member_id
         WHERE t.id=?1 AND t.revoked=0 AND t.expires_at>?2 AND m.kind='ai' AND m.active=1)",
        params![id, now()],
        |row| row.get(0),
    )?;
    if !active {
        return Err(AppError::unauthorized(
            "AI employee token expired or revoked",
        ));
    }
    Ok(())
}

fn browser_only(headers: &HeaderMap) -> AppResult<()> {
    if headers.contains_key("authorization") || headers.contains_key("x-hireferry-ai") {
        return Err(AppError::forbidden(
            "Manage team access from your own browser session",
        ));
    }
    Ok(())
}

fn owner(s: &AppState, headers: &HeaderMap, write: bool) -> AppResult<auth::Session> {
    browser_only(headers)?;
    let session = auth::require_admin(s, headers)?;
    if write {
        auth::csrf_check(s, headers, &session)?;
    }
    Ok(session)
}

fn ensure_owner(db: &Connection, session: &auth::Session, recent: bool) -> AppResult<()> {
    let authenticated_at: Option<i64> = db.query_row(
        "SELECT recent_auth_at FROM sessions WHERE token_hash=?1 AND admin=1 AND bootstrap=0 AND user_id IS NULL AND expires_at>?2",
        params![session.token_hash,now()], |row| row.get(0),
    ).optional()?;
    let authenticated_at = authenticated_at
        .ok_or_else(|| AppError::unauthorized("Administrator session expired or revoked"))?;
    if recent && now().saturating_sub(authenticated_at) > RECENT_AUTH_SECONDS {
        return Err(AppError::forbidden(
            "Sign in with a passkey again before granting team access",
        ));
    }
    Ok(())
}

fn owner_actor(session: &auth::Session) -> String {
    format!("admin_session:{}", session.token_hash)
}

fn capacity(db: &Connection, s: &AppState) -> AppResult<()> {
    let count: i64 = db.query_row(
        "SELECT count(*) FROM team_members WHERE active=1",
        [],
        |r| r.get(0),
    )?;
    if count >= s.cfg.team.max_members {
        return Err(AppError::conflict(
            "The team has reached its active member limit",
        ));
    }
    Ok(())
}

fn invitation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let status: String = row.get("status")?;
    let expires_at: i64 = row.get("expires_at")?;
    Ok(json!({
        "id":row.get::<_,String>("id")?,"email":row.get::<_,String>("email")?,
        "name":row.get::<_,String>("name")?,"role":"staff",
        "status":if status=="pending" && expires_at<=now(){"expired"}else{&status},
        "created_at":row.get::<_,i64>("created_at")?,"expires_at":expires_at,
        "delivery_status":row.get::<_,String>("delivery_status")?,
        "last_sent_at":row.get::<_,Option<i64>>("last_sent_at")?
    }))
}

fn invitation(db: &Connection, id: &str) -> AppResult<Value> {
    db.query_row(
        "SELECT * FROM team_invitations WHERE id=?1",
        [id],
        invitation_row,
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("Invitation not found"))
}

const MEMBER_QUERY: &str = "SELECT m.*,u.email,COALESCE(u.banned,0) AS account_suspended,
    (SELECT max(last_used_at) FROM team_ai_tokens WHERE member_id=m.id) AS last_used_at,
    (SELECT max(expires_at) FROM team_ai_tokens WHERE member_id=m.id AND revoked=0) AS token_expires_at
    FROM team_members m LEFT JOIN users u ON u.id=m.user_id";

fn member_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id":row.get::<_,String>("id")?,"name":row.get::<_,String>("name")?,
        "kind":row.get::<_,String>("kind")?,"role":"staff",
        "email":row.get::<_,Option<String>>("email")?,"active":row.get::<_,bool>("active")?,
        "account_suspended":row.get::<_,bool>("account_suspended")?,
        "created_at":row.get::<_,i64>("created_at")?,
        "last_used_at":row.get::<_,Option<i64>>("last_used_at")?,
        "token_expires_at":row.get::<_,Option<i64>>("token_expires_at")?
    }))
}

fn member(db: &Connection, id: &str) -> AppResult<Value> {
    db.query_row(&format!("{MEMBER_QUERY} WHERE m.id=?1"), [id], member_row)
        .optional()?
        .ok_or_else(|| AppError::not_found("Team member not found"))
}

async fn list(State(s): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let session = owner(&s, &headers, false)?;
    let db = s.conn()?;
    ensure_owner(&db, &session, false)?;
    let members = db
        .prepare(&format!(
            "{MEMBER_QUERY} ORDER BY m.active DESC,m.created_at DESC,m.id LIMIT 1000"
        ))?
        .query_map([], member_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let invitations = db.prepare("SELECT * FROM team_invitations ORDER BY (status='pending' AND expires_at>unixepoch()) DESC,created_at DESC,id LIMIT 1000")?
        .query_map([], invitation_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Json(json!({
        "members":members,"invitations":invitations,"smtp_enabled":s.cfg.smtp.enabled,
        "invitation_ttl_hours":s.cfg.team.invitation_ttl_hours,
        "ai_token_ttl_days":s.cfg.team.ai_token_ttl_days,"max_members":s.cfg.team.max_members
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteInput {
    email: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    send_email: bool,
}

fn check_invitee(db: &Connection, email: &str) -> AppResult<()> {
    let unavailable: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM users u LEFT JOIN team_members m ON m.user_id=u.id
         WHERE u.email=?1 COLLATE NOCASE AND (u.banned=1 OR m.active=1))",
        [email],
        |row| row.get(0),
    )?;
    if unavailable {
        return Err(AppError::conflict(
            "This account is already an active employee or is suspended",
        ));
    }
    Ok(())
}

fn expire_invitations(db: &Connection) -> AppResult<()> {
    db.execute("UPDATE team_invitations SET status='expired',updated_at=?1 WHERE status='pending' AND expires_at<=?1", [now()])?;
    Ok(())
}

fn mail_requested(s: &AppState, send: bool) -> AppResult<()> {
    if send && !s.cfg.smtp.enabled {
        return Err(AppError::unavailable(
            "SMTP is not configured; create a copyable invitation link instead",
        ));
    }
    Ok(())
}

async fn invite(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<InviteInput>,
) -> AppResult<Json<Value>> {
    let session = owner(&s, &headers, true)?;
    let email = auth::normalized_email(&input.email)?;
    safety::bounded_text(&input.name, 80, "Employee name")?;
    mail_requested(&s, input.send_email)?;
    let id = uuid::Uuid::new_v4().to_string();
    let raw = new_token(INVITE_PREFIX);
    let hash = auth::hash_token(&raw);
    let t = now();
    {
        let mut db = s.conn()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_owner(&tx, &session, true)?;
        capacity(&tx, &s)?;
        expire_invitations(&tx)?;
        check_invitee(&tx, &email)?;
        let pending: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM team_invitations WHERE email=?1 AND status='pending')",
            [&email],
            |r| r.get(0),
        )?;
        if pending {
            return Err(AppError::conflict(
                "An invitation is already pending for this email; resend or revoke it first",
            ));
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM team_invitations WHERE status='pending'",
            [],
            |r| r.get(0),
        )?;
        if count >= s.cfg.team.max_pending_invitations {
            return Err(AppError::conflict(
                "Too many pending invitations; revoke an unused invitation first",
            ));
        }
        auth::rate_limit(
            &tx,
            "team-invitations",
            s.cfg.team.invitations_per_hour,
            3600,
        )?;
        tx.execute(
            "INSERT INTO team_invitations(id,email,name,token_hash,created_by,created_at,updated_at,expires_at,delivery_status,last_sent_at)
             VALUES(?1,?2,?3,?4,?5,?6,?6,?7,?8,?9)",
            params![id,email,input.name.trim(),hash,owner_actor(&session),t,t+s.cfg.team.invitation_ttl_hours*3600,
                if input.send_email{"sending"}else{"not_sent"},input.send_email.then_some(t)],
        )?;
        audit(&tx, &owner_actor(&session), "team_invite_create", &id, "")?;
        tx.commit()?;
    }
    finish_invitation(&s, &id, &raw, input.send_email).await
}

fn default_send() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResendInput {
    #[serde(default = "default_send")]
    send_email: bool,
}

async fn resend(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<ResendInput>,
) -> AppResult<Json<Value>> {
    let session = owner(&s, &headers, true)?;
    mail_requested(&s, input.send_email)?;
    let raw = new_token(INVITE_PREFIX);
    let t = now();
    {
        let mut db = s.conn()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_owner(&tx, &session, true)?;
        capacity(&tx, &s)?;
        expire_invitations(&tx)?;
        let (email, status, updated_at): (String, String, i64) = tx
            .query_row(
                "SELECT email,status,updated_at FROM team_invitations WHERE id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| AppError::not_found("Invitation not found"))?;
        if !matches!(status.as_str(), "pending" | "expired") {
            return Err(AppError::conflict(
                "Accepted or revoked invitations cannot be resent",
            ));
        }
        if status == "pending" && t.saturating_sub(updated_at) < s.cfg.team.resend_cooldown_seconds
        {
            return Err(AppError {
                status: StatusCode::TOO_MANY_REQUESTS,
                message: "Wait before generating another invitation link".into(),
            });
        }
        check_invitee(&tx, &email)?;
        let newer: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM team_invitations WHERE email=?1 AND status='pending' AND id<>?2)", params![email,id],|r|r.get(0))?;
        if newer {
            return Err(AppError::conflict(
                "Use the newer pending invitation for this email",
            ));
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM team_invitations WHERE status='pending' AND id<>?1",
            [&id],
            |r| r.get(0),
        )?;
        if count >= s.cfg.team.max_pending_invitations {
            return Err(AppError::conflict("Too many pending invitations"));
        }
        auth::rate_limit(
            &tx,
            "team-invitations",
            s.cfg.team.invitations_per_hour,
            3600,
        )?;
        tx.execute(
            "UPDATE team_invitations SET token_hash=?2,status='pending',updated_at=?3,expires_at=?4,delivery_status=?5,last_sent_at=?6 WHERE id=?1",
            params![id,auth::hash_token(&raw),t,t+s.cfg.team.invitation_ttl_hours*3600,
                if input.send_email{"sending"}else{"not_sent"},input.send_email.then_some(t)],
        )?;
        audit(&tx, &owner_actor(&session), "team_invite_rotate", &id, "")?;
        tx.commit()?;
    }
    finish_invitation(&s, &id, &raw, input.send_email).await
}

async fn finish_invitation(
    s: &AppState,
    id: &str,
    raw: &str,
    send: bool,
) -> AppResult<Json<Value>> {
    let invitation_url = format!("{}/team/invite#{raw}", s.cfg.public_url);
    let current_invitation = || -> AppResult<Value> {
        let db = s.conn()?;
        let current: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM team_invitations WHERE id=?1 AND token_hash=?2 AND status='pending' AND expires_at>?3)",
            params![id, auth::hash_token(raw), now()],
            |row| row.get(0),
        )?;
        if !current {
            return Err(AppError::conflict(
                "The invitation changed while sending; refresh the invitation list",
            ));
        }
        invitation(&db, id)
    };
    let before = current_invitation()?;
    if send {
        let email = before["email"].as_str().unwrap_or_default();
        let expires =
            chrono::DateTime::from_timestamp(before["expires_at"].as_i64().unwrap_or(0), 0)
                .map(|date| date.to_rfc3339())
                .unwrap_or_default();
        let body = format!(
            "你收到聘渡运营团队的员工邀请。\n\n请使用 {email} 登录，查看权限并确认加入：\n{invitation_url}\n\n员工可以处理全平台的资料、岗位、匹配、会话、委托、订单与审计记录；不能邀请其他成员或管理团队、Passkey 和访问凭据。\n链接有效至 {expires}，仅能使用一次。打开链接不会自动加入。不是你的邀请请忽略。\n\nYou have been invited to the HireFerry operations team.\nSign in with {email}, review the permissions, and explicitly accept using the link above.\nEmployees can work with platform profiles, jobs, matches, conversations, mandates, orders and audit records. They cannot manage the team, Passkeys or credentials.\nThis single-use link expires at {expires}. Opening it does not accept the invitation. Ignore this message if it was not intended for you."
        );
        let sent = auth::send_mail(
            s,
            email,
            "聘渡 HireFerry · 团队邀请 / Team invitation",
            body,
        )
        .await
        .is_ok();
        let db = s.conn()?;
        // A revoke or another rotation during SMTP delivery must never revive
        // this link or overwrite the delivery result of its replacement.
        let changed = db.execute(
            "UPDATE team_invitations SET delivery_status=?3 WHERE id=?1 AND token_hash=?2 AND status='pending'",
            params![id,auth::hash_token(raw),if sent{"sent"}else{"failed"}],
        )?;
        if changed == 1 {
            audit(
                &db,
                "system",
                if sent {
                    "team_invite_email_sent"
                } else {
                    "team_invite_email_failed"
                },
                id,
                "",
            )?;
        }
    }
    let result = current_invitation()?;
    Ok(Json(
        json!({"delivery_status":result["delivery_status"],"invitation":result,"invitation_url":invitation_url}),
    ))
}

async fn revoke_invitation(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let session = owner(&s, &headers, true)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_owner(&tx, &session, false)?;
    let value = invitation(&tx, &id)?;
    if value["status"] == "accepted" {
        return Err(AppError::conflict(
            "This invitation was accepted; disable the employee instead",
        ));
    }
    tx.execute(
        "UPDATE team_invitations SET status='revoked',updated_at=?2 WHERE id=?1",
        params![id, now()],
    )?;
    audit(&tx, &owner_actor(&session), "team_invite_revoke", &id, "")?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvitationToken {
    token: String,
}

fn token_invitation(db: &Connection, raw: &str) -> AppResult<Value> {
    if !valid_token(raw, INVITE_PREFIX) {
        return Err(AppError::not_found("Invitation not found"));
    }
    db.query_row(
        "SELECT * FROM team_invitations WHERE token_hash=?1",
        [auth::hash_token(raw)],
        invitation_row,
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("Invitation not found"))
}

fn email_hint(email: &str) -> String {
    let (local, domain) = email.rsplit_once('@').unwrap_or(("", ""));
    format!("{}***@{domain}", local.chars().next().unwrap_or('*'))
}

async fn preview(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<InvitationToken>,
) -> AppResult<Json<Value>> {
    let session = auth::session(&s, &headers)?;
    let db = s.conn()?;
    let invitation = token_invitation(&db, &input.token)?;
    let email = invitation["email"].as_str().unwrap_or_default();
    let email_matches = if let Some(uid) = session
        .as_ref()
        .and_then(|session| session.user_id.as_deref())
    {
        Some(
            db.query_row(
                "SELECT email=?2 COLLATE NOCASE FROM users WHERE id=?1 AND banned=0",
                params![uid, email],
                |r| r.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false),
        )
    } else {
        None
    };
    Ok(Json(
        json!({"status":invitation["status"],"expires_at":invitation["expires_at"],
        "email_hint":email_hint(email),"name":invitation["name"],"role":"staff","email_matches":email_matches}),
    ))
}

async fn accept(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<InvitationToken>,
) -> AppResult<Json<Value>> {
    browser_only(&headers)?;
    let session = auth::require_session(&s, &headers)?;
    if session.admin || session.bootstrap {
        return Err(AppError::forbidden(
            "Sign in with the invited email account to join the team",
        ));
    }
    let uid = session
        .user_id
        .as_deref()
        .ok_or_else(|| AppError::unauthorized("Sign in with the invited email account"))?;
    auth::csrf_check(&s, &headers, &session)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (email,user_name): (String,String) = tx.query_row(
        "SELECT u.email,u.name FROM sessions s JOIN users u ON u.id=s.user_id
         WHERE s.token_hash=?1 AND s.user_id=?2 AND s.admin=0 AND s.bootstrap=0 AND s.expires_at>?3 AND u.banned=0",
        params![session.token_hash,uid,now()],|r|Ok((r.get(0)?,r.get(1)?)),
    ).optional()?.ok_or_else(||AppError::unauthorized("Your session expired or this account is unavailable"))?;
    let invitation = token_invitation(&tx, &input.token)?;
    if invitation["status"] != "pending" {
        return Err(AppError::conflict(
            "This invitation has expired, was revoked or was already accepted",
        ));
    }
    if !email.eq_ignore_ascii_case(invitation["email"].as_str().unwrap_or_default()) {
        return Err(AppError::forbidden(
            "Sign in with the email address that received this invitation",
        ));
    }
    if human_member(&tx, uid)?.is_some() {
        return Err(AppError::conflict(
            "This account is already an active employee",
        ));
    }
    capacity(&tx, &s)?;
    let id = tx
        .query_row("SELECT id FROM team_members WHERE user_id=?1", [uid], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let name = invitation["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .unwrap_or(if user_name.is_empty() {
            "Team member"
        } else {
            &user_name
        });
    let invitation_id = invitation["id"].as_str().unwrap_or_default();
    let t = now();
    tx.execute(
        "INSERT INTO team_members(id,kind,user_id,name,created_by,created_at,updated_at)
         VALUES(?1,'human',?2,?3,?4,?5,?5) ON CONFLICT(user_id) DO UPDATE SET name=excluded.name,active=1,updated_at=excluded.updated_at",
        params![id,uid,name,format!("invitation:{invitation_id}"),t],
    )?;
    let changed = tx.execute(
        "UPDATE team_invitations SET status='accepted',accepted_by=?2,accepted_at=?3,updated_at=?3 WHERE id=?1 AND status='pending' AND expires_at>?3",
        params![invitation_id,uid,t],
    )?;
    if changed != 1 {
        return Err(AppError::conflict("This invitation is no longer available"));
    }
    audit(
        &tx,
        &format!("user:{uid}"),
        "team_invite_accept",
        &id,
        invitation_id,
    )?;
    let result = member(&tx, &id)?;
    tx.commit()?;
    Ok(Json(json!({"member":result})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberUpdate {
    active: bool,
}

async fn update_member(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<MemberUpdate>,
) -> AppResult<Json<Value>> {
    let session = owner(&s, &headers, true)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_owner(&tx, &session, input.active)?;
    let old = member(&tx, &id)?;
    if input.active && old["account_suspended"] == true {
        return Err(AppError::conflict(
            "Restore the suspended account before enabling employee access",
        ));
    }
    if input.active && old["active"] != true {
        capacity(&tx, &s)?;
    }
    tx.execute(
        "UPDATE team_members SET active=?2,updated_at=?3 WHERE id=?1",
        params![id, input.active, now()],
    )?;
    // Do not leave an unused invitation that could re-enable a removed employee.
    if !input.active
        && let Some(email) = old["email"].as_str()
    {
        tx.execute("UPDATE team_invitations SET status='revoked',updated_at=?2 WHERE email=?1 AND status='pending'",params![email,now()])?;
    }
    audit(
        &tx,
        &owner_actor(&session),
        if input.active {
            "team_member_enable"
        } else {
            "team_member_disable"
        },
        &id,
        "",
    )?;
    let result = member(&tx, &id)?;
    tx.commit()?;
    Ok(Json(json!({"member":result})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AiInput {
    name: String,
}

fn issue_ai(db: &Connection, s: &AppState, id: &str) -> AppResult<(String, i64)> {
    let raw = new_token(AI_PREFIX);
    let expires_at = now() + s.cfg.team.ai_token_ttl_days * 86400;
    db.execute(
        "UPDATE team_ai_tokens SET revoked=1 WHERE member_id=?1",
        [id],
    )?;
    db.execute("INSERT INTO team_ai_tokens(id,member_id,token_hash,created_at,expires_at) VALUES(?1,?2,?3,?4,?5)",
        params![uuid::Uuid::new_v4().to_string(),id,auth::hash_token(&raw),now(),expires_at])?;
    Ok((raw, expires_at))
}

async fn create_ai(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AiInput>,
) -> AppResult<Json<Value>> {
    let session = owner(&s, &headers, true)?;
    safety::bounded_text(&input.name, 80, "AI employee name")?;
    if input.name.trim().is_empty() {
        return Err(AppError::bad_request("AI employee name is required"));
    }
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_owner(&tx, &session, true)?;
    capacity(&tx, &s)?;
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute("INSERT INTO team_members(id,kind,name,created_by,created_at,updated_at) VALUES(?1,'ai',?2,?3,?4,?4)",
        params![id,input.name.trim(),owner_actor(&session),now()])?;
    let (raw, expires_at) = issue_ai(&tx, &s, &id)?;
    audit(&tx, &owner_actor(&session), "team_ai_create", &id, "")?;
    let result = member(&tx, &id)?;
    tx.commit()?;
    Ok(Json(
        json!({"member":result,"token":raw,"expires_at":expires_at}),
    ))
}

async fn rotate_ai_token(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let session = owner(&s, &headers, true)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_owner(&tx, &session, true)?;
    let old = member(&tx, &id)?;
    if old["kind"] != "ai" || old["active"] != true {
        return Err(AppError::conflict(
            "Enable an AI employee before creating a new token",
        ));
    }
    let (raw, expires_at) = issue_ai(&tx, &s, &id)?;
    audit(&tx, &owner_actor(&session), "team_ai_token_rotate", &id, "")?;
    let result = member(&tx, &id)?;
    tx.commit()?;
    Ok(Json(
        json!({"member":result,"token":raw,"expires_at":expires_at}),
    ))
}
