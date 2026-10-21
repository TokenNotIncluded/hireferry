//! Host-bound consent and revocation. A normal GET never approves a request.

use super::*;
use axum::routing::delete;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/humangrant/review/{link}", get(handle))
        .route("/api/humangrant/review/{link}/approve", post(handle))
        .route("/api/humangrant/review/{link}/deny", post(handle))
        .route("/api/humangrant/grants", get(handle))
        .route("/api/humangrant/grants/{id}", delete(handle))
}

fn member(session: Option<auth::Session>) -> Option<auth::Session> {
    session.filter(|s| !s.admin && !s.bootstrap && s.user_id.is_some())
}

fn current(db: &Connection, session: &auth::Session, time: i64) -> Result<String> {
    let uid = session.user_id.as_deref().ok_or_else(Error::session)?;
    let active:bool=db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=?1 AND s.user_id=?2 AND s.csrf=?3 AND s.expires_at>?4 AND s.admin=0 AND s.bootstrap=0 AND u.banned=0)",
        params![session.token_hash,uid,session.csrf,time],|row|row.get(0),
    )?;
    if !active {
        return Err(Error::session());
    }
    Ok(uid.to_owned())
}

fn csrf_message(scope: &str, request: &str, user: &str) -> Result<String> {
    Ok(serde_json::to_string(&json!([
        "HumanGrant browser action 0.1",
        [scope, request, user]
    ]))?)
}
fn csrf(session: &auth::Session, scope: &str, request: &str, user: &str) -> Result<String> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(session.csrf.as_bytes()).map_err(|_| Error::internal())?;
    mac.update(csrf_message(scope, request, user)?.as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}
fn check_csrf(
    session: &auth::Session,
    headers: &HeaderMap,
    scope: &str,
    request: &str,
    user: &str,
) -> Result<()> {
    let bad = || {
        Error::new(
            403,
            "csrf_failed",
            "Reload this page and confirm the displayed account and permissions.",
        )
    };
    let mut values = headers.get_all("x-hg-csrf").iter();
    let supplied = values
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or_else(bad)?;
    if values.next().is_some() {
        return Err(bad());
    }
    let tag = URL_SAFE_NO_PAD.decode(supplied).map_err(|_| bad())?;
    if URL_SAFE_NO_PAD.encode(&tag) != supplied {
        return Err(bad());
    }
    let mut mac =
        Hmac::<Sha256>::new_from_slice(session.csrf.as_bytes()).map_err(|_| Error::internal())?;
    mac.update(csrf_message(scope, request, user)?.as_bytes());
    mac.verify_slice(&tag).map_err(|_| bad())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    permissions: Vec<Permission>,
    expires_at: i64,
}

