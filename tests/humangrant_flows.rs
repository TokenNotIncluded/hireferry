//! HumanGrant tests use the real email challenge, browser session, SQLite, and MCP handlers.
//! Signing keys and email addresses are disposable test fixtures. No external mail is sent.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode, header},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ed25519_dalek::{Signer, SigningKey};
use hireferry::{AppState, config::Config, now};
use http_body_util::BodyExt;
use rand::{RngCore, rngs::OsRng};
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tower::ServiceExt;

const ORIGIN: &str = "http://localhost:3000";
const COMPONENTS: &str = "(\"@method\" \"@target-uri\" \"content-digest\" \"content-type\" \"x-hg-key\" \"authorization\")";

struct Observation {
    status: StatusCode,
    headers: HeaderMap,
    value: Value,
    text: String,
}
impl Observation {
    fn ok(&self) {
        assert!(self.status.is_success(), "{}: {}", self.status, self.text);
    }
    fn denied(&self) {
        assert!(
            self.status.is_client_error(),
            "{}: {}",
            self.status,
            self.text
        );
        assert!(self.value.get("access_token").is_none());
    }
    fn tool_denied(&self) {
        if self.status.is_success() {
            assert!(
                self.value["result"]["isError"] == true || self.value.get("error").is_some(),
                "{}",
                self.text
            );
        } else {
            self.denied();
        }
    }
}
async fn observe(app: &Router, request: Request<Body>) -> Observation {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Observation {
        status,
        headers,
        value: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        text: String::from_utf8(bytes.to_vec()).unwrap(),
    }
}
fn state() -> AppState {
    AppState::new(Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    })
    .unwrap()
}
fn permission(action: &str, resource: &str) -> Value {
    json!({"action": action, "resource": resource})
}

struct Browser {
    app: Router,
    cookies: BTreeMap<String, String>,
    csrf: String,
}
impl Browser {
    fn new(app: &Router) -> Self {
        Self {
            app: app.clone(),
            cookies: BTreeMap::new(),
            csrf: String::new(),
        }
    }
    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
    async fn request(
        &mut self,
        method: &str,
        path: &str,
        body: Value,
        hg_csrf: Option<&str>,
    ) -> Observation {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, self.cookie_header());
        if !self.csrf.is_empty() {
            request = request.header("x-csrf-token", &self.csrf);
        }
        if let Some(csrf) = hg_csrf {
            request = request.header("x-hg-csrf", csrf);
        }
        let body = if method == "GET" {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
        let result = observe(&self.app, request.body(body).unwrap()).await;
        for value in result.headers.get_all(header::SET_COOKIE).iter() {
            let (key, value) = value
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .split_once('=')
                .unwrap();
            if value.is_empty() {
                self.cookies.remove(key);
            } else {
                self.cookies.insert(key.into(), value.into());
            }
        }
        if matches!(path, "/api/auth/email/verify" | "/api/session") {
            if let Some(csrf) = result.value["csrf"].as_str() {
                self.csrf = csrf.into();
            }
        }
        result
    }
    async fn sign_in(&mut self, email: &str) -> String {
        let start = self
            .request(
                "POST",
                "/api/auth/email/start",
                json!({"email": email}),
                None,
            )
            .await;
        start.ok();
        let result = self
            .request(
                "POST",
                "/api/auth/email/verify",
                json!({"email": email, "code": start.value["dev_code"]}),
                None,
            )
            .await;
        result.ok();
        result.value["user"]["id"].as_str().unwrap().into()
    }
    async fn seeker(&mut self, email: &str) -> String {
        let id = self.sign_in(email).await;
        self.request("POST", "/api/role", json!({"role": "seeker"}), None)
            .await
            .ok();
        id
    }
    async fn review(&mut self, request: &Pending) -> Value {
        let result = self
            .request("GET", &request.review_path(), Value::Null, None)
            .await;
        result.ok();
        result.value
    }
    async fn approve(&mut self, request: &Pending, permissions: Value) -> Observation {
        let review = self.review(request).await;
        self.request(
            "POST",
            &format!("{}/approve", request.review_path()),
            json!({"permissions": permissions, "expires_at": review["requested_grant_expires_at"]}),
            review["csrf"].as_str(),
        )
        .await
    }
}

