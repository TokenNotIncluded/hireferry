//! Shared identity and mediation boundaries for HTTP, MCP and chat.
use crate::{AppError, AppResult, AppState, auth, models, now};
use axum::http::HeaderMap;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct Principal {
    pub user_id: Option<String>,
    pub admin: bool,
    pub via_ai: bool,
    pub actor: String,
}

pub fn eligibility(db: &Connection, uid: &str) -> AppResult<Value> {
    let user = models::load_user(db, uid)?;
    let (eligible, reason) = if user.banned {
        (false, "account_suspended")
    } else if user.role == "seeker" {
        (true, "seeker_free")
    } else if user.role == "employer" {
        let qualifying: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM jobs j JOIN orders o ON o.job_id=j.id AND o.kind='publish' AND o.status IN ('paid','partially_refunded') WHERE j.owner_id=?1 AND j.status='published' AND j.salary_cents>=j.ai_min_salary_cents)",
            [uid],
            |r| r.get(0),
        )?;
        if user.employer_paid && qualifying {
            (true, "qualifying_paid_job")
        } else {
            (false, "paid_job_required")
        }
    } else {
        (false, "choose_role_first")
    };
    Ok(json!({"eligible":eligible,"reason":reason,"role":user.role}))
}

pub fn require_user_ai(db: &Connection, uid: &str) -> AppResult<()> {
    let status = eligibility(db, uid)?;
    if status["eligible"] == true {
        Ok(())
    } else {
        Err(AppError::forbidden(
            if status["reason"] == "paid_job_required" {
                "Employer AI access requires a fulfilled, currently published job meeting its recorded salary threshold"
            } else {
                "Choose an active account role before connecting AI"
            },
        ))
    }
}

pub fn principal(s: &AppState, h: &HeaderMap, write: bool) -> AppResult<Principal> {
    if let Some(value) = h.get("authorization") {
        let raw = value
            .to_str()
            .ok()
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| AppError::unauthorized("Invalid bearer token"))?;
        if raw.starts_with("hf_admin_") {
            let db = s.conn()?;
            crate::cli_auth::check_business_token(&db, &s.cfg, raw)?;
            let id = crate::admin_ai::verify_db(&db, raw)?
                .ok_or_else(|| AppError::unauthorized("Admin token expired or revoked"))?;
            return Ok(Principal {
                user_id: None,
                admin: true,
                via_ai: true,
                actor: format!("admin_ai:{id}"),
            });
        }
        let uid = crate::user_id(s, h, write)?;
        return Ok(Principal {
            actor: uid.clone(),
            user_id: Some(uid),
            admin: false,
            via_ai: true,
        });
    }
    let session = auth::session(s, h)?.ok_or_else(|| AppError::unauthorized("Sign in required"))?;
    if session.bootstrap {
        return Err(AppError::forbidden(
            "Enroll the first passkey before using the platform",
        ));
    }
    if write {
        auth::csrf_check(s, h, &session)?;
    }
    let webmcp = h.get("x-hireferry-ai").is_some_and(|v| v == "webmcp");
    if session.admin {
        auth::require_admin(s, h)?;
        Ok(Principal {
            user_id: None,
            admin: true,
            via_ai: webmcp,
            actor: if webmcp {
                format!("admin_webmcp:{}", session.token_hash)
            } else {
                "admin".into()
            },
        })
    } else {
        let uid = auth::require_user(s, h)?;
        if webmcp {
            require_user_ai(&*s.conn()?, &uid)?;
        }
        // Staff access is deliberately limited to a human browser session.
        // A member's ordinary CLI bearer / WebMCP never gains manager authority.
        let manager = !webmcp
            && crate::staff::active_role(&s.conn()?, &uid)?.as_deref() == Some("manager");
        Ok(Principal {
            actor: if manager { format!("staff:{uid}") } else { uid.clone() },
            user_id: Some(uid),
            admin: manager,
            via_ai: webmcp,
        })
    }
}

