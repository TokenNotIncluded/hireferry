//! All paid changes pass through this module, from both the website and MCP.
use crate::{
    AppState,
    db::audit,
    error::{AppError, AppResult},
    models::{self, Job, Order, User},
    now,
    payments::PaymentReceipt,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

pub fn fee(salary_cents: i64, basis_points: i64) -> AppResult<i64> {
    if salary_cents < 100
        || salary_cents > 1_000_000_000
        || basis_points <= 0
        || basis_points > 10000
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
pub fn provider_enabled(state: &AppState, provider: &str) -> AppResult<()> {
    match provider {
        "stripe" if state.cfg.payments.stripe.enabled => Ok(()),
        "waffo" if state.cfg.payments.waffo.enabled => Ok(()),
        _ => Err(AppError::unavailable(
            "Payment provider is not enabled. Please contact the service.",
        )),
    }
}
fn provider_live(state: &AppState, provider: &str) -> bool {
    match provider {
        "stripe" => state.cfg.payments.stripe.live_mode,
        "waffo" => state.cfg.payments.waffo.mode == "prod",
        _ => false,
    }
}
fn employer(db: &Connection, id: &str) -> AppResult<User> {
    let u = models::load_user(db, id)?;
    if u.banned || u.role != "employer" || !u.employer_paid {
        return Err(AppError::forbidden("A paid employer account is required"));
    }
    Ok(u)
}
fn insert_order(
    db: &Connection,
    user: &str,
    kind: &str,
    job: Option<&str>,
    amount: i64,
    provider: &str,
    live_mode: bool,
    snapshot: Value,
) -> AppResult<Order> {
    let id = uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,live_mode,snapshot_json,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id,user,kind,job,amount,provider,live_mode,snapshot.to_string(),now()])?;
    models::load_order(db, &id)
}
pub fn choose_role(
    state: &AppState,
    uid: &str,
    role: &str,
    provider: Option<&str>,
) -> AppResult<(User, Option<Order>)> {
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
        let existing = tx
            .query_row(
                "SELECT * FROM orders WHERE user_id=?1 AND kind='onboarding' AND status='pending'",
                [uid],
                models::order_row,
            )
            .optional()?;
        if existing.is_some() {
            existing
        } else if state.cfg.pricing.onboarding_cents == 0 {
            tx.execute("UPDATE users SET employer_paid=1 WHERE id=?1", [uid])?;
            None
        } else if let Some(provider) = provider {
            provider_enabled(state, provider)?;
            Some(insert_order(
                &tx,
                uid,
                "onboarding",
                None,
                state.cfg.pricing.onboarding_cents,
                provider,
                provider_live(state, provider),
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
    provider_enabled(state, provider)?;
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 120 {
        return Err(AppError::bad_request("Job title must be 1–120 characters"));
    }
    crate::safety::bounded_text(jd, 80_000, "Job description")?;
    if jd.trim().is_empty() {
        return Err(AppError::bad_request("Job description is required"));
    }
    let amount = fee(salary, state.cfg.pricing.publish_bps)?;
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    employer(&tx, uid)?;
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM orders WHERE user_id=?1 AND status='pending'",
        [uid],
        |r| r.get(0),
    )?;
    if count >= 20 {
        return Err(AppError::conflict("Complete existing pending orders first"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?6)",params![id,uid,title,jd,salary,now()])?;
    let order = insert_order(
        &tx,
        uid,
        "publish",
        Some(&id),
        amount,
        provider,
        provider_live(state, provider),
        json!({"version":0,"max_edits":state.cfg.pricing.max_edits,"edit_bps":state.cfg.pricing.edit_bps}),
    )?;
    let job = models::load_job(&tx, &id)?;
    audit(&tx, uid, "job_draft", &id, "")?;
    tx.commit()?;
    Ok((job, order))
}
pub fn edit_job(
    state: &AppState,
    uid: &str,
    id: &str,
    jd: &str,
    provider: &str,
) -> AppResult<(Job, Order)> {
    provider_enabled(state, provider)?;
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
        "SELECT snapshot_json FROM orders WHERE job_id=?1 AND kind='publish' AND status='paid'",
        [id],
        |r| r.get(0),
    )?;
    let terms: Value = serde_json::from_str(&terms)?;
    let max_edits = terms["max_edits"].as_i64().unwrap_or(3);
    let edit_bps = terms["edit_bps"].as_i64().unwrap_or(100);
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
    let order = insert_order(
        &tx,
        uid,
        "edit",
        Some(id),
        fee(job.salary_cents, edit_bps)?,
        provider,
        provider_live(state, provider),
        json!({"jd_md":jd,"version":job.version,"max_edits":max_edits}),
    )?;
    audit(&tx, uid, "job_edit_pending", id, "")?;
    tx.commit()?;
    Ok((job, order))
}
pub fn close_job(state: &AppState, uid: &str, id: &str) -> AppResult<Job> {
    let db = state.conn()?;
    employer(&db, uid)?;
    let job = models::load_job(&db, id)?;
    if job.owner_id != uid {
        return Err(AppError::forbidden("This job belongs to another account"));
    }
    if job.status != "published" {
        return Err(AppError::conflict("Only a published job can be closed"));
    }
    let pending: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM orders WHERE job_id=?1 AND status='pending')",
        [id],
        |r| r.get(0),
    )?;
    if pending {
        return Err(AppError::conflict(
            "Resolve the pending job payment before closing",
        ));
    }
    db.execute(
        "UPDATE jobs SET status='closed',updated_at=?2,version=version+1 WHERE id=?1",
        params![id, now()],
    )?;
    audit(&db, uid, "job_close", id, "")?;
    models::load_job(&db, id)
}
pub async fn checkout(state: &AppState, uid: &str, id: &str) -> AppResult<Order> {
    let (order, email) = {
        let db = state.conn()?;
        let order = models::load_order(&db, id)?;
        if order.user_id != uid {
            return Err(AppError::forbidden("This order belongs to another account"));
        }
        let user = models::load_user(&db, uid)?;
        if user.banned {
            return Err(AppError::forbidden("Account is suspended"));
        }
        if order.status != "pending" {
            return Err(AppError::conflict("This order is not pending"));
        }
        provider_enabled(state, &order.provider)?;
        if order.checkout_url.is_some() {
            return Ok(order);
        }
        if now() - order.created_at > 23 * 3600 {
            return Err(AppError::conflict(
                "This checkout is too old to retry safely. Please contact support.",
            ));
        }
        let claimed=db.execute("UPDATE orders SET checkout_started_at=?2 WHERE id=?1 AND (checkout_started_at IS NULL OR checkout_started_at<?3) AND status='pending'",params![id,now(),now()-30])?;
        if claimed != 1 {
            return Err(AppError::conflict(
                "Checkout creation is still in progress. Wait 30 seconds before retrying this same order.",
            ));
        }
        (order, user.email)
    };
    let checkout = crate::payments::create_checkout(state, &order, &email).await?;
    let db = state.conn()?;
    db.execute(
        "UPDATE orders SET provider_session=?2,checkout_url=?3 WHERE id=?1",
        params![id, checkout.session_id, checkout.url],
    )?;
    models::load_order(&db, id)
}
/// Verified receipts are applied in one SQLite transaction. Redirects never call this.
pub async fn settle_payment(state: &AppState, receipt: PaymentReceipt) -> AppResult<()> {
    let mut db = state.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
    let duplicate: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM payment_events WHERE provider=?1 AND event_id=?2)",
        params![receipt.provider, receipt.event_id],
        |r| r.get(0),
    )?;
    if duplicate {
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
    let mut applicable = order.status == "pending" && !user.banned;
    if applicable {
        match order.kind.as_str() {
            "onboarding" => {
                applicable = user.role == "employer" && !user.employer_paid;
                if applicable {
                    tx.execute(
                        "UPDATE users SET employer_paid=1,updated_at=?2 WHERE id=?1",
                        params![user.id, now()],
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
                        "UPDATE jobs SET status='published',version=1,updated_at=?2 WHERE id=?1",
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
