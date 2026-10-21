//! Outbound webhooks exercise the real HTTP and transactional outbox. Transport
//! is injected: these tests do not resolve public DNS or send network requests.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{
    AppState,
    access::Principal,
    auth, chat,
    config::{Config, Webhooks},
    webhooks::{self, DeliveryFailure, OutboundRequest, Transport, TransportFuture},
};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{
        Mutex,
        atomic::{AtomicU16, Ordering},
    },
};
use tempfile::TempDir;
use tower::ServiceExt;

struct Fixture {
    _dir: TempDir,
    state: AppState,
}
struct Member {
    id: String,
    cookie: String,
    csrf: String,
}
impl Fixture {
    fn new() -> Self {
        Self::with_limits(Webhooks::default())
    }
    fn with_limits(webhooks: Webhooks) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(Config {
            development: true,
            database: dir
                .path()
                .join("webhooks.sqlite3")
                .to_string_lossy()
                .into_owned(),
            webhooks,
            ..Default::default()
        })
        .unwrap();
        Self { _dir: dir, state }
    }
    fn user(&self, role: &str) -> Member {
        let id = uuid::Uuid::new_v4().to_string();
        let raw = uuid::Uuid::new_v4().to_string();
        let csrf = uuid::Uuid::new_v4().to_string();
        let db = self.state.conn().unwrap();
        db.execute("INSERT INTO users(id,email,name,role,employer_paid,resume_md,created_at,updated_at) VALUES(?1,?2,'Private person',?3,?4,'RESUME_PRIVATE_SENTINEL',?5,?5)",params![id,format!("private-{id}@example.invalid"),role,role=="employer",hireferry::now()]).unwrap();
        db.execute("INSERT INTO sessions(token_hash,user_id,admin,bootstrap,csrf,expires_at,recent_auth_at) VALUES(?1,?2,0,0,?3,?4,?5)",params![auth::hash_token(&raw),id,csrf,hireferry::now()+86400,hireferry::now()]).unwrap();
        Member {
            id,
            cookie: format!("hf_session={raw}"),
            csrf,
        }
    }
    fn matched(&self, seeker: &Member, employer: &Member) -> (String, String) {
        let job = uuid::Uuid::new_v4().to_string();
        let matched = uuid::Uuid::new_v4().to_string();
        let db = self.state.conn().unwrap();
        db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,status,created_at,updated_at) VALUES(?1,?2,'Private job','JD_PRIVATE_SENTINEL',300000,'published',?3,?3)",params![job,employer.id,hireferry::now()]).unwrap();
        db.execute("INSERT INTO matches(id,seeker_id,job_id,status,created_at,updated_at) VALUES(?1,?2,?3,'introduced',?4,?4)",params![matched,seeker.id,job,hireferry::now()]).unwrap();
        (matched, job)
    }
}
fn principal(u: &Member) -> Principal {
    Principal {
        user_id: Some(u.id.clone()),
        admin: false,
        via_ai: false,
        actor: u.id.clone(),
    }
}
fn message(f: &Fixture, u: &Member, thread: &str, body: &str) -> Value {
    chat::send_message(
        &f.state,
        &principal(u),
        thread,
        chat::MessageInput {
            body: body.into(),
            client_id: uuid::Uuid::new_v4().to_string(),
        },
    )
    .unwrap()
}
async fn request(
    f: &Fixture,
    u: &Member,
    method: &str,
    path: &str,
    payload: Value,
    csrf: bool,
    extra: Option<(&str, &str)>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("origin", "http://localhost:3000")
        .header("cookie", &u.cookie);
    if csrf {
        builder = builder.header("x-csrf-token", &u.csrf);
    }
    if let Some((k, v)) = extra {
        builder = builder.header(k, v);
    }
    let response = hireferry::app(f.state.clone())
        .oneshot(builder.body(Body::from(payload.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn endpoint(f: &Fixture, u: &Member, events: &[&str]) -> (String, String) {
    let (status, value) = request(
        f,
        u,
        "POST",
        "/api/webhooks",
        json!({"name":"My endpoint","url":"https://hooks.example.com/hireferry","events":events}),
        true,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    (
        value["endpoint"]["id"].as_str().unwrap().to_owned(),
        value["signing_secret"].as_str().unwrap().to_owned(),
    )
}
fn rows(f: &Fixture, id: &str) -> Vec<Value> {
    let db = f.state.conn().unwrap();
    db.prepare("SELECT d.id,d.status,d.attempts,d.next_attempt_at,d.event_id,w.event_type,w.body_json FROM webhook_deliveries d JOIN webhook_events w ON w.id=d.event_id WHERE d.endpoint_id=?1 ORDER BY d.created_at,d.id").unwrap().query_map([id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"status":r.get::<_,String>(1)?,"attempts":r.get::<_,i64>(2)?,"next_attempt_at":r.get::<_,i64>(3)?,"event_id":r.get::<_,String>(4)?,"type":r.get::<_,String>(5)?,"body":serde_json::from_str::<Value>(&r.get::<_,String>(6)?).unwrap()}))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}
type ResolveHook = Box<dyn FnOnce() + Send>;
struct FakeTransport {
    addresses: Vec<SocketAddr>,
    status: AtomicU16,
    requests: Mutex<Vec<OutboundRequest>>,
    on_resolve: Mutex<Option<ResolveHook>>,
    on_post: Mutex<Option<ResolveHook>>,
}
impl FakeTransport {
    fn new(status: u16) -> Self {
        Self {
            addresses: vec!["8.8.8.8:443".parse().unwrap()],
            status: AtomicU16::new(status),
            requests: Mutex::new(Vec::new()),
            on_resolve: Mutex::new(None),
            on_post: Mutex::new(None),
        }
    }
}
impl Transport for FakeTransport {
    fn resolve<'a>(
        &'a self,
        _: &'a url::Url,
        _: &'a Webhooks,
    ) -> TransportFuture<'a, Result<Vec<SocketAddr>, DeliveryFailure>> {
        if let Some(hook) = self.on_resolve.lock().unwrap().take() {
            hook();
        }
        let addresses = self.addresses.clone();
        Box::pin(async move { Ok(addresses) })
    }
    fn post<'a>(
        &'a self,
        request: OutboundRequest,
        _: &'a Webhooks,
    ) -> TransportFuture<'a, Result<u16, DeliveryFailure>> {
        self.requests.lock().unwrap().push(request);
        if let Some(hook) = self.on_post.lock().unwrap().take() {
            hook();
        }
        let status = self.status.load(Ordering::SeqCst);
        Box::pin(async move { Ok(status) })
    }
}

