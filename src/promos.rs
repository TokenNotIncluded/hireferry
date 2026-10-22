//! Approved promotion codes. Quotas and fulfillment share the commerce transaction.
use crate::{
    AppError, AppResult, AppState,
    access::{self, Principal},
    commerce,
    db::audit,
    models, now, safety, user_id,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, patch, post},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const RESERVATION_SECONDS: i64 = 24 * 3600;
const MAX_USES: i64 = 1_000_000;

pub fn migrate(db: &Connection) -> AppResult<()> {
    // The caller holds the schema-version transaction.
    db.execute_batch(include_str!("promos.sql"))?;
    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/promo-applications", get(my_applications).post(apply))
        .route("/api/promo-codes/preview", post(preview))
        .route("/api/promo-codes/redeem-activation", post(activate))
        .route("/api/admin/promo-applications", get(admin_applications))
        .route("/api/admin/promo-applications/{id}/review", post(review))
        .route("/api/admin/promo-codes", get(admin_codes).post(create))
        .route("/api/admin/promo-codes/{id}", patch(update))
        .route("/api/orders/{id}/cancel", post(cancel))
}

fn one() -> i64 {
    1
}
fn full_discount() -> i64 {
    100
}
fn enabled() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CodeInput {
    pub code: Option<String>,
    pub label: String,
    pub free_activation_limit: i64,
    pub publish_limit: i64,
    pub discount_percent: i64,
    pub expires_at: Option<i64>,
    pub enabled: bool,
}
impl Default for CodeInput {
    fn default() -> Self {
        Self {
            code: None,
            label: String::new(),
            free_activation_limit: one(),
            publish_limit: one(),
            discount_percent: full_discount(),
            expires_at: None,
            enabled: enabled(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PromoCode {
    pub id: String,
    pub code: String,
    pub label: String,
    pub free_activation_limit: i64,
    pub activation_used: i64,
    pub activation_remaining: i64,
    pub publish_limit: i64,
    pub publish_used: i64,
    pub publish_reserved: i64,
    pub publish_remaining: i64,
    pub discount_percent: i64,
    pub expires_at: Option<i64>,
    pub enabled: bool,
    pub created_at: i64,
}
fn code_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PromoCode> {
    let activation_limit: i64 = row.get("free_activation_limit")?;
    let activation_used: i64 = row.get("activation_used")?;
    let publish_limit: i64 = row.get("publish_limit")?;
    let publish_used: i64 = row.get("publish_used")?;
    let publish_reserved: i64 = row.get("publish_reserved")?;
    Ok(PromoCode {
        id: row.get("id")?,
        code: row.get("code")?,
        label: row.get("label")?,
        free_activation_limit: activation_limit,
        activation_used,
        activation_remaining: (activation_limit - activation_used).max(0),
        publish_limit,
        publish_used,
        publish_reserved,
        publish_remaining: (publish_limit - publish_used - publish_reserved).max(0),
        discount_percent: row.get("discount_percent")?,
        expires_at: row.get("expires_at")?,
        enabled: row.get("enabled")?,
        created_at: row.get("created_at")?,
    })
}

pub fn normalize_code(raw: &str) -> AppResult<String> {
    let code = raw.trim();
    if !(6..=64).contains(&code.len())
        || !code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(AppError::bad_request(
            "Use a promotion code of 6–64 letters, numbers, hyphens or underscores",
        ));
    }
    Ok(code.to_ascii_uppercase())
}

fn load_code(db: &Connection, id: &str) -> AppResult<PromoCode> {
    db.query_row(
        "SELECT * FROM promo_code_balances WHERE id=?1",
        [id],
        code_row,
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("Promotion code not found"))
}

pub(crate) fn available_code(db: &Connection, raw: &str, purpose: &str) -> AppResult<PromoCode> {
    let code = normalize_code(raw)?;
    let code = db
        .query_row(
            "SELECT * FROM promo_code_balances WHERE code=?1",
            [code],
            code_row,
        )
        .optional()?
        .ok_or_else(|| AppError::bad_request("Promotion code is unavailable"))?;
    if !code.enabled || code.expires_at.is_some_and(|end| end <= now()) {
        return Err(AppError::conflict("Promotion code is disabled or expired"));
    }
    let remaining = match purpose {
        "activation" => code.activation_remaining,
        "publish" => code.publish_remaining,
        _ => return Err(AppError::bad_request("Choose activation or publish")),
    };
    if remaining == 0 {
        return Err(AppError::conflict(if purpose == "activation" {
            "This promotion code has no free activations remaining"
        } else {
            "This promotion code has no publication uses remaining"
        }));
    }
    Ok(code)
}

fn validate_settings(input: &CodeInput) -> AppResult<()> {
    safety::bounded_text(&input.label, 120, "Promotion label")?;
    if !(0..=MAX_USES).contains(&input.free_activation_limit)
        || !(0..=MAX_USES).contains(&input.publish_limit)
        || !(0..=100).contains(&input.discount_percent)
    {
        return Err(AppError::bad_request(
            "Promotion limits must be 0–1000000 and the discount must be 0–100 percent",
        ));
    }
    if input
        .expires_at
        .is_some_and(|end| end <= now() || end > 253_402_300_799)
    {
        return Err(AppError::bad_request(
            "Promotion expiry must be a future Unix timestamp",
        ));
    }
    Ok(())
}

fn insert_code(
    db: &Connection,
    actor: &str,
    application: Option<&str>,
    input: &CodeInput,
) -> AppResult<PromoCode> {
    validate_settings(input)?;
    let raw = input
        .code
        .clone()
        .unwrap_or_else(|| format!("HF-{}", uuid::Uuid::new_v4().simple()));
    let code = normalize_code(&raw)?;
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM promo_codes WHERE code=?1)",
        [&code],
        |row| row.get(0),
    )?;
    if exists {
        return Err(AppError::conflict("Promotion code already exists"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO promo_codes(id,code,label,application_id,free_activation_limit,publish_limit,discount_percent,expires_at,enabled,created_by,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11)",
        params![id,code,input.label.trim(),application,input.free_activation_limit,input.publish_limit,input.discount_percent,input.expires_at,input.enabled,actor,now()])?;
    audit(db, actor, "promo_create", &id, &json!({"free_activation_limit":input.free_activation_limit,"publish_limit":input.publish_limit,"discount_percent":input.discount_percent}).to_string())?;
    load_code(db, &id)
}

pub fn create_code(s: &AppState, p: &Principal, input: &CodeInput) -> AppResult<PromoCode> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
    let code = insert_code(&tx, &p.actor, None, input)?;
    tx.commit()?;
    Ok(code)
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CodePatch {
    pub label: Option<String>,
    pub free_activation_limit: Option<i64>,
    pub publish_limit: Option<i64>,
    pub discount_percent: Option<i64>,
    pub expires_at: Option<i64>,
    pub clear_expiry: bool,
    pub enabled: Option<bool>,
}
pub fn update_code(
    s: &AppState,
    p: &Principal,
    id: &str,
    patch: &CodePatch,
) -> AppResult<PromoCode> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
    expire_reservations(&tx)?;
    let old = load_code(&tx, id)?;
    if patch.clear_expiry && patch.expires_at.is_some() {
        return Err(AppError::bad_request("Choose an expiry or clear it"));
    }
    let input = CodeInput {
        code: None,
        label: patch.label.clone().unwrap_or(old.label),
        free_activation_limit: patch
            .free_activation_limit
            .unwrap_or(old.free_activation_limit),
        publish_limit: patch.publish_limit.unwrap_or(old.publish_limit),
        discount_percent: patch.discount_percent.unwrap_or(old.discount_percent),
        expires_at: if patch.clear_expiry {
            None
        } else {
            patch.expires_at.or(old.expires_at)
        },
        enabled: patch.enabled.unwrap_or(old.enabled),
    };
    // An expired code may still be disabled or have its label/limits corrected.
    let mut validation = input.clone();
    if patch.expires_at.is_none() {
        validation.expires_at = None;
    }
    validate_settings(&validation)?;
    if input.free_activation_limit < old.activation_used
        || input.publish_limit < old.publish_used + old.publish_reserved
    {
        return Err(AppError::conflict(
            "Limits cannot be lower than consumed and reserved uses",
        ));
    }
    tx.execute("UPDATE promo_codes SET label=?2,free_activation_limit=?3,publish_limit=?4,discount_percent=?5,expires_at=?6,enabled=?7,updated_at=?8 WHERE id=?1",
        params![id,input.label.trim(),input.free_activation_limit,input.publish_limit,input.discount_percent,input.expires_at,input.enabled,now()])?;
    audit(&tx, &p.actor, "promo_update", id, &json!({"free_activation_limit":input.free_activation_limit,"publish_limit":input.publish_limit,"discount_percent":input.discount_percent,"enabled":input.enabled,"expires_at":input.expires_at}).to_string())?;
    let code = load_code(&tx, id)?;
    tx.commit()?;
    Ok(code)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplicationInput {
    reason: String,
}

fn active_employer_or_unassigned(db: &Connection, uid: &str) -> AppResult<models::User> {
    let user = models::load_user(db, uid)?;
    if user.banned || !matches!(user.role.as_str(), "employer" | "unassigned") {
        return Err(AppError::forbidden(
            "An active employer or unassigned account is required",
        ));
    }
    Ok(user)
}

fn application(db: &Connection, id: &str) -> AppResult<Value> {
    let mut result = db.query_row("SELECT * FROM promo_applications WHERE id=?1", [id], |row| {
        Ok(json!({"id":row.get::<_,String>("id")?,"user_id":row.get::<_,String>("user_id")?,"reason":row.get::<_,String>("reason")?,"status":row.get::<_,String>("status")?,"review_note":row.get::<_,String>("review_note")?,"created_at":row.get::<_,i64>("created_at")?,"reviewed_at":row.get::<_,Option<i64>>("reviewed_at")?}))
    }).optional()?.ok_or_else(|| AppError::not_found("Promotion application not found"))?;
    let code = db
        .query_row(
            "SELECT * FROM promo_code_balances WHERE application_id=?1",
            [id],
            code_row,
        )
        .optional()?;
    result["promo_code"] = serde_json::to_value(code)?;
    Ok(result)
}

pub fn submit_application(s: &AppState, uid: &str, reason: &str) -> AppResult<Value> {
    let reason = reason.trim();
    safety::bounded_text(reason, 2000, "Application reason")?;
    if reason.is_empty() {
        return Err(AppError::bad_request("Tell us what you are hiring for"));
    }
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    active_employer_or_unassigned(&tx, uid)?;
    let existing: Option<(String, String)> = tx
        .query_row(
            "SELECT id,reason FROM promo_applications WHERE user_id=?1 AND status='pending'",
            [uid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((id, old_reason)) = existing {
        if old_reason != reason {
            return Err(AppError::conflict(
                "You already have an application awaiting review",
            ));
        }
        return application(&tx, &id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO promo_applications(id,user_id,reason,created_at) VALUES(?1,?2,?3,?4)",
        params![id, uid, reason, now()],
    )?;
    audit(&tx, uid, "promo_apply", &id, "")?;
    let result = application(&tx, &id)?;
    tx.commit()?;
    Ok(result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewInput {
    pub decision: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub settings: CodeInput,
}
pub fn review_application(
    s: &AppState,
    p: &Principal,
    id: &str,
    input: &ReviewInput,
) -> AppResult<Value> {
    if !matches!(input.decision.as_str(), "approved" | "rejected") {
        return Err(AppError::bad_request("Approve or reject the application"));
    }
    safety::bounded_text(&input.note, 2000, "Review note")?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
    let old = application(&tx, id)?;
    if old["status"] != "pending" {
        if old["status"] == input.decision {
            return Ok(old);
        }
        return Err(AppError::conflict(
            "This application has already been reviewed",
        ));
    }
    if input.decision == "approved" {
        active_employer_or_unassigned(&tx, old["user_id"].as_str().unwrap_or_default())?;
        insert_code(&tx, &p.actor, Some(id), &input.settings)?;
    }
    tx.execute("UPDATE promo_applications SET status=?2,review_note=?3,reviewed_by=?4,reviewed_at=?5 WHERE id=?1", params![id,input.decision,input.note.trim(),p.actor,now()])?;
    audit(&tx, &p.actor, "promo_review", id, &input.decision)?;
    let result = application(&tx, id)?;
    tx.commit()?;
    Ok(result)
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ListInput {
    status: Option<String>,
    page: Option<i64>,
}
fn page(input: &ListInput) -> AppResult<(i64, i64)> {
    let page = input.page.unwrap_or(1);
    if !(1..=1_000_000).contains(&page) {
        return Err(AppError::bad_request("Invalid page"));
    }
    if input
        .status
        .as_deref()
        .is_some_and(|s| !matches!(s, "pending" | "approved" | "rejected"))
    {
        return Err(AppError::bad_request("Invalid application status"));
    }
    Ok((page, (page - 1) * 25))
}
fn list_applications(
    s: &AppState,
    p: Option<&Principal>,
    uid: Option<&str>,
    input: &ListInput,
) -> AppResult<Value> {
    let (page, offset) = page(input)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(p) = p {
        access::ensure_admin(&tx, p)?;
    } else {
        active_employer_or_unassigned(
            &tx,
            uid.ok_or_else(|| AppError::unauthorized("Sign in required"))?,
        )?;
    }
    expire_reservations(&tx)?;
    let condition = "(?1 IS NULL OR user_id=?1) AND (?2 IS NULL OR status=?2)";
    let total: i64 = tx.query_row(
        &format!("SELECT count(*) FROM promo_applications WHERE {condition}"),
        params![uid, input.status],
        |r| r.get(0),
    )?;
    let ids = tx.prepare(&format!("SELECT id FROM promo_applications WHERE {condition} ORDER BY created_at DESC,id LIMIT 25 OFFSET ?3"))?
        .query_map(params![uid,input.status,offset], |r| r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let items = ids
        .iter()
        .map(|id| application(&tx, id))
        .collect::<AppResult<Vec<_>>>()?;
    tx.commit()?;
    Ok(json!({"items":items,"total":total,"page":page,"limit":25}))
}

/// Exact integer minor units; 100 is a complete reduction, not a price multiplier.
pub fn discounted_amount(original: i64, percent: i64) -> AppResult<i64> {
    if original < 0 || !(0..=100).contains(&percent) {
        return Err(AppError::bad_request("Invalid discount"));
    }
    original
        .checked_mul(100 - percent)
        .and_then(|v| v.checked_add(50))
        .map(|v| v / 100)
        .ok_or_else(|| AppError::bad_request("Discounted amount is too large"))
}

/// Called inside a write transaction before spending quotas, reading orders or
/// applying receipts. The trigger releases reservations on every terminal order.
pub(crate) fn expire_reservations(db: &Connection) -> AppResult<()> {
    db.execute(
        "UPDATE orders SET status='expired',checkout_url=NULL WHERE rowid IN
         (SELECT o.rowid FROM promo_redemptions r JOIN orders o ON o.id=r.order_id
          WHERE r.status='reserved' AND r.reserved_until<=?1 AND o.status='pending'
          ORDER BY r.reserved_until LIMIT ?2)",
        params![now(), crate::maintenance::DEFAULT_BATCH_SIZE],
    )?;
    Ok(())
}

/// Business decisions about one order must not depend on its position in a
/// global housekeeping backlog. This changes at most one order/redemption.
pub(crate) fn expire_order_reservation(db: &Connection, id: &str) -> AppResult<()> {
    db.execute(
        "UPDATE orders SET status='expired',checkout_url=NULL WHERE id=?1 AND status='pending'
         AND EXISTS(SELECT 1 FROM promo_redemptions WHERE order_id=?1
                    AND status='reserved' AND reserved_until<=?2)",
        params![id, now()],
    )?;
    Ok(())
}

pub(crate) fn request_hash(value: &Value) -> String {
    hex::encode(Sha256::digest(value.to_string().as_bytes()))
}
pub(crate) fn validate_request_key(key: Option<&str>) -> AppResult<&str> {
    let key = key.ok_or_else(|| {
        AppError::bad_request("A request_key is required when publishing with a promotion code")
    })?;
    if !(8..=128).contains(&key.len())
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(AppError::bad_request(
            "Use a request_key of 8–128 letters, numbers, hyphens or underscores",
        ));
    }
    Ok(key)
}
pub(crate) fn replay_publication(
    db: &Connection,
    uid: &str,
    key: &str,
    hash: &str,
) -> AppResult<Option<(models::Job, models::Order)>> {
    let found: Option<(String,String)> = db.query_row("SELECT order_id,request_hash FROM promo_redemptions WHERE user_id=?1 AND purpose='publish' AND request_key=?2", params![uid,key], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((id, previous)) = found {
        if previous != hash {
            return Err(AppError::conflict(
                "This request_key was already used for different publication details",
            ));
        }
        expire_order_reservation(db, &id)?;
        let order = models::load_order(db, &id)?;
        let job = models::load_job(
            db,
            order
                .job_id
                .as_deref()
                .ok_or_else(|| AppError::internal("Missing promotion job"))?,
        )?;
        return Ok(Some((job, order)));
    }
    Ok(None)
}

pub(crate) fn record_publication(
    db: &Connection,
    code: &PromoCode,
    order: &models::Order,
    key: &str,
    hash: &str,
) -> AppResult<()> {
    let timestamp = now();
    let deadline = code
        .expires_at
        .unwrap_or(i64::MAX)
        .min(timestamp + RESERVATION_SECONDS);
    if deadline <= timestamp {
        return Err(AppError::conflict(
            "Promotion code expired before publication; request a new quote",
        ));
    }
    let reserved_until = (order.amount_cents > 0).then_some(deadline);
    db.execute("INSERT INTO promo_redemptions(order_id,code_id,user_id,purpose,status,request_key,request_hash,reserved_until,created_at) VALUES(?1,?2,?3,'publish',?4,?5,?6,?7,?8)",
        params![order.id,code.id,order.user_id,if order.amount_cents == 0 {"consumed"} else {"reserved"},key,hash,reserved_until,timestamp])?;
    if let Some(deadline) = reserved_until {
        // The order exposes the same immutable deadline as its reservation;
        // a hosted payment link can remain valid beyond this promotion.
        db.execute(
            "UPDATE orders SET snapshot_json=json_set(snapshot_json,'$.promo.reserved_until',?2) WHERE id=?1",
            params![order.id, deadline],
        )?;
    }
    audit(db, &order.user_id, "promo_publish", &order.id, &json!({"code_id":code.id,"status":if order.amount_cents==0 {"consumed"} else {"reserved"}}).to_string())?;
    Ok(())
}

pub fn redeem_activation(
    s: &AppState,
    uid: &str,
    raw: &str,
    requested_currency: Option<&str>,
) -> AppResult<(models::User, Option<models::Order>)> {
    let raw = normalize_code(raw)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let user = active_employer_or_unassigned(&tx, uid)?;
    if user.employer_paid {
        let order = tx.query_row("SELECT * FROM orders WHERE user_id=?1 AND kind='onboarding' AND status IN ('paid','partially_refunded') ORDER BY created_at DESC,id LIMIT 1", [uid], models::order_row).optional()?;
        return Ok((user, order));
    }
    let currency = requested_currency.unwrap_or(&s.cfg.pricing.currency);
    let original = s.cfg.pricing.onboarding_amount(currency)?;
    // Preserve unresolved real money; an administrator must reconcile it first.
    let unresolved: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM orders o JOIN ledger l ON l.order_id=o.id WHERE o.user_id=?1 AND o.kind='onboarding' AND o.refunded_cents<l.amount_cents)", [uid], |r| r.get(0))?;
    if unresolved {
        return Err(AppError::conflict(
            "An activation payment needs review before using a promotion code",
        ));
    }
    let code = available_code(&tx, &raw, "activation")?;
    tx.execute("UPDATE orders SET status='cancelled',checkout_url=NULL WHERE user_id=?1 AND kind='onboarding' AND status='pending'", [uid])?;
    tx.execute(
        "UPDATE users SET role='employer',employer_paid=1,updated_at=?2 WHERE id=?1",
        params![uid, now()],
    )?;
    let order = commerce::insert_order(
        &tx,
        uid,
        "onboarding",
        None,
        0,
        "free",
        false,
        currency,
        json!({"promo":{"code_id":code.id,"code":code.code,"discount_percent":100,"original_amount_cents":original}}),
    )?;
    tx.execute("INSERT INTO promo_redemptions(order_id,code_id,user_id,purpose,status,request_key,request_hash,created_at) VALUES(?1,?2,?3,'activation','consumed',?1,'',?4)", params![order.id,code.id,uid,now()])?;
    audit(
        &tx,
        uid,
        "promo_activate",
        &order.id,
        &json!({"code_id":code.id}).to_string(),
    )?;
    let user = models::load_user(&tx, uid)?;
    tx.commit()?;
    Ok((user, Some(order)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewInput {
    code: String,
    purpose: String,
    currency: Option<String>,
    salary_cents: Option<i64>,
}
async fn preview(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<PreviewInput>,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, true)?;
    let currency = input.currency.as_deref().unwrap_or(&s.cfg.pricing.currency);
    let (original, percent) = match input.purpose.as_str() {
        "activation" => (s.cfg.pricing.onboarding_amount(currency)?, 100),
        "publish" => (
            commerce::publication_amount(
                &s,
                input.salary_cents.ok_or_else(|| {
                    AppError::bad_request("Salary is required for a publication quote")
                })?,
                currency,
            )?,
            0,
        ),
        _ => return Err(AppError::bad_request("Choose activation or publish")),
    };
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let user = active_employer_or_unassigned(&tx, &uid)?;
    if input.purpose == "publish" && !user.employer_paid {
        return Err(AppError::forbidden("Activate your employer account first"));
    }
    expire_reservations(&tx)?;
    let code = available_code(&tx, &input.code, &input.purpose)?;
    let percent = if input.purpose == "publish" {
        code.discount_percent
    } else {
        percent
    };
    let amount = discounted_amount(original, percent)?;
    tx.commit()?;
    Ok(Json(
        json!({"promo_code":code,"original_amount_cents":original,"discount_percent":percent,"amount_cents":amount,"currency":currency,"currency_decimals":crate::config::currency_decimals(currency)}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivationInput {
    code: String,
    currency: Option<String>,
}
async fn activate(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<ActivationInput>,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, true)?;
    let (user, order) = redeem_activation(&s, &uid, &input.code, input.currency.as_deref())?;
    Ok(Json(json!({"user":user,"order":order})))
}
async fn apply(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<ApplicationInput>,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, true)?;
    Ok(Json(submit_application(&s, &uid, &input.reason)?))
}
async fn my_applications(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(input): Query<ListInput>,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, false)?;
    Ok(Json(list_applications(&s, None, Some(&uid), &input)?))
}
async fn admin_applications(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(input): Query<ListInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(list_applications(&s, Some(&p), None, &input)?))
}
async fn review(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<ReviewInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(review_application(&s, &p, &id, &input)?))
}
async fn create(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<CodeInput>,
) -> AppResult<Json<PromoCode>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(create_code(&s, &p, &input)?))
}
async fn update(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<CodePatch>,
) -> AppResult<Json<PromoCode>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(update_code(&s, &p, &id, &input)?))
}
async fn admin_codes(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(input): Query<ListInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    let (page, offset) = page(&input)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, &p)?;
    expire_reservations(&tx)?;
    let total: i64 = tx.query_row("SELECT count(*) FROM promo_codes", [], |r| r.get(0))?;
    let items = tx
        .prepare(
            "SELECT * FROM promo_code_balances ORDER BY created_at DESC,id LIMIT 25 OFFSET ?1",
        )?
        .query_map([offset], code_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    tx.commit()?;
    Ok(Json(
        json!({"items":items,"total":total,"page":page,"limit":25}),
    ))
}
async fn cancel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<models::Order>> {
    let uid = user_id(&s, &h, true)?;
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    active_employer_or_unassigned(&tx, &uid)?;
    expire_order_reservation(&tx, &id)?;
    let order = models::load_order(&tx, &id)?;
    if order.user_id != uid {
        return Err(AppError::not_found("Order not found"));
    }
    let promo: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM promo_redemptions WHERE order_id=?1 AND purpose='publish')",
        [&id],
        |r| r.get(0),
    )?;
    if !promo {
        return Err(AppError::bad_request(
            "Only promotion publication orders can be cancelled here",
        ));
    }
    if !matches!(order.status.as_str(), "pending" | "cancelled" | "expired") {
        return Err(AppError::conflict("This order cannot be cancelled"));
    }
    if order.status == "pending" {
        tx.execute(
            "UPDATE orders SET status='cancelled',checkout_url=NULL WHERE id=?1",
            [&id],
        )?;
        audit(&tx, &uid, "promo_order_cancel", &id, "")?;
    }
    let order = models::load_order(&tx, &id)?;
    tx.commit()?;
    Ok(Json(order))
}
