use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{
    AppState, auth, commerce,
    config::{Config, WaffoProduction},
    models,
};
use http_body_util::BodyExt;
use rsa::{
    RsaPrivateKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
};
use serde_json::{Value, json};
use tower::ServiceExt;

fn config(path: &str) -> Config {
    let key = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
    let private = key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
    let public = key
        .to_public_key()
        .to_public_key_pem(LineEnding::LF)
        .unwrap();
    let mut c = Config {
        development: true,
        database: path.into(),
        ..Default::default()
    };
    let w = &mut c.payments.waffo;
    w.enabled = true;
    w.merchant_id = "MER_test".into();
    w.store_id = "STO_test".into();
    w.product_id = "PRO_test".into();
    w.tax_category = "saas".into();
    w.private_key_pem = private.clone();
    w.webhook_public_key_pem = public.clone();
    let prod_key = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
    w.production = Some(WaffoProduction {
        merchant_id: "MER_prod".into(),
        store_id: "STO_prod".into(),
        product_id: "PRO_prod".into(),
        tax_category: "saas".into(),
        private_key_pem: prod_key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string(),
        webhook_public_key_pem: prod_key
            .to_public_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap(),
    });
    c
}
async fn request(
    s: &AppState,
    body: Option<Value>,
    admin: bool,
    csrf: bool,
) -> (StatusCode, Value) {
    let mut r = Request::builder()
        .method(if body.is_some() { "POST" } else { "GET" })
        .uri("/api/admin/payments/waffo");
    if admin {
        r = r.header("cookie", "hf_session=local-admin");
    }
    if csrf {
        r = r
            .header("origin", "http://localhost:3000")
            .header("x-csrf-token", "local-csrf");
    }
    let r = r
        .header("content-type", "application/json")
        .body(
            body.map(|b| Body::from(b.to_string()))
                .unwrap_or(Body::empty()),
        )
        .unwrap();
    let response = hireferry::app(s.clone()).oneshot(r).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn order(s: &AppState) -> models::Order {
    let id = uuid::Uuid::new_v4().to_string();
    s.conn()
        .unwrap()
        .execute(
            "INSERT INTO users(id,email,created_at,updated_at) VALUES(?1,?2,0,0)",
            rusqlite::params![id, format!("{id}@example.invalid")],
        )
        .unwrap();
    commerce::choose_role(s, &id, "employer", Some("waffo"))
        .unwrap()
        .1
        .unwrap()
}
#[tokio::test]
async fn switching_is_authorized_audited_persistent_and_keeps_old_orders() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path().join("db.sqlite").to_str().unwrap());
    let s = AppState::new(cfg.clone()).unwrap();
    s.conn().unwrap().execute("INSERT INTO sessions(token_hash,admin,bootstrap,csrf,expires_at,recent_auth_at) VALUES(?1,1,0,'local-csrf',?2,?3)", rusqlite::params![auth::hash_token("local-admin"), hireferry::now()+3600, hireferry::now()]).unwrap();
    assert_eq!(
        request(&s, None, false, false).await.0,
        StatusCode::UNAUTHORIZED
    );
    let old = order(&s);
    assert!(!old.live_mode);
    let change = json!({"mode":"prod","expected_mode":"test"});
    assert!(
        !request(&s, Some(change.clone()), true, false)
            .await
            .0
            .is_success()
    );
    let (status, data) = request(&s, Some(change.clone()), true, true).await;
    assert_eq!(status, StatusCode::OK, "{data}");
    assert_eq!(data["mode"], "prod");
    assert!(!data.to_string().contains("PRIVATE"));
    assert_eq!(
        request(&s, Some(change), true, true).await.0,
        StatusCode::CONFLICT
    );
    assert!(order(&s).live_mode);
    assert!(
        !models::load_order(&s.conn().unwrap(), &old.id)
            .unwrap()
            .live_mode
    );
    assert_eq!(
        hireferry::api::public_config(&s)["payment_modes"]["waffo"],
        "prod"
    );
    let restarted = AppState::new(cfg).unwrap();
    assert_eq!(
        request(&restarted, None, true, false).await.1["mode"],
        "prod"
    );
    assert_eq!(
        request(
            &restarted,
            Some(json!({"mode":"test","expected_mode":"prod"})),
            true,
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(!order(&restarted).live_mode);
    let db = restarted.conn().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM audit WHERE action='payment_mode_change'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}
#[tokio::test]
async fn production_requires_a_complete_separate_profile() {
    let mut cfg = config(":memory:");
    cfg.payments.waffo.production = None;
    let s = AppState::new(cfg).unwrap();
    assert!(s.cfg.payments.waffo.for_mode("prod").is_err());
    assert!(s.cfg.payments.waffo.for_mode("test").is_ok());
    assert!(s.cfg.payments.waffo.for_mode("unknown").is_err());
}

async fn webhook(s: &AppState, event: Value, signing_mode: &str) -> StatusCode {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use rsa::{
        pkcs1v15::SigningKey,
        pkcs8::DecodePrivateKey,
        signature::{SignatureEncoding, Signer},
    };
    let profile = s.cfg.payments.waffo.for_mode(signing_mode).unwrap();
    let key = RsaPrivateKey::from_pkcs8_pem(&profile.private_key_pem).unwrap();
    let timestamp = chrono::Utc::now().timestamp_millis();
    let body = event.to_string();
    let signed = format!("{timestamp}.{body}");
    let signature = SigningKey::<sha2::Sha256>::new(key).sign(signed.as_bytes());
    let r = Request::builder()
        .method("POST")
        .uri("/api/webhooks/waffo")
        .header("content-type", "application/json")
        .header(
            "x-waffo-signature",
            format!("t={timestamp},v1={}", STANDARD.encode(signature.to_bytes())),
        )
        .body(Body::from(body))
        .unwrap();
    hireferry::app(s.clone()).oneshot(r).await.unwrap().status()
}
fn receipt(order: &models::Order, store: &str) -> Value {
    json!({"id":format!("evt_{}",order.id),"eventType":"order.completed", "storeId":store,
        "mode": if order.live_mode {"prod"} else {"test"},
        "data":{"orderStatus":"completed","paymentStatus":"succeeded","currency":order.currency,
        "chargedAmount":format!("{}.{:02}",order.amount_cents/100,order.amount_cents%100),
        "orderMerchantExternalId":order.id,"paymentId":format!("pay_{}",order.id)}})
}
#[tokio::test]
async fn late_callbacks_keep_their_environment_and_reject_cross_signed_events() {
    let s = AppState::new(config(":memory:")).unwrap();
    let old = order(&s);
    s.conn()
        .unwrap()
        .execute("INSERT INTO payment_settings VALUES('waffo','prod')", [])
        .unwrap();
    let live = order(&s);
    let test_event = receipt(&old, "STO_test");
    assert_eq!(
        webhook(&s, test_event.clone(), "prod").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        webhook(&s, test_event, "test").await,
        StatusCode::NO_CONTENT
    );
    s.conn()
        .unwrap()
        .execute("UPDATE payment_settings SET mode='test'", [])
        .unwrap();
    let prod_event = receipt(&live, "STO_prod");
    assert_eq!(
        webhook(&s, prod_event.clone(), "test").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        webhook(&s, prod_event, "prod").await,
        StatusCode::NO_CONTENT
    );
    let db = s.conn().unwrap();
    for o in [&old, &live] {
        assert_eq!(models::load_order(&db, &o.id).unwrap().status, "paid");
    }
}
