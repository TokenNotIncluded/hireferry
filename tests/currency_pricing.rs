//! Currency and pricing changes never reinterpret historical amounts.
use hireferry::{
    AppState, access, api, commerce,
    config::{Config, CurrencyAmounts},
    db, models,
    payments::{PaymentReceipt, decimal_to_minor, minor_to_decimal},
};
use rusqlite::{Connection, params};
use serde_json::json;
use tempfile::TempDir;

fn config(dir: &TempDir) -> Config {
    let mut cfg = Config {
        development: true,
        database: dir
            .path()
            .join("currency.sqlite3")
            .to_string_lossy()
            .into_owned(),
        ..Config::default()
    };
    cfg.payments.stripe.enabled = true;
    cfg.payments.stripe.secret_key = "sk_test_currency_fixture".into();
    cfg.payments.stripe.webhook_secret = "whsec_currency_fixture".into();
    cfg
}
fn employer(state: &AppState) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    state.conn().unwrap().execute("INSERT INTO users(id,email,role,employer_paid,created_at,updated_at) VALUES(?1,?2,'employer',1,1,1)",params![id,format!("{id}@example.invalid")]).unwrap();
    id
}
fn receipt(order: &models::Order) -> PaymentReceipt {
    PaymentReceipt {
        provider: order.provider.clone(),
        event_id: format!("evt_{}", order.id),
        order_id: order.id.clone(),
        session_id: None,
        payment_id: format!("pay_{}", order.id),
        amount_cents: order.amount_cents,
        currency: order.currency.clone(),
        live_mode: order.live_mode,
    }
}

#[test]
fn display_amounts_use_currency_precision_without_float_rounding() {
    assert_eq!(decimal_to_minor("123.45", "USD"), Some(12345));
    assert_eq!(decimal_to_minor("123", "JPY"), Some(123));
    assert_eq!(decimal_to_minor("123.00", "JPY"), Some(123));
    assert_eq!(decimal_to_minor("123.01", "JPY"), None);
    assert_eq!(decimal_to_minor("1.001", "EUR"), None);
    assert_eq!(minor_to_decimal(123, "JPY").as_deref(), Some("123"));
    assert_eq!(minor_to_decimal(123, "USD").as_deref(), Some("1.23"));
    assert_eq!(decimal_to_minor("1e3", "USD"), None);
    assert_eq!(decimal_to_minor("1.234", "KWD"), None); // no unverified gateway support
    assert_eq!(decimal_to_minor("92233720368547758.08", "USD"), None);
}

#[test]
fn currency_and_fee_configuration_rejects_ambiguous_or_invalid_values() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&dir);
    cfg.pricing.allowed_currencies = vec!["CNY".into(), "JPY".into()];
    assert!(cfg.validate().is_err()); // no inferred exchange rate or salary threshold
    cfg.pricing.currency_amounts.insert(
        "JPY".into(),
        CurrencyAmounts {
            publish_fixed_minor: 3000,
            edit_fixed_minor: 300,
            employer_ai_min_salary_minor: 300000,
        },
    );
    cfg.validate().unwrap();
    cfg.pricing.currency = "usd".into();
    assert!(cfg.validate().is_err());
    cfg.pricing.currency = "CNY".into();
    cfg.pricing.publish_mode = "unknown".into();
    assert!(cfg.validate().is_err());
    cfg.pricing.publish_mode = "free".into();
    cfg.pricing.publish_bps = 0;
    cfg.validate().unwrap();
    cfg.limits.list_page_size = 201;
    assert!(cfg.validate().is_err());
    cfg.limits.list_page_size = 50;
    for value in [0, 101] {
        cfg.limits.workbench_page_size = value;
        assert!(cfg.validate().is_err());
    }
    for value in [1, 100] {
        cfg.limits.workbench_page_size = value;
        cfg.validate().unwrap();
    }
    for value in [0, 1001] {
        cfg.limits.recent_list_limit = value;
        assert!(cfg.validate().is_err());
    }
    for value in [1, 1000] {
        cfg.limits.recent_list_limit = value;
        cfg.validate().unwrap();
    }
}

