//! Snapshot-bound, bounded queries for the administrator relationship workbench.
//!
//! The directory includes every account role. Only seeker/job matches are graph
//! edges. A page is never used as the graph's data boundary. Group membership is
//! computed in SQL, with at most nineteen named groups and an exact remainder.
//! This first version detects stale snapshots rather than retaining old copies of
//! personal data. A caller must refresh on `resync_required`, never append a new
//! revision's rows to an old page.
//! `possible_pairs` is the filtered directory's seeker/job Cartesian product,
//! not a claim that those pairs satisfy the rules for creating new matches.
use crate::{
    AppError, AppResult, AppState,
    access::{self, Principal},
};
use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use rusqlite::{Connection, OptionalExtension, params_from_iter, types::Value as SqlValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};

const PAGE_DEFAULT: i64 = 20;
const PAGE_MAX: i64 = 100;
const GROUP_MAX: usize = 20;
const CURSOR_SECONDS: i64 = 900;
const STAGES: [&str; 5] = ["introduced", "interview", "hired", "declined", "unknown"];
const EDGE_FIELDS: &str = "f.id AS edge_id,f.seeker_id,f.job_id,f.status AS match_status,f.updated_at AS match_updated_at,f.left_group_id,f.right_group_id";

/// Per database/application cache of aggregate bytes only. Authorization and the
/// database revision are checked before every lookup. Keeping serialized bytes
/// gives a concrete payload memory bound instead of estimating Value tree sizes.
#[derive(Default)]
pub struct Cache {
    revision: Option<i64>,
    entries: VecDeque<(String, Vec<u8>)>,
    bytes: usize,
}
impl Cache {
    const MAX_ENTRIES: usize = 32;
    const MAX_BYTES: usize = 4 * 1024 * 1024;

    fn reset_revision(&mut self, rev: i64) {
        if self.revision != Some(rev) {
            self.entries.clear();
            self.bytes = 0;
            self.revision = Some(rev);
        }
    }

    fn get(&mut self, rev: i64, key: &str) -> AppResult<Option<Value>> {
        self.reset_revision(rev);
        let Some(index) = self.entries.iter().position(|(id, _)| id == key) else {
            return Ok(None);
        };
        let item = self.entries.remove(index).unwrap();
        let value = serde_json::from_slice(&item.1)?;
        self.entries.push_back(item);
        Ok(Some(value))
    }

