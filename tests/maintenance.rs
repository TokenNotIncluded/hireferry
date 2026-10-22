//! Lifecycle checks use real SQLite state and the production cleanup worker.
use hireferry::{AppState, config::Config, maintenance, now, webhooks};
use rusqlite::{Connection, params};
use std::time::Duration;

fn state(batch: i64) -> AppState {
    let mut cfg = Config::default();
    cfg.development = true;
    cfg.database = ":memory:".into();
    cfg.maintenance.batch_size = batch;
    AppState::new(cfg).unwrap()
}

fn count(db: &Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

fn user(db: &Connection, at: i64) {
    db.execute("INSERT INTO users(id,email,role,created_at,updated_at) VALUES('maintenance-user','maintenance@example.invalid','seeker',?1,?1)", [at]).unwrap();
}

fn session(db: &Connection, id: &str, expires: i64) {
    db.execute("INSERT INTO sessions(token_hash,admin,csrf,expires_at,recent_auth_at) VALUES(?1,1,'test',?2,0)", params![id,expires]).unwrap();
}

#[test]
fn each_pass_is_bounded_and_large_cli_families_do_not_cascade() {
    let s = state(2);
    let at = now();
    let db = s.conn().unwrap();
    user(&db, at);
    for i in 0..7 {
        session(&db, &format!("old-{i}"), at - 2 * 86_400);
    }
    session(&db, "live-session", at + 3600);
    for (id, expires) in [("expired", at - 31 * 86_400), ("live", at + 3600)] {
        db.execute("INSERT INTO cli_authorizations(id,user_id,admin,client_id,scope,created_at,expires_at) VALUES(?1,'maintenance-user',0,'hireferry-cli','identity mcp',?2,?3)", params![id,at-40*86_400,expires]).unwrap();
    }
    for i in 0..7 {
        db.execute("INSERT INTO cli_tokens(token_hash,family_id,admin,created_at,expires_at) VALUES(?1,'expired',0,?2,?2)", params![format!("old-access-{i}"),at-40*86_400]).unwrap();
        db.execute("INSERT INTO cli_refresh_tokens(token_hash,family_id,created_at,used_at) VALUES(?1,'expired',?2,?2)", params![format!("old-refresh-{i}"),at-40*86_400]).unwrap();
    }
    db.execute("INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES('backing','maintenance-user','CLI','backing-hash',?1,?1)", [at-40*86_400]).unwrap();
    db.execute("INSERT INTO cli_tokens(token_hash,token_id,family_id,admin,created_at,expires_at) VALUES('live-access','backing','live',0,?1,?1)", [at-40*86_400]).unwrap();
    db.execute("INSERT INTO cli_refresh_tokens(token_hash,family_id,created_at,used_at) VALUES('live-used-refresh','live',?1,?1)", [at-40*86_400]).unwrap();

    maintenance::prune(&db, &s.cfg, at).unwrap();
    assert_eq!(count(&db, "sessions"), 6);
    assert_eq!(count(&db, "cli_tokens"), 6);
    assert_eq!(count(&db, "cli_refresh_tokens"), 6);
    assert_eq!(
        count(&db, "cli_authorizations"),
        2,
        "parent must remain while a batch of children remains"
    );
    assert_eq!(
        count(&db, "api_tokens"),
        1,
        "live refresh family retains its backing token"
    );
    for _ in 0..3 {
        maintenance::prune(&db, &s.cfg, at).unwrap();
    }
    for table in [
        "sessions",
        "cli_authorizations",
        "cli_tokens",
        "cli_refresh_tokens",
        "api_tokens",
    ] {
        assert_eq!(count(&db, table), 1, "only live state remains in {table}");
    }
}

fn human_grant(db: &Connection, id: &str, at: i64, expires: i64) {
    db.execute("INSERT INTO hg_requests(id,human_link,actor_id,public_key,client_id,purpose,requested_permissions,phase,created_at,status_updated_at,expires_at,requested_grant_expires_at) VALUES(?1,?1,'actor','key','hireferry-cli','test','[]','succeeded',?2,?2,?3,?4)", params![id,at-100*86_400,at-91*86_400,expires]).unwrap();
    db.execute("INSERT INTO hg_grants(id,request_id,user_id,actor_id,client_id,permissions,created_at,expires_at,audit_id) VALUES(?1,?1,'maintenance-user','actor','hireferry-cli','[]',?2,?3,?1)", params![id,at-100*86_400,expires]).unwrap();
    db.execute("INSERT INTO hg_sessions(request_id,grant_id,token_hash,actor_id,created_at) VALUES(?1,?1,?1,'actor',?2)", params![id,at-100*86_400]).unwrap();
}

#[test]
fn human_grant_cleanup_respects_foreign_keys_and_active_replay_windows() {
    let s = state(2);
    let at = now();
    let db = s.conn().unwrap();
    user(&db, at);
    for i in 0..5 {
        human_grant(&db, &format!("expired-{i}"), at, at - 91 * 86_400);
    }
    human_grant(&db, "live", at, at + 3600);
    db.execute("INSERT INTO hg_nonces(actor_id,nonce,expires_at) VALUES('actor','old',?1),('actor','live',?2)", params![at-1,at+60]).unwrap();
    db.execute(
        "INSERT INTO hg_limits(actor_id,created_at) VALUES('actor',?1),('actor',?2)",
        params![at - 3601, at],
    )
    .unwrap();
    maintenance::prune(&db, &s.cfg, at).unwrap();
    for table in ["hg_sessions", "hg_grants", "hg_requests"] {
        assert_eq!(count(&db, table), 4, "one bounded batch from {table}");
    }
    assert_eq!(count(&db, "hg_nonces"), 1);
    assert_eq!(count(&db, "hg_limits"), 1);
    for _ in 0..2 {
        maintenance::prune(&db, &s.cfg, at).unwrap();
    }
    for table in ["hg_sessions", "hg_grants", "hg_requests"] {
        assert_eq!(
            count(&db, table),
            1,
            "active grant chain in {table} must remain"
        );
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}

fn webhook_queue(db: &Connection, at: i64, rows: i64) {
    user(db, at);
    db.execute("INSERT INTO webhook_endpoints(id,user_id,name,url,events_json,signing_secret,created_at,updated_at) VALUES('endpoint','maintenance-user','test','https://example.invalid/hook','[\"webhook.test\"]','secret',?1,?1)", [at-100*86_400]).unwrap();
    for i in 0..rows {
        db.execute("INSERT INTO webhook_events(id,event_type,resource_id,data_json,created_at) VALUES(?1,'webhook.test','endpoint','{}',?2)", params![format!("event-{i}"),at-100*86_400]).unwrap();
    }
}

#[test]
fn webhook_retention_is_bounded_and_does_not_remove_inflight_delivery() {
    let s = state(2);
    let at = now();
    let db = s.conn().unwrap();
    webhook_queue(&db, at, 7);
    assert_eq!(count(&db, "webhook_deliveries"), 7);
    db.execute(
        "UPDATE webhook_deliveries SET status='processing',lease_until=?1 WHERE event_id='event-6'",
        [at + 60],
    )
    .unwrap();
    db.execute("UPDATE webhook_endpoints SET enabled=0,revoked=1", [])
        .unwrap();
    webhooks::cleanup_batch(&db, &s.cfg.webhooks, at, 2).unwrap();
    assert_eq!(count(&db, "webhook_deliveries"), 5);
    assert_eq!(count(&db, "webhook_events"), 5);
    assert_eq!(count(&db, "webhook_endpoints"), 1);
    for _ in 0..2 {
        webhooks::cleanup_batch(&db, &s.cfg.webhooks, at, 2).unwrap();
    }
    assert_eq!(
        count(&db, "webhook_deliveries"),
        1,
        "active send lease is preserved"
    );
    webhooks::cleanup_batch(&db, &s.cfg.webhooks, at + 61, 2).unwrap();
    for table in ["webhook_deliveries", "webhook_events", "webhook_endpoints"] {
        assert_eq!(count(&db, table), 0);
    }
    assert!(webhooks::cleanup_batch(&db, &s.cfg.webhooks, at, 0).is_err());
}

#[tokio::test]
async fn background_worker_cleans_disabled_features_and_drains_backlog() {
    let mut cfg = Config::default();
    cfg.development = true;
    cfg.database = ":memory:".into();
    cfg.maintenance.batch_size = 2;
    cfg.maintenance.interval_minutes = 1440;
    cfg.feedback.enabled = false;
    cfg.cli_auth.enabled = false;
    cfg.humangrant.enabled = false;
    cfg.webhooks.enabled = false;
    let s = AppState::new(cfg).unwrap();
    let at = now();
    {
        let db = s.conn().unwrap();
        for i in 0..5 {
            session(&db, &format!("expired-{i}"), at - 2 * 86_400);
        }
        db.execute("INSERT INTO feedback(actor_key,actor_kind,source,category,message,created_at,updated_at) VALUES('guest','guest','web','bug','expired',?1,?1)", [at-181*86_400]).unwrap();
        db.execute("INSERT INTO auth_challenges(id,kind,subject,expires_at) VALUES('old','email','test',?1)", [at-2*86_400]).unwrap();
        db.execute(
            "INSERT INTO hg_nonces(actor_id,nonce,expires_at) VALUES('actor','old',?1)",
            [at - 1],
        )
        .unwrap();
        webhook_queue(&db, at, 1);
    }
    let worker = maintenance::spawn_worker(s.clone());
    let outcome = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let remaining = {
                let db = s.conn().unwrap();
                [
                    "sessions",
                    "feedback",
                    "auth_challenges",
                    "hg_nonces",
                    "webhook_deliveries",
                    "webhook_events",
                ]
                .iter()
                .map(|table| count(&db, table))
                .sum::<i64>()
            };
            if remaining == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    worker.abort();
    let _ = worker.await;
    assert!(
        outcome.is_ok(),
        "worker must run without a request and drain more than one batch before its daily interval"
    );
}

#[test]
fn financial_audit_and_conversation_history_are_preserved() {
    let s = state(2);
    let at = now();
    let old = at - 10 * 365 * 86_400;
    let db = s.conn().unwrap();
    user(&db, at);
    db.execute("INSERT INTO orders(id,user_id,kind,amount_cents,provider,status,created_at) VALUES('financial','maintenance-user','onboarding',100,'stripe','paid',?1)", [old]).unwrap();
    db.execute("INSERT INTO payment_events(provider,event_id,order_id,created_at) VALUES('stripe','payment','financial',?1)", [old]).unwrap();
    db.execute("INSERT INTO ledger(order_id,provider,payment_id,amount_cents,currency,live_mode,created_at) VALUES('financial','stripe','payment',100,'USD',0,?1)", [old]).unwrap();
    db.execute(
        "INSERT INTO audit(actor,action,target,created_at) VALUES('admin','review','financial',?1)",
        [old],
    )
    .unwrap();
    db.execute("INSERT INTO chat_conversations(id,kind,user_id,created_at,updated_at) VALUES('conversation','platform','maintenance-user',?1,?1)", [old]).unwrap();
    db.execute("INSERT INTO chat_messages(conversation_id,sender_kind,sender_id,sender_key,sender_actor,body,client_id,created_at) VALUES('conversation','user','maintenance-user','maintenance-user','maintenance-user','Business evidence','message',?1)", [old]).unwrap();
    maintenance::prune(&db, &s.cfg, at).unwrap();
    for table in [
        "orders",
        "payment_events",
        "ledger",
        "audit",
        "chat_conversations",
        "chat_messages",
    ] {
        assert_eq!(
            count(&db, table),
            1,
            "business record in {table} must not be evicted by age"
        );
    }
}

#[test]
fn staff_invitation_cleanup_is_bounded_and_retains_members_and_recent_decisions() {
    let s = state(2);
    let at = now();
    let db = s.conn().unwrap();
    user(&db, at);
    db.execute("INSERT INTO staff_members(user_id,role,joined_at,updated_at) VALUES('maintenance-user','manager',?1,?1)", [at-100*86_400]).unwrap();
    for i in 0..5 {
        db.execute("INSERT INTO staff_invitations(id,token_hash,email,role,created_at,expires_at) VALUES(?1,?1,'staff@example.invalid','manager',?2,?3)", params![format!("old-{i}"),at-40*86_400,at-31*86_400]).unwrap();
    }
    db.execute("UPDATE staff_invitations SET accepted_at=?1,accepted_by='maintenance-user' WHERE id='old-3'", [at-31*86_400]).unwrap();
    db.execute(
        "UPDATE staff_invitations SET revoked_at=?1 WHERE id='old-4'",
        [at - 31 * 86_400],
    )
    .unwrap();
    for id in ["live", "recent-accept", "recent-revoke"] {
        db.execute("INSERT INTO staff_invitations(id,token_hash,email,role,created_at,expires_at) VALUES(?1,?1,'staff@example.invalid','manager',?2,?3)", params![id,at-40*86_400,at+60]).unwrap();
    }
    db.execute("UPDATE staff_invitations SET accepted_at=?1,accepted_by='maintenance-user' WHERE id='recent-accept'", [at-1]).unwrap();
    db.execute(
        "UPDATE staff_invitations SET revoked_at=?1 WHERE id='recent-revoke'",
        [at - 1],
    )
    .unwrap();
    maintenance::prune(&db, &s.cfg, at).unwrap();
    assert_eq!(count(&db, "staff_invitations"), 6);
    for _ in 0..2 {
        maintenance::prune(&db, &s.cfg, at).unwrap();
    }
    assert_eq!(count(&db, "staff_invitations"), 3);
    assert_eq!(count(&db, "staff_members"), 1);
    assert_eq!(
        hireferry::staff::active_role(&db, "maintenance-user")
            .unwrap()
            .as_deref(),
        Some("manager")
    );
}

#[tokio::test]
async fn checkout_expiry_is_bounded_and_late_settlement_keeps_financial_history() {
    use hireferry::{commerce, payments::PaymentReceipt};
    let s = state(2);
    let at = now();
    {
        let db = s.conn().unwrap();
        user(&db, at);
        db.execute(
            "UPDATE users SET role='employer' WHERE id='maintenance-user'",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO orders(id,user_id,kind,amount_cents,provider,provider_session,checkout_url,checkout_expires_at,created_at) VALUES('late-onboarding','maintenance-user','onboarding',100,'stripe','cs_late','https://example.invalid/checkout',?1,?1)", [at-1]).unwrap();
        db.execute("INSERT INTO promo_codes(id,code,created_by,created_at,updated_at) VALUES('promotion','LIFECYCLE','admin',?1,?1)", [at-100]).unwrap();
        for i in 0..3 {
            let id = format!("promotion-{i}");
            db.execute("INSERT INTO orders(id,user_id,kind,amount_cents,provider,created_at) VALUES(?1,'maintenance-user','publish',0,'stripe',?2)", params![id,at-100]).unwrap();
            db.execute("INSERT INTO promo_redemptions(order_id,code_id,user_id,purpose,status,request_key,request_hash,reserved_until,created_at) VALUES(?1,'promotion','maintenance-user','publish','reserved',?1,'hash',?2,?2)", params![id,at-1]).unwrap();
        }
        maintenance::prune(&db, &s.cfg, at).unwrap();
        let expired: i64 = db
            .query_row(
                "SELECT count(*) FROM orders WHERE status='expired'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            expired, 2,
            "onboarding and promotions share one orders batch"
        );
        let released: i64 = db
            .query_row(
                "SELECT count(*) FROM promo_redemptions WHERE status='released'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            released, 1,
            "expiry releases the promotional reservation via its trigger"
        );
        maintenance::prune(&db, &s.cfg, at).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM orders WHERE status='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        let old = hireferry::models::load_order(&db, "late-onboarding").unwrap();
        assert!(old.checkout_url.is_none());
        assert_eq!(old.provider_session.as_deref(), Some("cs_late"));
    }
    let receipt = || PaymentReceipt {
        provider: "stripe".into(),
        event_id: "late-payment".into(),
        order_id: "late-onboarding".into(),
        session_id: Some("cs_late".into()),
        payment_id: "pi_late".into(),
        amount_cents: 100,
        currency: "CNY".into(),
        live_mode: false,
    };
    commerce::settle_payment(&s, receipt()).await.unwrap();
    commerce::settle_payment(&s, receipt()).await.unwrap();
    let db = s.conn().unwrap();
    maintenance::prune(&db, &s.cfg, at + 366 * 86_400).unwrap();
    assert_eq!(
        hireferry::models::load_order(&db, "late-onboarding")
            .unwrap()
            .status,
        "paid"
    );
    assert!(
        hireferry::models::load_user(&db, "maintenance-user")
            .unwrap()
            .employer_paid
    );
    assert_eq!(count(&db, "orders"), 4);
    assert_eq!(count(&db, "promo_redemptions"), 3);
    assert_eq!(count(&db, "ledger"), 1);
    assert_eq!(count(&db, "payment_events"), 1);
}

#[tokio::test]
async fn one_failed_phase_does_not_prevent_other_cleanup() {
    let s = state(2);
    let at = now();
    {
        let db = s.conn().unwrap();
        session(&db, "old", at - 2 * 86_400);
        webhook_queue(&db, at, 1);
        db.execute("DROP TABLE feedback", []).unwrap();
    }
    assert!(maintenance::run_once(&s, at).await.is_err());
    let db = s.conn().unwrap();
    assert_eq!(count(&db, "sessions"), 0);
    assert_eq!(count(&db, "webhook_deliveries"), 0);
}

#[test]
fn invalid_batch_configuration_is_rejected() {
    for batch in [-1, 0, 1001] {
        let mut cfg = Config::default();
        cfg.development = true;
        cfg.maintenance.batch_size = batch;
        assert!(cfg.validate().is_err());
    }
}

struct NoTransport;
impl webhooks::Transport for NoTransport {
    fn resolve<'a>(
        &'a self,
        _: &'a url::Url,
        _: &'a hireferry::config::Webhooks,
    ) -> webhooks::TransportFuture<'a, Result<Vec<std::net::SocketAddr>, webhooks::DeliveryFailure>>
    {
        Box::pin(async { panic!("an unauthorized queue item must not reach DNS") })
    }

    fn post<'a>(
        &'a self,
        _: webhooks::OutboundRequest,
        _: &'a hireferry::config::Webhooks,
    ) -> webhooks::TransportFuture<'a, Result<u16, webhooks::DeliveryFailure>> {
        Box::pin(async { panic!("an unauthorized queue item must not be delivered") })
    }
}

