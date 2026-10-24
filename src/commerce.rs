//! All paid changes pass through this module, from both the website and MCP.
use crate::{
    AppState,
    config::Pricing,
    db::audit,
    error::{AppError, AppResult},
    models::{self, Job, Order, User},
    now,
    payments::{PaymentReceipt, RefundReceipt},
    promos,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

// Keep an unstarted order within the provider idempotency window. Once checkout
// starts, its actual lifetime replaces this deadline; retries never extend it.
const UNSTARTED_ORDER_TTL: i64 = 23 * 3600;

/// Runs after the core orders/ledger tables, including for existing databases.
pub fn migrate(db: &Connection) -> AppResult<()> {
    let columns = {
        let mut query = db.prepare("PRAGMA table_info(orders)")?;
        query
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if !columns.iter().any(|column| column == "refunded_cents") {
        db.execute_batch("ALTER TABLE orders ADD COLUMN refunded_cents INTEGER NOT NULL DEFAULT 0 CHECK(refunded_cents>=0);")?;
    }
    migrate_free_orders(db)?;
    let has_checkout_expiry: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('orders') WHERE name='checkout_expires_at')",
        [],
        |row| row.get(0),
    )?;
    if !has_checkout_expiry {
        db.execute_batch("ALTER TABLE orders ADD COLUMN checkout_expires_at INTEGER;")?;
    }
    // Older versions cached hosted URLs forever. Preserve the old session for
    // receipt matching, but reconstruct its deadline from the first known start.
    db.execute(
        "UPDATE orders SET checkout_expires_at=CASE WHEN checkout_started_at IS NOT NULL OR checkout_url IS NOT NULL OR provider_session IS NOT NULL THEN COALESCE(checkout_started_at,created_at)+CASE provider WHEN 'waffo' THEN ?1 ELSE ?2 END ELSE created_at+?3 END WHERE kind='onboarding' AND status='pending' AND checkout_expires_at IS NULL",
        params![crate::payments::checkout_ttl_seconds("waffo"), crate::payments::checkout_ttl_seconds("stripe"), UNSTARTED_ORDER_TTL],
    )?;
    db.execute_batch(r#"
 CREATE TABLE IF NOT EXISTS payment_refunds(
 provider TEXT NOT NULL,refund_id TEXT NOT NULL,order_id TEXT NOT NULL REFERENCES orders(id),
 payment_id TEXT NOT NULL,provider_order_id TEXT NOT NULL,amount_cents INTEGER NOT NULL CHECK(amount_cents>0),
 currency TEXT NOT NULL,live_mode INTEGER NOT NULL CHECK(live_mode IN (0,1)),
 status TEXT NOT NULL CHECK(status IN ('failed','succeeded')),reason TEXT NOT NULL DEFAULT '',
 created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,succeeded_at INTEGER,
 PRIMARY KEY(provider,live_mode,refund_id),
 CHECK((status='succeeded' AND succeeded_at IS NOT NULL) OR (status='failed' AND succeeded_at IS NULL)));
 CREATE INDEX IF NOT EXISTS refunds_order ON payment_refunds(order_id,status);
 CREATE INDEX IF NOT EXISTS refunds_revenue ON payment_refunds(live_mode,status,succeeded_at);
 CREATE TABLE IF NOT EXISTS refund_events(
 provider TEXT NOT NULL,event_id TEXT NOT NULL,event_type TEXT NOT NULL CHECK(event_type IN ('refund.succeeded','refund.failed')),
 refund_id TEXT NOT NULL,order_id TEXT NOT NULL REFERENCES orders(id),payment_id TEXT NOT NULL,
 amount_cents INTEGER NOT NULL CHECK(amount_cents>0),currency TEXT NOT NULL,
 live_mode INTEGER NOT NULL CHECK(live_mode IN (0,1)),created_at INTEGER NOT NULL,
 PRIMARY KEY(provider,live_mode,event_id,event_type));
 "#)?;
    Ok(())
}

/// Expiration releases the unique pending activation slot without deleting its
/// payment history. A signed, late receipt can still activate an unpaid account.
pub(crate) fn expire_onboarding_orders(db: &Connection, uid: &str) -> AppResult<usize> {
    Ok(db.execute(
        "UPDATE orders SET status='expired',checkout_url=NULL WHERE user_id=?1 AND kind='onboarding' AND status='pending' AND checkout_expires_at<=?2",
        params![uid, now()],
    )?)
}

fn activation_payment_recorded(db: &Connection, uid: &str) -> AppResult<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM orders WHERE user_id=?1 AND kind='onboarding' AND status IN ('paid','paid_needs_review','partially_refunded') AND amount_cents>refunded_cents)",
        [uid],
        |row| row.get(0),
    )?)
}

fn migrate_free_orders(db: &Connection) -> AppResult<()> {
    let sql: String = db.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='orders'",
        [],
        |row| row.get(0),
    )?;
    if !sql
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase()
        .contains("check(amount_cents>0)")
    {
        return Ok(());
    }
    // SQLite cannot alter a CHECK constraint. Rebuild only the parent table,
    // keeping its name and all child references unchanged. No account, payment,
    // or historical currency is rewritten. Webhook triggers are recreated by
    // the next migration stage, after this transaction commits.
    let foreign_keys: bool = db.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    db.pragma_update(None, "foreign_keys", false)?;
    let result = db.execute_batch(r#"
BEGIN IMMEDIATE;
DROP TRIGGER IF EXISTS webhook_fanout;
CREATE TABLE orders_free_migration(id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),kind TEXT NOT NULL CHECK(kind IN ('onboarding','publish','edit')),job_id TEXT REFERENCES jobs(id),amount_cents INTEGER NOT NULL CHECK(amount_cents>=0),currency TEXT NOT NULL DEFAULT 'CNY',provider TEXT NOT NULL,live_mode INTEGER NOT NULL DEFAULT 0,status TEXT NOT NULL DEFAULT 'pending',snapshot_json TEXT NOT NULL DEFAULT '{}',provider_session TEXT,checkout_url TEXT,checkout_started_at INTEGER,created_at INTEGER NOT NULL,paid_at INTEGER,refunded_cents INTEGER NOT NULL DEFAULT 0 CHECK(refunded_cents>=0));
INSERT INTO orders_free_migration SELECT id,user_id,kind,job_id,amount_cents,currency,provider,live_mode,status,snapshot_json,provider_session,checkout_url,checkout_started_at,created_at,paid_at,refunded_cents FROM orders;
DROP TABLE orders;
ALTER TABLE orders_free_migration RENAME TO orders;
CREATE UNIQUE INDEX one_job_pending ON orders(job_id) WHERE status='pending';
CREATE UNIQUE INDEX one_onboarding_pending ON orders(user_id) WHERE kind='onboarding' AND status='pending';
CREATE INDEX orders_user ON orders(user_id,created_at);
COMMIT;
"#);
    if result.is_err() {
        let _ = db.execute_batch("ROLLBACK;");
    }
    db.pragma_update(None, "foreign_keys", foreign_keys)?;
    result?;
    Ok(())
}