#[tokio::test]
async fn management_requires_member_cookie_csrf_and_returns_secret_only_once() {
    let f = Fixture::new();
    let user = f.user("seeker");
    let stranger = f.user("employer");
    let payload = json!({"name":"My hook","url":"https://hooks.example.com/hireferry","events":["message.created"]});
    assert_eq!(
        request(
            &f,
            &user,
            "POST",
            "/api/webhooks",
            payload.clone(),
            false,
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f,
            &user,
            "POST",
            "/api/webhooks",
            payload.clone(),
            true,
            Some(("authorization", "Bearer fake"))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f,
            &user,
            "POST",
            "/api/webhooks",
            payload.clone(),
            true,
            Some(("x-hireferry-ai", "webmcp"))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (id, secret) = endpoint(&f, &user, &["message.created"]).await;
    assert!(secret.starts_with("hf_wh_") && secret.len() > 60);
    let list = request(&f, &user, "GET", "/api/webhooks", json!({}), false, None).await;
    assert_eq!(list.0, StatusCode::OK);
    assert_eq!(list.1["items"].as_array().unwrap().len(), 1);
    assert!(!list.1.to_string().contains(&secret));
    assert!(!list.1.to_string().contains("signing_secret"));
    let path = format!("/api/webhooks/{id}");
    let changed = request(
        &f,
        &user,
        "PATCH",
        &path,
        json!({"name":"Renamed","enabled":false}),
        true,
        None,
    )
    .await;
    assert_eq!(changed.0, StatusCode::OK);
    assert_eq!(changed.1["endpoint"]["name"], "Renamed");
    assert!(!changed.1.to_string().contains(&secret));
    assert!(
        request(
            &f,
            &user,
            "PATCH",
            &path,
            json!({"url":"https://other.example.com"}),
            true,
            None
        )
        .await
        .0
        .is_client_error()
    );
    assert_eq!(
        request(&f, &stranger, "DELETE", &path, json!({}), true, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &f,
            &stranger,
            "GET",
            &format!("{path}/deliveries"),
            json!({}),
            false,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    for _ in 0..4 {
        endpoint(&f, &user, &["message.created"]).await;
    }
    assert_eq!(
        request(&f, &user, "POST", "/api/webhooks", payload, true, None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&f, &user, "DELETE", &path, json!({}), true, None)
            .await
            .0,
        StatusCode::OK
    );
    let stored: String = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT signing_secret FROM webhook_endpoints WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(stored.is_empty());
    let audit: String = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT group_concat(detail,' ') FROM audit WHERE action LIKE 'webhook_%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!audit.contains(&secret));
}

#[test]
fn destination_policy_rejects_private_reserved_and_ambiguous_addresses_without_dev_bypass() {
    for value in [
        "http://example.com/hook",
        "https://example.com:444/hook",
        "https://user:pass@example.com/hook",
        "https://example.com/#fragment",
        "https://localhost/hook",
        "https://app.local/hook",
        "https://127.0.0.1/hook",
        "https://2130706433/hook",
        "https://0x7f000001/hook",
        "https://10.0.0.1/hook",
        "https://169.254.169.254/latest/meta-data",
        "https://168.63.129.16/hook",
        "https://100.100.100.200/hook",
        "https://192.0.2.1/hook",
        "https://198.18.0.1/hook",
        "https://[::1]/hook",
        "https://[::ffff:127.0.0.1]/hook",
        "https://[fe80::1]/hook",
        "https://[fd00::1]/hook",
        "https://[2001:db8::1]/hook",
        "https://[2002:7f00:1::]/hook",
    ] {
        assert!(
            webhooks::validate_destination(value).is_err(),
            "Accepted {value}"
        );
    }
    assert!(
        webhooks::validate_destination("https://hooks.example.com:443/callback?version=1").is_ok()
    );
    assert!(webhooks::validate_destination("https://[2606:4700:4700::1111]/hook").is_ok());
    assert!(
        webhooks::validate_addresses(&[
            "8.8.8.8:443".parse().unwrap(),
            "127.0.0.1:443".parse().unwrap()
        ])
        .is_err()
    );
    assert!(webhooks::validate_addresses(&["8.8.8.8:80".parse().unwrap()]).is_err());
    assert!(webhooks::validate_addresses(&[]).is_err());
}

#[tokio::test]
async fn test_delivery_is_nonsensitive_signed_and_keeps_stable_ids_across_retries() {
    let f = Fixture::new();
    let user = f.user("seeker");
    let (id, secret) = endpoint(&f, &user, &["message.created"]).await;
    let (status, queued) = request(
        &f,
        &user,
        "POST",
        &format!("/api/webhooks/{id}/test"),
        json!({}),
        true,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(queued["delivery"]["event_type"], "webhook.test");
    assert_eq!(
        request(
            &f,
            &user,
            "POST",
            &format!("/api/webhooks/{id}/test"),
            json!({}),
            true,
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let at = hireferry::now();
    let transport = FakeTransport::new(500);
    assert!(
        webhooks::run_once_with(&f.state, &transport, at)
            .await
            .unwrap()
    );
    let pending = rows(&f, &id);
    assert_eq!(pending[0]["status"], "pending");
    assert_eq!(pending[0]["attempts"], 1);
    assert!(
        !webhooks::run_once_with(&f.state, &transport, at + 1)
            .await
            .unwrap()
    );
    transport.status.store(204, Ordering::SeqCst);
    assert!(
        webhooks::run_once_with(&f.state, &transport, at + 30)
            .await
            .unwrap()
    );
    let captured = transport.requests.lock().unwrap();
    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0].delivery_id, captured[1].delivery_id);
    assert_eq!(captured[0].event_id, captured[1].event_id);
    assert_eq!(captured[0].body, captured[1].body);
    assert_eq!(captured[0].delivery_id, queued["delivery"]["id"]);
    let body: Value = serde_json::from_slice(&captured[0].body).unwrap();
    assert_eq!(body["type"], "webhook.test");
    assert_eq!(body["data"], json!({"test":true}));
    assert!(webhooks::verify_signature(
        &secret,
        captured[0].timestamp,
        &captured[0].body,
        &captured[0].signature,
        at
    ));
    assert!(!webhooks::verify_signature(
        &secret,
        captured[0].timestamp,
        b"tampered",
        &captured[0].signature,
        at
    ));
    assert!(!webhooks::verify_signature(
        &secret,
        captured[0].timestamp,
        &captured[0].body,
        &captured[0].signature,
        at + 301
    ));
    assert_eq!(rows(&f, &id)[0]["status"], "delivered");
}

#[tokio::test]
async fn outbox_is_atomic_and_does_not_merge_private_platform_and_matched_histories() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let employer = f.user("employer");
    let outsider = f.user("seeker");
    let (matched, _) = f.matched(&seeker, &employer);
    let (s, _) = endpoint(&f, &seeker, &["message.created"]).await;
    let (e, _) = endpoint(&f, &employer, &["message.created"]).await;
    let (o, _) = endpoint(&f, &outsider, &["message.created"]).await;
    let private = format!("platform_{}", seeker.id);
    let shared = format!("match_{matched}");
    message(&f, &seeker, &private, "PRIVATE_PLATFORM_MESSAGE_SENTINEL");
    assert_eq!(rows(&f, &s).len(), 1);
    assert!(rows(&f, &e).is_empty());
    assert!(rows(&f, &o).is_empty());
    message(&f, &seeker, &shared, "SHARED_MESSAGE_SENTINEL");
    assert_eq!(rows(&f, &s).len(), 2);
    assert_eq!(rows(&f, &e).len(), 1);
    assert!(rows(&f, &o).is_empty());
    {
        let mut db = f.state.conn().unwrap();
        let tx = db.transaction().unwrap();
        tx.execute("INSERT INTO chat_messages(conversation_id,sender_kind,sender_id,sender_key,sender_actor,body,client_id,via_ai,created_at) VALUES(?1,'user',?2,?3,?2,'ROLLED_BACK_MESSAGE','rolled-back',0,?4)",params![private,seeker.id,format!("user_{}",seeker.id),hireferry::now()]).unwrap();
        tx.rollback().unwrap();
    }
    assert_eq!(rows(&f, &s).len(), 2);
    let bodies = rows(&f, &s)
        .iter()
        .map(|v| v["body"].to_string())
        .collect::<String>();
    for secret in [
        "PRIVATE_PLATFORM_MESSAGE_SENTINEL",
        "SHARED_MESSAGE_SENTINEL",
        "RESUME_PRIVATE_SENTINEL",
        "example.invalid",
        "ROLLED_BACK_MESSAGE",
    ] {
        assert!(!bodies.contains(secret));
    }
    let transport = FakeTransport::new(204);
    for _ in 0..3 {
        assert!(
            webhooks::run_once_with(&f.state, &transport, hireferry::now())
                .await
                .unwrap()
        );
    }
    assert!(
        !webhooks::run_once_with(&f.state, &transport, hireferry::now())
            .await
            .unwrap()
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn authorization_is_rechecked_after_dns_and_bans_disable_queued_endpoints() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let employer = f.user("employer");
    let (matched, _) = f.matched(&seeker, &employer);
    let (id, _) = endpoint(&f, &seeker, &["message.created"]).await;
    message(
        &f,
        &seeker,
        &format!("match_{matched}"),
        "Already queued while authorized",
    );
    let transport = FakeTransport::new(204);
    let state = f.state.clone();
    *transport.on_resolve.lock().unwrap() = Some(Box::new(move || {
        state
            .conn()
            .unwrap()
            .execute(
                "UPDATE matches SET status='declined' WHERE id=?1",
                [matched],
            )
            .unwrap();
    }));
    assert!(
        webhooks::run_once_with(&f.state, &transport, hireferry::now())
            .await
            .unwrap()
    );
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(rows(&f, &id)[0]["status"], "cancelled");
    message(
        &f,
        &seeker,
        &format!("platform_{}", seeker.id),
        "Ban before dispatch",
    );
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id=?1", [&seeker.id])
        .unwrap();
    assert!(
        !webhooks::run_once_with(&f.state, &transport, hireferry::now())
            .await
            .unwrap()
    );
    assert!(rows(&f, &id).iter().all(|v| v["status"] == "cancelled"));
    let enabled: bool = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT enabled FROM webhook_endpoints WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!enabled);
}

#[tokio::test]
async fn a_response_arriving_after_suspension_cannot_resurrect_an_inflight_delivery() {
    let f = Fixture::new();
    let user = f.user("seeker");
    let (id, _) = endpoint(&f, &user, &["message.created"]).await;
    message(
        &f,
        &user,
        &format!("platform_{}", user.id),
        "Queued before suspension",
    );
    let transport = FakeTransport::new(503);
    let state = f.state.clone();
    let uid = user.id.clone();
    *transport.on_post.lock().unwrap() = Some(Box::new(move || {
        let db = state.conn().unwrap();
        db.execute("UPDATE users SET banned=1 WHERE id=?1", [&uid])
            .unwrap();
        db.execute("UPDATE users SET banned=0 WHERE id=?1", [&uid])
            .unwrap();
    }));
    assert!(
        webhooks::run_once_with(&f.state, &transport, hireferry::now())
            .await
            .unwrap()
    );
    // Already started network I/O cannot be recalled, but its eventual failure
    // cannot write a retry over the cancelled claim, even after account restore.
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    assert_eq!(rows(&f, &id)[0]["status"], "cancelled");
    assert!(
        !webhooks::run_once_with(&f.state, &transport, hireferry::now() + 3600)
            .await
            .unwrap()
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn mixed_public_private_dns_answers_are_never_posted_or_retried_automatically() {
    let f = Fixture::new();
    let user = f.user("seeker");
    let (id, _) = endpoint(&f, &user, &["message.created"]).await;
    request(
        &f,
        &user,
        "POST",
        &format!("/api/webhooks/{id}/test"),
        json!({}),
        true,
        None,
    )
    .await;
    let mut transport = FakeTransport::new(204);
    transport.addresses.push("10.0.0.1:443".parse().unwrap());
    assert!(
        webhooks::run_once_with(&f.state, &transport, hireferry::now())
            .await
            .unwrap()
    );
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(rows(&f, &id)[0]["status"], "failed");
    let error: String = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT last_error FROM webhook_deliveries WHERE endpoint_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(error, "destination_blocked");
}

#[tokio::test]
async fn retries_are_bounded_manual_retry_preserves_delivery_id_and_restarts_recover_leases() {
    let f = Fixture::new();
    let user = f.user("seeker");
    let (id, _) = endpoint(&f, &user, &["message.created"]).await;
    request(
        &f,
        &user,
        "POST",
        &format!("/api/webhooks/{id}/test"),
        json!({}),
        true,
        None,
    )
    .await;
    let transport = FakeTransport::new(503);
    let mut at = hireferry::now();
    for attempt in 1..=5 {
        assert!(
            webhooks::run_once_with(&f.state, &transport, at)
                .await
                .unwrap()
        );
        let row = &rows(&f, &id)[0];
        assert_eq!(row["attempts"], attempt);
        at = row["next_attempt_at"].as_i64().unwrap();
    }
    assert_eq!(rows(&f, &id)[0]["status"], "failed");
    assert!(
        !webhooks::run_once_with(&f.state, &transport, at + 1000)
            .await
            .unwrap()
    );
    let delivery = rows(&f, &id)[0]["id"].as_str().unwrap().to_owned();
    let retry = request(
        &f,
        &user,
        "POST",
        &format!("/api/webhooks/{id}/deliveries/{delivery}/retry"),
        json!({}),
        true,
        None,
    )
    .await;
    assert_eq!(retry.0, StatusCode::OK);
    assert_eq!(retry.1["delivery"]["id"], delivery);
    f.state.conn().unwrap().execute("UPDATE webhook_deliveries SET status='processing',attempts=1,lease_until=?2,claim_token='crashed-worker' WHERE id=?1",params![delivery,hireferry::now()-1]).unwrap();
    let reopened = AppState::new((*f.state.cfg).clone()).unwrap();
    transport.status.store(204, Ordering::SeqCst);
    assert!(
        webhooks::run_once_with(&reopened, &transport, hireferry::now())
            .await
            .unwrap()
    );
    assert_eq!(rows(&f, &id)[0]["status"], "delivered");
    assert_eq!(rows(&f, &id)[0]["attempts"], 2);
    assert_eq!(rows(&f, &id)[0]["id"], delivery);
}

#[tokio::test]
async fn queue_caps_retention_and_system_disable_bound_delivery_storage() {
    let f = Fixture::with_limits(Webhooks {
        max_pending_per_endpoint: 2,
        ..Default::default()
    });
    let user = f.user("seeker");
    let (id, _) = endpoint(&f, &user, &["message.created"]).await;
    for _ in 0..3 {
        message(
            &f,
            &user,
            &format!("platform_{}", user.id),
            "Small event only",
        );
    }
    assert_eq!(rows(&f, &id).len(), 2);
    let list = request(&f, &user, "GET", "/api/webhooks", json!({}), false, None).await;
    assert_eq!(list.1["items"][0]["dropped_events"], 1);
    let transport = FakeTransport::new(204);
    for _ in 0..2 {
        webhooks::run_once_with(&f.state, &transport, hireferry::now())
            .await
            .unwrap();
    }
    {
        let db = f.state.conn().unwrap();
        db.execute(
            "UPDATE webhook_deliveries SET created_at=?1",
            [hireferry::now() - 31 * 86400],
        )
        .unwrap();
        webhooks::cleanup(&db, &f.state.cfg.webhooks, hireferry::now()).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM webhook_events", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let mut cfg = (*f.state.cfg).clone();
    cfg.webhooks.enabled = false;
    let disabled = AppState::new(cfg).unwrap();
    message(
        &f,
        &user,
        &format!("platform_{}", user.id),
        "Not queued while globally disabled",
    );
    assert!(rows(&f, &id).is_empty());
    assert!(
        !webhooks::run_once_with(&disabled, &transport, hireferry::now())
            .await
            .unwrap()
    );
    let disabled_fixture = Fixture {
        _dir: tempfile::tempdir().unwrap(),
        state: disabled,
    };
    assert_eq!(
        request(
            &disabled_fixture,
            &user,
            "POST",
            &format!("/api/webhooks/{id}/test"),
            json!({}),
            true,
            None
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        request(
            &disabled_fixture,
            &user,
            "DELETE",
            &format!("/api/webhooks/{id}"),
            json!({}),
            true,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn business_events_include_only_ids_and_statuses_for_current_resource_owners() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let employer = f.user("employer");
    let outsider = f.user("seeker");
    let (matched, job) = f.matched(&seeker, &employer);
    let (s, _) = endpoint(&f, &seeker, &webhooks::EVENTS).await;
    let (e, _) = endpoint(&f, &employer, &webhooks::EVENTS).await;
    let (o, _) = endpoint(&f, &outsider, &webhooks::EVENTS).await;
    let admin = Principal {
        user_id: None,
        admin: true,
        via_ai: false,
        actor: "admin".into(),
    };
    chat::save_mandate(
        &f.state,
        &principal(&seeker),
        None,
        chat::MandateInput {
            target_industry: "PRIVATE_INDUSTRY".into(),
            target_roles: "PRIVATE_ROLE".into(),
            salary_min_cents: None,
            salary_max_cents: None,
            salary_currency: None,
            location: "PRIVATE_LOCATION".into(),
            preferences: "PRIVATE_PREFERENCES".into(),
            status: "active".into(),
            allow_external: true,
        },
    )
    .unwrap();
    chat::save_outreach(
        &f.state,
        &admin,
        &seeker.id,
        None,
        chat::OutreachInput {
            channel: "PRIVATE_CHANNEL".into(),
            opportunity: "PRIVATE_OPPORTUNITY".into(),
            source_url: "https://example.com/private".into(),
            stage: "identified".into(),
            note: "PRIVATE_NOTE".into(),
        },
    )
    .unwrap();
    {
        let db = f.state.conn().unwrap();
        db.execute(
            "UPDATE jobs SET status='closed',jd_md='SECRET_JOB_BODY',version=version+1 WHERE id=?1",
            [&job],
        )
        .unwrap();
        db.execute(
            "UPDATE matches SET status='interview',note='SECRET_INTRODUCTION' WHERE id=?1",
            [&matched],
        )
        .unwrap();
        db.execute("INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,created_at) VALUES('order-webhook',?1,'publish',?2,30000,'stripe',?3)",params![employer.id,job,hireferry::now()]).unwrap();
        db.execute("UPDATE orders SET status='partially_refunded',refunded_cents=100 WHERE id='order-webhook'",[]).unwrap();
    }
    message(&f, &seeker, &format!("match_{matched}"), "SECRET_MESSAGE");
    let seekers = rows(&f, &s);
    let employers = rows(&f, &e);
    assert!(rows(&f, &o).is_empty());
    for kind in [
        "mandate.updated",
        "outreach.updated",
        "job.updated",
        "match.updated",
        "message.created",
    ] {
        assert!(seekers.iter().any(|v| v["type"] == kind), "Missing {kind}");
    }
    assert!(!seekers.iter().any(|v| v["type"] == "order.updated"));
    assert!(employers.iter().any(|v| v["type"] == "order.updated"));
    assert!(
        !employers
            .iter()
            .any(|v| v["type"] == "mandate.updated" || v["type"] == "outreach.updated")
    );
    for row in seekers.iter().chain(&employers) {
        let body = &row["body"];
        for key in body["data"].as_object().unwrap().keys() {
            assert!(["id", "status", "conversation_id"].contains(&key.as_str()));
        }
        assert!(!body.to_string().contains("PRIVATE_"));
        assert!(!body.to_string().contains("SECRET_"));
        assert!(!body.to_string().contains("example.invalid"));
    }
}
