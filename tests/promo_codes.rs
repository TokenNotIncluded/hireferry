//! Promotion rules through HTTP and a real SQLite file. All accounts, sessions
//! and payment receipts are synthetic; no mail or payment gateway is contacted.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{AppState, auth, commerce, config::Config, models, payments::PaymentReceipt};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

struct Fixture {
    _directory: TempDir,
    state: AppState,
}

struct Client {
    app: Router,
    id: String,
    cookie: String,
    csrf: String,
}

impl Fixture {
    fn new() -> Self {
        Self::with_payments(false)
    }

    fn with_payments(enabled: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut cfg = Config {
            development: true,
            database: directory
                .path()
                .join("promos.sqlite3")
                .to_string_lossy()
                .into_owned(),
            ..Default::default()
        };
        cfg.payments.stripe.enabled = enabled;
        if enabled {
            cfg.payments.stripe.secret_key = "sk_test_synthetic_promotion_fixture".into();
            cfg.payments.stripe.webhook_secret = "whsec_synthetic_promotion_fixture".into();
        }
        Self {
            state: AppState::new(cfg).unwrap(),
            _directory: directory,
        }
    }

    fn client(&self, role: &str, activated: bool) -> Client {
        let id = uuid::Uuid::new_v4().to_string();
        self.state.conn().unwrap().execute(
            "INSERT INTO users(id,email,name,role,employer_paid,created_at,updated_at) VALUES(?1,?2,'Promotion fixture',?3,?4,?5,?5)",
            params![id, format!("{id}@example.invalid"), role, activated, hireferry::now()],
        ).unwrap();
        self.session(Some(id))
    }

    fn admin(&self) -> Client {
        self.session(None)
    }

    fn session(&self, uid: Option<String>) -> Client {
        let cookie = uuid::Uuid::new_v4().to_string();
        let csrf = uuid::Uuid::new_v4().to_string();
        self.state.conn().unwrap().execute(
            "INSERT INTO sessions(token_hash,user_id,admin,csrf,expires_at,recent_auth_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![auth::hash_token(&cookie), uid, uid.is_none(), csrf, hireferry::now()+3600, hireferry::now()],
        ).unwrap();
        Client {
            app: hireferry::app(self.state.clone()),
            id: uid.unwrap_or_default(),
            cookie,
            csrf,
        }
    }

    fn count(&self, table: &str) -> i64 {
        self.state
            .conn()
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn order(&self, id: &str) -> models::Order {
        models::load_order(&self.state.conn().unwrap(), id).unwrap()
    }
}

impl Client {
    async fn request(&self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("origin", "http://localhost:3000")
            .header("cookie", format!("hf_session={}", self.cookie))
            .header("x-csrf-token", &self.csrf)
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)}));
        (status, body)
    }

    async fn ok(&self, method: &str, path: &str, body: Value) -> Value {
        let (status, body) = self.request(method, path, body).await;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {body}");
        body
    }
}

fn receipt(order: &models::Order, event: &str) -> PaymentReceipt {
    PaymentReceipt {
        provider: order.provider.clone(),
        event_id: format!("evt_{}_{event}", order.id),
        order_id: order.id.clone(),
        session_id: None,
        payment_id: format!("pi_{}", order.id),
        amount_cents: order.amount_cents,
        currency: order.currency.clone(),
        live_mode: order.live_mode,
    }
}

fn publication(code: &str, key: &str) -> Value {
    json!({
        "title":"Rust developer", "jd_md":"Maintain a small Rust service.",
        "salary_cents":300_000, "headcount":1, "currency":"CNY",
        "promo_code":code, "request_key":key
    })
}

async fn code(admin: &Client, settings: Value) -> Value {
    admin.ok("POST", "/api/admin/promo-codes", settings).await
}

