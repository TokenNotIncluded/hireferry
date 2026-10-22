//! Bounded, administrator-only matching workbench queries.
//!
//! FTS5 trigram indexes provide literal substring searches, including Chinese.
//! A one/two-character term uses escaped LIKE after the other SQL filters; it
//! is intentionally not presented as an indexed full-text query. Pagination is
//! a date/id keyset, never an OFFSET or an in-memory filter. Jobs and candidates
//! use their creation date; follow-ups use the latest update date.
use crate::{
    AppError, AppResult, AppState,
    access::{self, Principal},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, params_from_iter, types::Value as SqlValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const JOB_FROM: &str = "FROM jobs j JOIN users e ON e.id=j.owner_id";
const JOB_FIELDS: &str = "j.id,j.owner_id,e.name AS employer_name,j.title,j.salary_cents,j.currency,j.headcount,j.status,substr(j.jd_md,1,240) AS jd_excerpt,j.created_at,j.updated_at,(SELECT count(*) FROM matches mt WHERE mt.job_id=j.id AND mt.status IN ('introduced','interview','hired')) AS active_matches";
const JOB_ELIGIBLE: &str = "j.status='published' AND j.headcount>0 AND e.role='employer' AND e.banned=0 AND e.employer_paid=1 AND EXISTS(SELECT 1 FROM orders o WHERE o.job_id=j.id AND o.kind='publish' AND o.status IN ('paid','partially_refunded'))";
const CANDIDATE_FROM: &str = "FROM mandates m JOIN users u ON u.id=m.user_id";
// The planner otherwise changes to a users scan plus a temporary sort at large
// cardinalities. CROSS JOIN fixes the driving table, INDEXED BY fixes the seek;
// neither constraint is applied to a selective FTS result set.
const CANDIDATE_ORDERED_FROM: &str = "FROM mandates m INDEXED BY wb_mandates_page CROSS JOIN users u INDEXED BY wb_active_seekers ON u.id=m.user_id";
const CANDIDATE_ELIGIBLE: &str = "m.status='active' AND u.role='seeker' AND u.banned=0";
const MATCH_FROM: &str = "FROM matches m JOIN users s ON s.id=m.seeker_id JOIN jobs j ON j.id=m.job_id JOIN users e ON e.id=j.owner_id";

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JobQuery {
    pub q: Option<String>,
    pub employer_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CandidateQuery {
    pub q: Option<String>,
    pub job_id: Option<String>,
    pub industry: Option<String>,
    pub roles: Option<String>,
    pub location: Option<String>,
    pub salary_min_cents: Option<i64>,
    pub salary_max_cents: Option<i64>,
    pub salary_currency: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MatchQuery {
    pub q: Option<String>,
    pub status: Option<String>,
    pub job_id: Option<String>,
    pub seeker_id: Option<String>,
    pub employer_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

/// These indexes contain no independently authoritative eligibility state.
/// Every result joins current users, jobs, mandates and payment grants.
pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch("SAVEPOINT workbench_v5;")?;
    let result = (|| -> AppResult<()> {
        db.execute_batch(r#"
CREATE INDEX IF NOT EXISTS wb_jobs_page ON jobs(created_at DESC,id DESC,owner_id) WHERE status='published' AND headcount>0;
CREATE INDEX IF NOT EXISTS wb_jobs_employer_page ON jobs(owner_id,created_at DESC,id DESC) WHERE status='published' AND headcount>0;
CREATE INDEX IF NOT EXISTS wb_paid_publications ON orders(job_id) WHERE kind='publish' AND status IN ('paid','partially_refunded');
CREATE INDEX IF NOT EXISTS wb_mandates_page ON mandates(created_at DESC,user_id DESC) WHERE status='active';
CREATE INDEX IF NOT EXISTS wb_mandates_salary_min ON mandates(salary_min_cents,user_id) WHERE status='active';
CREATE INDEX IF NOT EXISTS wb_mandates_salary_max ON mandates(salary_max_cents,user_id) WHERE status='active';
CREATE INDEX IF NOT EXISTS wb_active_seekers ON users(id,role,banned) WHERE role='seeker' AND banned=0;
CREATE INDEX IF NOT EXISTS wb_matches_page ON matches(updated_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS wb_matches_stage_page ON matches(status,updated_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS wb_matches_job_page ON matches(job_id,updated_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS wb_matches_seeker_page ON matches(seeker_id,updated_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS wb_matches_job_active ON matches(job_id,status,seeker_id);
"#)?;
        for (base, index, columns) in [
            ("users", "wb_users_fts", &["name", "resume_md"][..]),
            ("jobs", "wb_jobs_fts", &["title", "jd_md"][..]),
            (
                "mandates",
                "wb_mandates_fts",
                &["target_industry", "target_roles", "location", "preferences"][..],
            ),
            ("matches", "wb_matches_fts", &["note"][..]),
        ] {
            let existed: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [index],
                |r| r.get(0),
            )?;
            let fields = columns.join(",");
            let old = columns
                .iter()
                .map(|c| format!("old.{c}"))
                .collect::<Vec<_>>()
                .join(",");
            let new = columns
                .iter()
                .map(|c| format!("new.{c}"))
                .collect::<Vec<_>>()
                .join(",");
            // All identifiers come from the fixed table above, never a request.
            db.execute_batch(&format!(r#"
CREATE VIRTUAL TABLE IF NOT EXISTS {index} USING fts5({fields},content='{base}',content_rowid='rowid',tokenize='trigram');
CREATE TRIGGER IF NOT EXISTS {index}_insert AFTER INSERT ON {base} BEGIN
 INSERT INTO {index}(rowid,{fields}) VALUES(new.rowid,{new});
END;
CREATE TRIGGER IF NOT EXISTS {index}_delete AFTER DELETE ON {base} BEGIN
 INSERT INTO {index}({index},rowid,{fields}) VALUES('delete',old.rowid,{old});
END;
CREATE TRIGGER IF NOT EXISTS {index}_update AFTER UPDATE OF {fields} ON {base} BEGIN
 INSERT INTO {index}({index},rowid,{fields}) VALUES('delete',old.rowid,{old});
 INSERT INTO {index}(rowid,{fields}) VALUES(new.rowid,{new});
END;
"#))?;
            if !existed {
                db.execute(
                    &format!("INSERT INTO {index}({index}) VALUES('rebuild')"),
                    [],
                )?;
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            db.execute_batch("RELEASE workbench_v5;")?;
            Ok(())
        }
        Err(error) => {
            let _ = db.execute_batch("ROLLBACK TO workbench_v5; RELEASE workbench_v5;");
            Err(error)
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/workbench/jobs", get(http_jobs))
        .route("/api/admin/workbench/jobs/{id}", get(http_job))
        .route("/api/admin/workbench/candidates", get(http_candidates))
        .route("/api/admin/workbench/matches", get(http_matches))
}
async fn http_jobs(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<JobQuery>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(search_jobs(&s, &p, &q)?))
}
async fn http_job(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(get_job(&s, &p, &id)?))
}
async fn http_candidates(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<CandidateQuery>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(search_candidates(&s, &p, &q)?))
}
async fn http_matches(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<MatchQuery>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(search_matches(&s, &p, &q)?))
}

#[derive(Default)]
struct Filter {
    conditions: Vec<String>,
    params: Vec<SqlValue>,
    indexed_search: bool,
}
impl Filter {
    fn bind(&mut self, value: impl Into<SqlValue>) -> String {
        self.params.push(value.into());
        format!("?{}", self.params.len())
    }
    fn add(&mut self, condition: impl Into<String>) {
        self.conditions.push(condition.into());
    }
    fn full_text(&mut self, condition: impl Into<String>) {
        self.indexed_search = true;
        self.add(condition);
    }
    fn clause(&self) -> String {
        if self.conditions.is_empty() {
            "1=1".into()
        } else {
            self.conditions.join(" AND ")
        }
    }
    fn exact(&mut self, column: &str, value: &str) {
        if !value.is_empty() {
            let p = self.bind(value.to_owned());
            self.add(format!("{column}={p}"));
        }
    }
    fn short_term(&mut self, columns: &[&str], term: &str) {
        let p = self.bind(like_pattern(term));
        self.add(format!(
            "({})",
            columns
                .iter()
                .map(|c| format!("{c} LIKE {p} ESCAPE '\\'"))
                .collect::<Vec<_>>()
                .join(" OR ")
        ));
    }
    fn keyset(&mut self, time_column: &str, id_column: &str, cursor: Option<&Cursor>) {
        if let Some(c) = cursor {
            let t = self.bind(c.sort_at);
            let id = self.bind(c.id.clone());
            self.add(format!("({time_column},{id_column})<({t},{id})"));
        }
    }
}

fn clean(value: Option<&str>, max: usize) -> AppResult<String> {
    let value = value.unwrap_or_default().trim();
    if value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(AppError::bad_request(format!(
            "Search text must be at most {max} characters without control characters"
        )));
    }
    Ok(value.to_owned())
}
fn terms(value: &str) -> AppResult<Vec<&str>> {
    let terms = value.split_whitespace().collect::<Vec<_>>();
    if terms.len() > 8 {
        return Err(AppError::bad_request("Use at most eight search terms"));
    }
    Ok(terms)
}
fn phrase(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn like_pattern(value: &str) -> String {
    format!(
        "%{}%",
        value
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}
fn limit(value: Option<i64>, default: i64) -> AppResult<i64> {
    let value = value.unwrap_or(default);
    if !(1..=100).contains(&value) {
        return Err(AppError::bad_request("Page size must be between 1 and 100"));
    }
    Ok(value)
}
fn salary_range(min: Option<i64>, max: Option<i64>) -> AppResult<()> {
    if min
        .into_iter()
        .chain(max)
        .any(|v| !(0..=100_000_000).contains(&v))
        || matches!((min,max),(Some(a),Some(b)) if a>b)
    {
        return Err(AppError::bad_request(
            "Salary range must be ordered and between 0 and 100000000 minor currency units",
        ));
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    fingerprint: String,
    sort_at: i64,
    id: String,
}
fn fingerprint(scope: &str, filters: Value) -> String {
    hex::encode(Sha256::digest(format!("{scope}:{}", filters)))
}
fn decode_cursor(raw: Option<&str>, fingerprint: &str) -> AppResult<Option<Cursor>> {
    let Some(raw) = raw.filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let invalid = || {
        AppError::bad_request(
            "Invalid cursor or changed search filters; restart from the first page",
        )
    };
    if raw.len() > 2048 {
        return Err(invalid());
    }
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
    let c: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if c.version != 1
        || c.fingerprint != fingerprint
        || c.sort_at < 0
        || c.id.is_empty()
        || c.id.len() > 80
    {
        return Err(invalid());
    }
    Ok(Some(c))
}
fn page(
    mut rows: Vec<Value>,
    total: i64,
    limit: i64,
    fingerprint: String,
    sort_field: &str,
) -> AppResult<Value> {
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = if more {
        let row = rows
            .last()
            .ok_or_else(|| AppError::internal("Missing pagination row"))?;
        Some(
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Cursor {
                version: 1,
                fingerprint,
                sort_at: row[sort_field]
                    .as_i64()
                    .ok_or_else(|| AppError::internal("Missing pagination date"))?,
                id: row["id"]
                    .as_str()
                    .ok_or_else(|| AppError::internal("Missing pagination id"))?
                    .to_owned(),
            })?),
        )
    } else {
        None
    };
    Ok(json!({"items":rows,"total":total,"limit":limit,"next_cursor":next_cursor}))
}
fn count(db: &Connection, from: &str, f: &Filter) -> AppResult<i64> {
    Ok(db.query_row(
        &format!("SELECT count(*) {from} WHERE {}", f.clause()),
        params_from_iter(f.params.iter()),
        |r| r.get(0),
    )?)
}

pub fn search_jobs(s: &AppState, p: &Principal, q: &JobQuery) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_operator(&db, p)?;
    let query = clean(q.q.as_deref(), 120)?;
    let employer = clean(q.employer_id.as_deref(), 80)?;
    let limit = limit(q.limit, s.cfg.limits.workbench_page_size)?;
    let fingerprint = fingerprint("jobs", json!([query, employer]));
    let cursor = decode_cursor(q.cursor.as_deref(), &fingerprint)?;
    let mut f = Filter::default();
    f.add(JOB_ELIGIBLE);
    f.exact("j.owner_id", &employer);
    for term in terms(&query)? {
        if term.chars().count() >= 3 {
            let full = f.bind(phrase(term));
            let name = f.bind(format!("name : {}", phrase(term)));
            // The unit-name column is constrained explicitly; recruiter resumes
            // are not searched or copied into the job index.
            f.add(format!("(j.rowid IN (SELECT rowid FROM wb_jobs_fts WHERE wb_jobs_fts MATCH {full}) OR e.rowid IN (SELECT rowid FROM wb_users_fts WHERE wb_users_fts MATCH {name}))"));
        } else {
            f.short_term(&["j.title", "j.jd_md", "e.name"], term);
        }
    }
    let total = count(&db, JOB_FROM, &f)?;
    f.keyset("j.created_at", "j.id", cursor.as_ref());
    let n = f.bind(limit + 1);
    let sql = format!(
        "SELECT {JOB_FIELDS} {JOB_FROM} WHERE {} ORDER BY j.created_at DESC,j.id DESC LIMIT {n}",
        f.clause()
    );
    let rows = db
        .prepare(&sql)?
        .query_map(params_from_iter(f.params.iter()), job_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    page(rows, total, limit, fingerprint, "created_at")
}

fn job_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let currency: String = r.get("currency")?;
    Ok(
        json!({"id":r.get::<_,String>("id")?,"owner_id":r.get::<_,String>("owner_id")?,"employer_id":r.get::<_,String>("owner_id")?,"employer_name":r.get::<_,String>("employer_name")?,"title":r.get::<_,String>("title")?,"salary_cents":r.get::<_,i64>("salary_cents")?,"currency":currency,"currency_decimals":crate::config::currency_decimals(&currency),"headcount":r.get::<_,i64>("headcount")?,"status":r.get::<_,String>("status")?,"jd_excerpt":r.get::<_,String>("jd_excerpt")?,"created_at":r.get::<_,i64>("created_at")?,"updated_at":r.get::<_,i64>("updated_at")?,"active_matches":r.get::<_,i64>("active_matches")?}),
    )
}
pub fn get_job(s: &AppState, p: &Principal, id: &str) -> AppResult<Value> {
    use rusqlite::OptionalExtension;
    let db = s.conn()?;
    access::ensure_operator(&db, p)?;
    db.query_row(
        &format!("SELECT {JOB_FIELDS} {JOB_FROM} WHERE j.id=?1 AND {JOB_ELIGIBLE}"),
        [id],
        job_row,
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("Job not found or no longer available for matching"))
}

pub fn ensure_matchable_job(db: &Connection, id: &str) -> AppResult<()> {
    let exists: bool =
        db.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1)", [id], |r| {
            r.get(0)
        })?;
    if !exists {
        return Err(AppError::not_found("Job not found"));
    }
    let eligible: bool = db.query_row(
        &format!("SELECT EXISTS(SELECT 1 {JOB_FROM} WHERE j.id=?1 AND {JOB_ELIGIBLE})"),
        [id],
        |r| r.get(0),
    )?;
    if !eligible {
        return Err(AppError::conflict(
            "This job is no longer available for new matches; choose another job",
        ));
    }
    Ok(())
}

pub fn search_candidates(s: &AppState, p: &Principal, q: &CandidateQuery) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_operator(&db, p)?;
    candidate_data(
        &db,
        q,
        false,
        &s.cfg.pricing.currency,
        s.cfg.limits.workbench_page_size,
    )
}

/// Diagnostic for regression tests and local capacity measurements. It is not
/// registered as an HTTP or MCP endpoint and requires the same administrator.
#[doc(hidden)]
pub fn explain_candidates(s: &AppState, p: &Principal, q: &CandidateQuery) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_operator(&db, p)?;
    candidate_data(
        &db,
        q,
        true,
        &s.cfg.pricing.currency,
        s.cfg.limits.workbench_page_size,
    )
}

fn candidate_data(
    db: &Connection,
    q: &CandidateQuery,
    explain: bool,
    default_currency: &str,
    default_page_size: i64,
) -> AppResult<Value> {
    let query = clean(q.q.as_deref(), 120)?;
    let job = clean(q.job_id.as_deref(), 80)?;
    let industry = clean(q.industry.as_deref(), 80)?;
    let roles = clean(q.roles.as_deref(), 80)?;
    let location = clean(q.location.as_deref(), 80)?;
    salary_range(q.salary_min_cents, q.salary_max_cents)?;
    if !job.is_empty() {
        ensure_matchable_job(db, &job)?;
    }
    let salary_currency = if q.salary_currency.is_some()
        || q.salary_min_cents.is_some()
        || q.salary_max_cents.is_some()
    {
        let currency = if let Some(currency) = &q.salary_currency {
            currency.clone()
        } else if !job.is_empty() {
            db.query_row("SELECT currency FROM jobs WHERE id=?1", [&job], |row| {
                row.get(0)
            })?
        } else {
            default_currency.to_owned()
        };
        if crate::config::currency_decimals(&currency).is_none() {
            return Err(AppError::bad_request("Choose a supported salary currency"));
        }
        Some(currency)
    } else {
        None
    };
    let limit = limit(q.limit, default_page_size)?;
    let fingerprint = fingerprint(
        "candidates",
        json!([
            query,
            job,
            industry,
            roles,
            location,
            q.salary_min_cents,
            q.salary_max_cents,
            salary_currency
        ]),
    );
    let cursor = decode_cursor(q.cursor.as_deref(), &fingerprint)?;
    let mut f = Filter::default();
    f.add(CANDIDATE_ELIGIBLE);
    if let Some(currency) = &salary_currency {
        f.exact("m.salary_currency", currency);
    }
    for term in terms(&query)? {
        if term.chars().count() >= 3 {
            let term = f.bind(phrase(term));
            f.full_text(format!("m.user_id IN (SELECT ux.id FROM wb_users_fts JOIN users ux ON ux.rowid=wb_users_fts.rowid WHERE wb_users_fts MATCH {term} UNION SELECT mx.user_id FROM wb_mandates_fts JOIN mandates mx ON mx.rowid=wb_mandates_fts.rowid WHERE wb_mandates_fts MATCH {term})"));
        } else {
            f.short_term(
                &[
                    "u.name",
                    "u.resume_md",
                    "m.target_industry",
                    "m.target_roles",
                    "m.location",
                    "m.preferences",
                ],
                term,
            );
        }
    }
    for (column, text) in [
        ("target_industry", &industry),
        ("target_roles", &roles),
        ("location", &location),
    ] {
        for term in terms(text)? {
            if term.chars().count() >= 3 {
                let term = f.bind(format!("{column} : {}", phrase(term)));
                f.full_text(format!("m.rowid IN (SELECT rowid FROM wb_mandates_fts WHERE wb_mandates_fts MATCH {term})"));
            } else {
                f.short_term(&[&format!("m.{column}")], term);
            }
        }
    }
    if let Some(min) = q.salary_min_cents {
        let p = f.bind(min);
        f.add(format!(
            "(m.salary_max_cents IS NULL OR m.salary_max_cents>={p})"
        ));
    }
    if let Some(max) = q.salary_max_cents {
        let p = f.bind(max);
        f.add(format!(
            "(m.salary_min_cents IS NULL OR m.salary_min_cents<={p})"
        ));
    }
    let history = if job.is_empty() {
        "NULL".to_owned()
    } else {
        let p = f.bind(job);
        f.add(format!("NOT EXISTS(SELECT 1 FROM matches active WHERE active.seeker_id=u.id AND active.job_id={p} AND active.status IN ('introduced','interview','hired'))"));
        format!(
            "(SELECT json_object('id',past.id,'status',past.status,'conversation_id','match_'||past.id,'updated_at',past.updated_at) FROM matches past WHERE past.seeker_id=u.id AND past.job_id={p} AND past.status='declined')"
        )
    };
    let count_from = if f.indexed_search {
        CANDIDATE_FROM
    } else {
        CANDIDATE_ORDERED_FROM
    };
    let total = count(db, count_from, &f)?;
    // A selective FTS lookup should retain its short posting-list path. Broad
    // results and short-term searches instead seek the creation-date index and
    // read at most the matching page, with no full-result ORDER BY materializing.
    let selective_fts = f.indexed_search && total <= 1000;
    let page_from = if selective_fts {
        CANDIDATE_FROM
    } else {
        CANDIDATE_ORDERED_FROM
    };
    f.keyset("m.created_at", "m.user_id", cursor.as_ref());
    let n = f.bind(limit + 1);
    let sql = format!(
        "SELECT u.id,u.name,(u.avatar<>'') AS has_avatar,substr(u.resume_md,1,240) AS resume_excerpt,u.updated_at,m.target_industry,m.target_roles,m.salary_min_cents,m.salary_max_cents,m.salary_currency,m.location,substr(m.preferences,1,240) AS preferences_excerpt,m.created_at,m.updated_at AS mandate_updated_at,{history} AS history_match {page_from} WHERE {} ORDER BY m.created_at DESC,m.user_id DESC LIMIT {n}",
        f.clause()
    );
    if explain {
        let plan = db
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?
            .query_map(params_from_iter(f.params.iter()), |r| r.get::<_, String>(3))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        return Ok(
            json!({"plan":plan,"strategy":if selective_fts{"selective_fts"}else{"ordered_keyset"},"total":total}),
        );
    }
    let rows=db.prepare(&sql)?.query_map(params_from_iter(f.params.iter()),|r|{
        let id:String=r.get("id")?;
        let has_avatar:bool=r.get("has_avatar")?;
        let history:Option<String>=r.get("history_match")?;
        let currency:String=r.get("salary_currency")?;
        Ok(json!({"id":id,"name":r.get::<_,String>("name")?,"avatar":if has_avatar{format!("/api/avatars/{id}")}else{String::new()},"resume_excerpt":r.get::<_,String>("resume_excerpt")?,"target_industry":r.get::<_,String>("target_industry")?,"target_roles":r.get::<_,String>("target_roles")?,"salary_min_cents":r.get::<_,Option<i64>>("salary_min_cents")?,"salary_max_cents":r.get::<_,Option<i64>>("salary_max_cents")?,"salary_currency":currency,"currency_decimals":crate::config::currency_decimals(&currency),"location":r.get::<_,String>("location")?,"preferences_excerpt":r.get::<_,String>("preferences_excerpt")?,"created_at":r.get::<_,i64>("created_at")?,"updated_at":r.get::<_,i64>("updated_at")?,"mandate_updated_at":r.get::<_,i64>("mandate_updated_at")?,"history_match":history.and_then(|v|serde_json::from_str::<Value>(&v).ok())}))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    page(rows, total, limit, fingerprint, "created_at")
}

pub fn search_matches(s: &AppState, p: &Principal, q: &MatchQuery) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_operator(&db, p)?;
    let query = clean(q.q.as_deref(), 120)?;
    let status = clean(q.status.as_deref(), 20)?;
    if !matches!(
        status.as_str(),
        "" | "introduced" | "interview" | "hired" | "declined"
    ) {
        return Err(AppError::bad_request("Invalid match status"));
    }
    let job = clean(q.job_id.as_deref(), 80)?;
    let seeker = clean(q.seeker_id.as_deref(), 80)?;
    let employer = clean(q.employer_id.as_deref(), 80)?;
    let limit = limit(q.limit, s.cfg.limits.workbench_page_size)?;
    let fingerprint = fingerprint("matches", json!([query, status, job, seeker, employer]));
    let cursor = decode_cursor(q.cursor.as_deref(), &fingerprint)?;
    let mut f = Filter::default();
    f.exact("m.job_id", &job);
    f.exact("m.seeker_id", &seeker);
    f.exact("j.owner_id", &employer);
    for term in terms(&query)? {
        if term.chars().count() >= 3 {
            let full = f.bind(phrase(term));
            let name = f.bind(format!("name : {}", phrase(term)));
            let title = f.bind(format!("title : {}", phrase(term)));
            f.add(format!("(m.rowid IN (SELECT rowid FROM wb_matches_fts WHERE wb_matches_fts MATCH {full}) OR s.rowid IN (SELECT rowid FROM wb_users_fts WHERE wb_users_fts MATCH {name}) OR e.rowid IN (SELECT rowid FROM wb_users_fts WHERE wb_users_fts MATCH {name}) OR j.rowid IN (SELECT rowid FROM wb_jobs_fts WHERE wb_jobs_fts MATCH {title}))"));
        } else {
            f.short_term(&["s.name", "e.name", "j.title", "m.note"], term);
        }
    }
    let mut stage_counts = json!({"introduced":0,"interview":0,"hired":0,"declined":0});
    for row in db
        .prepare(&format!(
            "SELECT m.status,count(*) {MATCH_FROM} WHERE {} GROUP BY m.status",
            f.clause()
        ))?
        .query_map(params_from_iter(f.params.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
    {
        let (stage, n) = row?;
        stage_counts[stage] = json!(n);
    }
    f.exact("m.status", &status);
    let total = count(&db, MATCH_FROM, &f)?;
    f.keyset("m.updated_at", "m.id", cursor.as_ref());
    let n = f.bind(limit + 1);
    let rows=db.prepare(&format!("SELECT m.*,s.name AS seeker_name,j.title AS job_title,j.owner_id AS employer_id,e.name AS employer_name {MATCH_FROM} WHERE {} ORDER BY m.updated_at DESC,m.id DESC LIMIT {n}",f.clause()))?.query_map(params_from_iter(f.params.iter()),|r|{
        let id:String=r.get("id")?;let seeker:String=r.get("seeker_id")?;let employer:String=r.get("employer_id")?;
        Ok(json!({"id":id,"seeker_id":seeker,"seeker_name":r.get::<_,String>("seeker_name")?,"job_id":r.get::<_,String>("job_id")?,"job_title":r.get::<_,String>("job_title")?,"employer_id":employer,"employer_name":r.get::<_,String>("employer_name")?,"status":r.get::<_,String>("status")?,"note":r.get::<_,String>("note")?,"created_at":r.get::<_,i64>("created_at")?,"updated_at":r.get::<_,i64>("updated_at")?,"conversation_id":format!("match_{id}"),"seeker_conversation_id":format!("platform_{seeker}"),"employer_conversation_id":format!("platform_{employer}")}))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut result = page(rows, total, limit, fingerprint, "updated_at")?;
    result["stage_counts"] = stage_counts;
    Ok(result)
}