#[tokio::test]
async fn queue_cancellation_is_bounded_and_never_claims_unauthorized_remainder() {
    let s = state(2);
    let at = now();
    {
        let db = s.conn().unwrap();
        webhook_queue(&db, at, 7);
        db.execute("UPDATE webhook_endpoints SET enabled=0", [])
            .unwrap();
    }
    for expected in [5, 3, 1, 0] {
        assert!(!webhooks::run_once_with(&s, &NoTransport, at).await.unwrap());
        let db = s.conn().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM webhook_deliveries WHERE status='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            expected
        );
    }
}

#[tokio::test]
async fn expired_promotion_cannot_settle_or_checkout_behind_a_cleanup_backlog() {
    use hireferry::{commerce, payments::PaymentReceipt};
    let s = state(2);
    let at = now();
    {
        let db = s.conn().unwrap();
        user(&db, at);
        db.execute(
            "UPDATE users SET role='employer',employer_paid=1 WHERE id='maintenance-user'",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO promo_codes(id,code,created_by,created_at,updated_at) VALUES('backlog','BACKLOG','admin',?1,?1)", [at]).unwrap();
        // More than the opportunistic batch size. The target's deadline sorts
        // last, so global cleanup alone cannot authorize it safely.
        for i in 0..202 {
            let id = format!("backlog-{i:03}");
            db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,created_at,updated_at) VALUES(?1,'maintenance-user','Job','Description',100,?2,?2)", params![id,at]).unwrap();
            db.execute("INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,checkout_url,created_at) VALUES(?1,'maintenance-user','publish',?1,100,'stripe','https://example.invalid/old-promotion',?2)", params![id,at]).unwrap();
            db.execute("INSERT INTO promo_redemptions(order_id,code_id,user_id,purpose,status,request_key,request_hash,reserved_until,created_at) VALUES(?1,'backlog','maintenance-user','publish','reserved',?1,'hash',?2,?3)", params![id,at-203+i,at]).unwrap();
        }
    }
    let error = commerce::checkout(&s, "maintenance-user", "backlog-201")
        .await
        .unwrap_err();
    assert_eq!(error.status, axum::http::StatusCode::CONFLICT);
    // A different expired order still lies behind 200 earlier reservations.
    let receipt = || PaymentReceipt {
        provider: "stripe".into(),
        event_id: "late-promotion".into(),
        order_id: "backlog-200".into(),
        session_id: None,
        payment_id: "pi_late_promotion".into(),
        amount_cents: 100,
        currency: "CNY".into(),
        live_mode: false,
    };
    commerce::settle_payment(&s, receipt()).await.unwrap();
    commerce::settle_payment(&s, receipt()).await.unwrap();
    let db = s.conn().unwrap();
    assert_eq!(
        hireferry::models::load_order(&db, "backlog-200")
            .unwrap()
            .status,
        "paid_needs_review"
    );
    assert_eq!(
        hireferry::models::load_job(&db, "backlog-200")
            .unwrap()
            .status,
        "draft"
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM promo_redemptions WHERE order_id='backlog-200'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "released"
    );
    assert_eq!(count(&db, "orders"), 202);
    assert_eq!(count(&db, "payment_events"), 1);
    assert_eq!(count(&db, "ledger"), 1);
}