async fn code_after(admin: &Client, id: &str) -> Value {
    let result = admin.ok("GET", "/api/admin/promo-codes", json!({})).await;
    result["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap()
        .clone()
}

#[tokio::test]
async fn applications_are_private_and_repeated_approval_issues_one_code() {
    let f = Fixture::new();
    let applicant = f.client("unassigned", false);
    let other = f.client("unassigned", false);
    let admin = f.admin();
    let reason = json!({"reason":"Hiring our first engineer; requesting a trial."});
    let application = applicant
        .ok("POST", "/api/promo-applications", reason.clone())
        .await;
    assert_eq!(application["status"], "pending");
    assert!(application["promo_code"].is_null());
    let repeated = applicant
        .ok("POST", "/api/promo-applications", reason)
        .await;
    assert_eq!(repeated["id"], application["id"]);
    assert_eq!(f.count("promo_applications"), 1);
    assert_eq!(
        other.ok("GET", "/api/promo-applications", json!({})).await["total"],
        0
    );

    let path = format!(
        "/api/admin/promo-applications/{}/review",
        application["id"].as_str().unwrap()
    );
    let review = json!({"decision":"approved","note":"Trial approved","settings":{}});
    assert_eq!(
        applicant.request("POST", &path, review.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        other
            .request("GET", "/api/admin/promo-applications", json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let approved = admin.ok("POST", &path, review.clone()).await;
    assert_eq!(approved["status"], "approved");
    assert_eq!(approved["promo_code"]["free_activation_limit"], 1);
    assert_eq!(approved["promo_code"]["publish_limit"], 1);
    assert_eq!(approved["promo_code"]["discount_percent"], 100);
    let retried = admin.ok("POST", &path, review).await;
    assert_eq!(retried["promo_code"]["id"], approved["promo_code"]["id"]);
    assert_eq!(f.count("promo_codes"), 1);
    let own = applicant
        .ok("GET", "/api/promo-applications", json!({}))
        .await;
    assert_eq!(
        own["items"][0]["promo_code"]["code"],
        approved["promo_code"]["code"]
    );
    assert_eq!(
        other.ok("GET", "/api/promo-applications", json!({})).await["total"],
        0
    );
}

#[tokio::test]
async fn rejection_is_visible_and_cannot_be_reapproved_silently() {
    let f = Fixture::new();
    let applicant = f.client("unassigned", false);
    let admin = f.admin();
    let application = applicant
        .ok(
            "POST",
            "/api/promo-applications",
            json!({"reason":"New hiring request"}),
        )
        .await;
    let path = format!(
        "/api/admin/promo-applications/{}/review",
        application["id"].as_str().unwrap()
    );
    let rejected = admin
        .ok(
            "POST",
            &path,
            json!({"decision":"rejected","note":"Please describe the hiring plan."}),
        )
        .await;
    assert_eq!(rejected["status"], "rejected");
    assert_eq!(rejected["review_note"], "Please describe the hiring plan.");
    assert!(rejected["promo_code"].is_null());
    assert_eq!(f.count("promo_codes"), 0);
    assert_eq!(
        admin
            .request("POST", &path, json!({"decision":"approved","settings":{}}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let own = applicant
        .ok("GET", "/api/promo-applications", json!({}))
        .await;
    assert_eq!(own["items"][0]["status"], "rejected");
}

#[tokio::test]
async fn default_activation_is_free_atomic_and_idempotent() {
    let f = Fixture::new();
    let admin = f.admin();
    let promo = code(&admin, json!({})).await;
    let member = f.client("unassigned", false);
    let redeem = json!({"code":promo["code"]});
    let result = member
        .ok("POST", "/api/promo-codes/redeem-activation", redeem.clone())
        .await;
    assert_eq!(result["user"]["role"], "employer");
    assert_eq!(result["user"]["employer_paid"], true);
    assert_eq!(result["order"]["amount_cents"], 0);
    assert_eq!(result["order"]["provider"], "free");
    assert_eq!(result["order"]["status"], "paid");
    member
        .ok("POST", "/api/promo-codes/redeem-activation", redeem.clone())
        .await;
    let current = code_after(&admin, promo["id"].as_str().unwrap()).await;
    assert_eq!(current["activation_used"], 1);
    assert_eq!(current["activation_remaining"], 0);
    assert_eq!(current["publish_remaining"], 1);
    assert_eq!(f.count("orders"), 1);
    assert_eq!(f.count("ledger"), 0);
    let other = f.client("unassigned", false);
    assert!(
        !other
            .request("POST", "/api/promo-codes/redeem-activation", redeem)
            .await
            .0
            .is_success()
    );
    let unchanged = models::load_user(&f.state.conn().unwrap(), &other.id).unwrap();
    assert_eq!(unchanged.role, "unassigned");
    assert!(!unchanged.employer_paid);
}

#[tokio::test]
async fn activation_can_be_disabled_without_disabling_free_publication() {
    let f = Fixture::new();
    let admin = f.admin();
    let promo = code(
        &admin,
        json!({"free_activation_limit":0,"publish_limit":2,"discount_percent":100}),
    )
    .await;
    let unassigned = f.client("unassigned", false);
    assert!(
        !unassigned
            .request(
                "POST",
                "/api/promo-codes/redeem-activation",
                json!({"code":promo["code"]})
            )
            .await
            .0
            .is_success()
    );
    let employer = f.client("employer", true);
    let preview = employer.ok("POST", "/api/promo-codes/preview", json!({"code":promo["code"],"purpose":"publish","salary_cents":300_000,"currency":"CNY"})).await;
    assert_eq!(preview["original_amount_cents"], 30_000);
    assert_eq!(preview["amount_cents"], 0);
    assert_eq!(preview["discount_percent"], 100);
    let published = employer
        .ok(
            "POST",
            "/api/jobs",
            publication(promo["code"].as_str().unwrap(), "free-publish-01"),
        )
        .await;
    assert_eq!(published["job"]["status"], "published");
    assert_eq!(published["order"]["status"], "paid");
    assert_eq!(published["order"]["provider"], "free");
    assert_eq!(published["order"]["amount_cents"], 0);
    assert!(published["order"]["checkout_url"].is_null());
    let free_snapshot: Value =
        serde_json::from_str(published["order"]["snapshot_json"].as_str().unwrap()).unwrap();
    assert!(free_snapshot["promo"].get("reserved_until").is_none());
    assert_eq!(f.count("ledger"), 0);
    let current = code_after(&admin, promo["id"].as_str().unwrap()).await;
    assert_eq!(current["activation_used"], 0);
    assert_eq!(current["publish_used"], 1);
    assert_eq!(current["publish_reserved"], 0);
    assert_eq!(current["publish_remaining"], 1);
}

#[tokio::test]
async fn activation_and_publication_have_independent_limits() {
    let f = Fixture::new();
    let admin = f.admin();
    let promo = code(&admin, json!({"free_activation_limit":2,"publish_limit":1})).await;
    let first = f.client("unassigned", false);
    let second = f.client("unassigned", false);
    first
        .ok(
            "POST",
            "/api/promo-codes/redeem-activation",
            json!({"code":promo["code"]}),
        )
        .await;
    first
        .ok(
            "POST",
            "/api/jobs",
            publication(promo["code"].as_str().unwrap(), "independent-first"),
        )
        .await;
    second
        .ok(
            "POST",
            "/api/promo-codes/redeem-activation",
            json!({"code":promo["code"]}),
        )
        .await;
    assert!(
        !second
            .request(
                "POST",
                "/api/jobs",
                publication(promo["code"].as_str().unwrap(), "independent-second")
            )
            .await
            .0
            .is_success()
    );
    let current = code_after(&admin, promo["id"].as_str().unwrap()).await;
    assert_eq!(current["activation_used"], 2);
    assert_eq!(current["publish_used"], 1);
    assert_eq!(f.count("jobs"), 1);
}

#[tokio::test]
async fn failed_publication_keeps_quota_and_request_key_retryable() {
    let f = Fixture::new();
    let admin = f.admin();
    let promo = code(&admin, json!({"publish_limit":1})).await;
    let employer = f.client("employer", true);
    let mut body = publication(promo["code"].as_str().unwrap(), "retry-after-failure");
    body["jd_md"] = json!("");
    assert_eq!(
        employer.request("POST", "/api/jobs", body.clone()).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(f.count("jobs"), 0);
    assert_eq!(f.count("orders"), 0);
    assert_eq!(f.count("promo_redemptions"), 0);
    body["jd_md"] = json!("A complete role description.");
    let successful = employer.ok("POST", "/api/jobs", body).await;
    assert_eq!(successful["job"]["status"], "published");
    assert_eq!(
        code_after(&admin, promo["id"].as_str().unwrap()).await["publish_used"],
        1
    );
}

#[tokio::test]
async fn repeated_publication_reuses_result_and_rejects_changed_payload() {
    let f = Fixture::new();
    let admin = f.admin();
    let promo = code(&admin, json!({"publish_limit":2})).await;
    let employer = f.client("employer", true);
    let mut body = publication(promo["code"].as_str().unwrap(), "same-business-request");
    let first = employer.ok("POST", "/api/jobs", body.clone()).await;
    let second = employer.ok("POST", "/api/jobs", body.clone()).await;
    assert_eq!(first["job"]["id"], second["job"]["id"]);
    assert_eq!(first["order"]["id"], second["order"]["id"]);
    body["jd_md"] = json!("Different role content.");
    assert_eq!(
        employer.request("POST", "/api/jobs", body).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(f.count("jobs"), 1);
    assert_eq!(f.count("orders"), 1);
    assert_eq!(
        code_after(&admin, promo["id"].as_str().unwrap()).await["publish_used"],
        1
    );
}

#[tokio::test]
async fn admin_validation_normalization_and_csrf_are_enforced() {
    let f = Fixture::new();
    let mut admin = f.admin();
    let member = f.client("employer", true);
    assert_eq!(
        member
            .request("POST", "/api/admin/promo-codes", json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let csrf = std::mem::take(&mut admin.csrf);
    assert_eq!(
        admin
            .request("POST", "/api/admin/promo-codes", json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    admin.csrf = csrf;
    for settings in [
        json!({"discount_percent":101}),
        json!({"discount_percent":-1}),
        json!({"free_activation_limit":-1}),
        json!({"publish_limit":-1}),
    ] {
        assert_eq!(
            admin
                .request("POST", "/api/admin/promo-codes", settings)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(f.count("promo_codes"), 0);
    let promo = code(&admin, json!({"code":" Trial-code-01 "})).await;
    assert_eq!(promo["code"], "TRIAL-CODE-01");
    assert_eq!(
        admin
            .request(
                "POST",
                "/api/admin/promo-codes",
                json!({"code":"trial-code-01"})
            )
            .await
            .0,
        StatusCode::CONFLICT
    );
    let preview = member
        .ok(
            "POST",
            "/api/promo-codes/preview",
            json!({"code":" trial-code-01 ","purpose":"publish","salary_cents":300_000}),
        )
        .await;
    assert_eq!(preview["promo_code"]["id"], promo["id"]);
}

#[tokio::test]
async fn disabled_expired_or_unknown_codes_never_fall_back_to_full_price() {
    let f = Fixture::with_payments(true);
    let admin = f.admin();
    let employer = f.client("employer", true);
    let disabled = code(&admin, json!({"enabled":false})).await;
    let expired = code(&admin, json!({"expires_at":hireferry::now()+3600})).await;
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE promo_codes SET expires_at=?2 WHERE id=?1",
            params![expired["id"].as_str().unwrap(), hireferry::now() - 1],
        )
        .unwrap();
    for text in [
        disabled["code"].as_str().unwrap(),
        expired["code"].as_str().unwrap(),
        "DOES-NOT-EXIST",
    ] {
        let mut body = publication(text, &uuid::Uuid::new_v4().to_string());
        body["provider"] = json!("stripe");
        assert!(
            !employer
                .request("POST", "/api/jobs", body)
                .await
                .0
                .is_success()
        );
    }
    assert_eq!(f.count("jobs"), 0);
    assert_eq!(f.count("orders"), 0);
    assert_eq!(f.count("promo_redemptions"), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn last_publication_use_is_atomic_across_database_connections() {
    let f = Fixture::new();
    let admin = f.admin();
    let promo = code(&admin, json!({"publish_limit":1})).await;
    let first = f.client("employer", true);
    let mut second = f.client("employer", true);
    // A second connection ensures this is testing SQLite transaction isolation,
    // rather than only the shared AppState mutex.
    second.app = hireferry::app(AppState::new((*f.state.cfg).clone()).unwrap());
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let left_barrier = barrier.clone();
    let first_body = publication(promo["code"].as_str().unwrap(), "concurrent-first");
    let second_body = publication(promo["code"].as_str().unwrap(), "concurrent-second");
    let left = tokio::spawn(async move {
        left_barrier.wait().await;
        first.request("POST", "/api/jobs", first_body).await
    });
    let right = tokio::spawn(async move {
        barrier.wait().await;
        second.request("POST", "/api/jobs", second_body).await
    });
    let results = [left.await.unwrap(), right.await.unwrap()];
    assert_eq!(
        results
            .iter()
            .filter(|result| result.0.is_success())
            .count(),
        1,
        "{results:?}"
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| result.0 == StatusCode::CONFLICT)
            .count(),
        1,
        "{results:?}"
    );
    assert_eq!(f.count("jobs"), 1);
    assert_eq!(f.count("orders"), 1);
    assert_eq!(f.count("promo_redemptions"), 1);
    assert_eq!(
        code_after(&admin, promo["id"].as_str().unwrap()).await["publish_used"],
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn last_activation_use_is_atomic_across_database_connections() {
    let f = Fixture::new();
    let admin = f.admin();
    let promo = code(&admin, json!({"free_activation_limit":1})).await;
    let first = f.client("unassigned", false);
    let mut second = f.client("unassigned", false);
    second.app = hireferry::app(AppState::new((*f.state.cfg).clone()).unwrap());
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let left_barrier = barrier.clone();
    let left_body = json!({"code":promo["code"]});
    let right_body = left_body.clone();
    let left = tokio::spawn(async move {
        left_barrier.wait().await;
        first
            .request("POST", "/api/promo-codes/redeem-activation", left_body)
            .await
    });
    let right = tokio::spawn(async move {
        barrier.wait().await;
        second
            .request("POST", "/api/promo-codes/redeem-activation", right_body)
            .await
    });
    let results = [left.await.unwrap(), right.await.unwrap()];
    assert_eq!(
        results
            .iter()
            .filter(|result| result.0.is_success())
            .count(),
        1,
        "{results:?}"
    );
    let activated: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM users WHERE employer_paid=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(activated, 1);
    assert_eq!(f.count("orders"), 1);
    assert_eq!(
        code_after(&admin, promo["id"].as_str().unwrap()).await["activation_used"],
        1
    );
}

#[tokio::test]
async fn partial_discount_reserves_then_consumes_once_and_locks_order_amount() {
    let f = Fixture::with_payments(true);
    let admin = f.admin();
    let promo_deadline = hireferry::now() + 3600;
    let promo = code(
        &admin,
        json!({"free_activation_limit":0,"publish_limit":1,"discount_percent":50,"expires_at":promo_deadline}),
    )
    .await;
    let employer = f.client("employer", true);
    let mut body = publication(promo["code"].as_str().unwrap(), "paid-reservation");
    body["provider"] = json!("stripe");
    let result = employer.ok("POST", "/api/jobs", body).await;
    assert_eq!(result["job"]["status"], "draft");
    assert_eq!(result["order"]["status"], "pending");
    assert_eq!(result["order"]["amount_cents"], 15_000);
    let order_id = result["order"]["id"].as_str().unwrap();
    let reservation_deadline: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT reserved_until FROM promo_redemptions WHERE order_id=?1",
            [order_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reservation_deadline, promo_deadline);
    let created_snapshot: Value =
        serde_json::from_str(result["order"]["snapshot_json"].as_str().unwrap()).unwrap();
    assert_eq!(
        created_snapshot["promo"]["reserved_until"],
        reservation_deadline
    );
    // A cached gateway session may outlive the promotion. Both order reads and
    // checkout retries must still expose the original, earlier promo deadline.
    f.state.conn().unwrap().execute(
        "UPDATE orders SET checkout_url='https://checkout.stripe.com/promo-fixture',checkout_expires_at=?2 WHERE id=?1",
        params![order_id,promo_deadline+3600],
    ).unwrap();
    let fetched = employer
        .ok("GET", &format!("/api/orders/{order_id}"), json!({}))
        .await;
    let fetched_snapshot: Value =
        serde_json::from_str(fetched["snapshot_json"].as_str().unwrap()).unwrap();
    assert_eq!(
        fetched_snapshot["promo"]["reserved_until"],
        reservation_deadline
    );
    let checkout = commerce::checkout(&f.state, &employer.id, order_id)
        .await
        .unwrap();
    assert!(checkout.checkout_expires_at.unwrap() > reservation_deadline);
    let checkout_snapshot: Value = serde_json::from_str(&checkout.snapshot_json).unwrap();
    assert_eq!(
        checkout_snapshot["promo"]["reserved_until"],
        reservation_deadline
    );
    let before = code_after(&admin, promo["id"].as_str().unwrap()).await;
    assert_eq!(before["publish_used"], 0);
    assert_eq!(before["publish_reserved"], 1);
    assert_eq!(before["publish_remaining"], 0);
    let patch = format!("/api/admin/promo-codes/{}", promo["id"].as_str().unwrap());
    assert_eq!(
        admin
            .request("PATCH", &patch, json!({"publish_limit":0}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    admin
        .ok(
            "PATCH",
            &patch,
            json!({"discount_percent":100,"expires_at":promo_deadline+7200}),
        )
        .await;
    let order = f.order(result["order"]["id"].as_str().unwrap());
    assert_eq!(order.amount_cents, 15_000);
    let unchanged_snapshot: Value = serde_json::from_str(&order.snapshot_json).unwrap();
    assert_eq!(
        unchanged_snapshot["promo"]["reserved_until"],
        reservation_deadline
    );
    commerce::settle_payment(&f.state, receipt(&order, "first"))
        .await
        .unwrap();
    commerce::settle_payment(&f.state, receipt(&order, "first"))
        .await
        .unwrap();
    commerce::settle_payment(&f.state, receipt(&order, "redelivery"))
        .await
        .unwrap();
    assert_eq!(f.order(&order.id).status, "paid");
    assert_eq!(
        models::load_job(&f.state.conn().unwrap(), order.job_id.as_deref().unwrap())
            .unwrap()
            .status,
        "published"
    );
    let after = code_after(&admin, promo["id"].as_str().unwrap()).await;
    assert_eq!(after["publish_used"], 1);
    assert_eq!(after["publish_reserved"], 0);
    assert_eq!(after["publish_remaining"], 0);
    assert_eq!(f.count("ledger"), 1);
}

#[tokio::test]
async fn only_owner_can_cancel_reservation_and_late_payment_needs_review() {
    let f = Fixture::with_payments(true);
    let admin = f.admin();
    let promo = code(&admin, json!({"publish_limit":1,"discount_percent":50})).await;
    let employer = f.client("employer", true);
    let other = f.client("employer", true);
    let mut body = publication(promo["code"].as_str().unwrap(), "cancel-reservation");
    body["provider"] = json!("stripe");
    let created = employer.ok("POST", "/api/jobs", body).await;
    let order = f.order(created["order"]["id"].as_str().unwrap());
    let path = format!("/api/orders/{}/cancel", order.id);
    assert!(!other.request("POST", &path, json!({})).await.0.is_success());
    assert_eq!(f.order(&order.id).status, "pending");
    let cancelled = employer.ok("POST", &path, json!({})).await;
    assert_eq!(cancelled["status"], "cancelled");
    let current = code_after(&admin, promo["id"].as_str().unwrap()).await;
    assert_eq!(current["publish_remaining"], 1);
    assert_eq!(current["publish_reserved"], 0);
    commerce::settle_payment(&f.state, receipt(&order, "late"))
        .await
        .unwrap();
    assert_eq!(f.order(&order.id).status, "paid_needs_review");
    assert_eq!(
        models::load_job(&f.state.conn().unwrap(), order.job_id.as_deref().unwrap())
            .unwrap()
            .status,
        "draft"
    );
    assert_eq!(
        code_after(&admin, promo["id"].as_str().unwrap()).await["publish_used"],
        0
    );
    assert_eq!(f.count("ledger"), 1);
}

#[tokio::test]
async fn expiry_releases_quota_and_late_receipt_cannot_publish_after_reuse() {
    let f = Fixture::with_payments(true);
    let admin = f.admin();
    let promo = code(&admin, json!({"publish_limit":1,"discount_percent":50})).await;
    let employer = f.client("employer", true);
    let mut body = publication(promo["code"].as_str().unwrap(), "expiring-reservation");
    body["provider"] = json!("stripe");
    let old = employer.ok("POST", "/api/jobs", body).await;
    let old_order = f.order(old["order"]["id"].as_str().unwrap());
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE promo_redemptions SET reserved_until=?2 WHERE order_id=?1",
            params![old_order.id, hireferry::now() - 1],
        )
        .unwrap();
    let expired = employer
        .ok("GET", &format!("/api/orders/{}", old_order.id), json!({}))
        .await;
    assert_eq!(expired["status"], "expired");
    assert!(expired["checkout_url"].is_null());
    assert_eq!(
        code_after(&admin, promo["id"].as_str().unwrap()).await["publish_remaining"],
        1
    );
    admin
        .ok(
            "PATCH",
            &format!("/api/admin/promo-codes/{}", promo["id"].as_str().unwrap()),
            json!({"discount_percent":100}),
        )
        .await;
    let replacement = employer
        .ok(
            "POST",
            "/api/jobs",
            publication(promo["code"].as_str().unwrap(), "replacement-free-job"),
        )
        .await;
    assert_eq!(replacement["job"]["status"], "published");
    commerce::settle_payment(&f.state, receipt(&old_order, "late"))
        .await
        .unwrap();
    assert_eq!(f.order(&old_order.id).status, "paid_needs_review");
    assert_eq!(
        models::load_job(
            &f.state.conn().unwrap(),
            old_order.job_id.as_deref().unwrap()
        )
        .unwrap()
        .status,
        "draft"
    );
    let current = code_after(&admin, promo["id"].as_str().unwrap()).await;
    assert_eq!(current["publish_used"], 1);
    assert_eq!(current["publish_reserved"], 0);
    assert_eq!(current["publish_remaining"], 0);
}

#[tokio::test]
async fn free_activation_cancels_old_payment_and_records_late_receipt_without_granting_again() {
    let f = Fixture::with_payments(true);
    let admin = f.admin();
    let promo = code(&admin, json!({})).await;
    let employer = f.client("unassigned", false);
    let pending = employer
        .ok(
            "POST",
            "/api/role",
            json!({"role":"employer","provider":"stripe"}),
        )
        .await;
    let old_order = f.order(pending["order"]["id"].as_str().unwrap());
    assert_eq!(old_order.status, "pending");
    employer
        .ok(
            "POST",
            "/api/promo-codes/redeem-activation",
            json!({"code":promo["code"]}),
        )
        .await;
    assert_eq!(f.order(&old_order.id).status, "cancelled");
    commerce::settle_payment(&f.state, receipt(&old_order, "late"))
        .await
        .unwrap();
    assert_eq!(f.order(&old_order.id).status, "paid_needs_review");
    assert!(
        models::load_user(&f.state.conn().unwrap(), &employer.id)
            .unwrap()
            .employer_paid
    );
    assert_eq!(
        code_after(&admin, promo["id"].as_str().unwrap()).await["activation_used"],
        1
    );
    assert_eq!(f.count("ledger"), 1);
}

#[tokio::test]
async fn promotion_after_expired_activation_recovery_cancels_replacement_and_reviews_both_late_payments()
 {
    let f = Fixture::with_payments(true);
    let admin = f.admin();
    let promo = code(&admin, json!({})).await;
    let employer = f.client("unassigned", false);
    let role = json!({"role":"employer","provider":"stripe"});
    let initial = employer.ok("POST", "/api/role", role.clone()).await;
    let expired_order = f.order(initial["order"]["id"].as_str().unwrap());
    f.state.conn().unwrap().execute(
        "UPDATE orders SET checkout_expires_at=?2,checkout_url='https://checkout.stripe.com/synthetic-expired' WHERE id=?1",
        params![expired_order.id,hireferry::now()-1],
    ).unwrap();
    let expired = employer
        .ok(
            "GET",
            &format!("/api/orders/{}", expired_order.id),
            json!({}),
        )
        .await;
    assert_eq!(expired["status"], "expired");
    assert!(expired["checkout_url"].is_null());

    let replacement = employer.ok("POST", "/api/role", role).await;
    let replacement_order = f.order(replacement["order"]["id"].as_str().unwrap());
    assert_ne!(replacement_order.id, expired_order.id);
    assert_eq!(replacement_order.status, "pending");
    let redemption = json!({"code":promo["code"]});
    let activated = employer
        .ok(
            "POST",
            "/api/promo-codes/redeem-activation",
            redemption.clone(),
        )
        .await;
    let repeated = employer
        .ok("POST", "/api/promo-codes/redeem-activation", redemption)
        .await;
    assert_eq!(activated["user"]["employer_paid"], true);
    assert_eq!(activated["order"]["id"], repeated["order"]["id"]);
    assert_eq!(activated["order"]["amount_cents"], 0);
    assert_eq!(f.order(&replacement_order.id).status, "cancelled");
    assert_eq!(f.order(&expired_order.id).status, "expired");

    for order in [&expired_order, &replacement_order] {
        commerce::settle_payment(&f.state, receipt(order, "late-after-free-activation"))
            .await
            .unwrap();
        assert_eq!(f.order(&order.id).status, "paid_needs_review");
    }
    let free_order = f.order(activated["order"]["id"].as_str().unwrap());
    assert_eq!(free_order.status, "paid");
    assert_eq!(free_order.provider, "free");
    assert!(
        models::load_user(&f.state.conn().unwrap(), &employer.id)
            .unwrap()
            .employer_paid
    );
    let current = code_after(&admin, promo["id"].as_str().unwrap()).await;
    assert_eq!(current["activation_used"], 1);
    assert_eq!(current["publish_remaining"], 1);
    assert_eq!(f.count("orders"), 3);
    assert_eq!(f.count("ledger"), 2);
    assert_eq!(f.count("promo_redemptions"), 1);
    let fulfilled: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM orders WHERE user_id=?1 AND kind='onboarding' AND status='paid'",
            [&employer.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(fulfilled, 1);
}

#[tokio::test]
async fn mcp_publication_forwards_promotion_and_retry_fields_without_expanding_access() {
    let f = Fixture::new();
    let admin = f.admin();
    let promo = code(&admin, json!({"publish_limit":2})).await;
    let member = f.client("unassigned", false);
    let principal = hireferry::access::Principal {
        user_id: Some(member.id.clone()),
        admin: false,
        via_ai: true,
        actor: member.id.clone(),
    };
    let schemas = hireferry::mcp::tools();
    let activation = schemas
        .iter()
        .find(|tool| tool["name"] == "choose_role")
        .unwrap();
    assert_eq!(
        activation["inputSchema"]["properties"]["promo_code"]["type"],
        "string"
    );
    let publish = schemas
        .iter()
        .find(|tool| tool["name"] == "create_job")
        .unwrap();
    assert_eq!(
        publish["inputSchema"]["dependentRequired"]["promo_code"],
        json!(["request_key"])
    );
    let denied = hireferry::mcp::call(
        &f.state,
        &principal,
        "choose_role",
        json!({"role":"employer","promo_code":promo["code"]}),
    )
    .await
    .unwrap_err();
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    let activated = member
        .ok(
            "POST",
            "/api/role",
            json!({"role":"employer","promo_code":promo["code"],"currency":"CNY"}),
        )
        .await;
    assert_eq!(activated["user"]["employer_paid"], true);
    member
        .ok(
            "POST",
            "/api/jobs",
            publication(promo["code"].as_str().unwrap(), "initial-qualifying-job"),
        )
        .await;
    let body = publication(promo["code"].as_str().unwrap(), "mcp-promotion-key");
    let first = hireferry::mcp::call(&f.state, &principal, "create_job", body.clone())
        .await
        .unwrap();
    let repeated = hireferry::mcp::call(&f.state, &principal, "create_job", body)
        .await
        .unwrap();
    assert_eq!(first["job"]["status"], "published");
    assert_eq!(first["order"]["amount_cents"], 0);
    assert_eq!(first["job"]["id"], repeated["job"]["id"]);
    assert_eq!(f.count("jobs"), 2);
}

#[test]
fn discount_uses_integer_minor_units_and_checks_bounds() {
    use hireferry::promos::discounted_amount;
    assert_eq!(discounted_amount(30_000, 100).unwrap(), 0);
    assert_eq!(discounted_amount(30_000, 50).unwrap(), 15_000);
    assert_eq!(discounted_amount(30_000, 0).unwrap(), 30_000);
    assert_eq!(discounted_amount(101, 50).unwrap(), 51);
    assert_eq!(discounted_amount(1, 100).unwrap(), 0);
    assert_eq!(discounted_amount(0, 50).unwrap(), 0);
    for (amount, percent) in [(-1, 50), (1, -1), (1, 101), (i64::MAX, 0)] {
        assert!(discounted_amount(amount, percent).is_err());
    }
}

#[tokio::test]
async fn suspended_and_seeker_accounts_cannot_claim_employer_promotions() {
    let f = Fixture::new();
    let admin = f.admin();
    let promo = code(&admin, json!({})).await;
    let seeker = f.client("seeker", false);
    let suspended = f.client("unassigned", false);
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id=?1", [&suspended.id])
        .unwrap();
    for client in [&seeker, &suspended] {
        assert!(
            !client
                .request(
                    "POST",
                    "/api/promo-applications",
                    json!({"reason":"Request a trial"})
                )
                .await
                .0
                .is_success()
        );
        assert!(
            !client
                .request(
                    "POST",
                    "/api/promo-codes/redeem-activation",
                    json!({"code":promo["code"]})
                )
                .await
                .0
                .is_success()
        );
    }
    assert_eq!(f.count("promo_applications"), 0);
    assert_eq!(f.count("promo_redemptions"), 0);
}

fn downgrade_to_version_eight(db: &rusqlite::Connection) {
    db.execute_batch(
        "DROP TRIGGER promo_order_terminal;
         DROP VIEW promo_code_balances;
         DROP TABLE promo_redemptions;
         DROP TABLE promo_codes;
         DROP TABLE promo_applications;
         PRAGMA user_version=8;",
    )
    .unwrap();
}

#[tokio::test]
async fn version_nine_upgrade_preserves_orders_and_is_idempotent() {
    let f = Fixture::with_payments(true);
    let member = f.client("unassigned", false);
    let (_, old_order) =
        commerce::choose_role(&f.state, &member.id, "employer", Some("stripe")).unwrap();
    let old_order = old_order.unwrap();
    {
        let db = f.state.conn().unwrap();
        downgrade_to_version_eight(&db);
        hireferry::db::migrate(&db).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            12
        );
        assert_eq!(
            models::load_order(&db, &old_order.id).unwrap().amount_cents,
            old_order.amount_cents
        );
        assert_eq!(
            models::load_order(&db, &old_order.id).unwrap().status,
            "pending"
        );
        assert!(!models::load_user(&db, &member.id).unwrap().employer_paid);
    }
    let admin = f.admin();
    let promo = code(&admin, json!({"publish_limit":3})).await;
    hireferry::db::migrate(&f.state.conn().unwrap()).unwrap();
    let preserved = code_after(&admin, promo["id"].as_str().unwrap()).await;
    assert_eq!(preserved["code"], promo["code"]);
    assert_eq!(preserved["publish_remaining"], 3);
    assert_eq!(f.count("promo_codes"), 1);
    let schema_objects: i64 = f.state.conn().unwrap().query_row(
        "SELECT count(*) FROM sqlite_master WHERE name IN ('promo_applications','promo_codes','promo_redemptions','promo_code_balances','promo_order_terminal')", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(schema_objects, 5);
}

#[test]
fn failed_version_nine_upgrade_rolls_back_schema_and_keeps_version_eight() {
    let f = Fixture::new();
    let member = f.client("unassigned", false);
    let db = f.state.conn().unwrap();
    downgrade_to_version_eight(&db);
    // A pre-existing incompatible object makes creation of the redemption index
    // fail after the migration has attempted to create its earlier tables.
    db.execute_batch("CREATE TABLE promo_redemptions(sentinel TEXT); INSERT INTO promo_redemptions VALUES('preserve-existing-object');").unwrap();
    assert!(hireferry::db::migrate(&db).is_err());
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        8
    );
    let partial: i64 = db.query_row(
        "SELECT count(*) FROM sqlite_master WHERE name IN ('promo_applications','promo_codes','promo_code_balances','promo_order_terminal')", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(partial, 0);
    assert_eq!(
        db.query_row("SELECT sentinel FROM promo_redemptions", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "preserve-existing-object"
    );
    assert_eq!(
        models::load_user(&db, &member.id).unwrap().role,
        "unassigned"
    );
    // Repairing the incompatible fixture permits a clean retry of the upgrade.
    db.execute_batch("DROP TABLE promo_redemptions;").unwrap();
    hireferry::db::migrate(&db).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        12
    );
}

#[tokio::test]
async fn publication_retry_preserves_result_after_currency_fee_and_limit_configuration_changes() {
    let f = Fixture::new();
    let admin = f.admin();
    let promo = code(&admin, json!({"publish_limit":3})).await;
    let mut employer = f.client("employer", true);
    let mut explicit = publication(promo["code"].as_str().unwrap(), "replay-explicit-currency");
    explicit["headcount"] = json!(3);
    let mut implicit = publication(promo["code"].as_str().unwrap(), "replay-default-currency");
    implicit["headcount"] = json!(3);
    implicit.as_object_mut().unwrap().remove("currency");
    let first_explicit = employer.ok("POST", "/api/jobs", explicit.clone()).await;
    let first_implicit = employer.ok("POST", "/api/jobs", implicit.clone()).await;

    let mut cfg = (*f.state.cfg).clone();
    cfg.pricing.currency = "JPY".into();
    cfg.pricing.allowed_currencies = vec!["JPY".into()];
    cfg.pricing.publish_mode = "fixed".into();
    cfg.pricing.publish_fixed_minor = 12345;
    cfg.limits.max_job_headcount = 1;
    employer.app = hireferry::app(AppState::new(cfg).unwrap());
    for (body, original) in [
        (explicit.clone(), first_explicit),
        (implicit, first_implicit),
    ] {
        let repeated = employer.ok("POST", "/api/jobs", body).await;
        assert_eq!(repeated["job"]["id"], original["job"]["id"]);
        assert_eq!(repeated["order"]["id"], original["order"]["id"]);
        assert_eq!(repeated["job"]["currency"], "CNY");
        assert_eq!(repeated["job"]["headcount"], 3);
        assert_eq!(repeated["order"]["amount_cents"], 0);
    }
    // The same old inputs remain invalid for a new operation under current
    // policy; only recovery of the already-committed result bypasses repricing.
    explicit["request_key"] = json!("new-operation-after-config-change");
    assert_eq!(
        employer.request("POST", "/api/jobs", explicit).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(f.count("jobs"), 2);
    assert_eq!(
        code_after(&admin, promo["id"].as_str().unwrap()).await["publish_used"],
        2
    );
}

#[tokio::test]
async fn changed_quote_is_rejected_before_spending_quota_or_creating_a_paid_order() {
    let f = Fixture::with_payments(true);
    let admin = f.admin();
    let promo = code(&admin, json!({"publish_limit":1,"discount_percent":100})).await;
    let employer = f.client("employer", true);
    let quote = employer.ok("POST", "/api/promo-codes/preview", json!({"code":promo["code"],"purpose":"publish","salary_cents":300_000,"currency":"CNY"})).await;
    assert_eq!(quote["amount_cents"], 0);
    admin
        .ok(
            "PATCH",
            &format!("/api/admin/promo-codes/{}", promo["id"].as_str().unwrap()),
            json!({"discount_percent":50}),
        )
        .await;
    let mut body = publication(promo["code"].as_str().unwrap(), "quote-consent-check");
    body["expected_amount_cents"] = quote["amount_cents"].clone();
    let rejected = employer.request("POST", "/api/jobs", body.clone()).await;
    assert_eq!(rejected.0, StatusCode::CONFLICT, "{}", rejected.1);
    assert_eq!(f.count("jobs"), 0);
    assert_eq!(f.count("orders"), 0);
    assert_eq!(f.count("promo_redemptions"), 0);
    assert_eq!(
        code_after(&admin, promo["id"].as_str().unwrap()).await["publish_remaining"],
        1
    );

    let accepted = employer.ok("POST", "/api/promo-codes/preview", json!({"code":promo["code"],"purpose":"publish","salary_cents":300_000,"currency":"CNY"})).await;
    assert_eq!(accepted["amount_cents"], 15_000);
    body["expected_amount_cents"] = accepted["amount_cents"].clone();
    body["provider"] = json!("stripe");
    let result = employer.ok("POST", "/api/jobs", body).await;
    assert_eq!(result["order"]["amount_cents"], 15_000);
    assert_eq!(result["order"]["status"], "pending");
    assert_eq!(
        code_after(&admin, promo["id"].as_str().unwrap()).await["publish_reserved"],
        1
    );
}

#[tokio::test]
async fn permanent_deletion_keeps_original_promotion_receipt_after_code_terms_change() {
    let f = Fixture::new();
    let admin = f.admin();
    let promo = code(&admin, json!({"discount_percent":100})).await;
    let employer = f.client("employer", true);
    let published = employer
        .ok(
            "POST",
            "/api/jobs",
            publication(promo["code"].as_str().unwrap(), "deleted-promo-audit"),
        )
        .await;
    let order_id = published["order"]["id"].as_str().unwrap();
    let original: Value = serde_json::from_str(&f.order(order_id).snapshot_json).unwrap();
    assert_eq!(original["promo"]["discount_percent"], 100);
    assert_eq!(original["promo"]["original_amount_cents"], 30_000);
    admin
        .ok(
            "PATCH",
            &format!("/api/admin/promo-codes/{}", promo["id"].as_str().unwrap()),
            json!({"discount_percent":30}),
        )
        .await;

    let job_path = format!("/api/jobs/{}", published["job"]["id"].as_str().unwrap());
    let trashed = employer
        .ok(
            "DELETE",
            &job_path,
            json!({"expected_version":published["job"]["version"]}),
        )
        .await;
    let removed = employer
        .ok(
            "DELETE",
            &format!("{job_path}/permanent"),
            json!({"expected_version":trashed["version"]}),
        )
        .await;
    assert_eq!(removed["delete_count"], 2);
    assert_eq!(removed["title"], "");
    assert_eq!(removed["jd_md"], "");
    let preserved_order = f.order(order_id);
    let preserved: Value = serde_json::from_str(&preserved_order.snapshot_json).unwrap();
    assert_eq!(preserved["promo"], original["promo"]);
    assert_eq!(preserved_order.amount_cents, 0);
    assert_eq!(preserved_order.currency, "CNY");
    assert_eq!(preserved_order.status, "paid");
    assert!(preserved.get("jd_md").is_none());
    assert_eq!(
        code_after(&admin, promo["id"].as_str().unwrap()).await["publish_used"],
        1
    );
}