#[test]
fn gateway_currency_subsets_reject_unknown_duplicate_and_unsupported_codes() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&dir);
    assert_eq!(hireferry::config::STRIPE_CURRENCIES.len(), 32);
    assert_eq!(hireferry::config::WAFFO_CURRENCIES.len(), 6);
    for code in hireferry::config::STRIPE_CURRENCIES {
        let decimals = hireferry::config::currency_decimals(code).unwrap();
        assert_eq!(decimals, if matches!(*code, "JPY" | "KRW") { 0 } else { 2 });
    }
    for code in ["ISK", "UGX", "KWD", "INVALID"] {
        assert!(hireferry::config::currency_decimals(code).is_none());
    }
    cfg.payments.stripe.allowed_currencies = vec!["USD".into(), "USD".into()];
    assert!(cfg.validate().is_err());
    cfg.payments.stripe.allowed_currencies = vec!["usd".into()];
    assert!(cfg.validate().is_err());
    cfg.payments.stripe.allowed_currencies = vec!["USD".into(), "TWD".into()];
    cfg.validate().unwrap();
    assert!(cfg.payments.permits_currency("stripe", "TWD"));
    assert!(!cfg.payments.permits_currency("stripe", "CNY"));
    assert_eq!(
        cfg.payments.public_currencies()["stripe"],
        vec!["USD", "TWD"]
    );
    assert!(!cfg.payments.public_currencies().contains_key("waffo"));
    cfg.payments.waffo.allowed_currencies = vec!["AUD".into()];
    assert!(cfg.validate().is_err());
    cfg.payments.waffo.allowed_currencies = vec!["JPY".into(), "USD".into()];
    cfg.validate().unwrap();
}

