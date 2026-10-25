use crate::{
    AppError, AppResult, AppState,
    access::{self, Principal},
    commerce,
    db::audit,
    models, now, safety,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch, post},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/overview", get(overview))
        .route(
            "/api/admin/pricing",
            get(pricing_settings).post(change_pricing_mode),
        )
        .route(
            "/api/admin/payments/waffo",
            get(waffo_settings).post(change_waffo_mode),
        )
        .route("/api/admin/users", get(users))
        .route("/api/admin/users/{id}", get(user_detail).patch(update_user))
        .route(
            "/api/admin/users/{id}/access",
            get(user_access).post(change_user_access),
        )
        .route(
            "/api/admin/users/{id}/resume",
            axum::routing::delete(delete_resume),
        )
        .route("/api/admin/jobs", get(jobs))
        .route(
            "/api/admin/jobs/{id}",
            get(job_detail).patch(update_job).delete(delete_job),
        )
        .route("/api/admin/jobs/{id}/restore", post(restore_job))
        .route("/api/admin/orders", get(orders))
        .route("/api/admin/audit", get(audits))
        .route("/api/admin/matches", get(matches).post(create_match))
        .route("/api/admin/matches/{id}", patch(update_match))
        .route(
            "/api/admin/workbench/candidates/{id}",
            get(workbench_candidate),
        )
        .route("/api/admin/workbench/matches/{id}", get(workbench_match))
        .route("/api/admin/workbench/introductions", post(create_matches))
}

fn guard(s: &AppState, h: &HeaderMap, write: bool) -> AppResult<Principal> {
    let p = access::principal(s, h, write)?;
    access::ensure_operator(&*s.conn()?, &p)?;
    Ok(p)
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct QueryInput {
    pub from: Option<String>,
    pub to: Option<String>,
    pub status: Option<String>,
    pub q: Option<String>,
    pub role: Option<String>,
    pub page: Option<i64>,
    pub mode: Option<String>,
    pub currency: Option<String>,
}

fn range(q: &QueryInput) -> AppResult<(i64, i64)> {
    fn date(s: &str) -> AppResult<i64> {
        Ok(chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|_| AppError::bad_request("Use YYYY-MM-DD dates"))?
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp())
    }
    let from = q
        .from
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(date)
        .transpose()?
        .unwrap_or(0);
    let to =
        q.to.as_deref()
            .filter(|s| !s.is_empty())
            .map(date)
            .transpose()?
            .map(|t| t + 86400)
            .unwrap_or(now() + 1);
    if from >= to {
        return Err(AppError::bad_request(
            "Start date must not be after end date",
        ));
    }
    Ok((from, to))
}

fn payment_mode(q: &QueryInput) -> AppResult<Option<bool>> {
    match q.mode.as_deref() {
        Some("live") => Ok(Some(true)),
        Some("test") => Ok(Some(false)),
        None => Ok(None),
        Some(_) => Err(AppError::bad_request(
            "Choose live or test payment statistics",
        )),
    }
}