pub fn fee(salary_cents: i64, basis_points: i64) -> AppResult<i64> {
    if salary_cents < 1 || salary_cents > 1_000_000_000 || basis_points <= 0 || basis_points > 10000
    {
        return Err(AppError::bad_request(
            "Invalid base monthly salary or fee rate",
        ));
    }
    salary_cents
        .checked_mul(basis_points)
        .and_then(|v| v.checked_add(5000))
        .map(|v| v / 10000)
        .filter(|v| *v > 0)
        .ok_or_else(|| AppError::bad_request("Invalid fee"))
}
fn configured_fee(salary: i64, mode: &str, bps: i64, fixed: i64) -> AppResult<i64> {
    if !(1..=1_000_000_000).contains(&salary) {
        return Err(AppError::bad_request("Invalid base monthly salary"));
    }
    match mode {
        "salary_percent" => fee(salary, bps),
        "fixed" if fixed > 0 => Ok(fixed),
        "free" => Ok(0),
        _ => Err(AppError::bad_request("Invalid recorded fee policy")),
    }
}
pub fn provider_enabled(state: &AppState, provider: &str) -> AppResult<()> {
    match provider {
        "stripe" if state.cfg.payments.stripe.enabled => Ok(()),
        "waffo" if state.cfg.payments.waffo.enabled => Ok(()),
        _ => Err(AppError::unavailable(
            "Payment provider is not enabled. Please contact the service.",
        )),
    }
}
pub fn provider_currency_enabled(
    state: &AppState,
    provider: &str,
    currency: &str,
) -> AppResult<()> {
    provider_enabled(state, provider)?;
    if !state.cfg.payments.permits_currency(provider, currency) {
        return Err(AppError::bad_request(
            "This payment provider is not enabled for the order currency",
        ));
    }
    Ok(())
}
fn provider_live(state: &AppState, db: &Connection, provider: &str) -> AppResult<bool> {
    Ok(match provider {
        "stripe" => state.cfg.payments.stripe.live_mode,
        "waffo" => crate::payments::waffo_mode(state, db)? == "prod",
        _ => false,
    })
}