#[tokio::test]
async fn unsupported_gateway_currency_is_rejected_before_writes_or_io_and_receipts_keep_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&dir);
    cfg.pricing.currency = "AUD".into();
    cfg.payments.waffo.enabled = true;
    cfg.payments.waffo.merchant_id = "currency_fixture".into();
    cfg.payments.waffo.store_id = "currency_fixture".into();
    cfg.payments.waffo.product_id = "currency_fixture".into();
    cfg.payments.waffo.private_key_pem = "not used before currency validation".into();
    cfg.payments.waffo.webhook_public_key_pem = "not used before currency validation".into();
    cfg.payments.waffo.tax_category = "currency_fixture".into();
    let original = AppState::new(cfg.clone()).unwrap();
    let owner = employer(&original);
    assert!(commerce::new_job(&original, &owner, "AUD role", "JD", 300000, "waffo").is_err());
    assert_eq!(
        original
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM jobs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    original.conn().unwrap().execute("INSERT INTO users(id,email,role,created_at,updated_at) VALUES('new_owner','new_owner@example.invalid','unassigned',1,1)", []).unwrap();
    assert!(commerce::choose_role(&original, "new_owner", "employer", Some("waffo")).is_err());
    assert_eq!(
        models::load_user(&original.conn().unwrap(), "new_owner")
            .unwrap()
            .role,
        "unassigned"
    );
    let (job, order) =
        commerce::new_job(&original, &owner, "AUD role", "JD", 300000, "stripe").unwrap();
    original
        .conn()
        .unwrap()
        .execute(
            "UPDATE orders SET checkout_url='https://checkout.stripe.com/cached' WHERE id=?1",
            [&order.id],
        )
        .unwrap();
    cfg.payments.stripe.allowed_currencies = vec!["USD".into()];
    let restricted = AppState::new(cfg).unwrap();
    assert!(
        commerce::checkout(&restricted, &owner, &order.id)
            .await
            .is_err()
    );
    assert!(
        hireferry::payments::create_checkout(&restricted, &order, "ignored@example.invalid")
            .await
            .is_err()
    );
    let visible = api::user_order(&restricted, &owner, &order.id).unwrap();
    assert_eq!(visible.currency, "AUD");
    assert!(visible.checkout_url.is_none());
    assert!(
        restricted
            .conn()
            .unwrap()
            .query_row(
                "SELECT checkout_started_at FROM orders WHERE id=?1",
                [&order.id],
                |row| row.get::<_, Option<i64>>(0)
            )
            .unwrap()
            .is_none()
    );
    // Disabling new checkout must not discard an already received, authentic
    // payment for a historical order or relabel its amount.
    commerce::settle_payment(&restricted, receipt(&order))
        .await
        .unwrap();
    let paid = models::load_job(&restricted.conn().unwrap(), &job.id).unwrap();
    assert_eq!(
        (paid.currency.as_str(), paid.status.as_str()),
        ("AUD", "published")
    );
    assert!(commerce::edit_job(&restricted, &owner, &job.id, "New JD", "stripe").is_err());
    assert_eq!(
        restricted
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM orders", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let map = api::public_config(&restricted)["payment_currencies"].clone();
    assert_eq!(map["stripe"], json!(["USD"]));
    assert!(!map["waffo"].as_array().unwrap().contains(&json!("AUD")));
}

#[tokio::test]
async fn historical_jobs_orders_and_edit_terms_survive_currency_and_fee_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&dir);
    cfg.pricing.currency = "JPY".into();
    let original = AppState::new(cfg.clone()).unwrap();
    let owner = employer(&original);
    let (job, order) =
        commerce::new_job(&original, &owner, "JPY role", "Original", 300000, "stripe").unwrap();
    assert_eq!(
        (
            job.currency.as_str(),
            job.currency_decimals,
            order.amount_cents
        ),
        ("JPY", 0, 30000)
    );
    assert_eq!(order.currency, "JPY");
    commerce::settle_payment(&original, receipt(&order))
        .await
        .unwrap();
    cfg.pricing.currency = "USD".into();
    cfg.pricing.publish_mode = "fixed".into();
    cfg.pricing.publish_fixed_minor = 1234;
    cfg.pricing.edit_mode = "fixed".into();
    cfg.pricing.edit_fixed_minor = 999;
    cfg.pricing.max_edits = 0;
    cfg.pricing.employer_ai_min_salary_minor = 900000;
    let updated = AppState::new(cfg).unwrap();
    let (_, old_edit) =
        commerce::edit_job(&updated, &owner, &job.id, "New content", "stripe").unwrap();
    assert_eq!(
        (
            old_edit.currency.as_str(),
            old_edit.currency_decimals,
            old_edit.amount_cents
        ),
        ("JPY", 0, 3000)
    );
    commerce::settle_payment(&updated, receipt(&old_edit))
        .await
        .unwrap();
    assert_eq!(
        models::load_job(&updated.conn().unwrap(), &job.id)
            .unwrap()
            .ai_min_salary_cents,
        300000
    );
    assert_eq!(
        access::eligibility(&updated.conn().unwrap(), &owner).unwrap()["eligible"],
        true
    );
    let (new_job, new_order) =
        commerce::new_job(&updated, &owner, "USD role", "Different", 50000, "stripe").unwrap();
    assert_eq!(
        (
            new_job.currency.as_str(),
            new_order.currency.as_str(),
            new_order.amount_cents
        ),
        ("USD", "USD", 1234)
    );
    assert_eq!(
        api::user_order(&updated, &owner, &order.id)
            .unwrap()
            .currency,
        "JPY"
    );
    let mut mismatch = receipt(&new_order);
    mismatch.currency = "JPY".into();
    assert!(commerce::settle_payment(&updated, mismatch).await.is_err());
}