async fn handle(State(state): State<AppState>, request: Request) -> Response {
    match inner(&state, request).await {
        Ok(value) => json_response(StatusCode::OK, value),
        Err(error) => error.into_response(),
    }
}
async fn inner(state: &AppState, request: Request) -> Result<Value> {
    enabled(state)?;
    let (parts, body) = request.into_parts();
    if parts.uri.query().is_some() || parts.uri.path().contains(['%', '\\']) {
        return Err(Error::invalid());
    }
    // Human management accepts only host cookies. An AI credential is never a human login.
    if parts.headers.contains_key("authorization") {
        return Err(Error::forbidden());
    }
    let body = to_bytes(body, MAX_BODY)
        .await
        .map_err(|_| Error::body_too_large())?;
    if parts.method == Method::GET && !body.is_empty() {
        return Err(Error::invalid());
    }
    if parts.method != Method::GET {
        if parts.headers.get_all("origin").iter().count() != 1 {
            return Err(Error::new(
                403,
                "csrf_failed",
                "A same-origin browser request is required.",
            ));
        }
        auth::origin_check(state, &parts.headers)?;
    }
    let session = member(auth::session(state, &parts.headers)?);
    let path: Vec<_> = parts
        .uri
        .path()
        .trim_start_matches('/')
        .split('/')
        .collect();
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let time = now();
    expire(&tx, time)?;
    let value = match (parts.method.as_str(), path.as_slice()) {
        ("GET", ["api", "humangrant", "review", link]) => {
            let r = record(&tx, link, true)?;
            let human = match session.as_ref().map(|s| current(&tx, s, time)) {
                Some(Ok(human)) => Some(human),
                Some(Err(error)) if error.status != StatusCode::UNAUTHORIZED => return Err(error),
                _ => None,
            };
            let expected_ok = r
                .expected_human_id
                .as_deref()
                .is_none_or(|expected| human.as_deref() == Some(expected));
            let undecided = ["waiting_human", "authenticating_human", "awaiting_consent"]
                .contains(&r.phase.as_str());
            let mut grantable = Vec::new();
            let mut token = None;
            let mut name = None;
            if let (Some(human), Some(session)) = (&human, &session) {
                name = tx
                    .query_row("SELECT name FROM users WHERE id=?1", [human], |row| {
                        row.get::<_, String>(0)
                    })
                    .optional()?;
                if expected_ok && undecided {
                    for permission in &r.permissions {
                        if crate::humangrant_access::grantable(&tx, human, permission)? {
                            grantable.push(permission.clone());
                        }
                    }
                    token = Some(csrf(session, "consent", &r.id, human)?);
                }
            }
            json!({"request_id":r.id,"phase":r.phase,"actor_id":r.actor_id,"client_id":r.client_id,
                "target_service":"hireferry","purpose":r.purpose,"requested_permissions":r.permissions,
                "grantable_permissions":grantable,"expected_human_id":r.expected_human_id,
                "human_id":human,"human_name":name,"expires_at":r.expires_at,
                "requested_grant_expires_at":r.grant_expires_at,"checked_at":time,"csrf":token})
        }
        ("POST", ["api", "humangrant", "review", link, decision])
            if ["approve", "deny"].contains(decision) =>
        {
            let session = session.as_ref().ok_or_else(Error::session)?;
            let human = current(&tx, session, time)?;
            let r = record(&tx, link, true)?;
            check_csrf(session, &parts.headers, "consent", &r.id, &human)?;
            open(&r, time)?;
            if !["waiting_human", "authenticating_human", "awaiting_consent"]
                .contains(&r.phase.as_str())
            {
                return Err(Error::state());
            }
            if r.expected_human_id
                .as_deref()
                .is_some_and(|expected| expected != human)
            {
                return Err(Error::new(
                    403,
                    "expected_human_mismatch",
                    "This request requires a different signed-in account.",
                ));
            }
            if *decision == "deny" {
                let _: Empty = serde_json::from_slice(&body)?;
                tx.execute(
                    "UPDATE hg_requests SET phase='rejected',status_updated_at=?1 WHERE id=?2",
                    params![time, r.id],
                )?;
                crate::db::audit(&tx, &human, "humangrant.reject", &r.id, "")?;
                json!({"status":"rejected"})
            } else {
                let approval: Approval = serde_json::from_slice(&body)?;
                validate_permissions(&approval.permissions)?;
                access::require_user_ai(&tx, &human)?;
                if approval.expires_at <= time
                    || approval.expires_at > r.grant_expires_at
                    || !state.cfg.humangrant.client_ids.contains(&r.client_id)
                {
                    return Err(Error::forbidden());
                }
                for permission in &approval.permissions {
                    if !r.permissions.contains(permission)
                        || !crate::humangrant_access::grantable(&tx, &human, permission)?
                    {
                        return Err(Error::forbidden());
                    }
                }
                let id = crypto::random_token();
                tx.execute("INSERT INTO hg_grants(id,request_id,user_id,actor_id,client_id,permissions,created_at,expires_at,audit_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id,r.id,human,r.actor_id,r.client_id,serde_json::to_string(&approval.permissions)?,time,approval.expires_at,crypto::random_token()])?;
                tx.execute(
                    "UPDATE hg_requests SET phase='approved',status_updated_at=?1 WHERE id=?2",
                    params![time, r.id],
                )?;
                crate::db::audit(
                    &tx,
                    &human,
                    "humangrant.approve",
                    &id,
                    &serde_json::to_string(&approval.permissions)?,
                )?;
                json!({"status":"approved","grant_id":id,"expires_at":approval.expires_at})
            }
        }
        ("GET", ["api", "humangrant", "grants"]) => {
            let session = session.as_ref().ok_or_else(Error::session)?;
            let human = current(&tx, session, time)?;
            let mut stmt=tx.prepare("SELECT id,request_id,actor_id,client_id,permissions,created_at,expires_at,revoked_at,audit_id FROM hg_grants WHERE user_id=?1 ORDER BY created_at DESC,id")?;
            let rows = stmt.query_map([&human], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, Option<i64>>(7)?,
                    r.get::<_, String>(8)?,
                ))
            })?;
            let mut grants = Vec::new();
            for row in rows {
                let (id, request, actor, client, permissions, created, expires, revoked, audit) =
                    row?;
                let permissions: Value = serde_json::from_str(&permissions)?;
                grants.push(json!({"grant_id":id,"request_id":request,"grantor_id":human,"actor_id":actor,"public_key_id":actor,
                    "client_id":client,"target_service":"hireferry","permissions":permissions,"created_at":created,
                    "expires_at":expires,"revoked_at":revoked,"audit_id":audit}));
            }
            json!({"human_id":human,"csrf":csrf(session,"grants","",&human)?,"grants":grants})
        }
        ("DELETE", ["api", "humangrant", "grants", id]) => {
            if !safe_id(id) {
                return Err(Error::not_found());
            }
            if !body.is_empty() {
                let _: Empty = serde_json::from_slice(&body)?;
            }
            let session = session.as_ref().ok_or_else(Error::session)?;
            let human = current(&tx, session, time)?;
            check_csrf(session, &parts.headers, "grants", "", &human)?;
            let owner: Option<(String, String)> = tx
                .query_row(
                    "SELECT user_id,request_id FROM hg_grants WHERE id=?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let (owner, request_id) = owner.ok_or_else(Error::not_found)?;
            if owner != human {
                return Err(Error::not_found());
            }
            tx.execute(
                "UPDATE hg_grants SET revoked_at=?1 WHERE id=?2 AND revoked_at IS NULL",
                params![time, id],
            )?;
            tx.execute("UPDATE hg_requests SET phase='cancelled',status_updated_at=?1 WHERE id=?2 AND phase='approved'",params![time,request_id])?;
            crate::db::audit(&tx, &human, "humangrant.revoke", id, "")?;
            json!({"status":"revoked","grant_id":id})
        }
        _ => return Err(Error::not_found()),
    };
    tx.commit()?;
    Ok(value)
}