fn employer(db: &Connection, id: &str) -> AppResult<User> {
    let u = models::load_user(db, id)?;
    if u.banned || u.role != "employer" || !u.employer_paid {
        return Err(AppError::forbidden("A paid employer account is required"));
    }
    Ok(u)
}
pub(crate) fn insert_order(
    db: &Connection,
    user: &str,
    kind: &str,
    job: Option<&str>,
    amount: i64,
    provider: &str,
    live_mode: bool,
    currency: &str,
    snapshot: Value,
) -> AppResult<Order> {
    let id = uuid::Uuid::new_v4().to_string();
    let timestamp = now();
    let expires = (amount > 0 && kind == "onboarding").then_some(timestamp + UNSTARTED_ORDER_TTL);
    db.execute("INSERT INTO orders(id,user_id,kind,job_id,amount_cents,currency,provider,live_mode,snapshot_json,status,created_at,paid_at,checkout_expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![id,user,kind,job,amount,currency,if amount == 0 {"free"} else {provider},live_mode,snapshot.to_string(),if amount == 0 {"paid"} else {"pending"},timestamp,if amount == 0 {Some(timestamp)} else {None},expires])?;
    models::load_order(db, &id)
}
pub fn choose_role(
    state: &AppState,
    uid: &str,
    role: &str,
    provider: Option<&str>,
) -> AppResult<(User, Option<Order>)> {
    choose_role_with_currency(state, uid, role, provider, None)
}

pub fn choose_role_with_currency(
    state: &AppState,
    uid: &str,
    role: &str,
    provider: Option<&str>,
    currency: Option<&str>,
) -> AppResult<(User, Option<Order>)> {
    choose_role_with_promo(state, uid, role, provider, currency, None)
}

pub fn choose_role_with_promo(
    state: &AppState,
    uid: &str,
    role: &str,
    provider: Option<&str>,
    currency: Option<&str>,
    promo_code: Option<&str>,
) -> AppResult<(User, Option<Order>)> {
    if let Some(code) = promo_code {
        if role != "employer" {
            return Err(AppError::bad_request(
                "Promotion activation is only for employer accounts",
            ));
        }
        return promos::redeem_activation(state, uid, code, currency);
    }
    if !matches!(role, "seeker" | "employer") {
        return Err(AppError::bad_request("Choose seeker or employer"));
    }
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let user = models::load_user(&tx, uid)?;
    if user.banned {
        return Err(AppError::forbidden("Account is suspended"));
    }
    if user.role != "unassigned" && user.role != role {
        return Err(AppError::conflict("Account type is already selected"));
    }
    tx.execute(
        "UPDATE users SET role=?2,updated_at=?3 WHERE id=?1",
        params![uid, role, now()],
    )?;
    let order = if role == "employer" && !user.employer_paid {
        if activation_payment_recorded(&tx, uid)? {
            return Err(AppError::conflict(
                "An activation payment is already recorded. Please contact support to check it before paying again.",
            ));
        }
        expire_onboarding_orders(&tx, uid)?;
        let existing = tx
            .query_row(
                "SELECT * FROM orders WHERE user_id=?1 AND kind='onboarding' AND status='pending'",
                [uid],
                models::order_row,
            )
            .optional()?;
        let requested_currency = currency;
        let currency = currency.unwrap_or(&state.cfg.pricing.currency);
        let amount = state.cfg.pricing.onboarding_amount(currency)?;
        if let Some(existing) = existing {
            if requested_currency.is_some_and(|code| existing.currency != code)
                || provider.is_some_and(|p| p != existing.provider)
            {
                return Err(AppError::conflict(
                    "An activation order already exists in another currency or provider. Continue it from Orders or contact support.",
                ));
            }
            Some(existing)
        } else if amount == 0 {
            tx.execute("UPDATE users SET employer_paid=1 WHERE id=?1", [uid])?;
            None
        } else if let Some(provider) = provider {
            provider_currency_enabled(state, provider, currency)?;
            Some(insert_order(
                &tx,
                uid,
                "onboarding",
                None,
                amount,
                provider,
                provider_live(state, &tx, provider)?,
                currency,
                json!({}),
            )?)
        } else {
            None
        }
    } else {
        None
    };
    let user = models::load_user(&tx, uid)?;
    audit(&tx, uid, "choose_role", uid, role)?;
    tx.commit()?;
    Ok((user, order))
}
pub fn new_job(
    state: &AppState,
    uid: &str,
    title: &str,
    jd: &str,
    salary: i64,
    provider: &str,
) -> AppResult<(Job, Order)> {
    new_job_with_headcount(state, uid, title, jd, salary, provider, 1)
}

pub fn validate_headcount(headcount: i64) -> AppResult<()> {
    if !(0..=1_000_000).contains(&headcount) {
        return Err(AppError::bad_request(
            "Headcount must be between 0 and 1000000",
        ));
    }
    Ok(())
}

pub fn validate_configured_headcount(state: &AppState, headcount: i64) -> AppResult<()> {
    validate_headcount(headcount)?;
    if headcount > state.cfg.limits.max_job_headcount {
        return Err(AppError::bad_request(
            "Headcount exceeds the configured maximum",
        ));
    }
    Ok(())
}

pub fn new_job_with_headcount(
    state: &AppState,
    uid: &str,
    title: &str,
    jd: &str,
    salary: i64,
    provider: &str,
    headcount: i64,
) -> AppResult<(Job, Order)> {
    new_job_with_currency(state, uid, title, jd, salary, provider, headcount, None)
}

pub fn new_job_with_currency(
    state: &AppState,
    uid: &str,
    title: &str,
    jd: &str,
    salary: i64,
    provider: &str,
    headcount: i64,
    currency: Option<&str>,
) -> AppResult<(Job, Order)> {
    new_job_with_promo(
        state, uid, title, jd, salary, provider, headcount, currency, None, None, None,
    )
}

pub(crate) fn publication_amount(state: &AppState, salary: i64, currency: &str) -> AppResult<i64> {
    let pricing = &state.cfg.pricing;
    if !pricing.permits_currency(currency) {
        return Err(AppError::bad_request("Choose an enabled salary currency"));
    }
    let amounts = pricing.amounts(currency)?;
    let original = configured_fee(
        salary,
        &pricing.publish_mode,
        pricing.publish_bps,
        amounts.publish_fixed_minor,
    )?;
    Pricing::discount_amount(
        original,
        if currency == pricing.currency {
            pricing.publish_discount_percent
        } else {
            amounts.publish_discount_percent
        },
    )
}

pub fn new_job_with_promo(
    state: &AppState,
    uid: &str,
    title: &str,
    jd: &str,
    salary: i64,
    provider: &str,
    headcount: i64,
    currency: Option<&str>,
    promo_code: Option<&str>,
    request_key: Option<&str>,
    expected_amount_cents: Option<i64>,
) -> AppResult<(Job, Order)> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 120 {
        return Err(AppError::bad_request("Job title must be 1–120 characters"));
    }
    crate::safety::bounded_text(jd, 80_000, "Job description")?;
    if jd.trim().is_empty() {
        return Err(AppError::bad_request("Job description is required"));
    }
    let promo_code = promo_code.map(promos::normalize_code).transpose()?;
    let key = promo_code
        .as_ref()
        .map(|_| promos::validate_request_key(request_key))
        .transpose()?;
    let request_hash = promos::request_hash(
        &json!({"title":title,"jd_md":jd,"salary_cents":salary,"provider":provider,"headcount":headcount,"currency":currency,"promo_code":promo_code,"expected_amount_cents":expected_amount_cents}),
    );
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    employer(&tx, uid)?;
    promos::expire_reservations(&tx)?;
    if let Some(key) = key {
        if let Some(result) = promos::replay_publication(&tx, uid, key, &request_hash)? {
            tx.commit()?;
            return Ok(result);
        }
    }
    // A retry restores its immutable result even if current pricing, available
    // currencies or headcount limits have changed since the original operation.
    validate_configured_headcount(state, headcount)?;
    let pricing = &state.cfg.pricing;
    let currency = currency.unwrap_or(&pricing.currency);
    let original_amount = publication_amount(state, salary, currency)?;
    let amounts = pricing.amounts(currency)?;
    let promo = promo_code
        .as_deref()
        .map(|code| promos::available_code(&tx, code, "publish"))
        .transpose()?;
    let amount = match &promo {
        Some(code) => promos::discounted_amount(original_amount, code.discount_percent)?,
        None => original_amount,
    };
    if expected_amount_cents.is_some_and(|expected| expected != amount) {
        return Err(AppError::conflict(
            "The price changed; preview the promotion again before publishing",
        ));
    }
    if amount > 0 {
        provider_currency_enabled(state, provider, currency)?;
    }
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM orders o WHERE o.user_id=?1 AND o.status='pending'
         AND NOT EXISTS(SELECT 1 FROM promo_redemptions r WHERE r.order_id=o.id
                        AND r.status='reserved' AND r.reserved_until<=?2)",
        params![uid, now()],
        |r| r.get(0),
    )?;
    if amount > 0 && count >= state.cfg.limits.max_pending_orders_per_user {
        return Err(AppError::conflict("Complete existing pending orders first"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,currency,ai_min_salary_cents,headcount,status,version,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11)",params![id,uid,title,jd,salary,currency,amounts.employer_ai_min_salary_minor,headcount,if amount == 0 {"published"} else {"draft"},if amount == 0 {1} else {0},now()])?;
    let mut snapshot = json!({"version":0,"max_edits":pricing.max_edits,"publish_mode":pricing.publish_mode,"publish_bps":pricing.publish_bps,"publish_fixed_minor":amounts.publish_fixed_minor,"edit_mode":pricing.edit_mode,"edit_bps":pricing.edit_bps,"edit_fixed_minor":amounts.edit_fixed_minor,"edit_discount_percent":amounts.edit_discount_percent,"currency":currency,"employer_ai_min_salary_minor":amounts.employer_ai_min_salary_minor});
    if let Some(code) = &promo {
        snapshot["promo"] = json!({"code_id":code.id,"code":code.code,"discount_percent":code.discount_percent,"original_amount_cents":original_amount});
    }
    let mut order = insert_order(
        &tx,
        uid,
        "publish",
        Some(&id),
        amount,
        provider,
        provider_live(state, &tx, provider)?,
        currency,
        snapshot,
    )?;
    if let (Some(code), Some(key)) = (&promo, key) {
        promos::record_publication(&tx, code, &order, key, &request_hash)?;
        order = models::load_order(&tx, &order.id)?;
    }
    let job = models::load_job(&tx, &id)?;
    audit(
        &tx,
        uid,
        if amount == 0 {
            "job_publish_free"
        } else {
            "job_draft"
        },
        &id,
        "",
    )?;
    tx.commit()?;
    Ok((job, order))
}

/// Planned openings are maintained independently of paid content revisions.
/// Zero pauses new hiring without closing a paid advert or consuming an edit.
pub fn set_headcount(state: &AppState, uid: &str, id: &str, headcount: i64) -> AppResult<Job> {
    validate_configured_headcount(state, headcount)?;
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    employer(&tx, uid)?;
    let job = models::load_job(&tx, id)?;
    if job.owner_id != uid {
        return Err(AppError::forbidden("This job belongs to another account"));
    }
    if !matches!(job.status.as_str(), "draft" | "published") {
        return Err(AppError::conflict(
            "Headcount can only change for a draft or published job",
        ));
    }
    if job.headcount != headcount {
        tx.execute(
            "UPDATE jobs SET headcount=?2,updated_at=?3 WHERE id=?1",
            params![id, headcount, now()],
        )?;
        audit(
            &tx,
            uid,
            "job_headcount_update",
            id,
            &json!({"before":job.headcount,"after":headcount}).to_string(),
        )?;
    }
    let job = models::load_job(&tx, id)?;
    tx.commit()?;
    Ok(job)
}

pub fn edit_job(
    state: &AppState,
    uid: &str,
    id: &str,
    jd: &str,
    provider: &str,
) -> AppResult<(Job, Order)> {
    crate::safety::bounded_text(jd, 80_000, "Job description")?;
    if jd.trim().is_empty() {
        return Err(AppError::bad_request("Job description is required"));
    }
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    employer(&tx, uid)?;
    let job = models::load_job(&tx, id)?;
    if job.owner_id != uid {
        return Err(AppError::forbidden("This job belongs to another account"));
    }
    if job.status != "published" {
        return Err(AppError::conflict("Only a published job can be edited"));
    }
    let terms: String = tx.query_row(
        "SELECT snapshot_json FROM orders WHERE job_id=?1 AND kind='publish' AND status IN ('paid','partially_refunded')",
        [id],
        |r| r.get(0),
    )?;
    let terms: Value = serde_json::from_str(&terms)?;
    let max_edits = terms["max_edits"].as_i64().unwrap_or(3);
    let edit_bps = terms["edit_bps"].as_i64().unwrap_or(100);
    let edit_mode = terms["edit_mode"].as_str().unwrap_or("salary_percent");
    let original_amount = configured_fee(
        job.salary_cents,
        edit_mode,
        edit_bps,
        terms["edit_fixed_minor"].as_i64().unwrap_or(0),
    )?;
    let amount = Pricing::discount_amount(
        original_amount,
        terms["edit_discount_percent"].as_i64().unwrap_or(0),
    )?;
    if amount > 0 {
        provider_currency_enabled(state, provider, &job.currency)?;
    }
    if job.edits_used >= max_edits {
        return Err(AppError::conflict(
            "The maximum number of paid edits has been reached",
        ));
    }
    if jd == job.jd_md {
        return Err(AppError::bad_request("The job description has not changed"));
    }
    let pending: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM orders WHERE job_id=?1 AND status='pending')",
        [id],
        |r| r.get(0),
    )?;
    if pending {
        return Err(AppError::conflict("This job already has a pending payment"));
    }
    if amount > 0 {
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM orders o WHERE o.user_id=?1 AND o.status='pending'
             AND NOT EXISTS(SELECT 1 FROM promo_redemptions r WHERE r.order_id=o.id
                            AND r.status='reserved' AND r.reserved_until<=?2)",
            params![uid, now()],
            |row| row.get(0),
        )?;
        if count >= state.cfg.limits.max_pending_orders_per_user {
            return Err(AppError::conflict("Complete existing pending orders first"));
        }
    }
    let order = insert_order(
        &tx,
        uid,
        "edit",
        Some(id),
        amount,
        provider,
        provider_live(state, &tx, provider)?,
        &job.currency,
        json!({"jd_md":jd,"version":job.version,"max_edits":max_edits,"edit_mode":edit_mode,"edit_bps":edit_bps,"edit_discount_percent":terms["edit_discount_percent"],"edit_fixed_minor":terms["edit_fixed_minor"],"original_amount_cents":original_amount,"currency":job.currency}),
    )?;
    if amount == 0 {
        tx.execute("UPDATE jobs SET jd_md=?2,edits_used=edits_used+1,version=version+1,updated_at=?3 WHERE id=?1",params![id,jd,now()])?;
    }
    audit(
        &tx,
        uid,
        if amount == 0 {
            "job_edit_free"
        } else {
            "job_edit_pending"
        },
        id,
        "",
    )?;
    let job = models::load_job(&tx, id)?;
    tx.commit()?;
    Ok((job, order))
}
pub fn close_job(state: &AppState, uid: &str, id: &str) -> AppResult<Job> {
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    employer(&tx, uid)?;
    let job = models::load_job(&tx, id)?;
    if job.owner_id != uid {
        return Err(AppError::forbidden("This job belongs to another account"));
    }
    if job.status != "published" {
        return Err(AppError::conflict("Only a published job can be closed"));
    }
    let pending: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM orders WHERE job_id=?1 AND status='pending')",
        [id],
        |r| r.get(0),
    )?;
    if pending {
        return Err(AppError::conflict(
            "Resolve the pending job payment before closing",
        ));
    }
    tx.execute(
        "UPDATE jobs SET status='closed',headcount=0,updated_at=?2,version=version+1 WHERE id=?1",
        params![id, now()],
    )?;
    audit(&tx, uid, "job_close", id, "")?;
    let job = models::load_job(&tx, id)?;
    tx.commit()?;
    Ok(job)
}