    fn put(&mut self, rev: i64, key: &str, value: &Value) -> AppResult<()> {
        self.reset_revision(rev);
        let bytes = serde_json::to_vec(value)?;
        if bytes.len() > Self::MAX_BYTES {
            return Ok(());
        }
        if let Some(index) = self.entries.iter().position(|(id, _)| id == key) {
            self.bytes -= self.entries.remove(index).unwrap().1.len();
        }
        while self.entries.len() >= Self::MAX_ENTRIES || self.bytes + bytes.len() > Self::MAX_BYTES
        {
            if let Some((_, old)) = self.entries.pop_front() {
                self.bytes -= old.len();
            }
        }
        self.bytes += bytes.len();
        self.entries.push_back((key.to_owned(), bytes));
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Overview,
    Nodes,
    Neighbors,
    Edges,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LeftGrouping {
    #[default]
    Role,
    Industry,
    Roles,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RightGrouping {
    #[default]
    Status,
    Owner,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Grouping {
    pub left: LeftGrouping,
    pub right: RightGrouping,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Focus {
    pub side: Side,
    pub id: String,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Filters {
    pub user_q: Option<String>,
    pub job_q: Option<String>,
    pub role: Option<String>,
    pub account_status: Option<String>,
    pub job_status: Option<String>,
    pub match_status: Option<String>,
    pub employer_id: Option<String>,
    pub user_ids: Option<Vec<String>>,
    pub job_ids: Option<Vec<String>>,
    pub left_group_ids: Option<Vec<String>>,
    pub right_group_ids: Option<Vec<String>>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Query {
    pub mode: Mode,
    pub filters: Filters,
    pub grouping: Grouping,
    pub side: Option<Side>,
    pub focus: Option<Focus>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
    pub data_rev: Option<i64>,
}

/// Additive and idempotent. Revision and cursor signing key survive restarts.
pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch("SAVEPOINT relations_v1;")?;
    let result = (|| -> AppResult<()> {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS relation_clock(
               id INTEGER PRIMARY KEY CHECK(id=1),
               revision INTEGER NOT NULL CHECK(revision>=0),
               cursor_key BLOB NOT NULL CHECK(length(cursor_key)=32));
             INSERT OR IGNORE INTO relation_clock VALUES(1,0,randomblob(32));
             CREATE INDEX IF NOT EXISTS rel_users_page ON users(created_at DESC,id DESC);
             CREATE INDEX IF NOT EXISTS rel_users_role_page ON users(role,created_at DESC,id DESC);
             CREATE INDEX IF NOT EXISTS rel_jobs_page ON jobs(created_at DESC,id DESC);
             CREATE INDEX IF NOT EXISTS rel_jobs_status_page ON jobs(status,created_at DESC,id DESC);",
        )?;
        // All graph membership and label dependencies advance the same clock,
        // including metadata changes that move otherwise unchanged match rows.
        for table in ["users", "jobs", "mandates", "matches"] {
            for event in ["INSERT", "UPDATE", "DELETE"] {
                db.execute_batch(&format!(
                    "CREATE TRIGGER IF NOT EXISTS rel_{table}_{event} AFTER {event} ON {table}
                     BEGIN UPDATE relation_clock SET revision=revision+1 WHERE id=1; END;"
                ))?;
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            db.execute_batch("RELEASE relations_v1;")?;
            Ok(())
        }
        Err(error) => {
            let _ = db.execute_batch("ROLLBACK TO relations_v1; RELEASE relations_v1;");
            Err(error)
        }
    }
}

pub fn revision(db: &Connection) -> AppResult<i64> {
    Ok(
        db.query_row("SELECT revision FROM relation_clock WHERE id=1", [], |r| {
            r.get(0)
        })?,
    )
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/admin/relations/query", post(http_query))
}

async fn http_query(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(q): Json<Query>,
) -> AppResult<Json<Value>> {
    // POST carries a typed query; it does not mutate business or view state.
    let p = access::principal(&s, &headers, false)?;
    let result = tokio::task::spawn_blocking(move || query(&s, &p, &q))
        .await
        .map_err(AppError::internal)??;
    Ok(Json(result))
}

#[derive(Clone, Default)]
struct Sql {
    params: Vec<SqlValue>,
}
impl Sql {
    fn bind(&mut self, value: impl Into<SqlValue>) -> String {
        self.params.push(value.into());
        format!("?{}", self.params.len())
    }
    fn exact(&mut self, column: &str, value: Option<&str>, clauses: &mut Vec<String>) {
        if let Some(value) = value {
            let p = self.bind(value.to_owned());
            clauses.push(format!("{column}={p}"));
        }
    }
    fn ids(&mut self, column: &str, values: Option<&[String]>, clauses: &mut Vec<String>) {
        if let Some(values) = values {
            if values.is_empty() {
                clauses.push("0=1".into());
            } else {
                let placeholders = values
                    .iter()
                    .map(|v| self.bind(v.clone()))
                    .collect::<Vec<_>>();
                clauses.push(format!("{column} IN ({})", placeholders.join(",")));
            }
        }
    }
    fn search(&mut self, columns: &[&str], value: Option<&str>, clauses: &mut Vec<String>) {
        if let Some(value) = value {
            let p = self.bind(format!(
                "%{}%",
                value
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            ));
            clauses.push(format!(
                "({})",
                columns
                    .iter()
                    .map(|column| format!("{column} LIKE {p} ESCAPE '\\'"))
                    .collect::<Vec<_>>()
                    .join(" OR ")
            ));
        }
    }
}

fn normalize_text(value: &mut Option<String>, max: usize) -> AppResult<()> {
    if let Some(raw) = value {
        if raw.chars().any(char::is_control) || raw.chars().count() > max {
            return Err(AppError::bad_request(format!(
                "Text must be at most {max} characters without control characters"
            )));
        }
        let clean = raw.trim().to_owned();
        *value = (!clean.is_empty()).then_some(clean);
    }
    Ok(())
}

fn normalize_ids(values: &mut Option<Vec<String>>, maximum: usize) -> AppResult<()> {
    if let Some(values) = values {
        if values.len() > maximum
            || values
                .iter()
                .any(|v| v.is_empty() || v.len() > 120 || v.chars().any(char::is_control))
        {
            return Err(AppError::bad_request(format!(
                "Use at most {maximum} nonempty IDs of at most 120 bytes"
            )));
        }
        values.sort();
        values.dedup();
    }
    Ok(())
}

pub(crate) fn normalized(input: &Query) -> AppResult<Query> {
    let mut q = input.clone();
    let f = &mut q.filters;
    normalize_text(&mut f.user_q, 120)?;
    normalize_text(&mut f.job_q, 120)?;
    for v in [
        &mut f.role,
        &mut f.account_status,
        &mut f.job_status,
        &mut f.match_status,
        &mut f.employer_id,
    ] {
        normalize_text(v, 80)?;
    }
    for (value, allowed) in [
        (f.role.as_deref(), &["seeker", "employer", "unassigned"][..]),
        (f.account_status.as_deref(), &["active", "suspended"][..]),
        (
            f.job_status.as_deref(),
            &["draft", "published", "closed", "deleted"][..],
        ),
        (f.match_status.as_deref(), &STAGES[..]),
    ] {
        if value.is_some_and(|v| !allowed.contains(&v)) {
            return Err(AppError::bad_request("Unsupported relation filter value"));
        }
    }
    normalize_ids(&mut f.user_ids, 200)?;
    normalize_ids(&mut f.job_ids, 200)?;
    normalize_ids(&mut f.left_group_ids, GROUP_MAX)?;
    normalize_ids(&mut f.right_group_ids, GROUP_MAX)?;
    if !(1..=PAGE_MAX).contains(&q.limit.unwrap_or(PAGE_DEFAULT)) {
        return Err(AppError::bad_request("Page size must be between 1 and 100"));
    }
    if q.data_rev.is_some_and(|v| v < 0) {
        return Err(AppError::bad_request("Invalid data revision"));
    }
    if q.mode == Mode::Nodes && q.side.is_none() {
        return Err(AppError::bad_request("nodes requires side: left or right"));
    }
    if q.mode == Mode::Neighbors && q.focus.is_none() {
        return Err(AppError::bad_request("neighbors requires a focus"));
    }
    if let Some(focus) = &q.focus {
        if focus.id.is_empty() || focus.id.len() > 80 || focus.id.chars().any(char::is_control) {
            return Err(AppError::bad_request("Invalid focus ID"));
        }
    }
    if q.mode == Mode::Overview && q.cursor.is_some() {
        return Err(AppError::bad_request("Overview does not use a page cursor"));
    }
    Ok(q)
}

#[derive(Clone)]
struct Group {
    id: String,
    raw: Option<String>,
    label: String,
}

fn short(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(max).collect::<String>();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn role_label(role: &str) -> &str {
    match role {
        "seeker" => "求职者",
        "employer" => "招聘方",
        _ => "未选择身份",
    }
}

fn grouping_keys(q: &Query) -> (&'static str, &'static str, &'static str, &'static str) {
    let (left, left_name) = match q.grouping.left {
        LeftGrouping::Role => ("u.role", "role"),
        LeftGrouping::Industry => (
            "CASE WHEN u.role='seeker' THEN 'seeker:'||trim(coalesce(md.target_industry,'')) ELSE 'account:'||u.role END",
            "industry",
        ),
        LeftGrouping::Roles => (
            "CASE WHEN u.role='seeker' THEN 'seeker:'||trim(coalesce(md.target_roles,'')) ELSE 'account:'||u.role END",
            "roles",
        ),
    };
    let (right, right_name) = match q.grouping.right {
        RightGrouping::Status => ("j.status", "status"),
        RightGrouping::Owner => ("j.owner_id", "owner"),
    };
    (left, left_name, right, right_name)
}

fn group_catalog(
    db: &Connection,
    base: &str,
    sql: &Sql,
    q: &Query,
    side: Side,
    name: &str,
) -> AppResult<Vec<Group>> {
    let table = if side == Side::Left {
        "raw_users"
    } else {
        "raw_jobs"
    };
    let label = if side == Side::Right && q.grouping.right == RightGrouping::Owner {
        "min(owner_name)"
    } else {
        "raw_group"
    };
    let rows = db.prepare(&format!(
        "WITH {base} SELECT raw_group,{label} FROM {table} GROUP BY raw_group ORDER BY count(*) DESC,raw_group LIMIT 21"
    ))?.query_map(params_from_iter(sql.params.iter()), |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let overflow = rows.len() > GROUP_MAX;
    let prefix = if side == Side::Left { "left" } else { "right" };
    let mut groups = Vec::new();
    for (raw, source_label) in
        rows.into_iter()
            .take(if overflow { GROUP_MAX - 1 } else { GROUP_MAX })
    {
        let label = if side == Side::Left {
            match q.grouping.left {
                LeftGrouping::Role => role_label(&raw).to_owned(),
                LeftGrouping::Industry | LeftGrouping::Roles => {
                    if let Some(role) = raw.strip_prefix("account:") {
                        role_label(role).to_owned()
                    } else {
                        let text = raw.strip_prefix("seeker:").unwrap_or_default();
                        if text.is_empty() {
                            "未归类".into()
                        } else {
                            short(text, 80)
                        }
                    }
                }
            }
        } else if q.grouping.right == RightGrouping::Status {
            match raw.as_str() {
                "draft" => "草稿",
                "published" => "招聘中",
                "closed" => "已关闭",
                "deleted" => "已删除",
                _ => "其他",
            }
            .into()
        } else if source_label.trim().is_empty() {
            format!("招聘方 {}", short(&raw, 8))
        } else {
            short(&source_label, 80)
        };
        groups.push(Group {
            id: format!(
                "{prefix}:{name}:{}",
                hex::encode(Sha256::digest(raw.as_bytes()))
            ),
            raw: Some(raw),
            label,
        });
    }
    if overflow {
        groups.push(Group {
            id: format!("{prefix}:{name}:other"),
            raw: None,
            label: "其他（展开筛选查看）".into(),
        });
    }
    Ok(groups)
}

fn group_case(sql: &mut Sql, groups: &[Group], fallback: &str) -> (String, String) {
    let mut expression = "CASE raw_group".to_owned();
    let mut indices = "CASE raw_group".to_owned();
    for (index, group) in groups.iter().enumerate().filter(|(_, g)| g.raw.is_some()) {
        let raw = sql.bind(group.raw.clone().unwrap());
        let id = sql.bind(group.id.clone());
        expression.push_str(&format!(" WHEN {raw} THEN {id}"));
        indices.push_str(&format!(" WHEN {raw} THEN {index}"));
    }
    let other = sql.bind(
        groups
            .iter()
            .find(|g| g.raw.is_none())
            .map(|g| g.id.as_str())
            .unwrap_or(fallback)
            .to_owned(),
    );
    if groups.iter().all(|g| g.raw.is_none()) {
        return (other, "-1".into());
    }
    let other_index = groups
        .iter()
        .position(|g| g.raw.is_none())
        .map(|n| n as i64)
        .unwrap_or(-1);
    expression.push_str(&format!(" ELSE {other} END"));
    indices.push_str(&format!(" ELSE {other_index} END"));
    (expression, indices)
}

struct Scope {
    cte: String,
    sql: Sql,
    left: Vec<Group>,
    right: Vec<Group>,
}

fn scope(db: &Connection, q: &Query) -> AppResult<Scope> {
    let f = &q.filters;
    let mut sql = Sql::default();
    let mut users = vec!["1=1".to_owned()];
    let mut jobs = vec!["1=1".to_owned()];
    sql.exact("u.role", f.role.as_deref(), &mut users);
    if let Some(status) = f.account_status.as_deref() {
        users.push(format!("u.banned={}", i32::from(status == "suspended")));
    }
    sql.search(
        &[
            "u.name",
            "u.email",
            "u.id",
            "md.target_industry",
            "md.target_roles",
        ],
        f.user_q.as_deref(),
        &mut users,
    );
    sql.ids("u.id", f.user_ids.as_deref(), &mut users);
    sql.exact("j.status", f.job_status.as_deref(), &mut jobs);
    sql.exact("j.owner_id", f.employer_id.as_deref(), &mut jobs);
    sql.search(
        &["j.title", "j.id", "e.name"],
        f.job_q.as_deref(),
        &mut jobs,
    );
    sql.ids("j.id", f.job_ids.as_deref(), &mut jobs);
    let (left_key, left_name, right_key, right_name) = grouping_keys(q);
    let base = format!(
        "raw_users AS (SELECT u.id,substr(u.name,1,120) AS name,u.role,u.banned,u.created_at,u.updated_at,{left_key} AS raw_group FROM users u LEFT JOIN mandates md ON md.user_id=u.id WHERE {}),
         raw_jobs AS (SELECT j.id,substr(j.title,1,160) AS title,j.owner_id,substr(e.name,1,120) AS owner_name,j.headcount,j.salary_cents,j.currency,j.status,j.created_at,j.updated_at,{right_key} AS raw_group FROM jobs j JOIN users e ON e.id=j.owner_id WHERE {})",
        users.join(" AND "),jobs.join(" AND ")
    );
    let left = group_catalog(db, &base, &sql, q, Side::Left, left_name)?;
    let right = group_catalog(db, &base, &sql, q, Side::Right, right_name)?;
    for (ids, catalog) in [
        (f.left_group_ids.as_ref(), &left),
        (f.right_group_ids.as_ref(), &right),
    ] {
        if ids.is_some_and(|ids| ids.iter().any(|id| !catalog.iter().any(|g| &g.id == id))) {
            return Err(AppError::bad_request(
                "Unknown group in this filter scope; refresh groups after changing filters",
            ));
        }
    }
    let (left_case, left_index) = group_case(&mut sql, &left, "left:empty");
    let (right_case, right_index) = group_case(&mut sql, &right, "right:empty");
    let mut selected_users = vec!["1=1".to_owned()];
    let mut selected_jobs = vec!["1=1".to_owned()];
    sql.ids("group_id", f.left_group_ids.as_deref(), &mut selected_users);
    sql.ids("group_id", f.right_group_ids.as_deref(), &mut selected_jobs);
    let normalized_status = "CASE WHEN m.status IN ('introduced','interview','hired','declined') THEN m.status ELSE 'unknown' END";
    let mut edge_clauses = vec!["u.role='seeker'".to_owned()];
    if f.match_status.as_deref() == Some("unknown") {
        edge_clauses.push("m.status NOT IN ('introduced','interview','hired','declined')".into());
    } else {
        // Keep the stage predicate on the indexed stored column. Applying CASE
        // to the column here would defeat the existing match-status index.
        sql.exact("m.status", f.match_status.as_deref(), &mut edge_clauses);
    }
    let cte = format!(
        "{base},gu AS (SELECT raw_users.*,{left_case} AS group_id,{left_index} AS group_index FROM raw_users),gj AS (SELECT raw_jobs.*,{right_case} AS group_id,{right_index} AS group_index FROM raw_jobs),
         u AS MATERIALIZED (SELECT * FROM gu WHERE {}),j AS MATERIALIZED (SELECT * FROM gj WHERE {}),
         fm AS NOT MATERIALIZED (SELECT m.id,m.seeker_id,m.job_id,m.status AS raw_status,{normalized_status} AS status,m.updated_at,u.group_id AS left_group_id,j.group_id AS right_group_id,u.group_index AS left_group_index,j.group_index AS right_group_index FROM matches m JOIN u ON u.id=m.seeker_id JOIN j ON j.id=m.job_id WHERE {})",
        selected_users.join(" AND "),selected_jobs.join(" AND "),edge_clauses.join(" AND ")
    );
    Ok(Scope {
        cte,
        sql,
        left,
        right,
    })
}

fn state_counts() -> Value {
    json!({"introduced":0,"interview":0,"hired":0,"declined":0,"unknown":0})
}

fn stage_sums() -> String {
    STAGES
        .iter()
        .map(|stage| {
            let predicate = if *stage == "unknown" {
                "raw_status NOT IN ('introduced','interview','hired','declined')".to_owned()
            } else {
                format!("raw_status='{stage}'")
            };
            format!("coalesce(sum({predicate}),0)")
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn stats(db: &Connection, s: &Scope, extra: Option<(&str, &str)>) -> AppResult<(i64, Value)> {
    let mut sql = s.sql.clone();
    let condition = extra
        .map(|(column, id)| format!("{column}={}", sql.bind(id.to_owned())))
        .unwrap_or_else(|| "1=1".into());
    let sums = stage_sums();
    Ok(db.query_row(
        &format!(
            "WITH {} SELECT count(*),{sums} FROM fm WHERE {condition}",
            s.cte
        ),
        params_from_iter(sql.params.iter()),
        |r| {
            let mut counts = state_counts();
            for (i, stage) in STAGES.iter().enumerate() {
                counts[*stage] = json!(r.get::<_, i64>(i + 1)?);
            }
            Ok((r.get(0)?, counts))
        },
    )?)
}

fn endpoint_groups(db: &Connection, s: &Scope, side: Side) -> AppResult<Vec<Value>> {
    let (table, catalog) = if side == Side::Left {
        ("u", &s.left)
    } else {
        ("j", &s.right)
    };
    let seekers = if side == Side::Left {
        "sum(role='seeker')"
    } else {
        "0"
    };
    let mut counts = BTreeMap::new();
    for row in db
        .prepare(&format!(
            "WITH {} SELECT group_id,count(*),{seekers} FROM {table} GROUP BY group_id",
            s.cte
        ))?
        .query_map(params_from_iter(s.sql.params.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
    {
        let (id, count, seekers) = row?;
        counts.insert(id, (count, seekers));
    }
    Ok(catalog
        .iter()
        .filter_map(|g| {
            counts.get(&g.id).map(|(count, seekers)| {
                let mut result = json!({"id":g.id,"label":g.label,"count":count,"edges":0});
                if side == Side::Left {
                    result["seekers"] = json!(seekers);
                }
                result
            })
        })
        .collect())
}

fn overview(db: &Connection, s: &Scope) -> AppResult<Value> {
    let mut left = endpoint_groups(db, s, Side::Left)?;
    let mut right = endpoint_groups(db, s, Side::Right)?;
    let mut edges = 0_i64;
    let mut counts = state_counts();
    // Each matched endpoint needs only one indexed existence probe, rather than
    // a second whole-edge scan with two DISTINCT accumulators.
    let (matched_users, matched_jobs): (i64,i64) = db.query_row(
        &format!("WITH {} SELECT (SELECT count(*) FROM u n WHERE n.role='seeker' AND EXISTS(SELECT 1 FROM fm f WHERE f.seeker_id=n.id)),(SELECT count(*) FROM j n WHERE EXISTS(SELECT 1 FROM fm f WHERE f.job_id=n.id))",s.cte),
        params_from_iter(s.sql.params.iter()), |r| Ok((r.get(0)?,r.get(1)?))
    )?;
    let users = left
        .iter()
        .map(|g| g["count"].as_i64().unwrap_or(0))
        .sum::<i64>();
    let seekers = left
        .iter()
        .map(|g| g["seekers"].as_i64().unwrap_or(0))
        .sum::<i64>();
    let jobs = right
        .iter()
        .map(|g| g["count"].as_i64().unwrap_or(0))
        .sum::<i64>();
    let possible_pairs = seekers
        .checked_mul(jobs)
        .ok_or_else(|| AppError::internal("Pair count overflow"))?;
    let sums = stage_sums();
    let mut bundles = Vec::new();
    // Short integer group keys keep hashes and labels out of the million-row
    // aggregate's sorting hot path. Public group IDs remain stable strings.
    for row in db.prepare(&format!("WITH {} SELECT left_group_index,right_group_index,count(*),count(DISTINCT seeker_id),count(DISTINCT job_id),{sums} FROM fm GROUP BY left_group_index,right_group_index",s.cte))?.query_map(params_from_iter(s.sql.params.iter()),|r| {
        let mut counts=state_counts();
        for (i,stage) in STAGES.iter().enumerate() { counts[*stage]=json!(r.get::<_,i64>(i+5)?); }
        let left=s.left.get(r.get::<_,usize>(0)?).ok_or(rusqlite::Error::InvalidQuery)?;
        let right=s.right.get(r.get::<_,usize>(1)?).ok_or(rusqlite::Error::InvalidQuery)?;
        Ok(json!({"left_group_id":left.id,"right_group_id":right.id,"count":r.get::<_,i64>(2)?,"seekers":r.get::<_,i64>(3)?,"jobs":r.get::<_,i64>(4)?,"state_counts":counts}))
    })? {
        let mut bundle=row?;
        edges += bundle["count"].as_i64().unwrap_or(0);
        for stage in STAGES { counts[stage]=json!(counts[stage].as_i64().unwrap_or(0)+bundle["state_counts"][stage].as_i64().unwrap_or(0)); }
        let mut left_size=0;
        let mut right_size=0;
        for g in &mut left { if g["id"]==bundle["left_group_id"] { left_size=g["seekers"].as_i64().unwrap_or(0);g["edges"]=json!(g["edges"].as_i64().unwrap_or(0)+bundle["count"].as_i64().unwrap_or(0)); } }
        for g in &mut right { if g["id"]==bundle["right_group_id"] { right_size=g["count"].as_i64().unwrap_or(0);g["edges"]=json!(g["edges"].as_i64().unwrap_or(0)+bundle["count"].as_i64().unwrap_or(0)); } }
        bundle["possible_pairs"]=json!(left_size.checked_mul(right_size).ok_or_else(||AppError::internal("Pair count overflow"))?);
        bundle["pair_scope"]=json!("filtered_directory");
        bundles.push(bundle);
    }
    Ok(
        json!({"summary":{"users":users,"seekers":seekers,"jobs":jobs,"edges":edges,"matched_users":matched_users,"matched_jobs":matched_jobs,"possible_pairs":possible_pairs,"pair_scope":"filtered_directory","state_counts":counts},"groups":{"left":left,"right":right},"bundles":bundles}),
    )
}

fn node_row(r: &rusqlite::Row<'_>, side: Side) -> rusqlite::Result<Value> {
    let id: String = r.get("id")?;
    let mut v = json!({"id":id,"kind":if side==Side::Left{"user"}else{"job"},"group_id":r.get::<_,String>("group_id")?,"relation_count":r.get::<_,i64>("relation_count")?,"created_at":r.get::<_,i64>("created_at")?,"updated_at":r.get::<_,i64>("updated_at")?});
    if side == Side::Left {
        let name: String = r.get("name")?;
        let banned: bool = r.get("banned")?;
        v["label"] = json!(if name.trim().is_empty() {
            format!("用户 {}", short(&id, 8))
        } else {
            name.clone()
        });
        v["name"] = json!(name);
        v["role"] = json!(r.get::<_, String>("role")?);
        v["banned"] = json!(banned);
        v["status"] = json!(if banned { "suspended" } else { "active" });
    } else {
        let title: String = r.get("title")?;
        v["label"] = json!(if title.is_empty() {
            format!("岗位 {}", short(&id, 8))
        } else {
            title.clone()
        });
        v["title"] = json!(title);
        v["status"] = json!(r.get::<_, String>("status")?);
        v["owner_id"] = json!(r.get::<_, String>("owner_id")?);
        v["owner_name"] = json!(r.get::<_, String>("owner_name")?);
        v["headcount"] = json!(r.get::<_, i64>("headcount")?);
        v["salary_cents"] = json!(r.get::<_, i64>("salary_cents")?);
        v["currency"] = json!(r.get::<_, String>("currency")?);
    }
    Ok(v)
}

fn edge_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,String>("edge_id")?,"seeker_id":r.get::<_,String>("seeker_id")?,"job_id":r.get::<_,String>("job_id")?,"status":r.get::<_,String>("match_status")?,"updated_at":r.get::<_,i64>("match_updated_at")?,"left_group_id":r.get::<_,String>("left_group_id")?,"right_group_id":r.get::<_,String>("right_group_id")?}),
    )
}

fn degree(side: Side, alias: &str) -> String {
    let column = if side == Side::Left {
        "seeker_id"
    } else {
        "job_id"
    };
    format!("(SELECT count(*) FROM fm WHERE {column}={alias}.id)")
}

fn focus_node(db: &Connection, s: &Scope, focus: &Focus) -> AppResult<Value> {
    let mut sql = s.sql.clone();
    let id = sql.bind(focus.id.clone());
    let table = if focus.side == Side::Left { "u" } else { "j" };
    let row = db
        .query_row(
            &format!(
                "WITH {} SELECT n.*,{} AS relation_count FROM {table} n WHERE n.id={id}",
                s.cte,
                degree(focus.side, "n")
            ),
            params_from_iter(sql.params.iter()),
            |r| node_row(r, focus.side),
        )
        .optional()?;
    if let Some(row) = row {
        return Ok(row);
    }
    let base = if focus.side == Side::Left {
        "users"
    } else {
        "jobs"
    };
    let exists: bool = db.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {base} WHERE id=?1)"),
        [&focus.id],
        |r| r.get(0),
    )?;
    Err(if exists {
        AppError::conflict("excluded_by_filter: focus is outside the current filters")
    } else {
        AppError::not_found("Focus entity not found")
    })
}

/// Validate a view's focus inside its caller's already-authorized transaction.
/// In particular, view patching must not reacquire AppState's database mutex.
pub(crate) fn validate_focus(db: &Connection, input: &Query) -> AppResult<()> {
    let q = normalized(input)?;
    if let Some(focus) = &q.focus {
        let selected = scope(db, &q)?;
        focus_node(db, &selected, focus)?;
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    fingerprint: String,
    data_rev: i64,
    sort_at: i64,
    id: String,
    seen: i64,
    expires_at: i64,
}

fn cursor_encode(cursor: &Cursor, key: &[u8]) -> AppResult<String> {
    let bytes = serde_json::to_vec(cursor)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(AppError::internal)?;
    mac.update(&bytes);
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&bytes),
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}

fn cursor_decode(
    raw: Option<&str>,
    fingerprint: &str,
    rev: i64,
    key: &[u8],
) -> AppResult<Option<Cursor>> {
    let Some(raw) = raw else { return Ok(None) };
    let invalid =
        || AppError::bad_request("Invalid cursor or changed query; restart from the first page");
    if raw.len() > 4096 {
        return Err(invalid());
    }
    let (body, signature) = raw.split_once('.').ok_or_else(invalid)?;
    let body = URL_SAFE_NO_PAD.decode(body).map_err(|_| invalid())?;
    let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| invalid())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(AppError::internal)?;
    mac.update(&body);
    mac.verify_slice(&signature).map_err(|_| invalid())?;
    let cursor: Cursor = serde_json::from_slice(&body).map_err(|_| invalid())?;
    if cursor.version != 1
        || cursor.fingerprint != fingerprint
        || cursor.id.is_empty()
        || cursor.id.len() > 80
        || cursor.seen < 0
    {
        return Err(invalid());
    }
    if cursor.data_rev != rev || cursor.expires_at < crate::now() {
        return Err(AppError::conflict(
            "resync_required: query snapshot changed or expired",
        ));
    }
    Ok(Some(cursor))
}

fn pagination_condition(sql: &mut Sql, cursor: Option<&Cursor>, time: &str, id: &str) -> String {
    if let Some(cursor) = cursor {
        let at = sql.bind(cursor.sort_at);
        let last = sql.bind(cursor.id.clone());
        format!("({time},{id})<({at},{last})")
    } else {
        "1=1".into()
    }
}

fn page(
    mut rows: Vec<Value>,
    total: i64,
    q: &Query,
    cursor: Option<&Cursor>,
    fingerprint: &str,
    rev: i64,
    key: &[u8],
) -> AppResult<Value> {
    let limit = q.limit.unwrap_or(PAGE_DEFAULT);
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let seen = cursor.map_or(0, |c| c.seen) + rows.len() as i64;
    let next = if more {
        let row = rows
            .last()
            .ok_or_else(|| AppError::internal("Missing pagination row"))?;
        let item = if q.mode == Mode::Neighbors {
            &row["edge"]
        } else {
            row
        };
        let time = if q.mode == Mode::Nodes {
            "created_at"
        } else {
            "updated_at"
        };
        Some(cursor_encode(
            &Cursor {
                version: 1,
                fingerprint: fingerprint.into(),
                data_rev: rev,
                sort_at: item[time]
                    .as_i64()
                    .ok_or_else(|| AppError::internal("Missing pagination time"))?,
                id: item["id"]
                    .as_str()
                    .ok_or_else(|| AppError::internal("Missing pagination identity"))?
                    .into(),
                seen,
                expires_at: cursor.map_or(crate::now() + CURSOR_SECONDS, |c| c.expires_at),
            },
            key,
        )?)
    } else {
        None
    };
    Ok(
        json!({"items":rows,"total":total,"limit":limit,"next_cursor":next,"remaining":(total-seen).max(0)}),
    )
}

/// Shared by HTTP and MCP. All statements see one SQLite read snapshot, and
/// current operator authority is rechecked even when a caller holds a cursor.
pub fn query(s: &AppState, p: &Principal, input: &Query) -> AppResult<Value> {
    let db = s.conn()?;
    let tx = db.unchecked_transaction()?;
    access::ensure_operator(&tx, p)?;
    let q = normalized(input)?;
    let rev = revision(&tx)?;
    if q.data_rev.is_some_and(|expected| expected != rev) {
        return Err(AppError::conflict(
            "resync_required: data changed; refresh the workbench",
        ));
    }
    let mut identity = q.clone();
    identity.cursor = None;
    identity.data_rev = None;
    identity.limit = None;
    let fingerprint = hex::encode(Sha256::digest(serde_json::to_vec(&json!([
        p.actor, p.user_id, p.admin, p.via_ai, identity
    ]))?));
    let key: Vec<u8> = tx.query_row(
        "SELECT cursor_key FROM relation_clock WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    let cursor = cursor_decode(q.cursor.as_deref(), &fingerprint, rev, &key)?;
    let cached = {
        let mut cache = s
            .relations_cache
            .lock()
            .map_err(|_| AppError::internal("Relation cache mutex poisoned"))?;
        cache.reset_revision(rev);
        if q.mode == Mode::Overview {
            cache.get(rev, &fingerprint)?
        } else {
            None
        }
    };
    if let Some(result) = cached {
        tx.commit()?;
        return Ok(result);
    }
    let scope = scope(&tx, &q)?;
    let mut result = if q.mode == Mode::Overview {
        overview(&tx, &scope)?
    } else if q.mode == Mode::Nodes {
        let side = q.side.unwrap();
        let table = if side == Side::Left { "u" } else { "j" };
        let total: i64 = tx.query_row(
            &format!("WITH {} SELECT count(*) FROM {table}", scope.cte),
            params_from_iter(scope.sql.params.iter()),
            |r| r.get(0),
        )?;
        let mut sql = scope.sql.clone();
        let after = pagination_condition(&mut sql, cursor.as_ref(), "n.created_at", "n.id");
        let limit = sql.bind(q.limit.unwrap_or(PAGE_DEFAULT) + 1);
        let rows=tx.prepare(&format!("WITH {} SELECT n.*,{} AS relation_count FROM {table} n WHERE {after} ORDER BY n.created_at DESC,n.id DESC LIMIT {limit}",scope.cte,degree(side,"n")))?.query_map(params_from_iter(sql.params.iter()),|r|node_row(r,side))?.collect::<rusqlite::Result<Vec<_>>>()?;
        page(rows, total, &q, cursor.as_ref(), &fingerprint, rev, &key)?
    } else {
        let mut sql = scope.sql.clone();
        let focus = if q.mode == Mode::Neighbors {
            q.focus.as_ref()
        } else {
            None
        };
        let extra = focus.map(|f| {
            (
                if f.side == Side::Left {
                    "seeker_id"
                } else {
                    "job_id"
                },
                f.id.as_str(),
            )
        });
        let (total, counts) = stats(&tx, &scope, extra)?;
        let mut conditions = vec![pagination_condition(
            &mut sql,
            cursor.as_ref(),
            "f.updated_at",
            "f.id",
        )];
        if let Some((column, id)) = extra {
            conditions.push(format!("f.{column}={}", sql.bind(id.to_owned())));
        }
        let limit = sql.bind(q.limit.unwrap_or(PAGE_DEFAULT) + 1);
        let condition = conditions.join(" AND ");
        let (rows, focus_value) = if let Some(focus) = focus {
            let focus_value = focus_node(&tx, &scope, focus)?;
            let neighbor_side = if focus.side == Side::Left {
                Side::Right
            } else {
                Side::Left
            };
            let (table, column) = if neighbor_side == Side::Left {
                ("u", "seeker_id")
            } else {
                ("j", "job_id")
            };
            let rows=tx.prepare(&format!("WITH {} SELECT n.*,{} AS relation_count,{EDGE_FIELDS} FROM fm f JOIN {table} n ON n.id=f.{column} WHERE {condition} ORDER BY f.updated_at DESC,f.id DESC LIMIT {limit}",scope.cte,degree(neighbor_side,"n")))?.query_map(params_from_iter(sql.params.iter()),|r|Ok(json!({"node":node_row(r,neighbor_side)?,"edge":edge_row(r)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            (rows, Some(focus_value))
        } else {
            let rows=tx.prepare(&format!("WITH {} SELECT {EDGE_FIELDS} FROM fm f WHERE {condition} ORDER BY f.updated_at DESC,f.id DESC LIMIT {limit}",scope.cte))?.query_map(params_from_iter(sql.params.iter()),edge_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            (rows, None)
        };
        let mut result = page(rows, total, &q, cursor.as_ref(), &fingerprint, rev, &key)?;
        result["state_counts"] = counts;
        if let Some(focus) = focus_value {
            result["focus"] = focus;
        }
        result
    };
    result["data_rev"] = json!(rev);
    result["query_key"] = json!(fingerprint);
    if q.mode == Mode::Overview {
        s.relations_cache
            .lock()
            .map_err(|_| AppError::internal("Relation cache mutex poisoned"))?
            .put(rev, &fingerprint, &result)?;
    }
    tx.commit()?;
    Ok(result)
}