#[tokio::test]
async fn free_policy_needs_no_gateway_or_cash_ledger_and_keeps_recorded_edit_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&dir);
    cfg.payments.stripe.enabled = false;
    cfg.pricing.onboarding_cents = 0;
    cfg.pricing.publish_mode = "free".into();
    cfg.pricing.edit_mode = "free".into();
    cfg.pricing.max_edits = 1;
    cfg.pricing.employer_ai_min_salary_minor = 0;
    let state = AppState::new(cfg).unwrap();
    let owner = employer(&state);
    let (job, order) =
        commerce::new_job(&state, &owner, "Free role", "Original", 100, "ignored").unwrap();
    assert_eq!(
        (
            job.status.as_str(),
            order.provider.as_str(),
            order.status.as_str(),
            order.amount_cents
        ),
        ("published", "free", "paid", 0)
    );
    assert!(commerce::checkout(&state, &owner, &order.id).await.is_err());
    let (edited, edit_order) =
        commerce::edit_job(&state, &owner, &job.id, "Applied immediately", "").unwrap();
    assert_eq!(
        (
            edited.jd_md.as_str(),
            edited.edits_used,
            edit_order.amount_cents
        ),
        ("Applied immediately", 1, 0)
    );
    assert!(commerce::edit_job(&state, &owner, &job.id, "Over allowance", "").is_err());
    assert_eq!(
        state
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM ledger", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        access::eligibility(&state.conn().unwrap(), &owner).unwrap()["eligible"],
        true
    );
}

#[tokio::test]
async fn selected_currency_uses_explicit_amounts_and_limits_are_configurable() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&dir);
    cfg.pricing.allowed_currencies = vec!["CNY".into(), "USD".into()];
    cfg.pricing.currency_amounts.insert(
        "USD".into(),
        CurrencyAmounts {
            publish_fixed_minor: 888,
            edit_fixed_minor: 88,
            employer_ai_min_salary_minor: 12345,
        },
    );
    cfg.pricing.publish_mode = "fixed".into();
    cfg.limits.max_job_headcount = 9;
    cfg.limits.max_pending_orders_per_user = 1;
    cfg.limits.list_page_size = 1;
    let state = AppState::new(cfg).unwrap();
    let owner = employer(&state);
    assert!(
        commerce::new_job_with_currency(
            &state,
            &owner,
            "Role",
            "JD",
            10000,
            "stripe",
            10,
            Some("USD")
        )
        .is_err()
    );
    let (job, order) = commerce::new_job_with_currency(
        &state,
        &owner,
        "Role",
        "JD",
        10000,
        "stripe",
        9,
        Some("USD"),
    )
    .unwrap();
    assert_eq!(
        (
            job.currency.as_str(),
            job.ai_min_salary_cents,
            order.amount_cents
        ),
        ("USD", 12345, 888)
    );
    assert!(commerce::new_job(&state, &owner, "More", "JD", 10000, "stripe").is_err());
    assert_eq!(
        api::list_jobs(&state, &api::ListQuery::default(), Some(&owner)).unwrap()["limit"],
        1
    );
    assert_eq!(
        state.cfg.pricing.public()["currency_options"][1]["currency"],
        "USD"
    );
    commerce::settle_payment(&state, receipt(&order))
        .await
        .unwrap();
    commerce::new_job(&state, &owner, "Another", "JD", 10000, "stripe").unwrap();
    assert!(commerce::edit_job(&state, &owner, &job.id, "Pending limit", "stripe").is_err());
}