fn mutate_owned_job(
    state: &AppState,
    uid: &str,
    id: &str,
    action: impl FnOnce(&Connection) -> AppResult<Job>,
) -> AppResult<Job> {
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let user = models::load_user(&tx, uid)?;
    // Managing one's own content is free even if activation was refunded.
    // Restoring recruitment rights checks the original payments below.
    if user.banned || user.role != "employer" {
        return Err(AppError::forbidden(
            "An active employer account is required",
        ));
    }
    let job = models::load_job(&tx, id)?;
    if job.owner_id != uid {
        return Err(AppError::forbidden("This job belongs to another account"));
    }
    let job = action(&tx)?;
    tx.commit()?;
    Ok(job)
}

pub fn delete_job(state: &AppState, uid: &str, id: &str, expected_version: i64) -> AppResult<Job> {
    mutate_owned_job(state, uid, id, |db| {
        delete_job_record(db, id, expected_version, false, uid)
    })
}

pub fn permanently_delete_job(
    state: &AppState,
    uid: &str,
    id: &str,
    expected_version: i64,
) -> AppResult<Job> {
    mutate_owned_job(state, uid, id, |db| {
        delete_job_record(db, id, expected_version, true, uid)
    })
}

pub fn restore_job(state: &AppState, uid: &str, id: &str, expected_version: i64) -> AppResult<Job> {
    mutate_owned_job(state, uid, id, |db| {
        restore_job_record(db, id, expected_version, uid)
    })
}

