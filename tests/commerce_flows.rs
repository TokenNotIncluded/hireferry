//! Business invariants against an actual SQLite file. No gateway or email calls.
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{
    AppState, api, commerce,
    config::Config,
    models::{self, Job, Order},
    payments::PaymentReceipt,
};
use rusqlite::params;
use serde_json::json;
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