struct Agent {
    key: SigningKey,
    origin: String,
}
struct Pending {
    id: String,
    link: String,
}
impl Pending {
    fn review_path(&self) -> String {
        format!("/api/humangrant/review/{}", self.link)
    }
    fn claim_path(&self) -> String {
        format!("/v1/requests/{}/claim", self.id)
    }
}
impl Agent {
    fn new() -> Self {
        Self::for_origin(ORIGIN)
    }
    fn for_origin(origin: &str) -> Self {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        Self {
            key: SigningKey::from_bytes(&seed),
            origin: origin.into(),
        }
    }
    fn headers_at(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        token: &str,
        created: i64,
    ) -> HeaderMap {
        let public = self.key.verifying_key().to_bytes();
        let public_text = URL_SAFE_NO_PAD.encode(public);
        let actor = format!("ed25519:{}", URL_SAFE_NO_PAD.encode(Sha256::digest(public)));
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut nonce);
        let nonce = URL_SAFE_NO_PAD.encode(nonce);
        let digest = format!("sha-256=:{}:", STANDARD.encode(Sha256::digest(body)));
        let authorization = format!("HumanGrant {token}");
        let params = format!(
            "{COMPONENTS};created={created};expires={};nonce=\"{nonce}\";keyid=\"{actor}\";alg=\"ed25519\";tag=\"humangrant-0.1\"",
            created + 60
        );
        let base = format!(
            "\"@method\": {method}\n\"@target-uri\": {}{path}\n\"content-digest\": {digest}\n\"content-type\": application/json\n\"x-hg-key\": {public_text}\n\"authorization\": {authorization}\n\"@signature-params\": {params}",
            self.origin
        );
        let signature = format!(
            "hg=:{}:",
            STANDARD.encode(self.key.sign(base.as_bytes()).to_bytes())
        );
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("content-type", "application/json".to_owned()),
            ("content-digest", digest),
            ("x-hg-key", public_text),
            ("authorization", authorization),
            ("signature-input", format!("hg={params}")),
            ("signature", signature),
        ] {
            headers.insert(name, value.parse().unwrap());
        }
        headers
    }
    fn raw_request(&self, method: &str, path: &str, body: Vec<u8>, token: &str) -> Request<Body> {
        let headers = self.headers_at(method, path, &body, token, now());
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .body(Body::from(body))
            .unwrap();
        *request.headers_mut() = headers;
        request
    }
    async fn send(
        &self,
        app: &Router,
        method: &str,
        path: &str,
        body: Value,
        token: &str,
    ) -> Observation {
        let bytes = if method == "GET" {
            Vec::new()
        } else {
            serde_json::to_vec(&body).unwrap()
        };
        let response = observe(app, self.raw_request(method, path, bytes, token)).await;
        assert!(
            !response.headers.contains_key(header::SET_COOKIE),
            "Agent endpoints must not issue human cookies"
        );
        response
    }
    fn create_body(permissions: Value) -> Value {
        json!({"client_id":"hireferry-cli", "target_service":"hireferry", "purpose":"Read the account data requested by the human", "requested_permissions":permissions, "session_ttl_seconds":3600})
    }
    async fn create(&self, app: &Router, permissions: Value) -> Pending {
        let response = self
            .send(
                app,
                "POST",
                "/v1/requests",
                Self::create_body(permissions),
                "anonymous",
            )
            .await;
        response.ok();
        let url =
            url::Url::parse(response.value["human_authorization_url"].as_str().unwrap()).unwrap();
        assert_eq!(url.origin().ascii_serialization(), self.origin);
        assert!(url.query().is_none());
        let id = response.value["request_id"].as_str().unwrap().to_owned();
        let link = url.path().strip_prefix("/human/").unwrap().to_owned();
        assert_ne!(
            id, link,
            "Public request IDs must not be human review capabilities"
        );
        Pending { id, link }
    }
    async fn claim(&self, app: &Router, request: &Pending) -> Observation {
        self.send(app, "POST", &request.claim_path(), json!({}), "anonymous")
            .await
    }
    async fn tool(&self, app: &Router, token: &str, name: &str, arguments: Value) -> Observation {
        self.send(app, "POST", "/mcp", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}), token).await
    }
    async fn granted(
        &self,
        app: &Router,
        browser: &mut Browser,
        permissions: Value,
    ) -> (Pending, Value) {
        let request = self.create(app, permissions.clone()).await;
        browser.approve(&request, permissions).await.ok();
        let claimed = self.claim(app, &request).await;
        claimed.ok();
        (request, claimed.value)
    }
}

#[tokio::test]
async fn configured_origin_is_canonical_for_signatures_and_authorization_links() {
    let state = AppState::new(Config {
        public_url: "https://HireFerry.example:443/".into(),
        webauthn_rp_id: "hireferry.example".into(),
        database: ":memory:".into(),
        ..Default::default()
    })
    .unwrap();
    let canonical = "https://hireferry.example";
    assert_eq!(state.cfg.public_url, canonical);
    let app = hireferry::app(state);
    let actor = Agent::for_origin(canonical);
    // Uses the real Config -> AppState initialization and request verifier; the
    // helper also checks that the returned human link has the canonical origin.
    let request = actor
        .create(&app, json!([permission("profile.read", "self")]))
        .await;
    let mut cancel = actor.raw_request(
        "POST",
        &format!("/v1/requests/{}/cancel", request.id),
        b"{}".to_vec(),
        "anonymous",
    );
    cancel
        .headers_mut()
        .insert(header::ORIGIN, canonical.parse().unwrap());
    observe(&app, cancel).await.ok();
}