fn check_job_version(job: &Job, expected_version: i64) -> AppResult<()> {
    if expected_version < 0 {
        return Err(AppError::bad_request(
            "Expected job version must be nonnegative",
        ));
    }
    if job.version != expected_version {
        return Err(AppError::conflict(
            "The job changed; refresh before deleting or restoring it",
        ));
    }
    Ok(())
}

/// Shared with administrator HTTP/MCP. The caller must authorize the actor and
/// hold an IMMEDIATE transaction; this function never creates a payment order.
pub fn delete_job_record(
    db: &Connection,
    id: &str,
    expected_version: i64,
    permanent: bool,
    actor: &str,
) -> AppResult<Job> {
    if expected_version < 0 {
        return Err(AppError::bad_request(
            "Expected job version must be nonnegative",
        ));
    }
    let job = models::load_job(db, id)?;
    if job.status == "deleted" && (!permanent || job.delete_count >= 2) {
        // A lost-response retry of the first delete must not become the second
        // delete. After a restore the status/version change rejects that retry.
        return Ok(job);
    }
    check_job_version(&job, expected_version)?;
    let permanent = permanent || job.delete_count >= 1;
    if permanent {
        db.execute(
            "UPDATE jobs SET status='deleted',delete_count=2,headcount=0,title='',jd_md='',deleted_from_status=NULL,deleted_headcount=NULL,version=version+1,updated_at=?2 WHERE id=?1",
            params![id, now()],
        )?;
        // Pending and historical edit snapshots contain JD text. Receipts keep
        // amount/reference fields, but must not retain deleted job content.
        db.execute(
            "UPDATE orders SET snapshot_json=CASE WHEN json_valid(snapshot_json) AND json_type(snapshot_json,'$.promo')='object' THEN json_object('promo',json_extract(snapshot_json,'$.promo')) ELSE '{}' END,checkout_url=NULL,status=CASE WHEN status='pending' THEN 'cancelled' ELSE status END WHERE job_id=?1",
            [id],
        )?;
    } else {
        db.execute(
            "UPDATE jobs SET status='deleted',delete_count=1,deleted_from_status=?2,deleted_headcount=headcount,headcount=0,version=version+1,updated_at=?3 WHERE id=?1",
            params![id, job.status, now()],
        )?;
        // Deletion invalidates the content version of every pending edit. Such
        // an order can never be applied after recovery and must not block a new
        // edit or keep offering a checkout. Draft publication remains payable
        // after a valid recovery; it does not replace a particular JD version.
        db.execute(
            "UPDATE orders SET status='cancelled',checkout_url=NULL WHERE job_id=?1 AND kind='edit' AND status='pending'",
            [id],
        )?;
    }
    let updated = models::load_job(db, id)?;
    audit(
        db,
        actor,
        if permanent { "job_delete_permanent" } else { "job_delete" },
        id,
        &json!({"from_status":job.status,"permanent":permanent,"delete_count":updated.delete_count,"version":updated.version}).to_string(),
    )?;
    Ok(updated)
}

