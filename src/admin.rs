use crate::{
    AppError, AppResult, AppState,
    access::{self, Principal},
    db::audit,
    models, now, safety,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/overview", get(overview))
        .route("/api/admin/users", get(users))
        .route("/api/admin/users/{id}", get(user_detail).patch(update_user))
        .route(
            "/api/admin/users/{id}/resume",
            axum::routing::delete(delete_resume),
        )
        .route("/api/admin/jobs", get(jobs))
        .route(
            "/api/admin/jobs/{id}",
            get(job_detail).patch(update_job).delete(delete_job),
        )
        .route("/api/admin/orders", get(orders))
        .route("/api/admin/audit", get(audits))
        .route("/api/admin/matches", get(matches).post(create_match))
        .route("/api/admin/matches/{id}", patch(update_match))
}

fn guard(s: &AppState, h: &HeaderMap, write: bool) -> AppResult<Principal> {
    let p = access::principal(s, h, write)?;
    access::ensure_admin(&*s.conn()?, &p)?;
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

/// Shared business functions recheck the principal while holding the database
/// lock (and the write transaction for mutations). HTTP and MCP use these alike.
pub fn overview_data(s: &AppState, p: &Principal, q: &QueryInput) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    let (from, to) = range(q)?;
    let live = match q.mode.as_deref() {
        Some("live") => true,
        Some("test") => false,
        None => !s.cfg.development,
        Some(_) => {
            return Err(AppError::bad_request(
                "Choose live or test payment statistics",
            ));
        }
    };
    let count = |sql: &str| -> AppResult<i64> { Ok(db.query_row(sql, [], |r| r.get(0))?) };
    let (revenue, paid_orders): (i64, i64) = db.query_row(
        "SELECT COALESCE(sum(amount_cents),0),count(*) FROM ledger WHERE created_at>=?1 AND created_at<?2 AND live_mode=?3",
        params![from, to, live], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let first: i64 = db.query_row(
        "SELECT COALESCE(min(created_at),?1) FROM ledger",
        [now()],
        |r| r.get(0),
    )?;
    let monthly = to - from.max(first) > 366 * 86400;
    let bucket = if monthly { "%Y-%m" } else { "%Y-%m-%d" };
    let series = db.prepare(
        "SELECT strftime(?4,created_at,'unixepoch') AS day,sum(amount_cents),count(*) FROM ledger WHERE created_at>=?1 AND created_at<?2 AND live_mode=?3 GROUP BY day ORDER BY day")?
        .query_map(params![from, to, live, bucket], |r| Ok(json!({
            "date":r.get::<_,String>(0)?,
            "revenue_cents":r.get::<_,i64>(1)?,
            "orders":r.get::<_,i64>(2)?
        })))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let by_kind = db.prepare(
        "SELECT o.kind,sum(l.amount_cents),count(*) FROM ledger l JOIN orders o ON o.id=l.order_id WHERE l.created_at>=?1 AND l.created_at<?2 AND l.live_mode=?3 GROUP BY o.kind")?
        .query_map(params![from, to, live], |r| Ok(json!({
            "kind":r.get::<_,String>(0)?,
            "amount_cents":r.get::<_,i64>(1)?,
            "count":r.get::<_,i64>(2)?
        })))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let recent = db
        .prepare("SELECT * FROM orders ORDER BY created_at DESC,id LIMIT 8")?
        .query_map([], models::order_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({
        "stats":{
            "users":count("SELECT count(*) FROM users")?,
            "seekers":count("SELECT count(*) FROM users WHERE role='seeker'")?,
            "employers":count("SELECT count(*) FROM users WHERE role='employer' AND employer_paid=1")?,
            "active_jobs":count("SELECT count(*) FROM jobs j JOIN users u ON u.id=j.owner_id WHERE j.status='published' AND u.banned=0")?,
            "paid_orders":paid_orders,
            "revenue_cents":revenue,
            "pending_orders":count("SELECT count(*) FROM orders WHERE status='pending'")?,
            "review_orders":count("SELECT count(*) FROM orders WHERE status='paid_needs_review'")?
        },
        "series":series,"revenue_by_kind":by_kind,"recent_orders":recent,
        "bucket":if monthly{"month"}else{"day"},
        "timezone":"UTC","development":s.cfg.development,
        "payment_mode":if live{"live"}else{"test"}
    }))
}

pub fn list_users(s: &AppState, p: &Principal, q: &QueryInput) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    let search = format!(
        "%{}%",
        q.q.as_deref()
            .unwrap_or_default()
            .chars()
            .take(100)
            .collect::<String>()
    );
    let role = q.role.as_deref().unwrap_or_default();
    let offset = (q.page.unwrap_or(1).clamp(1, 2000) - 1) * 50;
    let condition = "(name LIKE ?1 OR email LIKE ?1 OR id LIKE ?1) AND (?2='' OR role=?2)";
    let items = db
        .prepare(&format!(
            "SELECT * FROM users WHERE {condition} ORDER BY created_at DESC,id LIMIT 50 OFFSET ?3"
        ))?
        .query_map(params![search, role, offset], models::user_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total: i64 = db.query_row(
        &format!("SELECT count(*) FROM users WHERE {condition}"),
        params![search, role],
        |r| r.get(0),
    )?;
    Ok(json!({"items":items,"total":total}))
}

pub fn get_user(s: &AppState, p: &Principal, id: &str) -> AppResult<models::User> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    models::load_user(&db, id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserPatch {
    pub name: Option<String>,
    pub public_email: Option<String>,
    pub banned: Option<bool>,
    pub visible: Option<bool>,
    pub resume_md: Option<String>,
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
    // Accept the legacy field for existing clients, but profiles are now
    // disclosed only to the platform and an explicitly matched counterpart.
    let visible = false;
    tx.execute(
        "UPDATE users SET name=?2,public_email=?3,banned=?4,visible=?5,resume_md=?6,updated_at=?7 WHERE id=?1",
        params![id, name, email, banned, visible, resume, now()])?;
    if banned {
        tx.execute("DELETE FROM sessions WHERE user_id=?1", [id])?;
        tx.execute("UPDATE api_tokens SET revoked=1 WHERE user_id=?1", [id])?;
    }
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
    access::ensure_admin(&db, p)?;
    let search = format!(
        "%{}%",
        q.q.as_deref()
            .unwrap_or_default()
            .chars()
            .take(100)
            .collect::<String>()
    );
    let status = q.status.as_deref().unwrap_or_default();
    let offset = (q.page.unwrap_or(1).clamp(1, 2000) - 1) * 50;
    let condition = "(title LIKE ?1 OR jd_md LIKE ?1 OR id LIKE ?1 OR owner_id LIKE ?1) AND (?2='' OR status=?2)";
    let items = db
        .prepare(&format!(
            "SELECT * FROM jobs WHERE {condition} ORDER BY created_at DESC,id LIMIT 50 OFFSET ?3"
        ))?
        .query_map(params![search, status, offset], models::job_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total: i64 = db.query_row(
        &format!("SELECT count(*) FROM jobs WHERE {condition}"),
        params![search, status],
        |r| r.get(0),
    )?;
    Ok(json!({"items":items,"total":total}))
}

pub fn get_job(s: &AppState, p: &Principal, id: &str) -> AppResult<models::Job> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    models::load_job(&db, id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobPatch {
    pub jd_md: Option<String>,
    pub status: Option<String>,
}

pub fn patch_job(s: &AppState, p: &Principal, id: &str, i: JobPatch) -> AppResult<models::Job> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
    let job = models::load_job(&tx, id)?;
    if !matches!(job.status.as_str(), "published" | "closed") {
        return Err(AppError::conflict("Only paid jobs can be changed"));
    }
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
    let status = i.status.unwrap_or(job.status);
    if !matches!(status.as_str(), "published" | "closed") {
        return Err(AppError::bad_request("Invalid job state"));
    }
    let jd = i.jd_md.unwrap_or(job.jd_md);
    safety::bounded_text(&jd, 80_000, "Job description")?;
    if jd.trim().is_empty() {
        return Err(AppError::bad_request("Job description is required"));
    }
    tx.execute(
        "UPDATE jobs SET jd_md=?2,status=?3,version=version+1,updated_at=?4 WHERE id=?1",
        params![id, jd, status, now()],
    )?;
    audit(&tx, &p.actor, "job_update", id, "")?;
    let job = models::load_job(&tx, id)?;
    tx.commit()?;
    Ok(job)
}

pub fn remove_job(s: &AppState, p: &Principal, id: &str) -> AppResult<()> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
    models::load_job(&tx, id)?;
    tx.execute(
        "UPDATE jobs SET status='deleted',jd_md='',version=version+1,updated_at=?2 WHERE id=?1",
        params![id, now()],
    )?;
    audit(&tx, &p.actor, "job_delete", id, "")?;
    tx.commit()?;
    Ok(())
}

pub fn list_orders(s: &AppState, p: &Principal, q: &QueryInput) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    let (from, to) = range(q)?;
    let status = q.status.as_deref().unwrap_or_default();
    let offset = (q.page.unwrap_or(1).clamp(1, 2000) - 1) * 50;
    let condition = "created_at>=?1 AND created_at<?2 AND (?3='' OR status=?3)";
    let items = db
        .prepare(&format!(
            "SELECT * FROM orders WHERE {condition} ORDER BY created_at DESC,id LIMIT 50 OFFSET ?4"
        ))?
        .query_map(params![from, to, status, offset], models::order_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total: i64 = db.query_row(
        &format!("SELECT count(*) FROM orders WHERE {condition}"),
        params![from, to, status],
        |r| r.get(0),
    )?;
    Ok(json!({"items":items,"total":total}))
}

pub fn list_audit(s: &AppState, p: &Principal) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    let items = db
        .prepare("SELECT * FROM audit ORDER BY id DESC LIMIT 200")?
        .query_map([], |r| {
            Ok(json!({
                "id":r.get::<_,i64>("id")?,"actor":r.get::<_,String>("actor")?,
                "action":r.get::<_,String>("action")?,"target":r.get::<_,String>("target")?,
                "detail":r.get::<_,String>("detail")?,"created_at":r.get::<_,i64>("created_at")?
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"items":items}))
}

pub fn list_matches(s: &AppState, p: &Principal) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    let items = db.prepare(
        "SELECT m.*,s.name AS seeker_name,j.title AS job_title,j.owner_id AS employer_id,e.name AS employer_name
         FROM matches m JOIN users s ON s.id=m.seeker_id
         JOIN jobs j ON j.id=m.job_id JOIN users e ON e.id=j.owner_id
         ORDER BY m.updated_at DESC,m.id LIMIT 200")?
        .query_map([], |r| {
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
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"items":items}))
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
        "SELECT EXISTS(SELECT 1 FROM orders WHERE job_id=?1 AND kind='publish' AND status='paid')",
        [job_id],
        |r| r.get(0),
    )?;
    if seeker.role != "seeker"
        || seeker.banned
        || !consent
        || job.status != "published"
        || !paid
        || owner.role != "employer"
        || owner.banned
        || !owner.employer_paid
    {
        return Err(AppError::bad_request(
            "Match requires an active seeker mandate and a paid, published job with active accounts",
        ));
    }
    Ok(owner.id)
}

pub fn add_match(s: &AppState, p: &Principal, i: MatchInput) -> AppResult<Value> {
    safety::bounded_text(&i.note, 2000, "Match note")?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
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
    access::ensure_admin(&tx, p)?;
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
) -> AppResult<StatusCode> {
    let p = guard(&s, &h, true)?;
    remove_job(&s, &p, &id)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn orders(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<QueryInput>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(list_orders(&s, &p, &q)?))
}
async fn audits(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(list_audit(&s, &p)?))
}
async fn matches(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, false)?;
    Ok(Json(list_matches(&s, &p)?))
}
async fn create_match(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(i): Json<MatchInput>,
) -> AppResult<Json<Value>> {
    let p = guard(&s, &h, true)?;
    Ok(Json(add_match(&s, &p, i)?))
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