pub fn ensure_admin(db: &Connection, p: &Principal) -> AppResult<()> {
    if !p.admin || p.user_id.is_some() {
        return Err(AppError::forbidden("Administrator access required"));
    }
    if p.via_ai {
        if let Some(id) = p.actor.strip_prefix("admin_ai:") {
            crate::admin_ai::ensure_token(db, id)?;
        } else if let Some(hash) = p.actor.strip_prefix("admin_webmcp:") {
            let valid:bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=?1 AND admin=1 AND bootstrap=0 AND user_id IS NULL AND expires_at>?2)",params![hash,now()],|r|r.get(0))?;
            if !valid {
                return Err(AppError::unauthorized(
                    "Administrator session expired or revoked",
                ));
            }
        } else {
            return Err(AppError::unauthorized("Invalid administrator identity"));
        }
    }
    Ok(())
}

/// Restricted operational access for owner or verified human team managers.
/// Mutations of membership, payments, moderation, passkeys and AI tokens must
/// continue to call `ensure_admin`, never this function.
pub fn ensure_operator(db: &Connection, p: &Principal) -> AppResult<()> {
    if !p.admin {
        return Err(AppError::forbidden("Team manager access required"));
    }
    if p.user_id.is_none() {
        return ensure_admin(db, p);
    }
    let uid = p.user_id.as_deref().unwrap();
    if p.via_ai || p.actor != format!("staff:{uid}") {
        return Err(AppError::forbidden("Human team manager session required"));
    }
    if crate::staff::active_role(db, uid)?.as_deref() != Some("manager") {
        return Err(AppError::forbidden("Team membership has been disabled"));
    }
    Ok(())
}

pub fn ensure_active(db: &Connection, p: &Principal) -> AppResult<()> {
    if p.admin {
        return ensure_operator(db, p);
    }
    let uid = p
        .user_id
        .as_deref()
        .ok_or_else(|| AppError::unauthorized("Sign in required"))?;
    let active: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND banned=0)",
        [uid],
        |r| r.get(0),
    )?;
    if !active {
        return Err(AppError::unauthorized("Account unavailable"));
    }
    if p.via_ai {
        require_user_ai(db, uid)?;
    }
    Ok(())
}

/// A match grants access only to its exact pair. Discovery never grants access.
pub fn matched_pair(db: &Connection, viewer: &str, subject: &str) -> AppResult<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM matches m JOIN jobs j ON j.id=m.job_id JOIN users s ON s.id=m.seeker_id JOIN users e ON e.id=j.owner_id WHERE m.status IN ('introduced','interview','hired') AND j.status<>'deleted' AND s.banned=0 AND e.banned=0 AND ((m.seeker_id=?1 AND j.owner_id=?2) OR (m.seeker_id=?2 AND j.owner_id=?1)))",
        params![viewer, subject], |r| r.get(0),
    )?)
}

pub fn can_read_job(db: &Connection, viewer: &str, job_id: &str) -> AppResult<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs j JOIN users e ON e.id=j.owner_id WHERE j.id=?1 AND e.banned=0 AND (j.owner_id=?2 OR (j.status<>'deleted' AND EXISTS(SELECT 1 FROM matches m JOIN users s ON s.id=m.seeker_id WHERE m.job_id=j.id AND m.seeker_id=?2 AND m.status IN ('introduced','interview','hired') AND s.banned=0))))",
        params![job_id, viewer], |r| r.get(0),
    )?)
}

/// Used only to attribute API-token actions; the token itself is never retained.
pub fn user_token_id(db: &Connection, h: &HeaderMap) -> AppResult<Option<String>> {
    let Some(raw) = h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return Ok(None);
    };
    Ok(db
        .query_row(
            "SELECT id FROM api_tokens WHERE token_hash=?1 AND revoked=0 AND expires_at>?2",
            params![auth::hash_token(raw), now()],
            |r| r.get(0),
        )
        .optional()?)
}
