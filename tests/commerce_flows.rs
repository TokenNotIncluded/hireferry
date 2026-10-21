//! Business invariants against an actual SQLite file. No gateway or email calls.
use std::sync::{Arc, OnceLock};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use hireferry::{
    AppState, api, commerce,
    config::Config,
    models::{self, Job, Order},
    payments::{PaymentReceipt, RefundReceipt},
};
use rsa::{
    RsaPrivateKey,
    pkcs1v15::SigningKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
    signature::{SignatureEncoding, Signer},
};
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tower::ServiceExt;

struct Fixture {
    _directory: TempDir,
    state: AppState,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.development = true;
        cfg.database = directory
            .path()
            .join("commerce.sqlite3")
            .to_string_lossy()
            .into_owned();
        cfg.payments.stripe.enabled = true;
        // Never sent to Stripe. These tests exercise local database settlement.
        cfg.payments.stripe.secret_key = "sk_test_local_fixture_not_a_real_key".into();
        cfg.payments.stripe.webhook_secret = "whsec_local_fixture_not_a_real_key".into();
        cfg.payments.stripe.live_mode = false;
        let state = AppState::new(cfg).unwrap();
        Self {
            _directory: directory,
            state,
        }
    }

    fn user(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.state.conn().unwrap().execute(
            "INSERT INTO users(id,email,name,created_at,updated_at) VALUES(?1,?2,'Test account',?3,?3)",
            params![id, format!("{id}@example.invalid"), hireferry::now()],
        ).unwrap();
        id
    }

    async fn employer(&self) -> String {
        let id = self.user();
        let (_, order) =
            commerce::choose_role(&self.state, &id, "employer", Some("stripe")).unwrap();
        commerce::settle_payment(&self.state, receipt(&order.unwrap(), "first"))
            .await
            .unwrap();
        id
    }

    async fn published_job(&self, employer: &str) -> Job {
        let (job, order) = commerce::new_job(
            &self.state,
            employer,
            "Rust developer",
            "Original description",
            300_000,
            "stripe",
        )
        .unwrap();
        commerce::settle_payment(&self.state, receipt(&order, "first"))
            .await
            .unwrap();
        models::load_job(&self.state.conn().unwrap(), &job.id).unwrap()
    }
}

fn receipt(order: &Order, delivery: &str) -> PaymentReceipt {
    PaymentReceipt {
        provider: order.provider.clone(),
        event_id: format!("evt_{}_{delivery}", order.id),
        order_id: order.id.clone(),
        session_id: Some(format!("cs_{}", order.id)),
        payment_id: format!("pi_{}", order.id),
        amount_cents: order.amount_cents,
        currency: order.currency.clone(),
        live_mode: false,
    }
}

