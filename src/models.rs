use crate::error::{AppError, AppResult};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing)]
    pub google_sub: Option<String>,
    pub name: String,
    pub role: String,
    pub employer_paid: bool,
    pub avatar: String,
    pub public_email: String,
    pub resume_md: String,
    pub visible: bool,
    pub banned: bool,
    pub created_at: i64,
    pub updated_at: i64,
}
pub fn user_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    let id: String = r.get("id")?;
    let avatar: String = r.get("avatar")?;
    let avatar = if avatar.is_empty() {
        String::new()
    } else {
        format!("/api/avatars/{id}")
    };
    Ok(User {
        id,
        email: r.get("email")?,
        google_sub: r.get("google_sub")?,
        name: r.get("name")?,
        role: r.get("role")?,
        employer_paid: r.get("employer_paid")?,
        avatar,
        public_email: r.get("public_email")?,
        resume_md: r.get("resume_md")?,
        visible: r.get("visible")?,
        banned: r.get("banned")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}
pub fn load_user(db: &Connection, id: &str) -> AppResult<User> {
    db.query_row("SELECT * FROM users WHERE id=?1", [id], user_row)
        .optional()?
        .ok_or_else(|| AppError::not_found("User not found"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub owner_id: String,
    pub title: String,
    pub jd_md: String,
    pub salary_cents: i64,
    pub currency: String,
    pub currency_decimals: u32,
    pub ai_min_salary_cents: i64,
    pub headcount: i64,
    pub status: String,
    pub delete_count: i64,
    pub edits_used: i64,
    pub version: i64,
    pub created_at: i64,
    pub updated_at: i64,
}
pub fn job_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    let currency: String = r.get("currency")?;
    Ok(Job {
        id: r.get("id")?,
        owner_id: r.get("owner_id")?,
        title: r.get("title")?,
        jd_md: r.get("jd_md")?,
        salary_cents: r.get("salary_cents")?,
        currency_decimals: crate::config::currency_decimals(&currency).unwrap_or(2),
        currency,
        ai_min_salary_cents: r.get("ai_min_salary_cents")?,
        headcount: r.get("headcount")?,
        status: r.get("status")?,
        delete_count: r.get("delete_count")?,
        edits_used: r.get("edits_used")?,
        version: r.get("version")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}
pub fn load_job(db: &Connection, id: &str) -> AppResult<Job> {
    db.query_row("SELECT * FROM jobs WHERE id=?1", [id], job_row)
        .optional()?
        .ok_or_else(|| AppError::not_found("Job not found"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub user_id: String,
    pub kind: String,
    pub job_id: Option<String>,
    pub amount_cents: i64,
    pub refunded_cents: i64,
    pub currency: String,
    pub currency_decimals: u32,
    pub provider: String,
    pub live_mode: bool,
    pub status: String,
    pub snapshot_json: String,
    pub provider_session: Option<String>,
    pub checkout_url: Option<String>,
    #[serde(default)]
    pub checkout_expires_at: Option<i64>,
    pub created_at: i64,
    pub paid_at: Option<i64>,
}
pub fn order_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Order> {
    let currency: String = r.get("currency")?;
    Ok(Order {
        id: r.get("id")?,
        user_id: r.get("user_id")?,
        kind: r.get("kind")?,
        job_id: r.get("job_id")?,
        amount_cents: r.get("amount_cents")?,
        refunded_cents: r.get("refunded_cents")?,
        currency_decimals: crate::config::currency_decimals(&currency).unwrap_or(2),
        currency,
        provider: r.get("provider")?,
        live_mode: r.get("live_mode")?,
        status: r.get("status")?,
        snapshot_json: r.get("snapshot_json")?,
        provider_session: r.get("provider_session")?,
        checkout_url: r.get("checkout_url")?,
        checkout_expires_at: r.get("checkout_expires_at")?,
        created_at: r.get("created_at")?,
        paid_at: r.get("paid_at")?,
    })
}
pub fn load_order(db: &Connection, id: &str) -> AppResult<Order> {
    db.query_row("SELECT * FROM orders WHERE id=?1", [id], order_row)
        .optional()?
        .ok_or_else(|| AppError::not_found("Order not found"))
}