#[tokio::test]
async fn actual_login_explicit_consent_scope_reduction_and_immediate_revocation() {
    let state = state();
    let app = hireferry::app(state.clone());
    let mut browser = Browser::new(&app);
    let uid = browser.seeker("scope-owner@example.invalid").await;
    let actor = Agent::new();
    let request = actor
        .create(
            &app,
            json!([
                permission("profile.read", "self"),
                permission("profile.write", "self")
            ]),
        )
        .await;

    let anonymous = Browser::new(&app).review(&request).await;
    assert!(anonymous["human_id"].is_null());
    assert_eq!(anonymous["grant_status"], "none");
    assert!(anonymous["grant_expires_at"].is_null());
    assert_eq!(anonymous["granted_permissions"], json!([]));
    actor.claim(&app, &request).await.denied();
    let before = browser.review(&request).await;
    let after = browser.review(&request).await;
    assert_eq!(
        before["phase"], after["phase"],
        "GET must not grant consent"
    );
    assert_eq!(after["human_id"], uid);
    assert!(after["csrf"].as_str().is_some_and(|s| !s.is_empty()));
    browser
        .approve(&request, json!([permission("profile.read", "self")]))
        .await
        .ok();
    let approved_review = browser.review(&request).await;
    assert_eq!(approved_review["phase"], "approved");
    assert_eq!(approved_review["grant_status"], "pending_claim");
    assert_eq!(
        approved_review["grant_expires_at"],
        before["requested_grant_expires_at"]
    );
    assert_eq!(
        approved_review["granted_permissions"],
        json!([permission("profile.read", "self")])
    );
    assert_eq!(
        approved_review["requested_permissions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let claimed = actor.claim(&app, &request).await;
    claimed.ok();
    assert_eq!(claimed.value["token_type"], "HumanGrant");
    let connected_review = browser.review(&request).await;
    assert_eq!(connected_review["phase"], "succeeded");
    assert_eq!(connected_review["grant_status"], "active");
    assert_eq!(
        connected_review["grant_expires_at"],
        claimed.value["expires_at"]
    );
    assert_eq!(
        connected_review["granted_permissions"],
        approved_review["granted_permissions"]
    );
    let token = claimed.value["access_token"].as_str().unwrap();
    actor.claim(&app, &request).await.denied();
    let profile = actor.tool(&app, token, "get_profile", json!({})).await;
    profile.ok();
    assert_eq!(
        profile.value["result"]["isError"], false,
        "{}",
        profile.text
    );
    assert_eq!(profile.value["result"]["structuredContent"]["id"], uid);
    actor
        .tool(
            &app,
            token,
            "update_profile",
            json!({"name":"not allowed","public_email":"","resume_md":""}),
        )
        .await
        .tool_denied();
    assert_eq!(
        state
            .conn()
            .unwrap()
            .query_row("SELECT name FROM users WHERE id=?1", [&uid], |r| r
                .get::<_, String>(0))
            .unwrap(),
        ""
    );

    let grants = browser
        .request("GET", "/api/humangrant/grants", Value::Null, None)
        .await;
    grants.ok();
    assert_eq!(grants.value["grants"].as_array().unwrap().len(), 1);
    assert!(!grants.text.contains(token));
    let grant_id = claimed.value["grant_id"].as_str().unwrap();
    browser
        .request(
            "DELETE",
            &format!("/api/humangrant/grants/{grant_id}"),
            json!({}),
            grants.value["csrf"].as_str(),
        )
        .await
        .ok();
    let revoked_review = browser.review(&request).await;
    assert_eq!(revoked_review["phase"], "succeeded");
    assert_eq!(revoked_review["grant_status"], "revoked");
    assert_eq!(
        revoked_review["grant_expires_at"],
        claimed.value["expires_at"]
    );
    assert_eq!(
        revoked_review["granted_permissions"],
        approved_review["granted_permissions"]
    );
    actor
        .tool(&app, token, "get_profile", json!({}))
        .await
        .tool_denied();
    let own_session = browser
        .request("GET", "/api/session", Value::Null, None)
        .await;
    own_session.ok();
    assert_eq!(
        own_session.value["user"]["id"], uid,
        "Revoking AI access must preserve the human login"
    );
}

#[tokio::test]
async fn signatures_cover_exact_body_path_and_credential_and_replay_is_persistent() {
    let directory = tempfile::tempdir().unwrap();
    let config = Config {
        development: true,
        database: directory
            .path()
            .join("replay.sqlite3")
            .to_str()
            .unwrap()
            .into(),
        ..Default::default()
    };
    let app = hireferry::app(AppState::new(config.clone()).unwrap());
    let actor = Agent::new();
    let body = serde_json::to_vec(&Agent::create_body(json!([permission(
        "profile.read",
        "self"
    )])))
    .unwrap();
    let headers = actor.headers_at("POST", "/v1/requests", &body, "anonymous", now());
    let make = |path: &str, bytes: Vec<u8>, headers: HeaderMap| {
        let mut req = Request::builder()
            .method("POST")
            .uri(path)
            .body(Body::from(bytes))
            .unwrap();
        *req.headers_mut() = headers;
        req
    };
    let mut changed = body.clone();
    changed.push(b' ');
    observe(&app, make("/v1/requests", changed, headers.clone()))
        .await
        .denied();
    observe(
        &app,
        make("/v1/requests/other/claim", body.clone(), headers.clone()),
    )
    .await
    .denied();
    let mut changed_auth = headers.clone();
    changed_auth.insert(
        header::AUTHORIZATION,
        format!("HumanGrant {}", URL_SAFE_NO_PAD.encode([9u8; 32]))
            .parse()
            .unwrap(),
    );
    observe(&app, make("/v1/requests", body.clone(), changed_auth))
        .await
        .denied();
    observe(&app, make("/v1/requests", body.clone(), headers.clone()))
        .await
        .ok();
    // Open a fresh database connection and auth service, as happens after a restart.
    let restarted_router = hireferry::app(AppState::new(config).unwrap());
    observe(
        &restarted_router,
        make("/v1/requests", body.clone(), headers),
    )
    .await
    .denied();
    let expired = actor.headers_at("POST", "/v1/requests", &body, "anonymous", now() - 61);
    observe(&app, make("/v1/requests", body.clone(), expired))
        .await
        .denied();
    let mut duplicate = actor.headers_at("POST", "/v1/requests", &body, "anonymous", now());
    duplicate.append("x-hg-key", duplicate["x-hg-key"].clone());
    observe(&app, make("/v1/requests", body, duplicate))
        .await
        .denied();
}

#[tokio::test]
async fn claimed_credential_is_key_bound_and_cannot_be_a_cookie_or_bearer() {
    let app = hireferry::app(state());
    let mut browser = Browser::new(&app);
    browser.seeker("key-owner@example.invalid").await;
    let actor = Agent::new();
    let other_actor = Agent::new();
    let (request, claimed) = actor
        .granted(
            &app,
            &mut browser,
            json!([permission("profile.read", "self")]),
        )
        .await;
    let token = claimed["access_token"].as_str().unwrap();
    other_actor
        .tool(&app, token, "get_profile", json!({}))
        .await
        .tool_denied();
    other_actor.claim(&app, &request).await.denied();
    let bare = observe(
        &app,
        Request::builder()
            .uri("/api/profile")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    bare.denied();
    let human = observe(
        &app,
        Request::builder()
            .uri("/api/session")
            .header(header::COOKIE, format!("hf_session={token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    human.ok();
    assert!(human.value["user"].is_null());
    let unsigned = observe(
        &app,
        Request::builder()
            .method("POST")
            .uri("/mcp")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("HumanGrant {token}"))
            .body(Body::from(
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    unsigned.denied();
    actor
        .send(&app, "GET", "/api/profile", Value::Null, token)
        .await
        .denied();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_claim_issues_at_most_one_credential() {
    let app = hireferry::app(state());
    let mut browser = Browser::new(&app);
    browser.seeker("claim-owner@example.invalid").await;
    let actor = Agent::new();
    let request = actor
        .create(&app, json!([permission("profile.read", "self")]))
        .await;
    browser
        .approve(&request, json!([permission("profile.read", "self")]))
        .await
        .ok();
    let first = actor.raw_request("POST", &request.claim_path(), b"{}".to_vec(), "anonymous");
    let second = actor.raw_request("POST", &request.claim_path(), b"{}".to_vec(), "anonymous");
    let first_app = app.clone();
    let second_app = app.clone();
    let one = tokio::spawn(async move { observe(&first_app, first).await });
    let two = tokio::spawn(async move { observe(&second_app, second).await });
    let (a, b) = tokio::join!(one, two);
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(
        usize::from(a.status.is_success()) + usize::from(b.status.is_success()),
        1,
        "{} / {}",
        a.text,
        b.text
    );
    let rejected = if a.status.is_success() { b } else { a };
    assert_eq!(rejected.status, StatusCode::CONFLICT, "{}", rejected.text);
    assert!(rejected.value.get("access_token").is_none());
}

#[tokio::test]
async fn consent_is_bound_to_the_reviewed_account_session_and_request() {
    let state = state();
    let app = hireferry::app(state.clone());
    let mut browser = Browser::new(&app);
    let first_user = browser.seeker("first-owner@example.invalid").await;
    let actor = Agent::new();
    let permissions = json!([permission("profile.read", "self")]);
    let first = actor.create(&app, permissions.clone()).await;
    let second = actor.create(&app, permissions.clone()).await;
    let review = browser.review(&first).await;
    let decision =
        json!({"permissions": permissions, "expires_at": review["requested_grant_expires_at"]});
    browser
        .request(
            "POST",
            &format!("{}/approve", second.review_path()),
            decision.clone(),
            review["csrf"].as_str(),
        )
        .await
        .denied();
    browser
        .request(
            "POST",
            &format!("{}/approve", first.review_path()),
            decision.clone(),
            None,
        )
        .await
        .denied();
    browser
        .request("POST", "/api/auth/logout", json!({}), None)
        .await
        .ok();
    let second_user = browser.seeker("second-owner@example.invalid").await;
    assert_ne!(first_user, second_user);
    browser
        .request(
            "POST",
            &format!("{}/approve", first.review_path()),
            decision,
            review["csrf"].as_str(),
        )
        .await
        .denied();
    let current = browser.review(&first).await;
    assert_eq!(current["human_id"], second_user);
    // A fresh view and explicit new consent from the current user is required.
    browser
        .approve(&first, json!([permission("profile.read", "self")]))
        .await
        .ok();
    let claimed = actor.claim(&app, &first).await;
    claimed.ok();
    let profile = actor
        .tool(
            &app,
            claimed.value["access_token"].as_str().unwrap(),
            "get_profile",
            json!({}),
        )
        .await;
    assert_eq!(
        profile.value["result"]["structuredContent"]["id"],
        second_user
    );
}

#[tokio::test]
async fn review_cannot_expand_permissions_or_authorize_other_users_data() {
    let app = hireferry::app(state());
    let mut browser = Browser::new(&app);
    browser.seeker("review-owner@example.invalid").await;
    let mut other = Browser::new(&app);
    let other_id = other.seeker("other-owner@example.invalid").await;
    let actor = Agent::new();
    let request = actor
        .create(&app, json!([permission("profile.read", "self")]))
        .await;
    browser
        .approve(
            &request,
            json!([
                permission("profile.read", "self"),
                permission("profile.write", "self")
            ]),
        )
        .await
        .denied();
    browser.approve(&request, json!([])).await.denied();
    let third_party = actor
        .create(&app, json!([permission("people.read", &other_id)]))
        .await;
    let review = browser.review(&third_party).await;
    assert!(
        !review["grantable_permissions"]
            .as_array()
            .unwrap()
            .contains(&permission("people.read", &other_id))
    );
    browser
        .approve(&third_party, json!([permission("people.read", &other_id)]))
        .await
        .denied();
    actor.claim(&app, &third_party).await.denied();
}

#[tokio::test]
async fn exact_conversation_permission_cannot_read_collections_or_other_threads() {
    let app = hireferry::app(state());
    let mut browser = Browser::new(&app);
    let uid = browser.seeker("thread-owner@example.invalid").await;
    let own_thread = format!("platform_{uid}");
    let actor = Agent::new();
    let (_, claim) = actor
        .granted(
            &app,
            &mut browser,
            json!([permission("chat.read", &own_thread)]),
        )
        .await;
    let token = claim["access_token"].as_str().unwrap();
    let own = actor
        .tool(
            &app,
            token,
            "get_messages",
            json!({"conversation_id":own_thread}),
        )
        .await;
    own.ok();
    assert_eq!(own.value["result"]["isError"], false, "{}", own.text);
    actor
        .tool(&app, token, "list_conversations", json!({}))
        .await
        .tool_denied();
    actor
        .tool(&app, token, "get_updates", json!({}))
        .await
        .tool_denied();
    actor
        .tool(
            &app,
            token,
            "get_messages",
            json!({"conversation_id":"platform_unrelated"}),
        )
        .await
        .tool_denied();
    actor
        .tool(
            &app,
            token,
            "send_message",
            json!({"conversation_id":own_thread,"body":"not approved","client_id":"not-approved"}),
        )
        .await
        .tool_denied();
}

#[tokio::test]
async fn revoked_login_and_suspended_account_cannot_approve_or_keep_using_a_grant() {
    let state = state();
    let app = hireferry::app(state.clone());
    let mut browser = Browser::new(&app);
    let uid = browser.seeker("suspension-owner@example.invalid").await;
    let actor = Agent::new();
    let pending = actor
        .create(&app, json!([permission("profile.read", "self")]))
        .await;
    let review = browser.review(&pending).await;
    let (_, claim) = actor
        .granted(
            &app,
            &mut browser,
            json!([permission("profile.read", "self")]),
        )
        .await;
    state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id=?1", [&uid])
        .unwrap();
    actor
        .tool(
            &app,
            claim["access_token"].as_str().unwrap(),
            "get_profile",
            json!({}),
        )
        .await
        .tool_denied();
    browser.request("POST", &format!("{}/approve", pending.review_path()), json!({"permissions":[permission("profile.read", "self")], "expires_at":review["requested_grant_expires_at"]}), review["csrf"].as_str()).await.denied();
    state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=0 WHERE id=?1", [&uid])
        .unwrap();
    state
        .conn()
        .unwrap()
        .execute(
            "UPDATE sessions SET expires_at=?1 WHERE user_id=?2",
            params![now() - 1, uid],
        )
        .unwrap();
    browser.request("POST", &format!("{}/approve", pending.review_path()), json!({"permissions":[permission("profile.read", "self")], "expires_at":review["requested_grant_expires_at"]}), review["csrf"].as_str()).await.denied();
}

#[tokio::test]
async fn restored_account_requires_new_login_and_consent_instead_of_reviving_old_grants() {
    let state = state();
    let app = hireferry::app(state.clone());
    let email = "restored-grants-owner@example.invalid";
    let mut browser = Browser::new(&app);
    let uid = browser.seeker(email).await;
    let actor = Agent::new();
    let permissions = json!([permission("profile.read", "self")]);
    let (_, old_claim) = actor.granted(&app, &mut browser, permissions.clone()).await;
    let token = old_claim["access_token"].as_str().unwrap();
    let working = actor.tool(&app, token, "get_profile", json!({})).await;
    working.ok();
    assert_eq!(working.value["result"]["structuredContent"]["id"], uid);
    let unclaimed = actor.create(&app, permissions.clone()).await;
    browser.approve(&unclaimed, permissions.clone()).await.ok();

    // Match the administrator-session fixture in account_access.rs. The member
    // account, member session, review, approval and claims all use real HTTP.
    let mut operator = Browser::new(&app);
    let admin_token = uuid::Uuid::new_v4().to_string();
    let admin_csrf = uuid::Uuid::new_v4().to_string();
    state.conn().unwrap().execute(
        "INSERT INTO sessions(token_hash,admin,csrf,expires_at,recent_auth_at) VALUES(?1,1,?2,?3,?4)",
        params![hireferry::auth::hash_token(&admin_token), admin_csrf, now() + 3600, now()],
    ).unwrap();
    operator.cookies.insert("hf_session".into(), admin_token);
    operator.csrf = admin_csrf;
    let access_path = format!("/api/admin/users/{uid}/access");
    // No credential is exercised while suspended: revocation must happen as
    // part of the ban itself, not as a side effect of a failed token request.
    for banned in [true, false] {
        let changed = operator
            .request(
                "POST",
                &access_path,
                json!({"banned":banned,"reason":"Synthetic HumanGrant suspension regression"}),
                None,
            )
            .await;
        changed.ok();
        assert_eq!(changed.value["user"]["banned"], banned);
    }
    assert_eq!(
        hireferry::access::eligibility(&state.conn().unwrap(), &uid).unwrap()["eligible"],
        true
    );
    actor
        .send(&app, "GET", "/v1/session", Value::Null, token)
        .await
        .denied();
    actor
        .tool(&app, token, "get_profile", json!({}))
        .await
        .tool_denied();
    actor.claim(&app, &unclaimed).await.denied();
    let old_browser = browser
        .request("GET", "/api/session", Value::Null, None)
        .await;
    old_browser.ok();
    assert!(old_browser.value["user"].is_null());
    let (grants, revoked): (i64, i64) = state
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*),sum(revoked_at IS NOT NULL) FROM hg_grants WHERE user_id=?1",
            [&uid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((grants, revoked), (2, 2));

    // Age only the email cooldown to make a fresh real login without sleeping.
    state
        .conn()
        .unwrap()
        .execute(
            "UPDATE auth_limits SET window_start=?1 WHERE key=?2",
            params![
                now() - 61,
                format!("email-short:{}", hireferry::auth::hash_token(email))
            ],
        )
        .unwrap();
    let mut fresh_browser = Browser::new(&app);
    assert_eq!(fresh_browser.sign_in(email).await, uid);
    let (_, fresh_claim) = actor.granted(&app, &mut fresh_browser, permissions).await;
    let fresh_token = fresh_claim["access_token"].as_str().unwrap();
    assert_ne!(fresh_token, token);
    let fresh_profile = actor
        .tool(&app, fresh_token, "get_profile", json!({}))
        .await;
    fresh_profile.ok();
    assert_eq!(
        fresh_profile.value["result"]["structuredContent"]["id"],
        uid
    );
    // A new grant cannot implicitly reactivate either earlier authorization.
    actor
        .send(&app, "GET", "/v1/session", Value::Null, token)
        .await
        .denied();
    actor.claim(&app, &unclaimed).await.denied();
}

#[tokio::test]
async fn deny_and_actor_cancel_are_terminal_and_private_status_requires_the_original_key() {
    let app = hireferry::app(state());
    let mut browser = Browser::new(&app);
    browser.seeker("terminal-owner@example.invalid").await;
    let actor = Agent::new();
    let request = actor
        .create(&app, json!([permission("profile.read", "self")]))
        .await;
    let review = browser.review(&request).await;
    browser
        .request(
            "POST",
            &format!("{}/deny", request.review_path()),
            json!({}),
            review["csrf"].as_str(),
        )
        .await
        .ok();
    actor.claim(&app, &request).await.denied();
    browser
        .approve(&request, json!([permission("profile.read", "self")]))
        .await
        .denied();
    let cancelled = actor
        .create(&app, json!([permission("profile.read", "self")]))
        .await;
    actor
        .send(
            &app,
            "POST",
            &format!("/v1/requests/{}/cancel", cancelled.id),
            json!({}),
            "anonymous",
        )
        .await
        .ok();
    actor.claim(&app, &cancelled).await.denied();
    let path = format!("/v1/requests/{}/status.md", request.id);
    Agent::new()
        .send(&app, "GET", &path, Value::Null, "anonymous")
        .await
        .denied();
    let status = actor
        .send(&app, "GET", &path, Value::Null, "anonymous")
        .await;
    status.ok();
    assert_eq!(
        status.headers[header::CONTENT_TYPE],
        "text/markdown; charset=utf-8"
    );
    assert!(
        status.text.contains("phase: \"rejected\""),
        "{}",
        status.text
    );
    assert!(status.text.contains("status_updated_at:"));
    assert_eq!(status.headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn disabled_configuration_and_unknown_actions_fail_closed() {
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.humangrant.enabled = false;
    let app = hireferry::app(AppState::new(cfg).unwrap());
    let actor = Agent::new();
    actor
        .send(
            &app,
            "POST",
            "/v1/requests",
            Agent::create_body(json!([permission("profile.read", "self")])),
            "anonymous",
        )
        .await
        .denied();
    let config = observe(
        &app,
        Request::builder()
            .uri("/api/config")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    config.ok();
    assert_eq!(config.value["humangrant_enabled"], false);
    let enabled = hireferry::app(state());
    for action in [
        "admin.users.write",
        "profile.*",
        "choose_role",
        "update_mandate",
        "create_checkout",
    ] {
        actor
            .send(
                &enabled,
                "POST",
                "/v1/requests",
                Agent::create_body(json!([permission(action, "self")])),
                "anonymous",
            )
            .await
            .denied();
    }
}

#[tokio::test]
async fn expiry_and_removed_permissions_are_rechecked_on_each_signed_operation() {
    let state = state();
    let app = hireferry::app(state.clone());
    let mut browser = Browser::new(&app);
    browser.seeker("expiry-owner@example.invalid").await;
    let actor = Agent::new();
    let pending = actor
        .create(&app, json!([permission("profile.read", "self")]))
        .await;
    state
        .conn()
        .unwrap()
        .execute(
            "UPDATE hg_requests SET expires_at=?1 WHERE id=?2",
            params![now() - 1, pending.id],
        )
        .unwrap();
    actor.claim(&app, &pending).await.denied();
    browser
        .approve(&pending, json!([permission("profile.read", "self")]))
        .await
        .denied();

    let (active_request, claimed) = actor
        .granted(
            &app,
            &mut browser,
            json!([
                permission("profile.read", "self"),
                permission("profile.write", "self")
            ]),
        )
        .await;
    let token = claimed["access_token"].as_str().unwrap();
    let write = actor
        .tool(
            &app,
            token,
            "update_profile",
            json!({"name":"Approved change", "public_email":"", "resume_md":"# Approved resume"}),
        )
        .await;
    write.ok();
    assert_eq!(write.value["result"]["isError"], false, "{}", write.text);
    assert_eq!(
        write.value["result"]["structuredContent"],
        json!({"updated": true})
    );
    for private_field in ["email", "resume_md", "name"] {
        assert!(
            write.value["result"]["structuredContent"]
                .get(private_field)
                .is_none()
        );
    }
    assert!(!write.text.contains("Approved resume"));
    state
        .conn()
        .unwrap()
        .execute(
            "UPDATE hg_grants SET permissions=?1 WHERE id=?2",
            params![
                json!([permission("profile.read", "self")]).to_string(),
                claimed["grant_id"].as_str().unwrap()
            ],
        )
        .unwrap();
    actor
        .tool(
            &app,
            token,
            "update_profile",
            json!({"name":"Must not change", "public_email":"", "resume_md":""}),
        )
        .await
        .tool_denied();
    let read = actor.tool(&app, token, "get_profile", json!({})).await;
    read.ok();
    assert_eq!(
        read.value["result"]["structuredContent"]["name"],
        "Approved change"
    );
    let session = actor
        .send(&app, "GET", "/v1/session", Value::Null, token)
        .await;
    session.ok();
    assert_eq!(
        session.value["effective_permissions"],
        json!([permission("profile.read", "self")])
    );
    let expired_at = now() - 1;
    state
        .conn()
        .unwrap()
        .execute(
            "UPDATE hg_grants SET expires_at=?1 WHERE id=?2",
            params![expired_at, claimed["grant_id"].as_str().unwrap()],
        )
        .unwrap();
    let expired_review = browser.review(&active_request).await;
    assert_eq!(expired_review["phase"], "succeeded");
    assert_eq!(expired_review["grant_status"], "expired");
    assert_eq!(expired_review["grant_expires_at"], expired_at);
    assert_eq!(
        expired_review["granted_permissions"],
        json!([permission("profile.read", "self")])
    );
    actor
        .tool(&app, token, "get_profile", json!({}))
        .await
        .tool_denied();
    actor
        .send(&app, "GET", "/v1/session", Value::Null, token)
        .await
        .denied();
}

#[tokio::test]
async fn expected_account_and_origin_cannot_be_substituted_at_consent() {
    let app = hireferry::app(state());
    let mut owner = Browser::new(&app);
    let uid = owner.seeker("expected-owner@example.invalid").await;
    let mut other = Browser::new(&app);
    other.seeker("unexpected-owner@example.invalid").await;
    let actor = Agent::new();
    let mut body = Agent::create_body(json!([permission("profile.read", "self")]));
    body["expected_human_id"] = json!(uid);
    let created = actor
        .send(&app, "POST", "/v1/requests", body, "anonymous")
        .await;
    created.ok();
    let url = url::Url::parse(created.value["human_authorization_url"].as_str().unwrap()).unwrap();
    let pending = Pending {
        id: created.value["request_id"].as_str().unwrap().into(),
        link: url.path().strip_prefix("/human/").unwrap().into(),
    };
    let wrong = other.review(&pending).await;
    assert_eq!(wrong["expected_human_id"], uid);
    assert!(wrong["csrf"].is_null());
    assert!(
        wrong["grantable_permissions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    other
        .approve(&pending, json!([permission("profile.read", "self")]))
        .await
        .denied();
    let review = owner.review(&pending).await;
    let approval = json!({"permissions":[permission("profile.read", "self")],"expires_at":review["requested_grant_expires_at"]});
    let cross_origin = Request::builder()
        .method("POST")
        .uri(format!("{}/approve", pending.review_path()))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "https://unrelated.example.invalid")
        .header(header::COOKIE, owner.cookie_header())
        .header("x-hg-csrf", review["csrf"].as_str().unwrap())
        .body(Body::from(approval.to_string()))
        .unwrap();
    observe(&app, cross_origin).await.denied();
    actor.claim(&app, &pending).await.denied();
    owner
        .approve(&pending, json!([permission("profile.read", "self")]))
        .await
        .ok();
}

#[tokio::test]
async fn consumed_nonce_and_request_limits_survive_failed_business_actions_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = Config {
        development: true,
        database: directory
            .path()
            .join("limits.sqlite3")
            .to_str()
            .unwrap()
            .into(),
        ..Default::default()
    };
    config.humangrant.request_limit_per_hour = 1;
    config.humangrant.service_limit_per_hour = 2;
    let app = hireferry::app(AppState::new(config.clone()).unwrap());
    let actor = Agent::new();
    let invalid = serde_json::to_vec(&Agent::create_body(json!([permission(
        "admin.all",
        "self"
    )])))
    .unwrap();
    let headers = actor.headers_at("POST", "/v1/requests", &invalid, "anonymous", now());
    let make = || {
        let mut request = Request::builder()
            .method("POST")
            .uri("/v1/requests")
            .body(Body::from(invalid.clone()))
            .unwrap();
        *request.headers_mut() = headers.clone();
        request
    };
    observe(&app, make()).await.denied();
    let replay = observe(&app, make()).await;
    assert_eq!(replay.value["error"], "signature_replay", "{}", replay.text);
    actor
        .create(&app, json!([permission("profile.read", "self")]))
        .await;
    let restarted = hireferry::app(AppState::new(config).unwrap());
    let limited = actor
        .send(
            &restarted,
            "POST",
            "/v1/requests",
            Agent::create_body(json!([permission("profile.read", "self")])),
            "anonymous",
        )
        .await;
    assert_eq!(
        limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.text
    );
    assert!(limited.headers.contains_key("retry-after"));
    Agent::new()
        .create(&restarted, json!([permission("profile.read", "self")]))
        .await;
    let service_limited = Agent::new()
        .send(
            &restarted,
            "POST",
            "/v1/requests",
            Agent::create_body(json!([permission("profile.read", "self")])),
            "anonymous",
        )
        .await;
    assert_eq!(
        service_limited.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        service_limited.text
    );
}

/// Use the same local settlement fixture as commerce_flows; no gateway is contacted.
async fn settle_fixture_order(state: &AppState, order_id: &str) {
    let order = hireferry::models::load_order(&state.conn().unwrap(), order_id).unwrap();
    hireferry::commerce::settle_payment(
        state,
        hireferry::payments::PaymentReceipt {
            provider: order.provider.clone(),
            event_id: format!("evt_hg_{}", order.id),
            order_id: order.id.clone(),
            session_id: Some(format!("cs_hg_{}", order.id)),
            payment_id: format!("pi_hg_{}", order.id),
            amount_cents: order.amount_cents,
            currency: order.currency.clone(),
            live_mode: order.live_mode,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn job_write_only_grant_returns_receipts_without_job_or_order_data() {
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.payments.stripe.enabled = true;
    cfg.payments.stripe.secret_key = "sk_test_local_hg_fixture_never_sent".into();
    cfg.payments.stripe.webhook_secret = "whsec_local_hg_fixture_never_sent".into();
    cfg.pricing.currency = "USD".into();
    cfg.pricing.employer_ai_min_salary_minor = 600_000;
    let state = AppState::new(cfg).unwrap();
    let app = hireferry::app(state.clone());
    let mut browser = Browser::new(&app);
    let uid = browser.sign_in("job-write-owner@example.invalid").await;
    let role = browser
        .request(
            "POST",
            "/api/role",
            json!({"role":"employer","provider":"stripe"}),
            None,
        )
        .await;
    role.ok();
    settle_fixture_order(&state, role.value["order"]["id"].as_str().unwrap()).await;
    // A fulfilled advert below its recorded currency-specific salary threshold
    // must not grant AI access, including after the other jobs are closed.
    let below_threshold = browser.request("POST", "/api/jobs", json!({"title":"Below the configured threshold","jd_md":"Published but not eligible for AI access","salary_cents":599_999,"currency":"USD","provider":"stripe"}), None).await;
    below_threshold.ok();
    settle_fixture_order(
        &state,
        below_threshold.value["order"]["id"].as_str().unwrap(),
    )
    .await;
    assert_eq!(
        hireferry::access::eligibility(&state.conn().unwrap(), &uid).unwrap()["eligible"],
        false
    );
    let baseline = browser.request("POST", "/api/jobs", json!({"title":"Existing private role","jd_md":"Private previous job description","salary_cents":700_000,"currency":"USD","provider":"stripe"}), None).await;
    baseline.ok();
    let job = baseline.value["job"]["id"].as_str().unwrap();
    settle_fixture_order(&state, baseline.value["order"]["id"].as_str().unwrap()).await;
    let stored = hireferry::models::load_job(&state.conn().unwrap(), job).unwrap();
    assert_eq!(stored.currency, "USD");
    assert_eq!(stored.ai_min_salary_cents, 600_000);
    assert_eq!(
        hireferry::access::eligibility(&state.conn().unwrap(), &uid).unwrap()["eligible"],
        true
    );

    let actor = Agent::new();
    let (_, claim) = actor
        .granted(
            &app,
            &mut browser,
            json!([permission("jobs.write", "self")]),
        )
        .await;
    let token = claim["access_token"].as_str().unwrap();
    let created = actor.tool(&app, token, "create_job", json!({"title":"Agent supplied role","jd_md":"Agent supplied description","salary_cents":600_000,"currency":"USD","provider":"stripe"})).await;
    created.ok();
    assert_eq!(
        created.value["result"]["isError"], false,
        "{}",
        created.text
    );
    let receipt = &created.value["result"]["structuredContent"];
    let new_job = receipt["job_id"].as_str().unwrap();
    let order_id = receipt["order_id"].as_str().unwrap();
    assert_eq!(
        receipt,
        &json!({"updated":true,"job_id":new_job,"order_id":order_id})
    );
    assert!(!created.text.contains("salary_cents"));
    assert!(!created.text.contains("jd_md"));
    settle_fixture_order(&state, order_id).await;
    actor
        .tool(&app, token, "get_job", json!({"id":new_job}))
        .await
        .tool_denied();
    actor
        .tool(&app, token, "get_order", json!({"id":order_id}))
        .await
        .tool_denied();
    actor
        .tool(&app, token, "list_my_jobs", json!({}))
        .await
        .tool_denied();

    let edited = actor
        .tool(
            &app,
            token,
            "edit_job",
            json!({"id":job,"jd_md":"Authorized replacement description","provider":"stripe"}),
        )
        .await;
    edited.ok();
    assert_eq!(edited.value["result"]["isError"], false, "{}", edited.text);
    let edit_receipt = &edited.value["result"]["structuredContent"];
    let edit_order = edit_receipt["order_id"].as_str().unwrap();
    assert_eq!(
        edit_receipt,
        &json!({"updated":true,"job_id":job,"order_id":edit_order})
    );
    assert!(!edited.text.contains("Private previous job description"));
    assert!(!edited.text.contains("salary_cents"));
    assert!(!edited.text.contains("snapshot_json"));
    settle_fixture_order(&state, edit_order).await;
    assert_eq!(
        hireferry::models::load_job(&state.conn().unwrap(), job)
            .unwrap()
            .jd_md,
        "Authorized replacement description"
    );

    let closed = actor
        .tool(&app, token, "close_job", json!({"id":job}))
        .await;
    closed.ok();
    assert_eq!(
        closed.value["result"]["structuredContent"],
        json!({"updated":true,"job_id":job}),
        "{}",
        closed.text
    );
    assert!(!closed.text.contains("Authorized replacement description"));
    assert_eq!(
        hireferry::models::load_job(&state.conn().unwrap(), job)
            .unwrap()
            .status,
        "closed"
    );
    // Another paid published role still qualifies the employer; these read failures are scope checks.
    assert_eq!(
        hireferry::access::eligibility(&state.conn().unwrap(), &uid).unwrap()["eligible"],
        true
    );
    actor
        .tool(&app, token, "get_job", json!({"id":job}))
        .await
        .tool_denied();
    actor
        .tool(&app, token, "list_orders", json!({}))
        .await
        .tool_denied();
    let last_closed = actor
        .tool(&app, token, "close_job", json!({"id":new_job}))
        .await;
    last_closed.ok();
    assert_eq!(
        last_closed.value["result"]["structuredContent"],
        json!({"updated":true,"job_id":new_job})
    );
    assert_eq!(
        hireferry::models::load_job(
            &state.conn().unwrap(),
            below_threshold.value["job"]["id"].as_str().unwrap()
        )
        .unwrap()
        .status,
        "published"
    );
    // The remaining paid advert is below its recorded threshold, so the last
    // qualifying job's removal disables the existing HumanGrant credential.
    actor
        .send(&app, "GET", "/v1/session", Value::Null, token)
        .await
        .denied();
}

#[tokio::test]
async fn chat_write_only_read_marker_does_not_expose_unread_counts_or_messages() {
    let state = state();
    let app = hireferry::app(state.clone());
    let mut browser = Browser::new(&app);
    let uid = browser.seeker("chat-write-owner@example.invalid").await;
    let thread = format!("platform_{uid}");
    let message = browser.request("POST", &format!("/api/chat/conversations/{thread}/messages"), json!({"body":"Private human conversation content","client_id":"human-existing-message"}), None).await;
    message.ok();
    let actor = Agent::new();
    let (_, claim) = actor
        .granted(
            &app,
            &mut browser,
            json!([permission("chat.write", &thread)]),
        )
        .await;
    let token = claim["access_token"].as_str().unwrap();
    let marked = actor
        .tool(
            &app,
            token,
            "mark_read",
            json!({"conversation_id":thread,"message_id":0}),
        )
        .await;
    marked.ok();
    assert_eq!(marked.value["result"]["isError"], false, "{}", marked.text);
    assert_eq!(
        marked.value["result"]["structuredContent"],
        json!({"updated":true})
    );
    assert!(!marked.text.contains("unread_count"));
    assert!(!marked.text.contains("Private human conversation content"));
    let stored: i64 = state
        .conn()
        .unwrap()
        .query_row(
            "SELECT last_message_id FROM chat_reads WHERE conversation_id=?1 AND reader_key=?2",
            params![thread, format!("user_{uid}")],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, 0);
    actor
        .tool(
            &app,
            token,
            "get_messages",
            json!({"conversation_id":thread}),
        )
        .await
        .tool_denied();
    actor
        .tool(&app, token, "list_conversations", json!({}))
        .await
        .tool_denied();
    actor
        .tool(&app, token, "get_updates", json!({}))
        .await
        .tool_denied();
}
