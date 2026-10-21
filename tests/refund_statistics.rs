//! Cash-flow reporting uses the refund date, preserves negative days, and keeps
//! test money separate from live money. All rows here are synthetic fixtures.
use hireferry::{AppState, access::Principal, admin, config::Config};
use rusqlite::params;

#[test]
fn refunds_reduce_net_revenue_on_their_own_day_and_only_in_their_payment_mode() {
    let state = AppState::new(Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    })
    .unwrap();
    let paid_at = (hireferry::now() / 86400 - 2) * 86400;
    let refund_at = paid_at + 86400;
    {
        let db = state.conn().unwrap();
        db.execute("INSERT INTO users(id,email,role,created_at,updated_at) VALUES('report-user','report@example.invalid','employer',?1,?1)", [paid_at]).unwrap();
        for (id, amount, live) in [("test-order", 1000, false), ("live-order", 2000, true)] {
            db.execute("INSERT INTO orders(id,user_id,kind,amount_cents,provider,live_mode,status,created_at,paid_at) VALUES(?1,'report-user','onboarding',?2,'waffo',?3,'paid',?4,?4)",params![id,amount,live,paid_at]).unwrap();
            db.execute("INSERT INTO ledger(order_id,provider,payment_id,amount_cents,currency,live_mode,created_at) VALUES(?1,'waffo',?1,?2,'CNY',?3,?4)",params![id,amount,live,paid_at]).unwrap();
        }
        for (id, status, at) in [
            ("refund-success", "succeeded", Some(refund_at)),
            ("refund-failed", "failed", None),
        ] {
            db.execute("INSERT INTO payment_refunds(provider,refund_id,order_id,payment_id,provider_order_id,amount_cents,currency,live_mode,status,reason,created_at,updated_at,succeeded_at) VALUES('waffo',?1,'test-order','test-order','ORD_test',500,'CNY',0,?2,'',?3,?3,?4)",params![id,status,refund_at,at]).unwrap();
        }
    }
    let platform = Principal {
        user_id: None,
        admin: true,
        via_ai: false,
        actor: "admin-test".into(),
    };
    let all = admin::overview_data(
        &state,
        &platform,
        &admin::QueryInput {
            mode: Some("test".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(all["stats"]["gross_cents"], 1000);
    assert_eq!(all["stats"]["refund_cents"], 500);
    assert_eq!(all["stats"]["revenue_cents"], 500);
    assert_eq!(all["stats"]["paid_orders"], 1);
    assert_eq!(all["revenue_by_kind"][0]["amount_cents"], 500);
    let date = chrono::DateTime::from_timestamp(refund_at, 0)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string();
    let refund_day = admin::overview_data(
        &state,
        &platform,
        &admin::QueryInput {
            from: Some(date.clone()),
            to: Some(date),
            mode: Some("test".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(refund_day["stats"]["gross_cents"], 0);
    assert_eq!(refund_day["stats"]["refund_cents"], 500);
    assert_eq!(refund_day["stats"]["revenue_cents"], -500);
    assert_eq!(refund_day["stats"]["paid_orders"], 0);
    assert_eq!(refund_day["series"][0]["revenue_cents"], -500);
    let live = admin::overview_data(
        &state,
        &platform,
        &admin::QueryInput {
            mode: Some("live".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(live["stats"]["revenue_cents"], 2000);
    assert_eq!(live["stats"]["refund_cents"], 0);
}

#[test]
fn different_currencies_are_never_added_in_overview_series_or_kind_totals() {
    let state = AppState::new(Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    })
    .unwrap();
    let at = hireferry::now() - 10;
    {
        let db = state.conn().unwrap();
        db.execute("INSERT INTO users(id,email,role,created_at,updated_at) VALUES('owner','money@example.test','employer',?1,?1)", [at]).unwrap();
        for (currency, amount) in [("CNY", 1000), ("USD", 2000), ("JPY", 3000)] {
            db.execute("INSERT INTO orders(id,user_id,kind,amount_cents,currency,provider,live_mode,status,created_at,paid_at) VALUES(?1,'owner','onboarding',?2,?1,'waffo',0,'paid',?3,?3)",params![currency,amount,at]).unwrap();
            db.execute("INSERT INTO ledger(order_id,provider,payment_id,amount_cents,currency,live_mode,created_at) VALUES(?1,'waffo',?1,?2,?1,0,?3)",params![currency,amount,at]).unwrap();
        }
        db.execute("INSERT INTO payment_refunds(provider,refund_id,order_id,payment_id,provider_order_id,amount_cents,currency,live_mode,status,reason,created_at,updated_at,succeeded_at) VALUES('waffo','usd-refund','USD','USD','ORD_usd',500,'USD',0,'succeeded','',?1,?1,?1)", [at]).unwrap();
    }
    let platform = Principal {
        user_id: None,
        admin: true,
        via_ai: false,
        actor: "admin".into(),
    };
    let report = |currency: Option<&str>| {
        admin::overview_data(
            &state,
            &platform,
            &admin::QueryInput {
                currency: currency.map(str::to_owned),
                ..Default::default()
            },
        )
    };
    let default = report(None).unwrap();
    assert_eq!(default["currency"], "CNY");
    assert_eq!(default["stats"]["revenue_cents"], 1000);
    let usd = report(Some("USD")).unwrap();
    assert_eq!(usd["stats"]["revenue_cents"], 1500);
    assert_eq!(usd["stats"]["gross_cents"], 2000);
    assert_eq!(usd["stats"]["refund_cents"], 500);
    assert_eq!(usd["series"][0]["revenue_cents"], 1500);
    assert_eq!(usd["revenue_by_kind"][0]["amount_cents"], 1500);
    assert_eq!(usd["currency_totals"].as_array().unwrap().len(), 3);
    let yen = usd["currency_totals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["currency"] == "JPY")
        .unwrap();
    assert_eq!(yen["currency_decimals"], 0);
    assert_eq!(yen["revenue_cents"], 3000);
    assert!(report(Some("unknown")).is_err());
}