/// Shared business functions recheck the principal while holding the database
/// lock (and the write transaction for mutations). HTTP and MCP use these alike.
pub fn overview_data(s: &AppState, p: &Principal, q: &QueryInput) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    let (from, to) = range(q)?;
    let live = payment_mode(q)?.unwrap_or(!s.cfg.development);
    let status = q.status.as_deref().unwrap_or_default();
    let currency = q.currency.as_deref().unwrap_or(&s.cfg.pricing.currency);
    let currency_decimals = crate::config::currency_decimals(currency)
        .ok_or_else(|| AppError::bad_request("Choose a supported reporting currency"))?;
    let count = |sql: &str| -> AppResult<i64> { Ok(db.query_row(sql, [], |r| r.get(0))?) };
    // Keep the original receipts immutable and deduct successful refunds on
    // their own cash-flow date. A refund-only day can have negative revenue.
    let movements = "WITH movements AS (
        SELECT order_id,amount_cents AS net,amount_cents AS gross,0 AS refund,1 AS payment,created_at,live_mode,currency FROM ledger
        UNION ALL
        SELECT order_id,-amount_cents,0,amount_cents,0,succeeded_at,live_mode,currency FROM payment_refunds WHERE status='succeeded'
    )";
    let (revenue, gross, refunds, paid_orders): (i64, i64, i64, i64) = db.query_row(
        &format!("{movements} SELECT COALESCE(sum(net),0),COALESCE(sum(gross),0),COALESCE(sum(refund),0),COALESCE(sum(payment),0) FROM movements WHERE created_at>=?1 AND created_at<?2 AND live_mode=?3 AND currency=?4 AND (?5='' OR order_id IN (SELECT id FROM orders WHERE status=?5))"),
        params![from, to, live, currency, status], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    let currency_totals = db.prepare(&format!(
        "{movements} SELECT currency,sum(net),sum(gross),sum(refund),sum(payment) FROM movements WHERE created_at>=?1 AND created_at<?2 AND live_mode=?3 AND (?4='' OR order_id IN (SELECT id FROM orders WHERE status=?4)) GROUP BY currency ORDER BY currency"
    ))?.query_map(params![from,to,live,status], |row| {
        let currency: String = row.get(0)?;
        Ok(json!({"currency":currency,"currency_decimals":crate::config::currency_decimals(&currency),"revenue_cents":row.get::<_,i64>(1)?,"gross_cents":row.get::<_,i64>(2)?,"refund_cents":row.get::<_,i64>(3)?,"paid_orders":row.get::<_,i64>(4)?}))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    let first: i64 = db.query_row(
        &format!(
            "{movements} SELECT COALESCE(min(created_at),?1) FROM movements WHERE live_mode=?2 AND currency=?3 AND (?4='' OR order_id IN (SELECT id FROM orders WHERE status=?4))"
        ),
        params![now(), live, currency, status],
        |r| r.get(0),
    )?;
    let monthly = to - from.max(first) > 366 * 86400;
    let bucket = if monthly { "%Y-%m" } else { "%Y-%m-%d" };
    let series = db.prepare(
        &format!("{movements} SELECT strftime(?4,created_at,'unixepoch') AS day,sum(net),sum(payment),sum(gross),sum(refund) FROM movements WHERE created_at>=?1 AND created_at<?2 AND live_mode=?3 AND currency=?5 AND (?6='' OR order_id IN (SELECT id FROM orders WHERE status=?6)) GROUP BY day ORDER BY day"))?
        .query_map(params![from, to, live, bucket, currency, status], |r| Ok(json!({
            "date":r.get::<_,String>(0)?,
            "revenue_cents":r.get::<_,i64>(1)?,
            "orders":r.get::<_,i64>(2)?,
            "gross_cents":r.get::<_,i64>(3)?,
            "refund_cents":r.get::<_,i64>(4)?
        })))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let by_kind = db.prepare(
        &format!("{movements} SELECT o.kind,sum(l.net),sum(l.payment),sum(l.gross),sum(l.refund) FROM movements l JOIN orders o ON o.id=l.order_id WHERE l.created_at>=?1 AND l.created_at<?2 AND l.live_mode=?3 AND l.currency=?4 AND (?5='' OR o.status=?5) GROUP BY o.kind"))?
        .query_map(params![from, to, live, currency, status], |r| Ok(json!({
            "kind":r.get::<_,String>(0)?,
            "amount_cents":r.get::<_,i64>(1)?,
            "count":r.get::<_,i64>(2)?,
            "gross_cents":r.get::<_,i64>(3)?,
            "refund_cents":r.get::<_,i64>(4)?
        })))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let recent = db
        .prepare("SELECT * FROM orders WHERE created_at>=?1 AND created_at<?2 AND live_mode=?3 AND currency=?4 AND (?5='' OR status=?5) ORDER BY created_at DESC,id LIMIT 8")?
        .query_map(params![from, to, live, currency, status], models::order_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let signup_sources = db
        .prepare("SELECT detail,count(*) FROM audit WHERE action='signup_source' AND created_at>=?1 AND created_at<?2 GROUP BY detail ORDER BY count(*) DESC,detail")?
        .query_map(params![from, to], |r| {
            Ok(json!({"source":r.get::<_,String>(0)?,"signups":r.get::<_,i64>(1)?}))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let referral_signups: i64 = db.query_row(
        "SELECT COUNT(*) FROM user_referrals WHERE created_at>=?1 AND created_at<?2",
        params![from, to],
        |r| r.get(0),
    )?;
    let referral_total: i64 =
        db.query_row("SELECT COUNT(*) FROM user_referrals", [], |r| r.get(0))?;
    let top_referrers = db
        .prepare(
            "SELECT u.id,u.name,u.handle,COUNT(*) AS invites
             FROM user_referrals r JOIN users u ON u.id=r.inviter_id
             WHERE r.created_at>=?1 AND r.created_at<?2
             GROUP BY u.id,u.name,u.handle
             ORDER BY invites DESC,u.id LIMIT 8",
        )?
        .query_map(params![from, to], |r| {
            Ok(json!({
                "user_id":r.get::<_,String>(0)?,
                "name":r.get::<_,String>(1)?,
                "handle":r.get::<_,String>(2)?,
                "invites":r.get::<_,i64>(3)?
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({
        "stats":{
            "users":count("SELECT count(*) FROM users")?,
            "seekers":count("SELECT count(*) FROM users WHERE role='seeker'")?,
            "employers":count("SELECT count(*) FROM users WHERE role='employer' AND employer_paid=1")?,
            "active_jobs":count("SELECT count(*) FROM jobs j JOIN users u ON u.id=j.owner_id WHERE j.status='published' AND j.headcount>0 AND u.banned=0")?,
            "paid_orders":paid_orders,
            "revenue_cents":revenue,
            "gross_cents":gross,
            "refund_cents":refunds,
            "pending_orders":count("SELECT count(*) FROM orders WHERE status='pending'")?,
            "review_orders":count("SELECT count(*) FROM orders WHERE status='paid_needs_review'")?
        },
        "series":series,"revenue_by_kind":by_kind,"recent_orders":recent,
        "growth":{"signup_sources":signup_sources,"referral_signups":referral_signups,"referral_total":referral_total,"top_referrers":top_referrers},
        "currency":currency,"currency_decimals":currency_decimals,"currency_totals":currency_totals,
        "bucket":if monthly{"month"}else{"day"},
        "timezone":"UTC","development":s.cfg.development,
        "payment_mode":if live{"live"}else{"test"}
    }))
}

pub fn list_users(s: &AppState, p: &Principal, q: &QueryInput) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_operator(&db, p)?;
    let search = format!(
        "%{}%",
        q.q.as_deref()
            .unwrap_or_default()
            .chars()
            .take(100)
            .collect::<String>()
    );
    let role = q.role.as_deref().unwrap_or_default();
    let access_status = q.status.as_deref().unwrap_or_default();
    let banned = match access_status {
        "" => None,
        "active" => Some(false),
        "suspended" | "banned" => Some(true),
        _ => return Err(AppError::bad_request("Choose active or suspended accounts")),
    };
    let page = q.page.unwrap_or(1).clamp(1, s.cfg.limits.list_max_page);
    let page_size = s.cfg.limits.list_page_size;
    let offset = (page - 1) * page_size;
    let condition = "(name LIKE ?1 OR email LIKE ?1 OR id LIKE ?1) AND (?2='' OR role=?2) AND (?3 IS NULL OR banned=?3)";
    let items = db
        .prepare(&format!(
            "SELECT * FROM users WHERE {condition} ORDER BY created_at DESC,id LIMIT ?4 OFFSET ?5"
        ))?
        .query_map(
            params![search, role, banned, page_size, offset],
            models::user_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total: i64 = db.query_row(
        &format!("SELECT count(*) FROM users WHERE {condition}"),
        params![search, role, banned],
        |r| r.get(0),
    )?;
    Ok(json!({"items":items,"total":total,"page":page,"page_size":page_size}))
}

pub fn get_user(s: &AppState, p: &Principal, id: &str) -> AppResult<models::User> {
    let db = s.conn()?;
    access::ensure_operator(&db, p)?;
    models::load_user(&db, id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserPatch {
    pub name: Option<String>,
    pub public_email: Option<String>,
    pub banned: Option<bool>,
    pub ban_reason: Option<String>,
    pub visible: Option<bool>,
    pub resume_md: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserAccessInput {
    pub banned: bool,
    pub reason: String,
}

fn access_record(db: &Connection, id: &str) -> AppResult<Value> {
    db.query_row(
        "SELECT u.banned,a.reason,a.changed_at,a.changed_by FROM users u LEFT JOIN account_moderation a ON a.user_id=u.id WHERE u.id=?1",
        [id],
        |row| Ok(json!({"banned":row.get::<_,bool>(0)?,"reason":row.get::<_,Option<String>>(1)?,"changed_at":row.get::<_,Option<i64>>(2)?,"changed_by":row.get::<_,Option<String>>(3)?})),
    ).optional()?.ok_or_else(|| AppError::not_found("User not found"))
}

fn record_access_change(
    db: &Connection,
    actor: &str,
    id: &str,
    previous: bool,
    banned: bool,
    reason: &str,
) -> AppResult<()> {
    let reason = reason.trim();
    safety::bounded_text(reason, 500, "Access change reason")?;
    if reason.is_empty() {
        return Err(AppError::bad_request("Give a reason for the access change"));
    }
    db.execute(
        "UPDATE users SET banned=?2,updated_at=?3 WHERE id=?1 AND banned<>?2",
        params![id, banned, now()],
    )?;
    // The database trigger invalidates credentials on every suspension path.
    // Also enforce it for an explicit repeated suspension request.
    if banned {
        db.execute("DELETE FROM sessions WHERE user_id=?1", [id])?;
        db.execute("UPDATE api_tokens SET revoked=1 WHERE user_id=?1", [id])?;
        crate::humangrant::revoke_member_access(db, id)?;
        db.execute("DELETE FROM auth_challenges WHERE kind='email' AND subject=(SELECT email FROM users WHERE id=?1) COLLATE NOCASE", [id])?;
    }
    db.execute(
        "INSERT INTO account_moderation(user_id,banned,reason,changed_by,changed_at,generation) VALUES(?1,?2,?3,?4,?5,(SELECT generation FROM account_access_clock WHERE id=1)) ON CONFLICT(user_id) DO UPDATE SET banned=excluded.banned,reason=excluded.reason,changed_by=excluded.changed_by,changed_at=excluded.changed_at",
        params![id,banned,reason,actor,now()],
    )?;
    audit(
        db,
        actor,
        if banned {
            "user_suspend"
        } else {
            "user_restore"
        },
        id,
        &json!({"previous_banned":previous,"banned":banned,"reason":reason}).to_string(),
    )?;
    Ok(())
}

pub fn get_user_access(s: &AppState, p: &Principal, id: &str) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    Ok(json!({"user":models::load_user(&db,id)?,"access":access_record(&db,id)?}))
}

pub fn set_user_access(
    s: &AppState,
    p: &Principal,
    id: &str,
    input: UserAccessInput,
) -> AppResult<Value> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
    let user = models::load_user(&tx, id)?;
    crate::team::ensure_account_access_change(&tx, p, id)?;
    record_access_change(&tx, &p.actor, id, user.banned, input.banned, &input.reason)?;
    let result = json!({"user":models::load_user(&tx,id)?,"access":access_record(&tx,id)?});
    tx.commit()?;
    Ok(result)
}

pub fn patch_user(s: &AppState, p: &Principal, id: &str, i: UserPatch) -> AppResult<models::User> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
    let u = models::load_user(&tx, id)?;
    let name = i.name.unwrap_or(u.name);
    safety::bounded_text(&name, 120, "Name")?;
    let resume = i.resume_md.unwrap_or(u.resume_md);
    safety::bounded_text(&resume, 80_000, "Resume")?;
    let email = i.public_email.unwrap_or(u.public_email);
    let email = if email.is_empty() {
        email
    } else {
        safety::email(&email)?
    };
    let banned = i.banned.unwrap_or(u.banned);
    if i.ban_reason.is_some() && i.banned.is_none() {
        return Err(AppError::bad_request(
            "Include banned with an access change reason",
        ));
    }
    if i.banned.is_some() {
        crate::team::ensure_account_access_change(&tx, p, id)?;
        let reason = i.ban_reason.as_deref().unwrap_or(if banned {
            "Suspended by an administrator"
        } else {
            "Access restored by an administrator"
        });
        record_access_change(&tx, &p.actor, id, u.banned, banned, reason)?;
    }
    // Accept the legacy field for existing clients, but profiles are now
    // disclosed only to the platform and an explicitly matched counterpart.
    let visible = false;
    tx.execute(
        "UPDATE users SET name=?2,public_email=?3,banned=?4,visible=?5,resume_md=?6,updated_at=?7 WHERE id=?1",
        params![id, name, email, banned, visible, resume, now()])?;
    audit(
        &tx,
        &p.actor,
        "user_update",
        id,
        if banned { "suspended" } else { "updated" },
    )?;
    let user = models::load_user(&tx, id)?;
    tx.commit()?;
    Ok(user)
}

pub fn remove_resume(s: &AppState, p: &Principal, id: &str) -> AppResult<()> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
    models::load_user(&tx, id)?;
    tx.execute(
        "UPDATE users SET resume_md='',visible=0,updated_at=?2 WHERE id=?1",
        params![id, now()],
    )?;
    audit(&tx, &p.actor, "resume_delete", id, "")?;
    tx.commit()?;
    Ok(())
}

pub fn list_jobs(s: &AppState, p: &Principal, q: &QueryInput) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_operator(&db, p)?;
    let search = format!(
        "%{}%",
        q.q.as_deref()
            .unwrap_or_default()
            .chars()
            .take(100)
            .collect::<String>()
    );
    let status = q.status.as_deref().unwrap_or_default();
    let page = q.page.unwrap_or(1).clamp(1, s.cfg.limits.list_max_page);
    let page_size = s.cfg.limits.list_page_size;
    let offset = (page - 1) * page_size;
    let condition = "(title LIKE ?1 OR jd_md LIKE ?1 OR id LIKE ?1 OR owner_id LIKE ?1) AND (?2='' OR status=?2)";
    let items = db
        .prepare(&format!(
            "SELECT * FROM jobs WHERE {condition} ORDER BY created_at DESC,id LIMIT ?3 OFFSET ?4"
        ))?
        .query_map(params![search, status, page_size, offset], models::job_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total: i64 = db.query_row(
        &format!("SELECT count(*) FROM jobs WHERE {condition}"),
        params![search, status],
        |r| r.get(0),
    )?;
    Ok(json!({"items":items,"total":total,"page":page,"page_size":page_size}))
}

pub fn get_job(s: &AppState, p: &Principal, id: &str) -> AppResult<models::Job> {
    let db = s.conn()?;
    access::ensure_operator(&db, p)?;
    models::load_job(&db, id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobPatch {
    pub jd_md: Option<String>,
    pub status: Option<String>,
    pub headcount: Option<i64>,
}

pub fn patch_job(s: &AppState, p: &Principal, id: &str, i: JobPatch) -> AppResult<models::Job> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
    let job = models::load_job(&tx, id)?;
    if !matches!(job.status.as_str(), "published" | "closed") {
        return Err(AppError::conflict("Only paid jobs can be changed"));
    }
    if let Some(headcount) = i.headcount {
        commerce::validate_configured_headcount(s, headcount)?;
    }
    let status = i.status.unwrap_or_else(|| job.status.clone());
    if !matches!(status.as_str(), "published" | "closed") {
        return Err(AppError::bad_request("Invalid job state"));
    }
    if job.status == "closed" && (status != "closed" || i.headcount.is_some_and(|n| n > 0)) {
        return Err(AppError::conflict("Closed jobs cannot resume hiring"));
    }
    let headcount = if status == "closed" {
        0
    } else {
        i.headcount.unwrap_or(job.headcount)
    };
    let jd = i.jd_md.unwrap_or_else(|| job.jd_md.clone());
    safety::bounded_text(&jd, 80_000, "Job description")?;
    if jd.trim().is_empty() {
        return Err(AppError::bad_request("Job description is required"));
    }
    let revision_changed = jd != job.jd_md || status != job.status;
    let headcount_changed = headcount != job.headcount;
    if revision_changed {
        let pending: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM orders WHERE job_id=?1 AND status='pending')",
            [id],
            |r| r.get(0),
        )?;
        if pending {
            return Err(AppError::conflict(
                "Resolve the pending payment before editing this job",
            ));
        }
    }
    // Planned headcount is free and independent of a paid JD revision. In
    // particular, a pending edit remains valid when only headcount changes.
    if revision_changed || headcount_changed {
        tx.execute(
            "UPDATE jobs SET jd_md=?2,status=?3,headcount=?4,version=version+?5,updated_at=?6 WHERE id=?1",
            params![id, jd, status, headcount, if revision_changed { 1 } else { 0 }, now()],
        )?;
    }
    if revision_changed {
        audit(&tx, &p.actor, "job_update", id, "")?;
    }
    if headcount_changed {
        audit(
            &tx,
            &p.actor,
            "job_headcount_update",
            id,
            &json!({"before":job.headcount,"after":headcount}).to_string(),
        )?;
    }
    let job = models::load_job(&tx, id)?;
    tx.commit()?;
    Ok(job)
}

pub fn remove_job(
    s: &AppState,
    p: &Principal,
    id: &str,
    expected_version: i64,
) -> AppResult<models::Job> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
    let job = commerce::delete_job_record(&tx, id, expected_version, true, &p.actor)?;
    tx.commit()?;
    Ok(job)
}

pub fn restore_job_data(
    s: &AppState,
    p: &Principal,
    id: &str,
    expected_version: i64,
) -> AppResult<models::Job> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
    let job = commerce::restore_job_record(&tx, id, expected_version, &p.actor)?;
    tx.commit()?;
    Ok(job)
}

pub fn list_orders(s: &AppState, p: &Principal, q: &QueryInput) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    let (from, to) = range(q)?;
    let status = q.status.as_deref().unwrap_or_default();
    let live = payment_mode(q)?;
    let currency = q.currency.as_deref().unwrap_or_default();
    if !currency.is_empty() && crate::config::currency_decimals(currency).is_none() {
        return Err(AppError::bad_request(
            "Choose a supported reporting currency",
        ));
    }
    let page = q.page.unwrap_or(1).clamp(1, s.cfg.limits.list_max_page);
    let page_size = s.cfg.limits.list_page_size;
    let offset = (page - 1) * page_size;
    let condition = "created_at>=?1 AND created_at<?2 AND (?3='' OR status=?3) AND (?4 IS NULL OR live_mode=?4) AND (?5='' OR currency=?5)";
    let items = db
        .prepare(&format!(
            "SELECT * FROM orders WHERE {condition} ORDER BY created_at DESC,id LIMIT ?6 OFFSET ?7"
        ))?
        .query_map(
            params![from, to, status, live, currency, page_size, offset],
            models::order_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total: i64 = db.query_row(
        &format!("SELECT count(*) FROM orders WHERE {condition}"),
        params![from, to, status, live, currency],
        |r| r.get(0),
    )?;
    Ok(json!({"items":items,"total":total,"page":page,"page_size":page_size}))
}

pub fn list_audit(s: &AppState, p: &Principal) -> AppResult<Value> {
    list_audit_page(s, p, &QueryInput::default())
}

pub fn list_audit_page(s: &AppState, p: &Principal, q: &QueryInput) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    let page = q.page.unwrap_or(1).clamp(1, s.cfg.limits.list_max_page);
    let page_size = s.cfg.limits.list_page_size;
    let total: i64 = db.query_row("SELECT count(*) FROM audit", [], |r| r.get(0))?;
    let items = db
        .prepare("SELECT * FROM audit ORDER BY id DESC LIMIT ?1 OFFSET ?2")?
        .query_map(params![page_size, (page - 1) * page_size], |r| {
            Ok(json!({
                "id":r.get::<_,i64>("id")?,"actor":r.get::<_,String>("actor")?,
                "action":r.get::<_,String>("action")?,"target":r.get::<_,String>("target")?,
                "detail":r.get::<_,String>("detail")?,"created_at":r.get::<_,i64>("created_at")?
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"items":items,"total":total,"page":page,"page_size":page_size}))
}

const MATCH_SELECT: &str = "SELECT m.*,s.name AS seeker_name,j.title AS job_title,j.owner_id AS employer_id,e.name AS employer_name
    FROM matches m JOIN users s ON s.id=m.seeker_id
    JOIN jobs j ON j.id=m.job_id JOIN users e ON e.id=j.owner_id";

fn match_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let id: String = r.get("id")?;
    let seeker_id: String = r.get("seeker_id")?;
    let employer_id: String = r.get("employer_id")?;
    Ok(json!({
        "id":id,"seeker_id":seeker_id,"seeker_name":r.get::<_,String>("seeker_name")?,
        "job_id":r.get::<_,String>("job_id")?,"job_title":r.get::<_,String>("job_title")?,
        "employer_id":employer_id,"employer_name":r.get::<_,String>("employer_name")?,
        "status":r.get::<_,String>("status")?,"note":r.get::<_,String>("note")?,
        "created_at":r.get::<_,i64>("created_at")?,"updated_at":r.get::<_,i64>("updated_at")?,
        "conversation_id":format!("match_{id}"),
        "seeker_conversation_id":format!("platform_{seeker_id}"),
        "employer_conversation_id":format!("platform_{employer_id}")
    }))
}

pub fn list_matches(s: &AppState, p: &Principal) -> AppResult<Value> {
    list_matches_page(s, p, &QueryInput::default())
}

pub fn list_matches_page(s: &AppState, p: &Principal, q: &QueryInput) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_operator(&db, p)?;
    let page = q.page.unwrap_or(1).clamp(1, s.cfg.limits.list_max_page);
    let page_size = s.cfg.limits.list_page_size;
    let total: i64 = db.query_row("SELECT count(*) FROM matches", [], |r| r.get(0))?;
    let items = db
        .prepare(&format!(
            "{MATCH_SELECT} ORDER BY m.updated_at DESC,m.id LIMIT ?1 OFFSET ?2"
        ))?
        .query_map(params![page_size, (page - 1) * page_size], match_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"items":items,"total":total,"page":page,"page_size":page_size}))
}

/// Targeted administrative detail never falls back to loading the full list.
/// Archived records remain auditable; reading them does not grant member access.
pub fn get_match(s: &AppState, p: &Principal, id: &str) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_operator(&db, p)?;
    safety::bounded_text(id, 80, "Match ID")?;
    db.query_row(&format!("{MATCH_SELECT} WHERE m.id=?1"), [id], match_row)
        .optional()?
        .ok_or_else(|| AppError::not_found("Match not found"))
}

pub fn candidate_detail(
    s: &AppState,
    p: &Principal,
    id: &str,
    job_id: Option<&str>,
) -> AppResult<Value> {
    access::ensure_operator(&*s.conn()?, p)?;
    safety::bounded_text(id, 80, "Candidate ID")?;
    if let Some(job) = job_id {
        safety::bounded_text(job, 80, "Job ID")?;
    }
    // Reuse the consent serializer, without making its fields editable here.
    let mandate = crate::chat::get_mandate(s, p, Some(id))?["mandate"].clone();
    let db = s.conn()?;
    // A token/session could be revoked between the separate read operations.
    access::ensure_operator(&db, p)?;
    let user = models::load_user(&db, id)?;
    if user.role != "seeker" {
        return Err(AppError::not_found("Candidate not found"));
    }
    let matched = if let Some(job) = job_id {
        models::load_job(&db, job)?;
        db.query_row(
            &format!("{MATCH_SELECT} WHERE m.seeker_id=?1 AND m.job_id=?2"),
            params![id, job],
            match_row,
        )
        .optional()?
    } else {
        None
    };
    Ok(json!({
        "resume_html":safety::markdown(&user.resume_md),
        "user":user,"mandate":mandate,"match":matched
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchInput {
    pub seeker_id: String,
    pub job_id: String,
    #[serde(default)]
    pub note: String,
}

/// A private candidate can be introduced only after their explicit mandate.
/// A deterministic conversation id does not itself grant any chat access.
fn match_eligible(db: &Connection, seeker_id: &str, job_id: &str) -> AppResult<String> {
    let seeker = models::load_user(db, seeker_id)?;
    let job = models::load_job(db, job_id)?;
    let owner = models::load_user(db, &job.owner_id)?;
    let consent: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM mandates WHERE user_id=?1 AND status='active')",
        [seeker_id],
        |r| r.get(0),
    )?;
    let paid: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM orders WHERE job_id=?1 AND kind='publish' AND status IN ('paid','partially_refunded'))",
        [job_id],
        |r| r.get(0),
    )?;
    if seeker.role != "seeker"
        || seeker.banned
        || !consent
        || job.status != "published"
        || job.headcount == 0
        || !paid
        || owner.role != "employer"
        || owner.banned
        || !owner.employer_paid
    {
        return Err(AppError::bad_request(
            "Match requires an active seeker mandate and a paid, published job with positive hiring headcount and active accounts",
        ));
    }
    Ok(owner.id)
}

pub fn add_match(s: &AppState, p: &Principal, i: MatchInput) -> AppResult<Value> {
    safety::bounded_text(&i.note, 2000, "Match note")?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_operator(&tx, p)?;
    let employer_id = match_eligible(&tx, &i.seeker_id, &i.job_id)?;
    let id = uuid::Uuid::new_v4().to_string();
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO matches(id,seeker_id,job_id,note,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5)",
        params![id, i.seeker_id, i.job_id, i.note, now()])?;
    if inserted != 1 {
        return Err(AppError::conflict("This match already exists"));
    }
    audit(&tx, &p.actor, "match_create", &id, "")?;
    tx.commit()?;
    Ok(json!({
        "id":id,
        "conversation_id":format!("match_{id}"),
        "seeker_conversation_id":format!("platform_{}",i.seeker_id),
        "employer_conversation_id":format!("platform_{employer_id}")
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkMatchInput {
    pub job_id: String,
    pub seeker_ids: Vec<String>,
    #[serde(default)]
    pub note: String,
}

/// Introduce a bounded selection atomically. Retrying an accepted selection
/// reuses active pairs without changing their notes or progress. Reopening a
/// declined match is a separate, explicit action in the follow-up detail.
pub fn add_matches(s: &AppState, p: &Principal, i: BulkMatchInput) -> AppResult<Value> {
    safety::bounded_text(&i.note, 2000, "Match note")?;
    safety::bounded_text(&i.job_id, 80, "Job ID")?;
    if i.job_id.is_empty() || !(1..=100).contains(&i.seeker_ids.len()) {
        return Err(AppError::bad_request(
            "Choose a job and between 1 and 100 candidates",
        ));
    }
    let mut selected = std::collections::HashSet::new();
    for id in &i.seeker_ids {
        safety::bounded_text(id, 80, "Candidate ID")?;
        if id.is_empty() || !selected.insert(id) {
            return Err(AppError::bad_request(
                "Candidate IDs must be non-empty and unique",
            ));
        }
    }
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_operator(&tx, p)?;
    let mut created = 0;
    let mut matches = Vec::with_capacity(i.seeker_ids.len());
    for seeker_id in &i.seeker_ids {
        let employer_id = match_eligible(&tx, seeker_id, &i.job_id)?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT id,status FROM matches WHERE seeker_id=?1 AND job_id=?2",
                params![seeker_id, i.job_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (id, status, inserted) = if let Some((id, status)) = existing {
            if !matches!(status.as_str(), "introduced" | "interview" | "hired") {
                return Err(AppError::conflict(
                    "A selected candidate has a closed introduction; reopen it from its detail page",
                ));
            }
            (id, status, false)
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO matches(id,seeker_id,job_id,note,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5)",
                params![id, seeker_id, i.job_id, i.note, now()],
            )?;
            audit(&tx, &p.actor, "match_create", &id, "bulk introduction")?;
            created += 1;
            (id, "introduced".to_owned(), true)
        };
        matches.push(json!({
            "id":id,"seeker_id":seeker_id,"job_id":i.job_id,
            "status":status,"created":inserted,
            "conversation_id":format!("match_{id}"),
            "seeker_conversation_id":format!("platform_{seeker_id}"),
            "employer_conversation_id":format!("platform_{employer_id}")
        }));
    }
    tx.commit()?;
    Ok(json!({"created":created,"reused":matches.len()-created,"matches":matches}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchPatch {
    pub status: String,
    #[serde(default)]
    pub note: String,
}

pub fn patch_match(s: &AppState, p: &Principal, id: &str, i: MatchPatch) -> AppResult<()> {
    if !matches!(
        i.status.as_str(),
        "introduced" | "interview" | "hired" | "declined"
    ) {
        return Err(AppError::bad_request("Invalid match status"));
    }
    safety::bounded_text(&i.note, 2000, "Match note")?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_operator(&tx, p)?;
    let existing: Option<(String, String, String)> = tx
        .query_row(
            "SELECT status,seeker_id,job_id FROM matches WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (old_status, seeker_id, job_id) =
        existing.ok_or_else(|| AppError::not_found("Match not found"))?;
    if old_status == "declined" && i.status != "declined" {
        match_eligible(&tx, &seeker_id, &job_id)?;
    }
    tx.execute(
        "UPDATE matches SET status=?2,note=?3,updated_at=?4 WHERE id=?1",
        params![id, i.status, i.note, now()],
    )?;
    audit(&tx, &p.actor, "match_update", id, &i.status)?;
    tx.commit()?;
    Ok(())
}

async fn overview(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<QueryInput>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(overview_data(&s, &p, &q)?))
}
async fn users(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<QueryInput>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(list_users(&s, &p, &q)?))
}
async fn user_detail(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<models::User>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(get_user(&s, &p, &id)?))
}

async fn user_access(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(get_user_access(&s, &p, &id)?))
}

async fn change_user_access(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<UserAccessInput>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, true)?;
    Ok(Json(set_user_access(&s, &p, &id, input)?))
}
async fn update_user(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(i): Json<UserPatch>,
) -> AppResult<Json<models::User>> {
    let p = guard(&s, &h, true)?;
    Ok(Json(patch_user(&s, &p, &id, i)?))
}
async fn delete_resume(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let p = guard(&s, &h, true)?;
    remove_resume(&s, &p, &id)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn jobs(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<QueryInput>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(list_jobs(&s, &p, &q)?))
}
async fn job_detail(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<models::Job>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(get_job(&s, &p, &id)?))
}
async fn update_job(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(i): Json<JobPatch>,
) -> AppResult<Json<models::Job>> {
    let p = guard(&s, &h, true)?;
    Ok(Json(patch_job(&s, &p, &id, i)?))
}
async fn delete_job(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(i): Json<JobVersion>,
) -> AppResult<Json<models::Job>> {
    let p = guard(&s, &h, true)?;
    Ok(Json(remove_job(&s, &p, &id, i.expected_version)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobVersion {
    pub expected_version: i64,
}
async fn restore_job(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(i): Json<JobVersion>,
) -> AppResult<Json<models::Job>> {
    let p = guard(&s, &h, true)?;
    Ok(Json(restore_job_data(&s, &p, &id, i.expected_version)?))
}
async fn orders(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<QueryInput>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(list_orders(&s, &p, &q)?))
}
async fn audits(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<QueryInput>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(list_audit_page(&s, &p, &q)?))
}
async fn matches(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<QueryInput>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(list_matches_page(&s, &p, &q)?))
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct CandidateDetailQuery {
    job_id: Option<String>,
}
async fn workbench_candidate(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<CandidateDetailQuery>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(candidate_detail(&s, &p, &id, q.job_id.as_deref())?))
}
async fn workbench_match(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(get_match(&s, &p, &id)?))
}
async fn create_match(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(i): Json<MatchInput>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, true)?;
    Ok(Json(add_match(&s, &p, i)?))
}
async fn create_matches(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(i): Json<BulkMatchInput>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, true)?;
    Ok(Json(add_matches(&s, &p, i)?))
}
async fn update_match(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(i): Json<MatchPatch>,
) -> AppResult<StatusCode> {
    let p = guard(&s, &h, true)?;
    patch_match(&s, &p, &id, i)?;
    Ok(StatusCode::NO_CONTENT)
}