/// Shared restoration policy. Content restoration is free; reopening recruitment
/// still requires valid activation and the original non-refunded publication.
pub fn restore_job_record(
    db: &Connection,
    id: &str,
    expected_version: i64,
    actor: &str,
) -> AppResult<Job> {
    let job = models::load_job(db, id)?;
    check_job_version(&job, expected_version)?;
    if job.status != "deleted" || job.delete_count != 1 {
        return Err(AppError::conflict("This job cannot be restored"));
    }
    let owner = models::load_user(db, &job.owner_id)?;
    if owner.banned || owner.role != "employer" {
        return Err(AppError::forbidden(
            "An active employer account is required",
        ));
    }
    let (from_status, from_headcount): (Option<String>, Option<i64>) = db.query_row(
        "SELECT deleted_from_status,deleted_headcount FROM jobs WHERE id=?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let from_status = from_status
        .filter(|status| matches!(status.as_str(), "draft" | "published" | "closed"))
        .ok_or_else(|| AppError::conflict("The job has no restorable state"))?;
    let from_headcount = from_headcount
        .filter(|count| (0..=1_000_000).contains(count))
        .ok_or_else(|| AppError::conflict("The job has no restorable headcount"))?;
    let payment: Option<String> = db.query_row(
        "SELECT status FROM orders WHERE job_id=?1 AND kind='publish' ORDER BY created_at,id LIMIT 1",
        [id], |row| row.get(0),
    ).optional()?;
    let (status, headcount) = if from_status == "published"
        && owner.employer_paid
        && matches!(payment.as_deref(), Some("paid" | "partially_refunded"))
    {
        ("published", from_headcount)
    } else if from_status == "draft" && payment.as_deref() == Some("pending") {
        ("draft", from_headcount)
    } else {
        // Preserve recoverable content without inventing another bill or a
        // paid grant. An already-settled draft cannot be paid/activated again.
        ("closed", 0)
    };
    db.execute(
        "UPDATE jobs SET status=?2,headcount=?3,deleted_from_status=NULL,deleted_headcount=NULL,version=version+1,updated_at=?4 WHERE id=?1",
        params![id, status, headcount, now()],
    )?;
    let job = models::load_job(db, id)?;
    audit(
        db,
        actor,
        "job_restore",
        id,
        &json!({"status":status,"headcount":headcount,"version":job.version}).to_string(),
    )?;
    Ok(job)
}

pub async fn checkout(state: &AppState, uid: &str, id: &str) -> AppResult<Order> {
    checkout_with_region(state, uid, id, None).await
}

pub async fn checkout_with_region(
    state: &AppState,
    uid: &str,
    id: &str,
    requested_region: Option<&str>,
) -> AppResult<Order> {
    let (order, email) = {
        let mut db = state.conn()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        promos::expire_order_reservation(&tx, id)?;
        let order = models::load_order(&tx, id)?;
        if order.user_id != uid {
            return Err(AppError::forbidden("This order belongs to another account"));
        }
        let user = models::load_user(&tx, uid)?;
        if user.banned {
            return Err(AppError::forbidden("Account is suspended"));
        }
        if order.kind == "onboarding" {
            if !user.employer_paid && activation_payment_recorded(&tx, uid)? {
                return Err(AppError::conflict(
                    "An activation payment is already recorded. Please contact support to check it before paying again.",
                ));
            }
            expire_onboarding_orders(&tx, uid)?;
            let current = models::load_order(&tx, id)?;
            if current.status == "expired" {
                tx.commit()?;
                return Err(AppError::conflict(
                    "This activation order expired. Create a new activation order from Orders.",
                ));
            }
        }
        if order.status != "pending" {
            tx.commit()?;
            return Err(AppError::conflict("This order is not pending"));
        }
        ensure_checkout_job(&tx, &order)?;
        provider_currency_enabled(state, &order.provider, &order.currency)?;
        let mut snapshot: Value =
            serde_json::from_str(&order.snapshot_json).map_err(AppError::internal)?;
        if order.provider == "waffo" {
            let saved_region = snapshot["payment_region"].as_str();
            if requested_region.is_some()
                && saved_region.is_some()
                && requested_region != saved_region
            {
                return Err(AppError::conflict(
                    "This checkout already uses another Waffo region. Continue the existing checkout.",
                ));
            }
            let (region, _) = state
                .cfg
                .payments
                .waffo
                .checkout_region(&order.currency, saved_region.or(requested_region))?;
            snapshot["payment_region"] = json!(region);
        } else if requested_region.is_some() {
            return Err(AppError::bad_request(
                "Payment region is only available for Waffo",
            ));
        }
        if order.checkout_url.is_some() {
            return Ok(order);
        }
        if order.kind != "onboarding" && now() - order.created_at > UNSTARTED_ORDER_TTL {
            return Err(AppError::conflict(
                "This checkout is too old to retry safely. Please contact support.",
            ));
        }
        let timestamp = now();
        let ttl = crate::payments::checkout_ttl_seconds(&order.provider);
        let claimed=tx.execute("UPDATE orders SET checkout_expires_at=CASE WHEN checkout_started_at IS NULL THEN ?2+?5 ELSE COALESCE(checkout_expires_at,checkout_started_at+?5) END,checkout_started_at=?2,snapshot_json=?4 WHERE id=?1 AND (checkout_started_at IS NULL OR checkout_started_at<?3) AND status='pending'",params![id,timestamp,timestamp-30,snapshot.to_string(),ttl])?;
        if claimed != 1 {
            return Err(AppError::conflict(
                "Checkout creation is still in progress. Wait 30 seconds before retrying this same order.",
            ));
        }
        let order = models::load_order(&tx, id)?;
        tx.commit()?;
        (order, user.email)
    };
    let checkout = crate::payments::create_checkout(state, &order, &email).await?;
    // The signed gateway request awaited I/O. Deletion, suspension or a receipt
    // may have changed the local state while it was in flight.
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    promos::expire_order_reservation(&tx, id)?;
    let current = models::load_order(&tx, id)?;
    if models::load_user(&tx, uid)?.banned {
        return Err(AppError::forbidden("Account is suspended"));
    }
    if current.status != "pending" {
        tx.commit()?;
        return Err(AppError::conflict("This order is not pending"));
    }
    ensure_checkout_job(&tx, &current)?;
    tx.execute(
        "UPDATE orders SET provider_session=?2,checkout_url=?3,checkout_expires_at=?4 WHERE id=?1",
        params![id, checkout.session_id, checkout.url, checkout.expires_at],
    )?;
    expire_onboarding_orders(&tx, uid)?;
    let order = models::load_order(&tx, id)?;
    tx.commit()?;
    Ok(order)
}

fn ensure_checkout_job(db: &Connection, order: &Order) -> AppResult<()> {
    if !job_checkout_available(db, order)? {
        return Err(AppError::conflict(
            "This payment can no longer be applied; refresh the order before continuing",
        ));
    }
    Ok(())
}

/// Both checkout creation and the member order views use the same current-state
/// check. An old hosted URL may still exist at the provider, so late verified
/// receipts continue through settle_payment even when this returns false.
pub(crate) fn job_checkout_available(db: &Connection, order: &Order) -> AppResult<bool> {
    let reservation_expired: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM promo_redemptions WHERE order_id=?1
                       AND status='reserved' AND reserved_until<=?2)",
        params![order.id, now()],
        |row| row.get(0),
    )?;
    if reservation_expired {
        return Ok(false);
    }
    let Some(id) = &order.job_id else {
        if order.kind == "onboarding" {
            let user = models::load_user(db, &order.user_id)?;
            return Ok(user.role == "employer"
                && !user.banned
                && !user.employer_paid
                && !activation_payment_recorded(db, &user.id)?);
        }
        return Ok(true);
    };
    let job = models::load_job(db, id)?;
    let owner = models::load_user(db, &order.user_id)?;
    if job.owner_id != owner.id || owner.banned || owner.role != "employer" || !owner.employer_paid
    {
        return Ok(false);
    }
    match order.kind.as_str() {
        "publish" => Ok(job.status == "draft"),
        "edit" if job.status == "published" => {
            let snapshot: Value = serde_json::from_str(&order.snapshot_json)?;
            Ok(job.version == snapshot["version"].as_i64().unwrap_or(-1)
                && job.edits_used < snapshot["max_edits"].as_i64().unwrap_or(3))
        }
        _ => Ok(false),
    }
}
/// Verified receipts are applied in one SQLite transaction. Redirects never call this.
pub async fn settle_payment(state: &AppState, receipt: PaymentReceipt) -> AppResult<()> {
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    promos::expire_order_reservation(&tx, &receipt.order_id)?;
    let order = models::load_order(&tx, &receipt.order_id)?;
    if order.live_mode != receipt.live_mode
        || order.provider != receipt.provider
        || order.amount_cents != receipt.amount_cents
        || order.currency != receipt.currency.to_ascii_uppercase()
    {
        return Err(AppError::bad_request(
            "Payment amount, currency or provider does not match the order",
        ));
    }
    if let (Some(expected), Some(actual)) = (&order.provider_session, &receipt.session_id) {
        if expected != actual {
            return Err(AppError::bad_request("Checkout session does not match"));
        }
    }
    let duplicate: Option<(String, String)> = tx
        .query_row(
            "SELECT e.order_id,l.payment_id FROM payment_events e JOIN ledger l ON l.order_id=e.order_id WHERE e.provider=?1 AND e.event_id=?2",
            params![receipt.provider, receipt.event_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((recorded_order, recorded_payment)) = duplicate {
        if recorded_order != order.id || recorded_payment != receipt.payment_id {
            return Err(AppError::conflict(
                "Payment event is already associated with different payment details",
            ));
        }
        return Ok(());
    }
    let recorded:Option<(String,String)>=tx.query_row("SELECT order_id,payment_id FROM ledger WHERE (provider=?1 AND payment_id=?2) OR order_id=?3",params![receipt.provider,receipt.payment_id,order.id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((paid_order, payment_id)) = recorded {
        if paid_order != order.id || payment_id != receipt.payment_id {
            return Err(AppError::conflict(
                "Payment is already associated with another order",
            ));
        }
        tx.execute(
            "INSERT INTO payment_events(provider,event_id,order_id,created_at) VALUES(?1,?2,?3,?4)",
            params![receipt.provider, receipt.event_id, order.id, now()],
        )?;
        tx.commit()?;
        return Ok(());
    }
    let user = models::load_user(&tx, &order.user_id)?;
    let snapshot: Value = serde_json::from_str(&order.snapshot_json)?;
    let mut applicable = (order.status == "pending"
        || (order.kind == "onboarding" && order.status == "expired"))
        && !user.banned;
    if applicable {
        match order.kind.as_str() {
            "onboarding" => {
                applicable = user.role == "employer" && !user.employer_paid;
                if applicable {
                    tx.execute(
                        "UPDATE users SET employer_paid=1,updated_at=?2 WHERE id=?1",
                        params![user.id, now()],
                    )?;
                    // A delayed receipt for an expired session may win the
                    // race against its replacement. Remove every other payable
                    // activation link; later duplicate money is kept for review.
                    tx.execute(
                        "UPDATE orders SET status='cancelled',checkout_url=NULL WHERE user_id=?1 AND kind='onboarding' AND status='pending' AND id<>?2",
                        params![user.id, order.id],
                    )?;
                }
            }
            "publish" => {
                let job = models::load_job(
                    &tx,
                    order
                        .job_id
                        .as_deref()
                        .ok_or_else(|| AppError::bad_request("Missing job"))?,
                )?;
                applicable = job.status == "draft"
                    && job.owner_id == user.id
                    && user.employer_paid
                    && user.role == "employer";
                if applicable {
                    tx.execute(
                        "UPDATE jobs SET status='published',version=version+1,updated_at=?2 WHERE id=?1",
                        params![job.id, now()],
                    )?;
                }
            }
            "edit" => {
                let job = models::load_job(
                    &tx,
                    order
                        .job_id
                        .as_deref()
                        .ok_or_else(|| AppError::bad_request("Missing job"))?,
                )?;
                applicable = job.status == "published"
                    && job.owner_id == user.id
                    && job.version == snapshot["version"].as_i64().unwrap_or(-1)
                    && job.edits_used < snapshot["max_edits"].as_i64().unwrap_or(3)
                    && user.employer_paid
                    && user.role == "employer";
                if applicable {
                    let jd = snapshot["jd_md"]
                        .as_str()
                        .ok_or_else(|| AppError::bad_request("Missing paid edit"))?;
                    tx.execute("UPDATE jobs SET jd_md=?2,edits_used=edits_used+1,version=version+1,updated_at=?3 WHERE id=?1",params![job.id,jd,now()])?;
                }
            }
            _ => return Err(AppError::bad_request("Invalid order kind")),
        }
    }
    tx.execute("INSERT INTO ledger(order_id,provider,payment_id,amount_cents,currency,live_mode,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![order.id,receipt.provider,receipt.payment_id,receipt.amount_cents,order.currency,receipt.live_mode,now()])?;
    tx.execute(
        "INSERT INTO payment_events(provider,event_id,order_id,created_at) VALUES(?1,?2,?3,?4)",
        params![receipt.provider, receipt.event_id, order.id, now()],
    )?;
    let status = if applicable {
        "paid"
    } else {
        "paid_needs_review"
    };
    tx.execute(
        "UPDATE orders SET status=?2,paid_at=?3 WHERE id=?1",
        params![order.id, status, now()],
    )?;
    audit(&tx, "payment", status, &order.id, "")?;
    tx.commit()?;
    Ok(())
}

/// Record an already-confirmed gateway refund. This never calls a refund API.
/// A successful outcome is terminal; a later failed delivery cannot undo it.
pub async fn settle_refund(state: &AppState, receipt: RefundReceipt) -> AppResult<()> {
    if receipt.provider != "waffo"
        || receipt.amount_cents <= 0
        || receipt.refund_id.is_empty()
        || receipt.event_id != receipt.refund_id
    {
        return Err(AppError::bad_request("Invalid refund provider or amount"));
    }
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let order = models::load_order(&tx, &receipt.order_id)?;
    let paid: Option<(String, String, i64, String, bool)> = tx.query_row(
        "SELECT provider,payment_id,amount_cents,currency,live_mode FROM ledger WHERE order_id=?1",
        [&order.id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
    ).optional()?;
    let (provider, payment_id, paid_amount, currency, live_mode) = paid.ok_or_else(|| {
        AppError::conflict("Original payment has not been recorded; retry the refund notification")
    })?;
    if provider != receipt.provider
        || order.provider != receipt.provider
        || payment_id != receipt.payment_id
        || currency != receipt.currency
        || order.currency != receipt.currency
        || live_mode != receipt.live_mode
        || order.live_mode != receipt.live_mode
        || order.amount_cents != paid_amount
        || receipt
            .original_amount_cents
            .is_some_and(|amount| amount != paid_amount)
    {
        return Err(AppError::bad_request(
            "Refund does not match its original recorded payment",
        ));
    }
    if !matches!(
        order.status.as_str(),
        "paid" | "paid_needs_review" | "partially_refunded" | "refunded"
    ) {
        return Err(AppError::conflict("Original order has no settled payment"));
    }

    // A refund ID cannot be replayed against another payment, amount or order.
    let previous: Option<(String, String, String, i64, String, String)> = tx
        .query_row(
            "SELECT order_id,payment_id,provider_order_id,amount_cents,currency,status
         FROM payment_refunds WHERE provider=?1 AND live_mode=?2 AND refund_id=?3",
            params![receipt.provider, receipt.live_mode, receipt.refund_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    if let Some((old_order, old_payment, old_provider_order, old_amount, old_currency, _)) =
        &previous
    {
        if old_order != &order.id
            || old_payment != &receipt.payment_id
            || old_provider_order != &receipt.provider_order_id
            || *old_amount != receipt.amount_cents
            || old_currency != &receipt.currency
        {
            return Err(AppError::conflict(
                "Refund ID is already associated with different details",
            ));
        }
    }
    let event_type = if receipt.succeeded {
        "refund.succeeded"
    } else {
        "refund.failed"
    };
    let duplicate: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM refund_events WHERE provider=?1 AND live_mode=?2 AND event_id=?3 AND event_type=?4)",
        params![receipt.provider,receipt.live_mode,receipt.event_id,event_type],
        |row| row.get(0),
    )?;
    if duplicate {
        return Ok(());
    }
    let already_succeeded = previous.as_ref().is_some_and(|row| row.5 == "succeeded");
    let timestamp = now();
    if receipt.succeeded && !already_succeeded {
        let refunded: i64 = tx.query_row(
            "SELECT COALESCE(sum(amount_cents),0) FROM payment_refunds WHERE order_id=?1 AND status='succeeded'",
            [&order.id],
            |row| row.get(0),
        )?;
        let refunded = refunded
            .checked_add(receipt.amount_cents)
            .filter(|total| *total <= paid_amount)
            .ok_or_else(|| {
                AppError::bad_request("Cumulative refunds exceed the original payment")
            })?;
        tx.execute(
            "INSERT INTO payment_refunds(provider,refund_id,order_id,payment_id,provider_order_id,amount_cents,currency,live_mode,status,reason,created_at,updated_at,succeeded_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'succeeded',?9,?10,?10,?10)
             ON CONFLICT(provider,live_mode,refund_id) DO UPDATE SET status='succeeded',reason=excluded.reason,updated_at=excluded.updated_at,succeeded_at=excluded.succeeded_at",
            params![receipt.provider,receipt.refund_id,order.id,receipt.payment_id,receipt.provider_order_id,receipt.amount_cents,receipt.currency,receipt.live_mode,receipt.reason,timestamp],
        )?;
        let fully_refunded = refunded == paid_amount;
        let status = if fully_refunded {
            "refunded"
        } else if order.status == "paid_needs_review" {
            // A partial refund must not turn an unfulfilled payment into a grant.
            "paid_needs_review"
        } else {
            "partially_refunded"
        };
        tx.execute(
            "UPDATE orders SET status=?2,refunded_cents=?3 WHERE id=?1",
            params![order.id, status, refunded],
        )?;
        if fully_refunded {
            match order.kind.as_str() {
                "onboarding" => {
                    // Another genuinely fulfilled activation must survive refund
                    // of an older payment that was recorded for manual review.
                    let remains_paid: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM orders WHERE user_id=?1 AND kind='onboarding' AND status IN ('paid','partially_refunded'))",
                        [&order.user_id],
                        |row| row.get(0),
                    )?;
                    tx.execute(
                        "UPDATE users SET employer_paid=?2,updated_at=?3 WHERE id=?1 AND employer_paid<>?2",
                        params![order.user_id,remains_paid,timestamp],
                    )?;
                }
                "publish" => {
                    tx.execute(
                        "UPDATE jobs SET headcount=0,status=CASE WHEN status='deleted' THEN 'deleted' ELSE 'closed' END,version=version+CASE WHEN status IN ('draft','published') THEN 1 ELSE 0 END,updated_at=?3 WHERE id=?1 AND owner_id=?2 AND (status IN ('draft','published') OR headcount<>0)",
                        params![order.job_id,order.user_id,timestamp],
                    )?;
                }
                // The editorial work has already been applied. Do not revert
                // newer text or restore an edit allowance after a cash refund.
                "edit" => {}
                _ => return Err(AppError::bad_request("Invalid refunded order kind")),
            }
        }
    } else if previous.is_none() {
        // Failed refunds describe attempted money, not cash movement.
        tx.execute(
            "INSERT INTO payment_refunds(provider,refund_id,order_id,payment_id,provider_order_id,amount_cents,currency,live_mode,status,reason,created_at,updated_at,succeeded_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'failed',?9,?10,?10,NULL)",
            params![receipt.provider,receipt.refund_id,order.id,receipt.payment_id,receipt.provider_order_id,receipt.amount_cents,receipt.currency,receipt.live_mode,receipt.reason,timestamp],
        )?;
    }
    // Include the outcome in the event key: Waffo uses the same entity ID for
    // refund.failed and refund.succeeded. Neither delivery may hide the other.
    tx.execute(
        "INSERT INTO refund_events(provider,event_id,event_type,refund_id,order_id,payment_id,amount_cents,currency,live_mode,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![receipt.provider,receipt.event_id,event_type,receipt.refund_id,order.id,receipt.payment_id,receipt.amount_cents,receipt.currency,receipt.live_mode,timestamp],
    )?;
    audit(
        &tx,
        "payment",
        event_type,
        &order.id,
        &json!({
            "provider":receipt.provider,"refund_id":receipt.refund_id,
            "payment_id":receipt.payment_id,"amount_cents":receipt.amount_cents,
            "currency":receipt.currency,"live_mode":receipt.live_mode
        })
        .to_string(),
    )?;
    tx.commit()?;
    Ok(())
}