fn count(state: &AppState, table: &str) -> i64 {
    state
        .conn()
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn stored_order(state: &AppState, id: &str) -> Order {
    models::load_order(&state.conn().unwrap(), id).unwrap()
}

#[test]
fn charges_are_exact_cents_with_checked_bounds() {
    assert_eq!(commerce::fee(300_000, 1000).unwrap(), 30_000);
    assert_eq!(commerce::fee(600_000, 1000).unwrap(), 60_000);
    assert_eq!(commerce::fee(300_000, 100).unwrap(), 3_000);
    assert_eq!(commerce::fee(600_000, 100).unwrap(), 6_000);
    // The half-cent rule is deterministic and never uses binary floats.
    assert_eq!(commerce::fee(30_005, 1000).unwrap(), 3_001);
    for (salary, rate) in [
        (-1, 1000),
        (0, 1000),
        (i64::MAX, 1000),
        (300_000, -1),
        (300_000, 0),
        (300_000, 10_001),
    ] {
        assert!(commerce::fee(salary, rate).is_err());
    }
}

#[tokio::test]
async fn seeker_is_free_and_recruiter_needs_the_verified_onboarding_payment() {
    let f = Fixture::new();
    let seeker = f.user();
    let (user, order) = commerce::choose_role(&f.state, &seeker, "seeker", None).unwrap();
    assert_eq!(user.role, "seeker");
    assert!(order.is_none());
    assert_eq!(count(&f.state, "orders"), 0);
    let forbidden = commerce::new_job(&f.state, &seeker, "A job", "Description", 300_000, "stripe")
        .unwrap_err();
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);

    let employer = f.user();
    let (user, order) =
        commerce::choose_role(&f.state, &employer, "employer", Some("stripe")).unwrap();
    let order = order.unwrap();
    assert!(!user.employer_paid);
    assert_eq!(order.amount_cents, 10_000);
    assert_eq!(order.currency, "CNY");
    assert_eq!(
        commerce::new_job(
            &f.state,
            &employer,
            "A job",
            "Description",
            300_000,
            "stripe"
        )
        .unwrap_err()
        .status,
        StatusCode::FORBIDDEN
    );
    let (_, existing) =
        commerce::choose_role(&f.state, &employer, "employer", Some("stripe")).unwrap();
    assert_eq!(existing.unwrap().id, order.id);
    assert_eq!(count(&f.state, "orders"), 1);

    commerce::settle_payment(&f.state, receipt(&order, "first"))
        .await
        .unwrap();
    assert!(
        models::load_user(&f.state.conn().unwrap(), &employer)
            .unwrap()
            .employer_paid
    );
    let (job, publication) = commerce::new_job(
        &f.state,
        &employer,
        "A job",
        "Description",
        300_000,
        "stripe",
    )
    .unwrap();
    assert_eq!(job.status, "draft");
    assert_eq!(publication.amount_cents, 30_000);
    assert!(commerce::choose_role(&f.state, &employer, "seeker", None).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_deliveries_across_database_connections_fulfill_once() {
    let f = Fixture::new();
    let id = f.user();
    let (_, order) = commerce::choose_role(&f.state, &id, "employer", Some("stripe")).unwrap();
    let order = order.unwrap();
    // Separate SQLite connections exercise the transaction, not just one Mutex.
    let states: Vec<_> = (0..8)
        .map(|_| AppState::new((*f.state.cfg).clone()).unwrap())
        .collect();
    let barrier = Arc::new(tokio::sync::Barrier::new(states.len()));
    let mut tasks = Vec::new();
    for state in states {
        let barrier = barrier.clone();
        let receipt = receipt(&order, "same_delivery");
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            commerce::settle_payment(&state, receipt).await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    assert_eq!(count(&f.state, "ledger"), 1);
    assert_eq!(count(&f.state, "payment_events"), 1);
    assert_eq!(stored_order(&f.state, &order.id).status, "paid");
    assert!(
        models::load_user(&f.state.conn().unwrap(), &id)
            .unwrap()
            .employer_paid
    );

    // A new delivery event for the same actual payment must not charge twice.
    commerce::settle_payment(&f.state, receipt(&order, "another_delivery"))
        .await
        .unwrap();
    assert_eq!(count(&f.state, "ledger"), 1);
    assert_eq!(count(&f.state, "payment_events"), 2);
    let sum: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT sum(amount_cents) FROM ledger", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sum, 10_000);
}

#[tokio::test]
async fn a_recorded_payment_event_cannot_be_reused_for_another_order_or_payment() {
    let f = Fixture::new();
    let first_user = f.user();
    let second_user = f.user();
    let (_, first) =
        commerce::choose_role(&f.state, &first_user, "employer", Some("stripe")).unwrap();
    let (_, second) =
        commerce::choose_role(&f.state, &second_user, "employer", Some("stripe")).unwrap();
    let first = first.unwrap();
    let second = second.unwrap();
    let first_event = format!("evt_{}_original", first.id);
    commerce::settle_payment(&f.state, receipt(&first, "original"))
        .await
        .unwrap();

    let mut wrong_order = receipt(&second, "original");
    wrong_order.event_id = first_event.clone();
    assert_eq!(
        commerce::settle_payment(&f.state, wrong_order)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(stored_order(&f.state, &second.id).status, "pending");
    assert!(
        !models::load_user(&f.state.conn().unwrap(), &second_user)
            .unwrap()
            .employer_paid
    );

    let mut wrong_payment = receipt(&first, "original");
    wrong_payment.payment_id = "pi_different_payment".into();
    assert_eq!(
        commerce::settle_payment(&f.state, wrong_payment)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(count(&f.state, "ledger"), 1);
    assert_eq!(count(&f.state, "payment_events"), 1);
    commerce::settle_payment(&f.state, receipt(&first, "original"))
        .await
        .unwrap();
    assert_eq!(count(&f.state, "ledger"), 1);
}

#[tokio::test]
async fn mismatched_receipts_cannot_change_the_order_or_entitlements() {
    let f = Fixture::new();
    let user = f.user();
    let (_, order) = commerce::choose_role(&f.state, &user, "employer", Some("stripe")).unwrap();
    let order = order.unwrap();
    let good = receipt(&order, "first");
    let mut wrong_amount = good.clone();
    wrong_amount.amount_cents -= 1;
    let mut wrong_currency = good.clone();
    wrong_currency.currency = "USD".into();
    let mut wrong_provider = good.clone();
    wrong_provider.provider = "waffo".into();
    let mut wrong_environment = good.clone();
    wrong_environment.live_mode = true;
    for invalid in [
        wrong_amount,
        wrong_currency,
        wrong_provider,
        wrong_environment,
    ] {
        assert_eq!(
            commerce::settle_payment(&f.state, invalid)
                .await
                .unwrap_err()
                .status,
            StatusCode::BAD_REQUEST
        );
    }
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE orders SET provider_session='cs_expected' WHERE id=?1",
            [&order.id],
        )
        .unwrap();
    assert_eq!(
        commerce::settle_payment(&f.state, good.clone())
            .await
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(count(&f.state, "ledger"), 0);
    assert_eq!(count(&f.state, "payment_events"), 0);
    assert_eq!(stored_order(&f.state, &order.id).status, "pending");
    assert!(
        !models::load_user(&f.state.conn().unwrap(), &user)
            .unwrap()
            .employer_paid
    );
    let mut matched = good;
    matched.session_id = Some("cs_expected".into());
    commerce::settle_payment(&f.state, matched).await.unwrap();
    assert_eq!(count(&f.state, "ledger"), 1);
}

#[tokio::test]
async fn one_payment_cannot_be_reused_for_another_account() {
    let f = Fixture::new();
    let first_user = f.user();
    let second_user = f.user();
    let (_, first) =
        commerce::choose_role(&f.state, &first_user, "employer", Some("stripe")).unwrap();
    let (_, second) =
        commerce::choose_role(&f.state, &second_user, "employer", Some("stripe")).unwrap();
    let first = first.unwrap();
    let second = second.unwrap();
    let paid = receipt(&first, "first");
    commerce::settle_payment(&f.state, paid.clone())
        .await
        .unwrap();
    let mut reused = receipt(&second, "reuse_attempt");
    reused.payment_id = paid.payment_id;
    assert_eq!(
        commerce::settle_payment(&f.state, reused)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert!(
        !models::load_user(&f.state.conn().unwrap(), &second_user)
            .unwrap()
            .employer_paid
    );
    assert_eq!(stored_order(&f.state, &second.id).status, "pending");
    assert_eq!(count(&f.state, "ledger"), 1);
}

#[tokio::test]
async fn ledger_failure_rolls_back_access_and_all_payment_writes() {
    let f = Fixture::new();
    let user = f.user();
    let (_, order) = commerce::choose_role(&f.state, &user, "employer", Some("stripe")).unwrap();
    let order = order.unwrap();
    f.state.conn().unwrap().execute_batch(
        "CREATE TRIGGER fail_ledger BEFORE INSERT ON ledger BEGIN SELECT RAISE(ABORT, 'injected ledger failure'); END;"
    ).unwrap();
    assert!(
        commerce::settle_payment(&f.state, receipt(&order, "first"))
            .await
            .is_err()
    );
    assert!(
        !models::load_user(&f.state.conn().unwrap(), &user)
            .unwrap()
            .employer_paid
    );
    assert_eq!(stored_order(&f.state, &order.id).status, "pending");
    assert!(stored_order(&f.state, &order.id).paid_at.is_none());
    assert_eq!(count(&f.state, "ledger"), 0);
    assert_eq!(count(&f.state, "payment_events"), 0);
    f.state
        .conn()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_ledger;")
        .unwrap();
    commerce::settle_payment(&f.state, receipt(&order, "first"))
        .await
        .unwrap();
    assert_eq!(stored_order(&f.state, &order.id).status, "paid");
}

#[tokio::test]
async fn descriptions_change_only_after_payment_with_one_pending_and_three_edits() {
    let mut f = Fixture::new();
    let employer = f.employer().await;
    let job = f.published_job(&employer).await;
    // Later configuration changes do not alter a published job's agreed terms.
    Arc::make_mut(&mut f.state.cfg).pricing.edit_bps = 500;
    Arc::make_mut(&mut f.state.cfg).pricing.max_edits = 10;
    for edit in 1..=3 {
        let before = models::load_job(&f.state.conn().unwrap(), &job.id).unwrap();
        let description = format!("Paid description revision {edit}");
        let (_, order) =
            commerce::edit_job(&f.state, &employer, &job.id, &description, "stripe").unwrap();
        assert_eq!(order.amount_cents, 3_000);
        assert_eq!(
            models::load_job(&f.state.conn().unwrap(), &job.id)
                .unwrap()
                .jd_md,
            before.jd_md
        );
        assert_eq!(
            commerce::edit_job(
                &f.state,
                &employer,
                &job.id,
                "Another pending edit",
                "stripe"
            )
            .unwrap_err()
            .status,
            StatusCode::CONFLICT
        );
        assert_eq!(
            commerce::close_job(&f.state, &employer, &job.id)
                .unwrap_err()
                .status,
            StatusCode::CONFLICT
        );
        commerce::settle_payment(&f.state, receipt(&order, "first"))
            .await
            .unwrap();
        // A repeated receipt cannot consume a second edit allowance.
        commerce::settle_payment(&f.state, receipt(&order, "first"))
            .await
            .unwrap();
        let updated = models::load_job(&f.state.conn().unwrap(), &job.id).unwrap();
        assert_eq!(updated.jd_md, description);
        assert_eq!(updated.edits_used, edit);
        assert_eq!(updated.title, "Rust developer");
        assert_eq!(updated.salary_cents, 300_000);
    }
    assert_eq!(
        commerce::edit_job(&f.state, &employer, &job.id, "Fourth edit", "stripe")
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(count(&f.state, "ledger"), 5);
    let sum: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT sum(amount_cents) FROM ledger", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sum, 49_000);
}

#[tokio::test]
async fn payments_after_a_ban_are_recorded_for_review_without_granting_access() {
    let f = Fixture::new();
    let user = f.user();
    let (_, order) = commerce::choose_role(&f.state, &user, "employer", Some("stripe")).unwrap();
    let order = order.unwrap();
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id=?1", [&user])
        .unwrap();
    commerce::settle_payment(&f.state, receipt(&order, "first"))
        .await
        .unwrap();
    assert_eq!(
        stored_order(&f.state, &order.id).status,
        "paid_needs_review"
    );
    assert!(
        !models::load_user(&f.state.conn().unwrap(), &user)
            .unwrap()
            .employer_paid
    );
    assert_eq!(count(&f.state, "ledger"), 1);
    assert_eq!(
        commerce::new_job(&f.state, &user, "A job", "Description", 300_000, "stripe")
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn paid_stale_edit_does_not_overwrite_a_newer_administrator_revision() {
    let f = Fixture::new();
    let employer = f.employer().await;
    let job = f.published_job(&employer).await;
    let (_, order) =
        commerce::edit_job(&f.state, &employer, &job.id, "Pending paid edit", "stripe").unwrap();
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET jd_md='Administrator revision',version=version+1 WHERE id=?1",
            [&job.id],
        )
        .unwrap();
    commerce::settle_payment(&f.state, receipt(&order, "first"))
        .await
        .unwrap();
    assert_eq!(
        stored_order(&f.state, &order.id).status,
        "paid_needs_review"
    );
    let current = models::load_job(&f.state.conn().unwrap(), &job.id).unwrap();
    assert_eq!(current.jd_md, "Administrator revision");
    assert_eq!(current.edits_used, 0);
    assert_eq!(count(&f.state, "ledger"), 3);
}

fn agent_token(state: &AppState, user: &str) -> String {
    let token = format!("hf_test_{}", uuid::Uuid::new_v4());
    let hash = hex::encode(Sha256::digest(token.as_bytes()));
    state.conn().unwrap().execute(
        "INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES(?1,?2,'test',?3,?4,?5)",
        params![uuid::Uuid::new_v4().to_string(), user, hash, hireferry::now(), hireferry::now() + 300],
    ).unwrap();
    token
}

#[tokio::test]
async fn edit_route_rejects_title_salary_and_other_accounts() {
    let f = Fixture::new();
    let employer = f.employer().await;
    let job = f.published_job(&employer).await;
    let token = agent_token(&f.state, &employer);
    let router = api::routes().with_state(f.state.clone());
    for protected in [json!({"title":"Replaced title"}), json!({"salary_cents":1})] {
        let mut body = json!({"jd_md":"New description","provider":"stripe"});
        body.as_object_mut()
            .unwrap()
            .extend(protected.as_object().unwrap().clone());
        let request = Request::builder()
            .method("POST")
            .uri(format!("/api/jobs/{}/edit", job.id))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let other = f.employer().await;
    let other_token = agent_token(&f.state, &other);
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/jobs/{}/edit", job.id))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {other_token}"))
        .body(Body::from(
            json!({"jd_md":"Unowned edit","provider":"stripe"}).to_string(),
        ))
        .unwrap();
    assert_eq!(
        router.oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let current = models::load_job(&f.state.conn().unwrap(), &job.id).unwrap();
    assert_eq!(current.title, job.title);
    assert_eq!(current.salary_cents, job.salary_cents);
    assert_eq!(current.jd_md, job.jd_md);
    let edits: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM orders WHERE kind='edit'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(edits, 0);
}

// Official Waffo refund envelopes are signed with a local RSA key here. These
// integration tests never contact Waffo or request a merchant-side refund.
fn refund_signing_key() -> &'static RsaPrivateKey {
    static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
    KEY.get_or_init(|| RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap())
}

impl Fixture {
    fn waffo() -> Self {
        let mut fixture = Self::new();
        let mut cfg = (*fixture.state.cfg).clone();
        cfg.payments.waffo.enabled = true;
        cfg.payments.waffo.merchant_id = "MER_local_refund_fixture".into();
        cfg.payments.waffo.store_id = "STO_local_refund_fixture".into();
        cfg.payments.waffo.product_id = "PRO_local_refund_fixture".into();
        cfg.payments.waffo.tax_category = "services".into();
        cfg.payments.waffo.mode = "test".into();
        cfg.payments.waffo.private_key_pem = refund_signing_key()
            .to_pkcs8_pem(LineEnding::LF)
            .unwrap()
            .to_string();
        cfg.payments.waffo.webhook_public_key_pem = refund_signing_key()
            .to_public_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        fixture.state = AppState::new(cfg).unwrap();
        fixture
    }

    async fn waffo_onboarding(&self) -> (String, Order) {
        let uid = self.user();
        let (_, order) =
            commerce::choose_role(&self.state, &uid, "employer", Some("waffo")).unwrap();
        let order = order.unwrap();
        commerce::settle_payment(&self.state, receipt(&order, "first"))
            .await
            .unwrap();
        (uid, stored_order(&self.state, &order.id))
    }
}

fn refund_receipt(order: &Order, id: &str, amount: i64, succeeded: bool) -> RefundReceipt {
    RefundReceipt {
        provider: "waffo".into(),
        event_id: id.into(),
        refund_id: id.into(),
        order_id: order.id.clone(),
        provider_order_id: format!("ORD_{}", order.id),
        payment_id: format!("pi_{}", order.id),
        amount_cents: amount,
        original_amount_cents: Some(order.amount_cents),
        currency: "CNY".into(),
        live_mode: order.live_mode,
        succeeded,
        reason: "Local test refund notification".into(),
    }
}

fn decimal(cents: i64) -> String {
    format!("{}.{:02}", cents / 100, cents % 100)
}

fn refund_event(order: &Order, id: &str, amount: i64, succeeded: bool) -> Value {
    let outcome = if succeeded { "succeeded" } else { "failed" };
    json!({
        "id":id, "eventId":id, "eventType":format!("refund.{outcome}"),
        "timestamp":chrono::Utc::now().to_rfc3339(),
        "storeId":"STO_local_refund_fixture",
        "mode":if order.live_mode {"prod"} else {"test"},
        "data":{
            "orderId":format!("ORD_{}",order.id), "orderStatus":"completed",
            "orderMerchantExternalId":order.id,
            "orderMetadata":{"hireferry_order_id":order.id},
            "currency":"CNY", "paymentId":format!("pi_{}",order.id),
            "paymentStatus":"succeeded", "refundedAmount":decimal(amount),
            "amount":decimal(amount), "originalChargedAmount":decimal(order.amount_cents),
            // Deliberately different list price: it is never the cash refund.
            "total":"99999.00", "originalPayment":{"total":"99999.00"},
            "refundStatus":outcome, "refundReason":"Local test refund notification",
            "refundCreatedAt":chrono::Utc::now().to_rfc3339()
        }
    })
}

fn waffo_signature(body: &[u8]) -> String {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let mut signed = format!("{timestamp}.").into_bytes();
    signed.extend_from_slice(body);
    let signature = SigningKey::<Sha256>::new(refund_signing_key().clone()).sign(&signed);
    format!("t={timestamp},v1={}", STANDARD.encode(signature.to_bytes()))
}

async fn post_waffo_body(state: &AppState, body: Vec<u8>, signature: &str) -> StatusCode {
    hireferry::payments::routes()
        .with_state(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webhooks/waffo")
                .header("content-type", "application/json")
                .header("x-waffo-signature", signature)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn post_waffo(state: &AppState, event: &Value) -> StatusCode {
    let body = serde_json::to_vec(event).unwrap();
    let signature = waffo_signature(&body);
    post_waffo_body(state, body, &signature).await
}

fn refund_total(state: &AppState, order_id: &str) -> i64 {
    state.conn().unwrap().query_row(
        "SELECT COALESCE(sum(amount_cents),0) FROM payment_refunds WHERE order_id=?1 AND status='succeeded'",
        [order_id], |row| row.get(0),
    ).unwrap()
}

#[test]
fn refund_migration_adds_cached_total_without_losing_legacy_orders() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE orders(id TEXT PRIMARY KEY,amount_cents INTEGER NOT NULL);
        INSERT INTO orders VALUES('old_order',10000);",
    )
    .unwrap();
    commerce::migrate(&db).unwrap();
    commerce::migrate(&db).unwrap();
    let row: (i64, i64) = db
        .query_row(
            "SELECT amount_cents,refunded_cents FROM orders WHERE id='old_order'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, (10000, 0));
    let tables: i64 = db.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('payment_refunds','refund_events')",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(tables, 2);
}

#[tokio::test]
async fn signed_waffo_refunds_reject_tampering_wrong_scope_and_unknown_payment() {
    let f = Fixture::waffo();
    let (_, order) = f.waffo_onboarding().await;
    let good = refund_event(&order, "REF_checked", 4000, true);

    let body = serde_json::to_vec(&good).unwrap();
    let signature = waffo_signature(&body);
    let mut tampered = body.clone();
    tampered.push(b' ');
    assert_eq!(
        post_waffo_body(&f.state, tampered, &signature).await,
        StatusCode::UNAUTHORIZED
    );

    for (pointer, value) in [
        ("/storeId", json!("STO_wrong")),
        ("/mode", json!("prod")),
        ("/eventId", json!("REF_other")),
        ("/data/refundStatus", json!("failed")),
        ("/data/currency", json!("USD")),
        ("/data/paymentId", json!("PAY_unknown")),
        (
            "/data/orderMetadata/hireferry_order_id",
            json!("other_order"),
        ),
        ("/data/orderMetadata/hireferry_order_id", json!(null)),
        ("/data/originalChargedAmount", json!("1.00")),
        ("/data/refundedAmount", json!("40.001")),
        ("/data/refundedAmount", json!("-1.00")),
        ("/data/refundedAmount", json!("100.01")),
    ] {
        let mut invalid = good.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            post_waffo(&f.state, &invalid).await,
            StatusCode::BAD_REQUEST,
            "{pointer}"
        );
    }
    let mut unknown_order = good.clone();
    unknown_order["data"]["orderMerchantExternalId"] = json!("unknown_order");
    unknown_order["data"]["orderMetadata"]["hireferry_order_id"] = json!("unknown_order");
    assert_eq!(
        post_waffo(&f.state, &unknown_order).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(count(&f.state, "refund_events"), 0);
    assert_eq!(count(&f.state, "payment_refunds"), 0);
    assert_eq!(stored_order(&f.state, &order.id).refunded_cents, 0);

    assert_eq!(post_waffo(&f.state, &good).await, StatusCode::NO_CONTENT);
    assert_eq!(post_waffo(&f.state, &good).await, StatusCode::NO_CONTENT);
    assert_eq!(refund_total(&f.state, &order.id), 4000);
    assert_eq!(stored_order(&f.state, &order.id).refunded_cents, 4000);
    assert_eq!(count(&f.state, "refund_events"), 1);

    let mut changed_duplicate = good.clone();
    changed_duplicate["data"]["refundedAmount"] = json!("30.00");
    assert_eq!(
        post_waffo(&f.state, &changed_duplicate).await,
        StatusCode::CONFLICT
    );
    assert_eq!(refund_total(&f.state, &order.id), 4000);
}

#[tokio::test]
async fn refund_before_original_payment_is_retryable_and_legacy_amount_is_supported() {
    let f = Fixture::waffo();
    let uid = f.user();
    let (_, order) = commerce::choose_role(&f.state, &uid, "employer", Some("waffo")).unwrap();
    let order = order.unwrap();
    let mut event = refund_event(&order, "REF_early", 2000, true);
    event["data"]
        .as_object_mut()
        .unwrap()
        .remove("refundedAmount");
    event["data"]
        .as_object_mut()
        .unwrap()
        .remove("originalChargedAmount");
    assert_eq!(post_waffo(&f.state, &event).await, StatusCode::CONFLICT);
    assert_eq!(count(&f.state, "refund_events"), 0);
    commerce::settle_payment(&f.state, receipt(&order, "first"))
        .await
        .unwrap();
    assert_eq!(post_waffo(&f.state, &event).await, StatusCode::NO_CONTENT);
    assert_eq!(refund_total(&f.state, &order.id), 2000);

    let mut no_cash_amount = refund_event(&order, "REF_no_amount", 1000, true);
    no_cash_amount["data"]
        .as_object_mut()
        .unwrap()
        .remove("refundedAmount");
    no_cash_amount["data"]
        .as_object_mut()
        .unwrap()
        .remove("amount");
    assert_eq!(
        post_waffo(&f.state, &no_cash_amount).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(refund_total(&f.state, &order.id), 2000);
}

#[tokio::test]
async fn failed_and_reordered_refunds_never_deduct_or_restore_money_twice() {
    let f = Fixture::waffo();
    let (uid, order) = f.waffo_onboarding().await;
    let failed = refund_event(&order, "REF_retry", 5000, false);
    assert_eq!(post_waffo(&f.state, &failed).await, StatusCode::NO_CONTENT);
    assert_eq!(post_waffo(&f.state, &failed).await, StatusCode::NO_CONTENT);
    assert_eq!(refund_total(&f.state, &order.id), 0);
    assert_eq!(stored_order(&f.state, &order.id).status, "paid");
    assert_eq!(stored_order(&f.state, &order.id).refunded_cents, 0);
    assert!(
        models::load_user(&f.state.conn().unwrap(), &uid)
            .unwrap()
            .employer_paid
    );

    let success = refund_event(&order, "REF_retry", 5000, true);
    assert_eq!(post_waffo(&f.state, &success).await, StatusCode::NO_CONTENT);
    assert_eq!(post_waffo(&f.state, &failed).await, StatusCode::NO_CONTENT);
    assert_eq!(refund_total(&f.state, &order.id), 5000);

    let success_first = refund_event(&order, "REF_late_failure", 5000, true);
    assert_eq!(
        post_waffo(&f.state, &success_first).await,
        StatusCode::NO_CONTENT
    );
    let failure_late = refund_event(&order, "REF_late_failure", 5000, false);
    assert_eq!(
        post_waffo(&f.state, &failure_late).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(refund_total(&f.state, &order.id), 10000);
    assert_eq!(stored_order(&f.state, &order.id).status, "refunded");
    assert!(
        !models::load_user(&f.state.conn().unwrap(), &uid)
            .unwrap()
            .employer_paid
    );
    assert_eq!(count(&f.state, "payment_refunds"), 2);
    assert_eq!(count(&f.state, "refund_events"), 4);
    let succeeded: i64 = f.state.conn().unwrap().query_row(
        "SELECT count(*) FROM payment_refunds WHERE status='succeeded' AND succeeded_at IS NOT NULL",[],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(succeeded, 2);
    // The immutable charge remains available for gross/net accounting.
    assert_eq!(count(&f.state, "ledger"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_refunds_are_exactly_once_and_cannot_exceed_the_charge() {
    let f = Fixture::waffo();
    let (_, order) = f.waffo_onboarding().await;
    let states: Vec<_> = (0..8)
        .map(|_| AppState::new((*f.state.cfg).clone()).unwrap())
        .collect();
    let barrier = Arc::new(tokio::sync::Barrier::new(states.len()));
    let mut tasks = Vec::new();
    for state in states {
        let barrier = barrier.clone();
        let refund = refund_receipt(&order, "REF_concurrent", 6000, true);
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            commerce::settle_refund(&state, refund).await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    assert_eq!(refund_total(&f.state, &order.id), 6000);
    assert_eq!(count(&f.state, "payment_refunds"), 1);
    assert_eq!(count(&f.state, "refund_events"), 1);
    assert_eq!(
        stored_order(&f.state, &order.id).status,
        "partially_refunded"
    );

    let oversized = refund_receipt(&order, "REF_over_remaining", 4001, true);
    assert_eq!(
        commerce::settle_refund(&f.state, oversized)
            .await
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(refund_total(&f.state, &order.id), 6000);
    assert_eq!(count(&f.state, "payment_refunds"), 1);
    // Distinct concurrent IDs must also share the remaining cash ceiling.
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut tasks = Vec::new();
    for index in 0..2 {
        let state = AppState::new((*f.state.cfg).clone()).unwrap();
        let barrier = barrier.clone();
        let remainder = refund_receipt(&order, &format!("REF_remainder_{index}"), 4000, true);
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            commerce::settle_refund(&state, remainder).await
        }));
    }
    let mut successes = 0;
    let mut rejected = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(()) => successes += 1,
            Err(error) => {
                assert_eq!(error.status, StatusCode::BAD_REQUEST);
                rejected += 1;
            }
        }
    }
    assert_eq!((successes, rejected), (1, 1));
    assert_eq!(stored_order(&f.state, &order.id).refunded_cents, 10000);
    assert_eq!(stored_order(&f.state, &order.id).status, "refunded");
}

#[tokio::test]
async fn partial_publish_refund_preserves_edits_and_full_refund_closes_without_regrant() {
    let f = Fixture::waffo();
    let (employer, _) = f.waffo_onboarding().await;
    let (job, order) = commerce::new_job(
        &f.state,
        &employer,
        "Rust role",
        "Original JD",
        300_000,
        "waffo",
    )
    .unwrap();
    commerce::settle_payment(&f.state, receipt(&order, "first"))
        .await
        .unwrap();
    assert_eq!(
        post_waffo(
            &f.state,
            &refund_event(&order, "REF_partial_publish", 5000, true)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        models::load_job(&f.state.conn().unwrap(), &job.id)
            .unwrap()
            .status,
        "published"
    );
    let (_, edit) =
        commerce::edit_job(&f.state, &employer, &job.id, "Pending revised JD", "waffo").unwrap();

    assert_eq!(
        post_waffo(
            &f.state,
            &refund_event(&order, "REF_full_publish", 25000, true)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(stored_order(&f.state, &order.id).status, "refunded");
    assert_eq!(
        models::load_job(&f.state.conn().unwrap(), &job.id)
            .unwrap()
            .status,
        "closed"
    );
    commerce::settle_payment(&f.state, receipt(&order, "late_duplicate"))
        .await
        .unwrap();
    assert_eq!(stored_order(&f.state, &order.id).status, "refunded");
    assert_eq!(
        models::load_job(&f.state.conn().unwrap(), &job.id)
            .unwrap()
            .status,
        "closed"
    );

    commerce::settle_payment(&f.state, receipt(&edit, "late_edit"))
        .await
        .unwrap();
    assert_eq!(stored_order(&f.state, &edit.id).status, "paid_needs_review");
    let latest = models::load_job(&f.state.conn().unwrap(), &job.id).unwrap();
    assert_eq!(latest.jd_md, "Original JD");
    assert_eq!(latest.edits_used, 0);
}

#[tokio::test]
async fn refunded_edit_keeps_applied_text_and_does_not_reset_the_edit_allowance() {
    let f = Fixture::waffo();
    let (employer, _) = f.waffo_onboarding().await;
    let (job, publish) = commerce::new_job(
        &f.state,
        &employer,
        "Rust role",
        "Original JD",
        300_000,
        "waffo",
    )
    .unwrap();
    commerce::settle_payment(&f.state, receipt(&publish, "first"))
        .await
        .unwrap();
    let (_, edit) =
        commerce::edit_job(&f.state, &employer, &job.id, "Paid revised JD", "waffo").unwrap();
    commerce::settle_payment(&f.state, receipt(&edit, "first"))
        .await
        .unwrap();
    let applied = models::load_job(&f.state.conn().unwrap(), &job.id).unwrap();
    assert_eq!(
        post_waffo(
            &f.state,
            &refund_event(&edit, "REF_edit", edit.amount_cents, true)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let retained = models::load_job(&f.state.conn().unwrap(), &job.id).unwrap();
    assert_eq!(retained.jd_md, applied.jd_md);
    assert_eq!(retained.edits_used, 1);
    assert_eq!(retained.version, applied.version);
    assert_eq!(retained.status, "published");
    assert_eq!(stored_order(&f.state, &edit.id).status, "refunded");
}

#[tokio::test]
async fn review_refunds_do_not_grant_access_or_revoke_a_different_paid_activation() {
    let f = Fixture::waffo();
    let uid = f.user();
    let (_, old) = commerce::choose_role(&f.state, &uid, "employer", Some("waffo")).unwrap();
    let old = old.unwrap();
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id=?1", [&uid])
        .unwrap();
    commerce::settle_payment(&f.state, receipt(&old, "first"))
        .await
        .unwrap();
    assert_eq!(stored_order(&f.state, &old.id).status, "paid_needs_review");
    assert_eq!(
        post_waffo(&f.state, &refund_event(&old, "REF_review_part", 4000, true)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(stored_order(&f.state, &old.id).status, "paid_needs_review");
    assert_eq!(stored_order(&f.state, &old.id).refunded_cents, 4000);
    assert!(
        !models::load_user(&f.state.conn().unwrap(), &uid)
            .unwrap()
            .employer_paid
    );

    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=0 WHERE id=?1", [&uid])
        .unwrap();
    let (_, replacement) =
        commerce::choose_role(&f.state, &uid, "employer", Some("waffo")).unwrap();
    commerce::settle_payment(&f.state, receipt(&replacement.unwrap(), "first"))
        .await
        .unwrap();
    assert!(
        models::load_user(&f.state.conn().unwrap(), &uid)
            .unwrap()
            .employer_paid
    );
    assert_eq!(
        post_waffo(&f.state, &refund_event(&old, "REF_review_rest", 6000, true)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(stored_order(&f.state, &old.id).status, "refunded");
    assert!(
        models::load_user(&f.state.conn().unwrap(), &uid)
            .unwrap()
            .employer_paid
    );
}

#[tokio::test]
async fn refund_statistics_use_cashflow_dates_and_keep_live_and_test_separate() {
    let f = Fixture::waffo();
    let (_, test_order) = f.waffo_onboarding().await;
    // The charge was yesterday; the confirmed refund is received today.
    let yesterday = chrono::Utc::now().date_naive() - chrono::Duration::days(1);
    let charged_at = yesterday
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE ledger SET created_at=?2 WHERE order_id=?1",
            params![test_order.id, charged_at],
        )
        .unwrap();
    commerce::settle_refund(
        &f.state,
        refund_receipt(&test_order, "REF_stat_failed", 2000, false),
    )
    .await
    .unwrap();
    commerce::settle_refund(
        &f.state,
        refund_receipt(&test_order, "REF_stat_test", 4000, true),
    )
    .await
    .unwrap();
    let admin = hireferry::access::Principal {
        user_id: None,
        admin: true,
        via_ai: false,
        actor: "admin".into(),
    };
    let query = |mode: &str| hireferry::admin::QueryInput {
        mode: Some(mode.into()),
        ..Default::default()
    };
    let test = hireferry::admin::overview_data(&f.state, &admin, &query("test")).unwrap();
    assert_eq!(test["stats"]["gross_cents"], 10000);
    assert_eq!(test["stats"]["refund_cents"], 4000);
    assert_eq!(test["stats"]["revenue_cents"], 6000);
    assert_eq!(test["stats"]["paid_orders"], 1);
    let today = chrono::Utc::now().date_naive().to_string();
    let refund_day = hireferry::admin::overview_data(
        &f.state,
        &admin,
        &hireferry::admin::QueryInput {
            from: Some(today.clone()),
            to: Some(today),
            ..query("test")
        },
    )
    .unwrap();
    assert_eq!(refund_day["stats"]["gross_cents"], 0);
    assert_eq!(refund_day["stats"]["refund_cents"], 4000);
    assert_eq!(refund_day["stats"]["revenue_cents"], -4000);
    assert_eq!(refund_day["series"][0]["revenue_cents"], -4000);
    assert_eq!(refund_day["revenue_by_kind"][0]["amount_cents"], -4000);

    let empty_live = hireferry::admin::overview_data(&f.state, &admin, &query("live")).unwrap();
    assert_eq!(empty_live["stats"]["gross_cents"], 0);
    assert_eq!(empty_live["stats"]["refund_cents"], 0);
    // A local fixture represents a distinct live-mode receipt; no production
    // gateway is enabled or called by this test.
    let live_uid = f.user();
    let (_, live_order) =
        commerce::choose_role(&f.state, &live_uid, "employer", Some("waffo")).unwrap();
    let live_order = live_order.unwrap();
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE orders SET live_mode=1 WHERE id=?1",
            [&live_order.id],
        )
        .unwrap();
    let live_order = stored_order(&f.state, &live_order.id);
    let mut live_payment = receipt(&live_order, "live_fixture");
    live_payment.live_mode = true;
    commerce::settle_payment(&f.state, live_payment)
        .await
        .unwrap();
    commerce::settle_refund(
        &f.state,
        refund_receipt(&live_order, "REF_stat_live", 3000, true),
    )
    .await
    .unwrap();
    let live = hireferry::admin::overview_data(&f.state, &admin, &query("live")).unwrap();
    assert_eq!(live["stats"]["gross_cents"], 10000);
    assert_eq!(live["stats"]["refund_cents"], 3000);
    assert_eq!(live["stats"]["revenue_cents"], 7000);
    let test_again = hireferry::admin::overview_data(&f.state, &admin, &query("test")).unwrap();
    assert_eq!(test_again["stats"]["revenue_cents"], 6000);
}
