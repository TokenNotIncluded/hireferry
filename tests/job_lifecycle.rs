//! One recoverable deletion, versioned retries, and permanent content removal.
//! All payment receipts below are local fixtures; no merchant is contacted.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{
    AppState, api, auth, commerce,
    config::Config,
    models::{self, Job, Order},
    payments::{PaymentReceipt, RefundReceipt},
};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

struct Fixture {
    _directory: TempDir,
    state: AppState,
    app: Router,
}
struct Browser {
    raw: String,
    csrf: String,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut cfg = Config {
            development: true,
            database: directory
                .path()
                .join("lifecycle.sqlite3")
                .to_string_lossy()
                .into_owned(),
            ..Default::default()
        };
        cfg.payments.stripe.enabled = true;
        cfg.payments.stripe.secret_key = "sk_test_local_lifecycle_fixture".into();
        cfg.payments.stripe.webhook_secret = "whsec_local_lifecycle_fixture".into();
        cfg.payments.waffo.enabled = true;
        cfg.payments.waffo.merchant_id = "local_fixture".into();
        cfg.payments.waffo.store_id = "local_fixture".into();
        cfg.payments.waffo.product_id = "local_fixture".into();
        cfg.payments.waffo.private_key_pem = "unused local fixture".into();
        cfg.payments.waffo.webhook_public_key_pem = "unused local fixture".into();
        cfg.payments.waffo.tax_category = "local_fixture".into();
        let state = AppState::new(cfg).unwrap();
        let app = hireferry::app(state.clone());
        Self {
            _directory: directory,
            state,
            app,
        }
    }
    fn employer(&self) -> String {
        let uid = uuid::Uuid::new_v4().to_string();
        self.state.conn().unwrap().execute("INSERT INTO users(id,email,name,role,employer_paid,created_at,updated_at) VALUES(?1,?2,'Local employer','employer',1,?3,?3)",params![uid,format!("{uid}@example.invalid"),hireferry::now()]).unwrap();
        uid
    }
    fn browser(&self, uid: &str) -> Browser {
        let browser = Browser {
            raw: uuid::Uuid::new_v4().to_string(),
            csrf: uuid::Uuid::new_v4().to_string(),
        };
        self.state.conn().unwrap().execute("INSERT INTO sessions(token_hash,user_id,csrf,expires_at,recent_auth_at) VALUES(?1,?2,?3,?4,?5)",params![auth::hash_token(&browser.raw),uid,browser.csrf,hireferry::now()+3600,hireferry::now()]).unwrap();
        browser
    }
    fn draft(&self, uid: &str, count: i64, provider: &str) -> (Job, Order) {
        commerce::new_job_with_headcount(
            &self.state,
            uid,
            "Private role title",
            "Private original JD",
            300000,
            provider,
            count,
        )
        .unwrap()
    }
    async fn publish(&self, uid: &str, count: i64, provider: &str) -> (Job, Order) {
        let (job, order) = self.draft(uid, count, provider);
        commerce::settle_payment(&self.state, receipt(&order))
            .await
            .unwrap();
        (self.job(&job.id), order)
    }
    fn job(&self, id: &str) -> Job {
        models::load_job(&self.state.conn().unwrap(), id).unwrap()
    }
    fn order(&self, id: &str) -> Order {
        models::load_order(&self.state.conn().unwrap(), id).unwrap()
    }
    fn count(&self, table: &str) -> i64 {
        self.state
            .conn()
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
    async fn refund(&self, order: &Order, amount: i64) {
        let id = format!("REF_{}", uuid::Uuid::new_v4());
        commerce::settle_refund(
            &self.state,
            RefundReceipt {
                provider: "waffo".into(),
                event_id: id.clone(),
                refund_id: id,
                order_id: order.id.clone(),
                provider_order_id: format!("ORD_{}", order.id),
                payment_id: format!("pay_{}", order.id),
                amount_cents: amount,
                original_amount_cents: Some(order.amount_cents),
                currency: "CNY".into(),
                live_mode: false,
                succeeded: true,
                reason: "Local fixture".into(),
            },
        )
        .await
        .unwrap();
    }
}
fn receipt(order: &Order) -> PaymentReceipt {
    PaymentReceipt {
        provider: order.provider.clone(),
        event_id: format!("evt_{}", order.id),
        order_id: order.id.clone(),
        session_id: (order.provider == "stripe").then(|| format!("cs_{}", order.id)),
        payment_id: format!("pay_{}", order.id),
        amount_cents: order.amount_cents,
        currency: order.currency.clone(),
        live_mode: order.live_mode,
    }
}
fn request(
    method: &str,
    path: &str,
    browser: Option<&Browser>,
    csrf: bool,
    origin: &str,
    body: Option<Value>,
) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("origin", origin);
    if let Some(browser) = browser {
        b = b.header("cookie", format!("hf_session={}", browser.raw));
        if csrf {
            b = b.header("x-csrf-token", &browser.csrf);
        }
    }
    let body = if let Some(body) = body {
        b = b.header("content-type", "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    b.body(body).unwrap()
}
async fn observe(f: &Fixture, request: Request<Body>) -> (StatusCode, Value) {
    let response = f.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn first_delete_retries_restore_and_second_delete_preserve_billing_but_remove_content() {
    let f = Fixture::new();
    let owner = f.employer();
    let (job, publish) = f.publish(&owner, 4, "stripe").await;
    let (_, edit) =
        commerce::edit_job(&f.state, &owner, &job.id, "Paid sensitive JD", "stripe").unwrap();
    commerce::settle_payment(&f.state, receipt(&edit))
        .await
        .unwrap();
    let before = f.job(&job.id);
    let financial = (f.count("orders"), f.count("ledger"));
    let trashed = commerce::delete_job(&f.state, &owner, &job.id, before.version).unwrap();
    assert_eq!(
        (
            trashed.status.as_str(),
            trashed.delete_count,
            trashed.headcount
        ),
        ("deleted", 1, 0)
    );
    assert_eq!(trashed.version, before.version + 1);
    assert_eq!(trashed.jd_md, before.jd_md);
    assert_eq!(trashed.edits_used, before.edits_used);
    let repeated = commerce::delete_job(&f.state, &owner, &job.id, before.version).unwrap();
    assert_eq!(
        (repeated.version, repeated.delete_count),
        (trashed.version, 1)
    );
    assert_eq!(
        api::deleted_jobs(&f.state, &owner).unwrap()["items"][0]["jd_md"],
        "Paid sensitive JD"
    );
    assert_eq!(
        api::list_jobs(&f.state, &api::ListQuery::default(), Some(&owner)).unwrap()["total"],
        0
    );
    let restored = commerce::restore_job(&f.state, &owner, &job.id, trashed.version).unwrap();
    assert_eq!(
        (
            restored.status.as_str(),
            restored.headcount,
            restored.delete_count
        ),
        ("published", 4, 1)
    );
    assert_eq!(restored.version, trashed.version + 1);
    assert_eq!(restored.jd_md, before.jd_md);
    assert_eq!(
        commerce::delete_job(&f.state, &owner, &job.id, before.version)
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(f.job(&job.id).status, "published");
    let permanent = commerce::delete_job(&f.state, &owner, &job.id, restored.version).unwrap();
    assert_eq!(
        (
            permanent.status.as_str(),
            permanent.delete_count,
            permanent.headcount
        ),
        ("deleted", 2, 0)
    );
    assert!(permanent.title.is_empty() && permanent.jd_md.is_empty());
    assert_eq!(permanent.edits_used, before.edits_used);
    assert_eq!(
        commerce::delete_job(&f.state, &owner, &job.id, restored.version)
            .unwrap()
            .version,
        permanent.version
    );
    assert_eq!(
        commerce::restore_job(&f.state, &owner, &job.id, permanent.version)
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        api::deleted_jobs(&f.state, &owner).unwrap()["items"],
        json!([])
    );
    assert_eq!((f.count("orders"), f.count("ledger")), financial);
    for order in [&publish, &edit] {
        let stored = f.order(&order.id);
        assert_eq!(stored.snapshot_json, "{}");
        assert_eq!(stored.status, "paid");
        assert_eq!(stored.amount_cents, order.amount_cents);
    }
    let events:i64=f.state.conn().unwrap().query_row("SELECT count(*) FROM audit WHERE target=?1 AND action IN ('job_delete','job_restore','job_delete_permanent')",[&job.id],|r|r.get(0)).unwrap();
    assert_eq!(events, 3);
}

#[tokio::test]
async fn trash_hides_cached_checkout_and_restored_pending_draft_keeps_monotonic_version() {
    let f = Fixture::new();
    let owner = f.employer();
    let (job, order) = f.draft(&owner, 2, "stripe");
    let cached = "https://checkout.stripe.com/c/pay/local_fixture";
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE orders SET checkout_url=?2,provider_session=?3 WHERE id=?1",
            params![order.id, cached, format!("cs_{}", order.id)],
        )
        .unwrap();
    assert_eq!(
        commerce::checkout(&f.state, &owner, &order.id)
            .await
            .unwrap()
            .checkout_url
            .as_deref(),
        Some(cached)
    );
    let deleted = commerce::delete_job(&f.state, &owner, &job.id, job.version).unwrap();
    assert_eq!(f.order(&order.id).status, "pending");
    assert!(
        api::user_order(&f.state, &owner, &order.id)
            .unwrap()
            .checkout_url
            .is_none()
    );
    assert!(
        api::user_orders(&f.state, &owner).unwrap()[0]
            .checkout_url
            .is_none()
    );
    assert_eq!(
        commerce::checkout(&f.state, &owner, &order.id)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    // The original provider session is retained internally, never recreated here.
    assert_eq!(f.order(&order.id).checkout_url.as_deref(), Some(cached));
    let restored = commerce::restore_job(&f.state, &owner, &job.id, deleted.version).unwrap();
    assert_eq!((restored.status.as_str(), restored.headcount), ("draft", 2));
    assert_eq!(
        commerce::checkout(&f.state, &owner, &order.id)
            .await
            .unwrap()
            .checkout_url
            .as_deref(),
        Some(cached)
    );
    commerce::settle_payment(&f.state, receipt(&order))
        .await
        .unwrap();
    let paid = f.job(&job.id);
    assert_eq!(paid.status, "published");
    assert_eq!(paid.version, restored.version + 1);
    assert_eq!(
        commerce::permanently_delete_job(&f.state, &owner, &job.id, deleted.version)
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(f.count("orders"), 1);
}

#[tokio::test]
async fn deleting_a_job_cancels_unusable_edits_and_restoration_allows_a_fresh_edit() {
    let f = Fixture::new();
    let owner = f.employer();
    let (job, _) = f.publish(&owner, 6, "stripe").await;
    let (_, old_edit) =
        commerce::edit_job(&f.state, &owner, &job.id, "Old pending text", "stripe").unwrap();
    f.state.conn().unwrap().execute(
        "UPDATE orders SET checkout_url='https://checkout.stripe.com/c/pay/local_fixture' WHERE id=?1",
        [&old_edit.id],
    ).unwrap();
    let deleted = commerce::delete_job(&f.state, &owner, &job.id, job.version).unwrap();
    assert_eq!(f.order(&old_edit.id).status, "cancelled");
    assert!(f.order(&old_edit.id).checkout_url.is_none());
    let restored = commerce::restore_job(&f.state, &owner, &job.id, deleted.version).unwrap();
    assert_eq!(restored.edits_used, 0);
    assert_eq!(
        commerce::checkout(&f.state, &owner, &old_edit.id)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert!(
        api::user_order(&f.state, &owner, &old_edit.id)
            .unwrap()
            .checkout_url
            .is_none()
    );
    let (_, fresh_edit) =
        commerce::edit_job(&f.state, &owner, &job.id, "Current intended text", "stripe").unwrap();
    // Payment on a previously opened gateway page is recorded, but cannot
    // overwrite either the restored content or the fresh intended edit.
    commerce::settle_payment(&f.state, receipt(&old_edit))
        .await
        .unwrap();
    assert_eq!(f.order(&old_edit.id).status, "paid_needs_review");
    assert_eq!(f.job(&job.id).jd_md, job.jd_md);
    commerce::settle_payment(&f.state, receipt(&fresh_edit))
        .await
        .unwrap();
    assert_eq!(f.job(&job.id).jd_md, "Current intended text");
    assert_eq!(f.job(&job.id).edits_used, 1);
    assert_eq!(f.count("ledger"), 3);
}

#[tokio::test]
async fn unusable_pending_edit_checkouts_are_hidden_and_rejected_before_gateway_io() {
    for reason in [
        "activation_revoked",
        "publication_refunded",
        "stale_version",
    ] {
        let f = Fixture::new();
        let owner = f.employer();
        let (job, publication) = f.publish(&owner, 2, "waffo").await;
        let (_, edit) =
            commerce::edit_job(&f.state, &owner, &job.id, "Pending text", "waffo").unwrap();
        f.state.conn().unwrap().execute(
            "UPDATE orders SET checkout_url='https://checkout.example.invalid/local_fixture' WHERE id=?1",
            [&edit.id],
        ).unwrap();
        match reason {
            "activation_revoked" => {
                f.state
                    .conn()
                    .unwrap()
                    .execute("UPDATE users SET employer_paid=0 WHERE id=?1", [&owner])
                    .unwrap();
            }
            "publication_refunded" => f.refund(&publication, publication.amount_cents).await,
            _ => {
                // A recovered database from the earlier implementation may
                // still contain an edit for a pre-delete content version.
                f.state
                    .conn()
                    .unwrap()
                    .execute("UPDATE jobs SET version=version+2 WHERE id=?1", [&job.id])
                    .unwrap();
            }
        }
        assert!(
            api::user_order(&f.state, &owner, &edit.id)
                .unwrap()
                .checkout_url
                .is_none(),
            "{reason}"
        );
        assert!(
            api::user_orders(&f.state, &owner)
                .unwrap()
                .iter()
                .all(|order| order.checkout_url.is_none()),
            "{reason}"
        );
        assert_eq!(
            commerce::checkout(&f.state, &owner, &edit.id)
                .await
                .unwrap_err()
                .status,
            StatusCode::CONFLICT,
            "{reason}"
        );
        f.state
            .conn()
            .unwrap()
            .execute(
                "UPDATE orders SET checkout_url=NULL WHERE id=?1",
                [&edit.id],
            )
            .unwrap();
        // This also rejects creating a replacement gateway session.
        assert_eq!(
            commerce::checkout(&f.state, &owner, &edit.id)
                .await
                .unwrap_err()
                .status,
            StatusCode::CONFLICT,
            "{reason}"
        );
        commerce::settle_payment(&f.state, receipt(&edit))
            .await
            .unwrap();
        assert_eq!(f.order(&edit.id).status, "paid_needs_review", "{reason}");
        assert_eq!(f.job(&job.id).jd_md, job.jd_md, "{reason}");
    }
}

#[tokio::test]
async fn paid_edit_delivered_in_trash_is_review_only_and_never_reapplies_after_restore() {
    let f = Fixture::new();
    let owner = f.employer();
    let (job, _) = f.publish(&owner, 6, "stripe").await;
    let (_, edit) =
        commerce::edit_job(&f.state, &owner, &job.id, "Pending private edit", "stripe").unwrap();
    let deleted = commerce::delete_job(&f.state, &owner, &job.id, job.version).unwrap();
    commerce::settle_payment(&f.state, receipt(&edit))
        .await
        .unwrap();
    assert_eq!(f.order(&edit.id).status, "paid_needs_review");
    assert_eq!(f.job(&job.id).status, "deleted");
    assert_eq!(f.job(&job.id).jd_md, job.jd_md);
    let restored = commerce::restore_job(&f.state, &owner, &job.id, deleted.version).unwrap();
    assert_eq!(
        (
            restored.status.as_str(),
            restored.headcount,
            restored.edits_used
        ),
        ("published", 6, 0)
    );
    commerce::settle_payment(&f.state, receipt(&edit))
        .await
        .unwrap();
    assert_eq!(f.job(&job.id).jd_md, job.jd_md);
    assert_eq!(f.job(&job.id).version, restored.version);
    assert_eq!(f.count("orders"), 2);
    assert_eq!(f.count("ledger"), 2);
}

#[tokio::test]
async fn permanent_deletion_cancels_pending_orders_but_records_late_real_receipts_for_review() {
    let f = Fixture::new();
    let owner = f.employer();
    for soft_first in [false, true] {
        let (job, order) = f.draft(&owner, 8, "stripe");
        f.state.conn().unwrap().execute("UPDATE orders SET checkout_url='https://checkout.stripe.com/c/pay/local_fixture' WHERE id=?1",[&order.id]).unwrap();
        let job = if soft_first {
            commerce::delete_job(&f.state, &owner, &job.id, job.version).unwrap()
        } else {
            job
        };
        let removed =
            commerce::permanently_delete_job(&f.state, &owner, &job.id, job.version).unwrap();
        assert_eq!((removed.delete_count, removed.headcount), (2, 0));
        assert!(removed.title.is_empty() && removed.jd_md.is_empty());
        let pending = f.order(&order.id);
        assert_eq!(pending.status, "cancelled");
        assert_eq!(pending.snapshot_json, "{}");
        assert!(pending.checkout_url.is_none());
        assert_eq!(
            commerce::checkout(&f.state, &owner, &order.id)
                .await
                .unwrap_err()
                .status,
            StatusCode::CONFLICT
        );
        commerce::settle_payment(&f.state, receipt(&order))
            .await
            .unwrap();
        assert_eq!(f.order(&order.id).status, "paid_needs_review");
        assert_eq!(f.job(&job.id).status, "deleted");
        assert_eq!(f.job(&job.id).headcount, 0);
        commerce::settle_payment(&f.state, receipt(&order))
            .await
            .unwrap();
        assert_eq!(
            commerce::restore_job(&f.state, &owner, &job.id, removed.version)
                .unwrap_err()
                .status,
            StatusCode::CONFLICT
        );
    }
    assert_eq!(f.count("orders"), 2);
    assert_eq!(f.count("ledger"), 2);
}

#[tokio::test]
async fn restoration_is_free_but_refunds_and_reviewed_drafts_cannot_regain_recruiting_rights() {
    let f = Fixture::new();
    let owner = f.employer();
    // A full publication refund while in the trash preserves recovery of text.
    let (job, order) = f.publish(&owner, 7, "waffo").await;
    let deleted = commerce::delete_job(&f.state, &owner, &job.id, job.version).unwrap();
    f.refund(&order, order.amount_cents).await;
    let restored = commerce::restore_job(&f.state, &owner, &job.id, deleted.version).unwrap();
    assert_eq!(
        (restored.status.as_str(), restored.headcount),
        ("closed", 0)
    );
    assert_eq!(restored.jd_md, job.jd_md);
    // Partially refunded publication rights remain, including manually chosen 0.
    for count in [0, 9] {
        let (job, order) = f.publish(&owner, count, "waffo").await;
        let deleted = commerce::delete_job(&f.state, &owner, &job.id, job.version).unwrap();
        f.refund(&order, 1000).await;
        let restored = commerce::restore_job(&f.state, &owner, &job.id, deleted.version).unwrap();
        assert_eq!(
            (restored.status.as_str(), restored.headcount),
            ("published", count)
        );
    }
    // A publication that settled while its draft was deleted needs human review.
    let (draft, payment) = f.draft(&owner, 3, "stripe");
    let deleted = commerce::delete_job(&f.state, &owner, &draft.id, draft.version).unwrap();
    commerce::settle_payment(&f.state, receipt(&payment))
        .await
        .unwrap();
    assert_eq!(f.order(&payment.id).status, "paid_needs_review");
    let restored = commerce::restore_job(&f.state, &owner, &draft.id, deleted.version).unwrap();
    assert_eq!(
        (restored.status.as_str(), restored.headcount),
        ("closed", 0)
    );
    // Revoked activation does not charge again merely to recover one's content.
    let (job, _) = f.publish(&owner, 4, "stripe").await;
    let deleted = commerce::delete_job(&f.state, &owner, &job.id, job.version).unwrap();
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET employer_paid=0 WHERE id=?1", [&owner])
        .unwrap();
    let before = (f.count("orders"), f.count("ledger"));
    let restored = commerce::restore_job(&f.state, &owner, &job.id, deleted.version).unwrap();
    assert_eq!(
        (restored.status.as_str(), restored.headcount),
        ("closed", 0)
    );
    assert_eq!(restored.jd_md, job.jd_md);
    assert_eq!((f.count("orders"), f.count("ledger")), before);
}

#[tokio::test]
async fn lifecycle_http_requires_owner_csrf_version_and_keeps_trash_private() {
    let f = Fixture::new();
    let owner = f.employer();
    let other = f.employer();
    let a = f.browser(&owner);
    let b = f.browser(&other);
    let (job, _) = f.publish(&owner, 2, "stripe").await;
    let path = format!("/api/jobs/{}", job.id);
    let body = json!({"expected_version":job.version});
    for (browser, csrf, origin, expected) in [
        (
            None,
            false,
            "http://localhost:3000",
            StatusCode::UNAUTHORIZED,
        ),
        (
            Some(&a),
            false,
            "http://localhost:3000",
            StatusCode::FORBIDDEN,
        ),
        (
            Some(&a),
            true,
            "https://unrelated.example",
            StatusCode::FORBIDDEN,
        ),
        (
            Some(&b),
            true,
            "http://localhost:3000",
            StatusCode::FORBIDDEN,
        ),
    ] {
        assert_eq!(
            observe(
                &f,
                request("DELETE", &path, browser, csrf, origin, Some(body.clone()))
            )
            .await
            .0,
            expected
        );
    }
    for invalid in [
        json!({}),
        json!({"expected_version":job.version,"permanent":true}),
        json!({"expected_version":"1"}),
    ] {
        assert_eq!(
            observe(
                &f,
                request(
                    "DELETE",
                    &path,
                    Some(&a),
                    true,
                    "http://localhost:3000",
                    Some(invalid)
                )
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let (status, deleted) = observe(
        &f,
        request(
            "DELETE",
            &path,
            Some(&a),
            true,
            "http://localhost:3000",
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(deleted["delete_count"], 1);
    let (_, trash) = observe(
        &f,
        request(
            "GET",
            "/api/my/jobs/trash",
            Some(&a),
            false,
            "http://localhost:3000",
            None,
        ),
    )
    .await;
    assert_eq!(trash["items"][0]["title"], job.title);
    assert_eq!(trash["items"][0]["jd_md"], job.jd_md);
    assert!(!trash.to_string().contains("deleted_from_status"));
    assert!(!trash.to_string().contains("deleted_headcount"));
    let (_, other_trash) = observe(
        &f,
        request(
            "GET",
            "/api/my/jobs/trash",
            Some(&b),
            false,
            "http://localhost:3000",
            None,
        ),
    )
    .await;
    assert_eq!(other_trash["items"], json!([]));
    assert_eq!(
        observe(
            &f,
            request("GET", &path, Some(&b), false, "http://localhost:3000", None)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        observe(
            &f,
            request(
                "POST",
                &format!("{path}/restore"),
                Some(&b),
                true,
                "http://localhost:3000",
                Some(json!({"expected_version":deleted["version"]}))
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, restored) = observe(
        &f,
        request(
            "POST",
            &format!("{path}/restore"),
            Some(&a),
            true,
            "http://localhost:3000",
            Some(json!({"expected_version":deleted["version"]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = observe(
        &f,
        request(
            "DELETE",
            &format!("{path}/permanent"),
            Some(&a),
            true,
            "http://localhost:3000",
            Some(json!({"expected_version":restored["version"]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        observe(
            &f,
            request("GET", &path, Some(&a), false, "http://localhost:3000", None)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        api::deleted_jobs(&f.state, &owner).unwrap()["items"],
        json!([])
    );
}

#[tokio::test]
async fn suspended_or_changed_role_cannot_restore_content() {
    let f = Fixture::new();
    let owner = f.employer();
    let (job, _) = f.publish(&owner, 1, "stripe").await;
    let deleted = commerce::delete_job(&f.state, &owner, &job.id, job.version).unwrap();
    for (role, banned) in [("employer", true), ("seeker", false)] {
        f.state
            .conn()
            .unwrap()
            .execute(
                "UPDATE users SET role=?2,banned=?3 WHERE id=?1",
                params![owner, role, banned],
            )
            .unwrap();
        assert_eq!(
            commerce::restore_job(&f.state, &owner, &job.id, deleted.version)
                .unwrap_err()
                .status,
            StatusCode::FORBIDDEN
        );
        // Administrator reuse of the record policy has the same owner checks.
        let mut db = f.state.conn().unwrap();
        let tx = db.transaction().unwrap();
        assert_eq!(
            commerce::restore_job_record(&tx, &job.id, deleted.version, "admin")
                .unwrap_err()
                .status,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        (f.job(&job.id).status.as_str(), f.job(&job.id).delete_count),
        ("deleted", 1)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_delete_deliveries_consume_only_one_recoverable_deletion() {
    let f = Fixture::new();
    let owner = f.employer();
    let (job, _) = f.publish(&owner, 2, "stripe").await;
    let states: Vec<_> = (0..6)
        .map(|_| AppState::new((*f.state.cfg).clone()).unwrap())
        .collect();
    let barrier = Arc::new(tokio::sync::Barrier::new(states.len()));
    let mut tasks = Vec::new();
    for state in states {
        let barrier = barrier.clone();
        let owner = owner.clone();
        let id = job.id.clone();
        let version = job.version;
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            commerce::delete_job(&state, &owner, &id, version)
        }));
    }
    for task in tasks {
        let deleted = task.await.unwrap().unwrap();
        assert_eq!(
            (deleted.delete_count, deleted.version),
            (1, job.version + 1)
        );
    }
    let recorded: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM audit WHERE target=?1 AND action='job_delete'",
            [&job.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(recorded, 1);
    assert_eq!(f.job(&job.id).delete_count, 1);
    assert_eq!(f.count("orders"), 1);
}