#[test]
fn legacy_cny_money_and_child_references_survive_zero_fee_table_upgrade() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(r#"
PRAGMA foreign_keys=ON;
CREATE TABLE users(id TEXT PRIMARY KEY,email TEXT NOT NULL UNIQUE,google_sub TEXT UNIQUE,name TEXT NOT NULL DEFAULT '',role TEXT NOT NULL DEFAULT 'employer',employer_paid INTEGER NOT NULL DEFAULT 1,avatar TEXT NOT NULL DEFAULT '',public_email TEXT NOT NULL DEFAULT '',resume_md TEXT NOT NULL DEFAULT '',visible INTEGER NOT NULL DEFAULT 0,banned INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE jobs(id TEXT PRIMARY KEY,owner_id TEXT NOT NULL REFERENCES users(id),title TEXT NOT NULL,jd_md TEXT NOT NULL,salary_cents INTEGER NOT NULL CHECK(salary_cents>0),status TEXT NOT NULL DEFAULT 'published',edits_used INTEGER NOT NULL DEFAULT 0,version INTEGER NOT NULL DEFAULT 1,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE orders(id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id),kind TEXT NOT NULL,job_id TEXT REFERENCES jobs(id),amount_cents INTEGER NOT NULL CHECK(amount_cents>0),currency TEXT NOT NULL DEFAULT 'CNY',provider TEXT NOT NULL,live_mode INTEGER NOT NULL DEFAULT 0,status TEXT NOT NULL DEFAULT 'paid',snapshot_json TEXT NOT NULL DEFAULT '{}',provider_session TEXT,checkout_url TEXT,checkout_started_at INTEGER,created_at INTEGER NOT NULL,paid_at INTEGER);
CREATE TABLE ledger(id INTEGER PRIMARY KEY,order_id TEXT NOT NULL UNIQUE REFERENCES orders(id),provider TEXT NOT NULL,payment_id TEXT NOT NULL,amount_cents INTEGER NOT NULL,currency TEXT NOT NULL,live_mode INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,UNIQUE(provider,payment_id));
CREATE TABLE payment_events(provider TEXT NOT NULL,event_id TEXT NOT NULL,order_id TEXT NOT NULL REFERENCES orders(id),created_at INTEGER NOT NULL,PRIMARY KEY(provider,event_id));
CREATE TABLE legacy_migration_probe(order_id TEXT NOT NULL);
CREATE TRIGGER webhook_fanout AFTER INSERT ON legacy_migration_probe BEGIN SELECT id FROM orders WHERE id=NEW.order_id; END;
INSERT INTO users(id,email,created_at,updated_at) VALUES('owner','legacy@example.invalid',1,1);
INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,created_at,updated_at) VALUES('job','owner','Legacy','JD',300000,1,1);
INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,created_at,paid_at,snapshot_json) VALUES('paid','owner','publish','job',30000,'stripe',1,1,'{"max_edits":3,"edit_bps":100}');
INSERT INTO ledger(order_id,provider,payment_id,amount_cents,currency,created_at) VALUES('paid','stripe','pay_legacy',30000,'CNY',1);
INSERT INTO payment_events(provider,event_id,order_id,created_at) VALUES('stripe','evt_legacy','paid',1);
PRAGMA user_version=3;
"#).unwrap();
    db::migrate(&db).unwrap();
    assert_eq!(models::load_job(&db, "job").unwrap().currency, "CNY");
    let order = models::load_order(&db, "paid").unwrap();
    assert_eq!(
        (order.currency.as_str(), order.amount_cents),
        ("CNY", 30000)
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&order.snapshot_json).unwrap(),
        json!({"max_edits":3,"edit_bps":100})
    );
    assert_eq!(
        db.query_row("SELECT l.order_id,l.currency,l.amount_cents,e.event_id FROM ledger l JOIN payment_events e ON e.order_id=l.order_id", [], |row| {
            Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,i64>(2)?,row.get::<_,String>(3)?))
        }).unwrap(),
        ("paid".to_owned(),"CNY".to_owned(),30000,"evt_legacy".to_owned())
    );
    assert_eq!(
        db.query_row(
            "SELECT tbl_name FROM sqlite_master WHERE type='trigger' AND name='webhook_fanout'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "webhook_events"
    );
    db.execute("INSERT INTO orders(id,user_id,kind,job_id,amount_cents,currency,provider,status,created_at) VALUES('free','owner','edit','job',0,'JPY','free','paid',2)",[]).unwrap();
    db::migrate(&db).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        7
    );
    assert_eq!(
        db.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
}
