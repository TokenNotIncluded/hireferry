//! Free planned-opening changes exercise the actual SQLite and HTTP boundaries.
//! No email, payment gateway, or outbound webhook is called by these fixtures.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{
    AppState, auth, commerce,
    config::Config,
    db,
    models::{self, Job, Order},
    payments::{PaymentReceipt, RefundReceipt},
};
use http_body_util::BodyExt;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
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
                .join("headcount.sqlite3")
                .to_string_lossy()
                .into_owned(),
            ..Default::default()
        };
        cfg.payments.stripe.enabled = true;
        cfg.payments.stripe.secret_key = "sk_test_local_headcount_fixture".into();
        cfg.payments.stripe.webhook_secret = "whsec_local_headcount_fixture".into();
        cfg.payments.waffo.enabled = true;
        cfg.payments.waffo.merchant_id = "local_fixture".into();
        cfg.payments.waffo.store_id = "local_fixture".into();
        cfg.payments.waffo.product_id = "local_fixture".into();
        // Nonempty configuration only; direct local settlement never uses keys.
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
    fn user(&self, role: &str, paid: bool) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.state.conn().unwrap().execute(
            "INSERT INTO users(id,email,name,role,employer_paid,created_at,updated_at) VALUES(?1,?2,'Local fixture',?3,?4,?5,?5)",
            params![id,format!("{id}@example.invalid"),role,paid,hireferry::now()],
        ).unwrap();
        id
    }
    fn browser(&self, uid: &str) -> Browser {
        let b = Browser {
            raw: uuid::Uuid::new_v4().to_string(),
            csrf: uuid::Uuid::new_v4().to_string(),
        };
        self.state.conn().unwrap().execute(
            "INSERT INTO sessions(token_hash,user_id,csrf,expires_at,recent_auth_at) VALUES(?1,?2,?3,?4,?5)",
            params![auth::hash_token(&b.raw),uid,b.csrf,hireferry::now()+3600,hireferry::now()],
        ).unwrap();
        b
    }
    fn job(&self, id: &str) -> Job {
        models::load_job(&self.state.conn().unwrap(), id).unwrap()
    }
    fn count(&self, table: &str) -> i64 {
        self.state
            .conn()
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
    async fn publish(&self, uid: &str, headcount: i64, provider: &str) -> (Job, Order) {
        let (job, order) = commerce::new_job_with_headcount(
            &self.state,
            uid,
            "Rust developer",
            "Original JD",
            300_000,
            provider,
            headcount,
        )
        .unwrap();
        commerce::settle_payment(&self.state, receipt(&order))
            .await
            .unwrap();
        (self.job(&job.id), order)
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
async fn observe(f: &Fixture, request: Request<Body>) -> (StatusCode, Value) {
    let response = f.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn request(
    method: &str,
    path: &str,
    browser: Option<&Browser>,
    csrf: Option<&str>,
    origin: Option<&str>,
    body: Value,
) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(user) = browser {
        b = b.header("cookie", format!("hf_session={}", user.raw));
    }
    if let Some(csrf) = csrf {
        b = b.header("x-csrf-token", csrf);
    }
    if let Some(origin) = origin {
        b = b.header("origin", origin);
    }
    b.body(Body::from(body.to_string())).unwrap()
}

#[test]
fn version_three_jobs_gain_openings_without_reopening_archives() {
    let db = Connection::open_in_memory().unwrap();
    // A legacy schema snapshot, intentionally without any new job fields.
    db.execute_batch(r#"
        CREATE TABLE users(id TEXT PRIMARY KEY,email TEXT NOT NULL UNIQUE,google_sub TEXT UNIQUE,name TEXT NOT NULL DEFAULT '',role TEXT NOT NULL DEFAULT 'unassigned',employer_paid INTEGER NOT NULL DEFAULT 0,avatar TEXT NOT NULL DEFAULT '',public_email TEXT NOT NULL DEFAULT '',resume_md TEXT NOT NULL DEFAULT '',visible INTEGER NOT NULL DEFAULT 0,banned INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
        CREATE TABLE jobs(id TEXT PRIMARY KEY,owner_id TEXT NOT NULL REFERENCES users(id),title TEXT NOT NULL,jd_md TEXT NOT NULL,salary_cents INTEGER NOT NULL CHECK(salary_cents>0),status TEXT NOT NULL DEFAULT 'draft',edits_used INTEGER NOT NULL DEFAULT 0,version INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
        INSERT INTO users(id,email,role,employer_paid,created_at,updated_at) VALUES('owner','old@example.invalid','employer',1,100,100);
        PRAGMA user_version=3;
    "#).unwrap();
    for status in ["draft", "published", "closed", "deleted"] {
        db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,status,edits_used,version,created_at,updated_at) VALUES(?1,'owner','Legacy title','Legacy JD',300000,?1,2,7,100,200)",[status]).unwrap();
    }
    db::migrate(&db).unwrap();
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 12);
    for status in ["draft", "published", "closed", "deleted"] {
        let job = models::load_job(&db, status).unwrap();
        assert_eq!(job.status, status);
        assert_eq!(
            job.headcount,
            if matches!(status, "closed" | "deleted") {
                0
            } else {
                1
            }
        );
        assert_eq!(
            (job.version, job.edits_used, job.salary_cents),
            (7, 2, 300000)
        );
        assert_eq!((job.created_at, job.updated_at), (100, 200));
        if status == "deleted" {
            assert_eq!(job.delete_count, 2);
            assert!(job.title.is_empty() && job.jd_md.is_empty());
        } else {
            assert_eq!(job.delete_count, 0);
            assert_eq!(job.title, "Legacy title");
            assert_eq!(job.jd_md, "Legacy JD");
        }
    }
    db.execute("UPDATE jobs SET headcount=7 WHERE id='published'", [])
        .unwrap();
    db::migrate(&db).unwrap();
    assert_eq!(models::load_job(&db, "published").unwrap().headcount, 7);
    assert!(
        db.execute("UPDATE jobs SET headcount=-1 WHERE id='published'", [])
            .is_err()
    );
    assert!(
        db.execute("UPDATE jobs SET headcount=1000001 WHERE id='published'", [])
            .is_err()
    );
    db.pragma_update(None, "user_version", 13).unwrap();
    assert_eq!(db::migrate(&db).unwrap_err().status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn creation_defaults_and_explicit_openings_are_atomic_and_do_not_multiply_fees() {
    let f = Fixture::new();
    let owner = f.user("employer", true);
    let browser = f.browser(&owner);
    for headcount in [None, Some(0), Some(5), Some(1_000_000)] {
        let mut body =
            json!({"title":"Rust role","jd_md":"JD","salary_cents":300000,"provider":"stripe"});
        if let Some(headcount) = headcount {
            body["headcount"] = json!(headcount);
        }
        let (status, value) = observe(
            &f,
            request(
                "POST",
                "/api/jobs",
                Some(&browser),
                Some(&browser.csrf),
                Some("http://localhost:3000"),
                body,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert_eq!(value["job"]["headcount"], headcount.unwrap_or(1));
        assert_eq!(value["job"]["status"], "draft");
        assert_eq!(value["order"]["amount_cents"], 30000);
        assert_eq!(
            f.job(value["job"]["id"].as_str().unwrap()).headcount,
            headcount.unwrap_or(1)
        );
    }
    let before = (f.count("jobs"), f.count("orders"), f.count("audit"));
    for invalid in [-1, 1_000_001, i64::MAX] {
        assert_eq!(
            commerce::new_job_with_headcount(
                &f.state, &owner, "Rust", "JD", 300000, "stripe", invalid
            )
            .unwrap_err()
            .status,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        (f.count("jobs"), f.count("orders"), f.count("audit")),
        before
    );
    f.state.conn().unwrap().execute_batch("CREATE TRIGGER reject_publication BEFORE INSERT ON orders WHEN NEW.kind='publish' BEGIN SELECT RAISE(ABORT,'local failure fixture'); END;").unwrap();
    assert!(
        commerce::new_job_with_headcount(&f.state, &owner, "Rollback", "JD", 300000, "stripe", 9)
            .is_err()
    );
    assert_eq!(
        (f.count("jobs"), f.count("orders"), f.count("audit")),
        before
    );
    f.state
        .conn()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_publication;")
        .unwrap();
    let (legacy, order) =
        commerce::new_job(&f.state, &owner, "Legacy caller", "JD", 300000, "stripe").unwrap();
    assert_eq!(legacy.headcount, 1);
    assert_eq!(order.amount_cents, 30000);
}

#[tokio::test]
async fn openings_remain_free_after_all_paid_jd_edits_are_used() {
    let f = Fixture::new();
    let owner = f.user("employer", true);
    let (job, _) = f.publish(&owner, 1, "stripe").await;
    for index in 1..=3 {
        let (_, order) = commerce::edit_job(
            &f.state,
            &owner,
            &job.id,
            &format!("Paid JD {index}"),
            "stripe",
        )
        .unwrap();
        assert_eq!(order.amount_cents, 3000);
        commerce::settle_payment(&f.state, receipt(&order))
            .await
            .unwrap();
    }
    let before = f.job(&job.id);
    let money_before = (f.count("orders"), f.count("ledger"));
    for index in 0..64 {
        let openings = if index % 2 == 0 { 0 } else { index + 1 };
        let after = commerce::set_headcount(&f.state, &owner, &job.id, openings).unwrap();
        assert_eq!(after.headcount, openings);
        assert_eq!(after.status, "published");
        assert_eq!((after.version, after.edits_used), (before.version, 3));
        assert_eq!(after.jd_md, before.jd_md);
        assert_eq!(
            (after.title, after.salary_cents),
            (before.title.clone(), before.salary_cents)
        );
    }
    assert_eq!((f.count("orders"), f.count("ledger")), money_before);
    let changes: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM audit WHERE action='job_headcount_update' AND target=?1",
            [&job.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(changes, 64);
    let total: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT sum(amount_cents) FROM ledger", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total, 39000);
}

#[tokio::test]
async fn pending_paid_revision_and_zero_openings_can_change_independently() {
    let f = Fixture::new();
    let owner = f.user("employer", true);
    let (draft, publish) =
        commerce::new_job(&f.state, &owner, "Rust", "Original JD", 300000, "stripe").unwrap();
    let paused = commerce::set_headcount(&f.state, &owner, &draft.id, 0).unwrap();
    assert_eq!((paused.status.as_str(), paused.version), ("draft", 0));
    commerce::settle_payment(&f.state, receipt(&publish))
        .await
        .unwrap();
    let published = f.job(&draft.id);
    assert_eq!(
        (published.status.as_str(), published.headcount),
        ("published", 0)
    );
    let (_, edit) =
        commerce::edit_job(&f.state, &owner, &draft.id, "New paid JD", "stripe").unwrap();
    let count_before = f.count("orders");
    for count in [10, 0, 3, 0, 8] {
        let updated = commerce::set_headcount(&f.state, &owner, &draft.id, count).unwrap();
        assert_eq!(updated.version, published.version);
        assert_eq!(updated.edits_used, 0);
    }
    assert_eq!(f.count("orders"), count_before);
    commerce::settle_payment(&f.state, receipt(&edit))
        .await
        .unwrap();
    let after = f.job(&draft.id);
    assert_eq!(
        (after.headcount, after.edits_used, after.version),
        (8, 1, published.version + 1)
    );
    assert_eq!(after.jd_md, "New paid JD");
    assert_eq!(
        models::load_order(&f.state.conn().unwrap(), &edit.id)
            .unwrap()
            .status,
        "paid"
    );
}

#[tokio::test]
async fn headcount_http_enforces_ownership_csrf_exact_schema_and_numeric_bounds() {
    let f = Fixture::new();
    let owner = f.user("employer", true);
    let other = f.user("employer", true);
    let (job, _) = f.publish(&owner, 3, "stripe").await;
    let owner_browser = f.browser(&owner);
    let other_browser = f.browser(&other);
    let path = format!("/api/jobs/{}/headcount", job.id);
    for (browser, csrf, origin, expected) in [
        (None, None, None, StatusCode::UNAUTHORIZED),
        (
            Some(&owner_browser),
            None,
            Some("http://localhost:3000"),
            StatusCode::FORBIDDEN,
        ),
        (
            Some(&owner_browser),
            Some("wrong"),
            Some("http://localhost:3000"),
            StatusCode::FORBIDDEN,
        ),
        (
            Some(&owner_browser),
            Some(owner_browser.csrf.as_str()),
            Some("https://unrelated.example"),
            StatusCode::FORBIDDEN,
        ),
        (
            Some(&other_browser),
            Some(other_browser.csrf.as_str()),
            Some("http://localhost:3000"),
            StatusCode::FORBIDDEN,
        ),
    ] {
        let (status, _) = observe(
            &f,
            request(
                "PATCH",
                &path,
                browser,
                csrf,
                origin,
                json!({"headcount":0}),
            ),
        )
        .await;
        assert_eq!(status, expected);
        assert_eq!(f.job(&job.id).headcount, 3);
    }
    for (body, expected) in [
        (json!({"headcount":-1}), StatusCode::BAD_REQUEST),
        (json!({"headcount":1000001}), StatusCode::BAD_REQUEST),
        (json!({"headcount":null}), StatusCode::UNPROCESSABLE_ENTITY),
        (json!({"headcount":"2"}), StatusCode::UNPROCESSABLE_ENTITY),
        (json!({"headcount":1.5}), StatusCode::UNPROCESSABLE_ENTITY),
        (
            json!({"headcount":2,"title":"Bypass"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        assert_eq!(
            observe(
                &f,
                request(
                    "PATCH",
                    &path,
                    Some(&owner_browser),
                    Some(&owner_browser.csrf),
                    Some("http://localhost:3000"),
                    body
                )
            )
            .await
            .0,
            expected
        );
    }
    for count in [0, 1_000_000] {
        let (status, value) = observe(
            &f,
            request(
                "PATCH",
                &path,
                Some(&owner_browser),
                Some(&owner_browser.csrf),
                Some("http://localhost:3000"),
                json!({"headcount":count}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["headcount"], count);
        assert_eq!(value["version"], job.version);
    }
    for (role, paid, banned) in [
        ("seeker", false, false),
        ("employer", false, false),
        ("employer", true, true),
    ] {
        f.state
            .conn()
            .unwrap()
            .execute(
                "UPDATE users SET role=?2,employer_paid=?3,banned=?4 WHERE id=?1",
                params![owner, role, paid, banned],
            )
            .unwrap();
        assert_eq!(
            commerce::set_headcount(&f.state, &owner, &job.id, 1)
                .unwrap_err()
                .status,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(f.job(&job.id).headcount, 1_000_000);
    assert_eq!(f.count("orders"), 1);
}

#[tokio::test]
async fn close_and_full_publication_refund_zero_openings_and_cannot_be_reopened_free() {
    let f = Fixture::new();
    let owner = f.user("employer", true);
    let (job, _) = f.publish(&owner, 5, "stripe").await;
    let closed = commerce::close_job(&f.state, &owner, &job.id).unwrap();
    assert_eq!((closed.status.as_str(), closed.headcount), ("closed", 0));
    assert_eq!(
        commerce::set_headcount(&f.state, &owner, &job.id, 3)
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    let (refunded, order) = f.publish(&owner, 9, "waffo").await;
    commerce::settle_refund(
        &f.state,
        RefundReceipt {
            provider: "waffo".into(),
            event_id: "REF_headcount".into(),
            refund_id: "REF_headcount".into(),
            order_id: order.id.clone(),
            provider_order_id: format!("ORD_{}", order.id),
            payment_id: format!("pay_{}", order.id),
            amount_cents: order.amount_cents,
            original_amount_cents: Some(order.amount_cents),
            currency: "CNY".into(),
            live_mode: false,
            succeeded: true,
            reason: "Local refund fixture".into(),
        },
    )
    .await
    .unwrap();
    let refunded = f.job(&refunded.id);
    assert_eq!(
        (refunded.status.as_str(), refunded.headcount),
        ("closed", 0)
    );
    assert_eq!(
        commerce::set_headcount(&f.state, &owner, &refunded.id, 2)
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    commerce::settle_payment(&f.state, receipt(&order))
        .await
        .unwrap();
    assert_eq!(
        (
            f.job(&refunded.id).status.as_str(),
            f.job(&refunded.id).headcount
        ),
        ("closed", 0)
    );
    let deleted = commerce::delete_job(&f.state, &owner, &job.id, closed.version).unwrap();
    assert_eq!(
        commerce::set_headcount(&f.state, &owner, &deleted.id, 1)
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(f.count("orders"), 2);
}