fn waffo_settings_data(s: &AppState, db: &Connection) -> AppResult<Value> {
    let w = &s.cfg.payments.waffo;
    Ok(
        json!({"enabled":w.enabled,"mode":crate::payments::waffo_mode(s, db)?,
        "test_ready":w.enabled && w.for_mode("test").is_ok(),
        "production_ready":w.enabled && w.for_mode("prod").is_ok()}),
    )
}

fn pricing_settings_data(s: &AppState, db: &Connection) -> AppResult<Value> {
    let mode = commerce::billing_mode(s, db)?;
    Ok(json!({
        "mode": mode,
        "donation": mode == "donation",
        "configured_mode": s.cfg.pricing.billing_mode.clone(),
    }))
}
async fn pricing_settings(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    let db = s.conn()?;
    access::ensure_admin(&db, &p)?;
    Ok(Json(pricing_settings_data(&s, &db)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PricingModeInput {
    mode: String,
    expected_mode: String,
}
async fn change_pricing_mode(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<PricingModeInput>,
) -> AppResult<Json<Value>> {
    if !matches!(input.mode.as_str(), "fees" | "donation") {
        return Err(AppError::bad_request("Choose fees or donation mode"));
    }
    let p = access::principal(&s, &h, true)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, &p)?;
    let previous = commerce::billing_mode(&s, &tx)?;
    if previous != input.expected_mode {
        return Err(AppError::conflict(
            "Billing mode changed; refresh and retry",
        ));
    }
    tx.execute(
        "INSERT INTO pricing_settings(id,mode) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET mode=excluded.mode",
        [&input.mode],
    )?;
    audit(
        &tx,
        &p.actor,
        "pricing_mode_change",
        "pricing",
        &json!({"from":previous,"to":input.mode}).to_string(),
    )?;
    let result = pricing_settings_data(&s, &tx)?;
    tx.commit()?;
    Ok(Json(result))
}
async fn waffo_settings(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    let db = s.conn()?;
    access::ensure_admin(&db, &p)?;
    Ok(Json(waffo_settings_data(&s, &db)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaffoModeInput {
    mode: String,
    expected_mode: String,
}
async fn change_waffo_mode(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<WaffoModeInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, &p)?;
    if !s.cfg.payments.waffo.enabled {
        return Err(AppError::unavailable("Waffo is disabled"));
    }
    s.cfg.payments.waffo.for_mode(&input.mode)?;
    let previous = crate::payments::waffo_mode(&s, &tx)?;
    if previous != input.expected_mode {
        return Err(AppError::conflict(
            "Payment mode changed; refresh and retry",
        ));
    }
    tx.execute("INSERT INTO payment_settings(provider,mode) VALUES('waffo',?1) ON CONFLICT(provider) DO UPDATE SET mode=excluded.mode", [&input.mode])?;
    audit(
        &tx,
        &p.actor,
        "payment_mode_change",
        "waffo",
        &json!({"from":previous,"to":input.mode}).to_string(),
    )?;
    let result = waffo_settings_data(&s, &tx)?;
    tx.commit()?;
    Ok(Json(result))
}
