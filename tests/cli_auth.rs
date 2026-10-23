//! Native-client protocol tests against real Axum handlers and SQLite.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hireferry::humangrant::{self, GrantContext, Permission};
use hireferry::{AppState, auth, config::Config, now};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    app: Router,
    uid: String,
    cookie: String,
}
struct Seen {
    status: StatusCode,
    value: Value,
    headers: HeaderMap,
}
impl Fixture {
    fn new() -> Self {
        Self::with_config(Config {
            development: true,
            database: ":memory:".into(),
            ..Default::default()
        })
    }
    fn with_config(config: Config) -> Self {
        let state = AppState::new(config).unwrap();
        let uid = uuid::Uuid::new_v4().to_string();
        let cookie = "browser-session-for-tests".to_owned();
        state.conn().unwrap().execute("INSERT INTO users(id,email,name,role,created_at,updated_at,resume_md) VALUES(?1,'member@example.invalid','Test member','seeker',?2,?2,'Private resume sentinel')",params![uid,now()]).unwrap();
        state.conn().unwrap().execute("INSERT INTO sessions(token_hash,user_id,csrf,expires_at,recent_auth_at) VALUES(?1,?2,'csrf-test',?3,?4)",params![auth::hash_token(&cookie),uid,now()+3600,now()]).unwrap();
        let app = hireferry::app(state.clone());
        Self {
            state,
            app,
            uid,
            cookie,
        }
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: String,
        content_type: &str,
        csrf: bool,
        bearer: Option<&str>,
    ) -> Seen {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", content_type)
            .header("origin", "http://localhost:3000")
            .header("cookie", format!("hf_session={}", self.cookie));
        if csrf {
            req = req.header("x-csrf-token", "csrf-test");
        }
        if let Some(token) = bearer {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        let response = self
            .app
            .clone()
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Seen {
            status,
            value,
            headers,
        }
    }
    async fn json(&self, path: &str, value: Value) -> Seen {
        self.request(
            "POST",
            path,
            value.to_string(),
            "application/json",
            true,
            None,
        )
        .await
    }
    async fn form(&self, path: &str, pairs: &[(&str, &str)]) -> Seen {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs.iter().copied())
            .finish();
        self.request(
            "POST",
            path,
            body,
            "application/x-www-form-urlencoded",
            false,
            None,
        )
        .await
    }
    async fn get(&self, path: &str, token: Option<&str>) -> Seen {
        self.request("GET", path, String::new(), "application/json", false, token)
            .await
    }
    async fn device(&self) -> Value {
        self.device_scope("mcp").await
    }
    async fn device_scope(&self, scope: &str) -> Value {
        let r = self
            .form(
                "/oauth/device/code",
                &[
                    ("client_id", &self.state.cfg.cli_auth.client_id),
                    ("scope", scope),
                ],
            )
            .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.value);
        assert_eq!(r.headers["cache-control"], "no-store");
        assert_eq!(r.headers["pragma"], "no-cache");
        r.value
    }
    async fn decision(&self, device: &Value, approve: bool) -> Seen {
        self.json(
            "/api/cli/authorization",
            json!({"flow":"device","user_code":device["user_code"],"approve":approve}),
        )
        .await
    }
    async fn poll(&self, device: &Value) -> Seen {
        self.form(
            "/oauth/token",
            &[
                ("client_id", &self.state.cfg.cli_auth.client_id),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device["device_code"].as_str().unwrap()),
            ],
        )
        .await
    }
    async fn browser_code(&self, request: &Value) -> String {
        let r = self.json("/api/cli/authorization", request.clone()).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.value);
        let url = url::Url::parse(r.value["redirect_uri"].as_str().unwrap()).unwrap();
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs["state"], request["state"]);
        assert_eq!(pairs["iss"], "http://localhost:3000");
        pairs["code"].clone()
    }
    async fn exchange(&self, code: &str, verifier: &str, redirect: &str) -> Seen {
        self.form(
            "/oauth/token",
            &[
                ("client_id", "hireferry-cli"),
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect),
                ("code_verifier", verifier),
            ],
        )
        .await
    }
    async fn login(&self, scope: &str) -> Value {
        let device = self.device_scope(scope).await;
        let decision = self.decision(&device, true).await;
        assert_eq!(decision.status, StatusCode::OK, "{}", decision.value);
        let result = self.poll(&device).await;
        assert_eq!(result.status, StatusCode::OK, "{}", result.value);
        result.value
    }
    async fn refresh(&self, credential: &str, scope: Option<&str>) -> Seen {
        let mut pairs = vec![
            ("client_id", self.state.cfg.cli_auth.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", credential),
        ];
        if let Some(scope) = scope {
            pairs.push(("scope", scope));
        }
        self.form("/oauth/token", &pairs).await
    }
    async fn delete(&self, path: &str) -> Seen {
        self.request(
            "DELETE",
            path,
            String::new(),
            "application/json",
            true,
            None,
        )
        .await
    }
    fn reconfigure(&mut self, change: impl FnOnce(&mut Config)) {
        let mut cfg = (*self.state.cfg).clone();
        change(&mut cfg);
        cfg.validate().unwrap();
        self.state.cfg = std::sync::Arc::new(cfg);
        self.app = hireferry::app(self.state.clone());
    }
    fn wait_poll(&self) {
        self.state
            .conn()
            .unwrap()
            .execute(
                "UPDATE cli_grants SET last_poll_at=?1-poll_interval-1",
                [now()],
            )
            .unwrap();
    }
    fn make_admin(&self, recent_at: i64) {
        self.state
            .conn()
            .unwrap()
            .execute(
                "UPDATE sessions SET user_id=NULL,admin=1,recent_auth_at=?1",
                [recent_at],
            )
            .unwrap();
    }
}
fn browser_request() -> Value {
    let verifier = "v".repeat(43);
    json!({"flow":"oauth","approve":true,"client_id":"hireferry-cli","response_type":"code",
        "redirect_uri":"http://127.0.0.1:49999/callback","scope":"mcp","state":"random-state-for-native-client-12345",
        "code_challenge":URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),"code_challenge_method":"S256"})
}
fn query(value: &Value) -> String {
    let pairs = value
        .as_object()
        .unwrap()
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|v| (k.as_str(), v)));
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

#[tokio::test]
async fn device_login_stores_hashes_and_revokes_the_real_api_token() {
    let f = Fixture::new();
    let device = f.device().await;
    assert_eq!(device["interval"], 5);
    assert_eq!(
        device["verification_uri"],
        "http://localhost:3000/oauth/device"
    );
    assert!(
        device["verification_uri_complete"]
            .as_str()
            .unwrap()
            .contains(device["user_code"].as_str().unwrap())
    );
    let details = f
        .get(
            &format!(
                "/api/cli/authorization?flow=device&user_code={}",
                device["user_code"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(details.status, StatusCode::OK);
    assert_eq!(details.value["flow"], "device");
    assert_eq!(details.value["can_approve"], true);
    assert_eq!(details.value["approval_error"], Value::Null);
    assert!(
        !details
            .value
            .to_string()
            .contains("Private resume sentinel")
    );
    assert_eq!(f.decision(&device, true).await.status, StatusCode::OK);
    assert_eq!(
        f.decision(&device, true).await.status,
        StatusCode::BAD_REQUEST
    );
    let result = f.poll(&device).await;
    assert_eq!(result.status, StatusCode::OK, "{}", result.value);
    let token = result.value["access_token"].as_str().unwrap();
    assert_eq!(result.value["scope"], "identity mcp");
    assert_eq!(result.value["token_type"], "Bearer");
    let refresh = result.value["refresh_token"].as_str().unwrap();
    assert!(refresh.starts_with("hf_refresh_"));
    assert!((899..=900).contains(&result.value["expires_in"].as_i64().unwrap()));
    let refresh_hash: String = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT token_hash FROM cli_refresh_tokens", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(refresh_hash, auth::hash_token(refresh));
    assert_ne!(refresh_hash, refresh);
    let stored: String = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT token_hash FROM api_tokens", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, auth::hash_token(token));
    assert_ne!(stored, token);
    let grant_hash: String = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT secret_hash FROM cli_grants", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        grant_hash,
        auth::hash_token(device["device_code"].as_str().unwrap())
    );
    let who = f.get("/oauth/userinfo", Some(token)).await;
    assert_eq!(who.status, StatusCode::OK);
    assert_eq!(who.value["user"]["id"], f.uid);
    assert_eq!(who.value["admin"], false);
    assert!(!who.value.to_string().contains("Private resume sentinel"));
    assert_eq!(f.poll(&device).await.value["error"], "invalid_grant");
    for _ in 0..2 {
        assert_eq!(
            f.form(
                "/oauth/revoke",
                &[("client_id", "hireferry-cli"), ("token", token)]
            )
            .await
            .status,
            StatusCode::OK
        );
    }
    assert_eq!(
        f.get("/oauth/userinfo", Some(token)).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn device_pending_slow_down_denial_and_expiry_are_distinct() {
    let f = Fixture::new();
    let device = f.device().await;
    assert_eq!(
        f.poll(&device).await.value["error"],
        "authorization_pending"
    );
    assert_eq!(f.poll(&device).await.value["error"], "slow_down");
    let interval: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT poll_interval FROM cli_grants", [], |r| r.get(0))
        .unwrap();
    assert_eq!(interval, 10);
    f.wait_poll();
    assert_eq!(
        f.poll(&device).await.value["error"],
        "authorization_pending"
    );
    assert_eq!(f.decision(&device, false).await.status, StatusCode::OK);
    assert_eq!(f.poll(&device).await.value["error"], "access_denied");
    let expired = f.device().await;
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE cli_grants SET expires_at=?1 WHERE secret_hash=?2",
            params![
                now() - 1,
                auth::hash_token(expired["device_code"].as_str().unwrap())
            ],
        )
        .unwrap();
    assert_eq!(f.poll(&expired).await.value["error"], "expired_token");
    assert_eq!(
        f.decision(&expired, true).await.status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn competing_device_exchanges_create_exactly_one_credential() {
    let f = Fixture::new();
    let device = f.device().await;
    assert_eq!(f.decision(&device, true).await.status, StatusCode::OK);
    let (a, b) = tokio::join!(f.poll(&device), f.poll(&device));
    assert_eq!(
        [a.status, b.status]
            .iter()
            .filter(|&&s| s == StatusCode::OK)
            .count(),
        1
    );
    let count: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM api_tokens", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn consent_needs_cookie_csrf_and_does_not_accept_a_bearer() {
    let f = Fixture::new();
    let device = f.device().await;
    let body = json!({"flow":"device","user_code":device["user_code"],"approve":true}).to_string();
    assert_eq!(
        f.request(
            "POST",
            "/api/cli/authorization",
            body.clone(),
            "application/json",
            false,
            None
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.request(
            "POST",
            "/api/cli/authorization",
            body,
            "application/json",
            true,
            Some("existing-token")
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let request = Request::builder()
        .method("POST")
        .uri("/api/cli/authorization")
        .header("content-type", "application/json")
        .header("origin", "http://localhost:3000")
        .body(Body::from(
            json!({"flow":"device","user_code":device["user_code"],"approve":true}).to_string(),
        ))
        .unwrap();
    assert_eq!(
        f.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.poll(&device).await.value["error"],
        "authorization_pending"
    );
}

#[tokio::test]
async fn browser_flow_binds_code_to_pkce_and_exact_redirect_and_detects_replay() {
    let f = Fixture::new();
    let request = browser_request();
    let page = f
        .get(&format!("/oauth/authorize?{}", query(&request)), None)
        .await;
    assert_eq!(page.status, StatusCode::OK);
    let count: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM cli_grants", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "GET must never issue authorization codes");
    let code = f.browser_code(&request).await;
    let redirect = request["redirect_uri"].as_str().unwrap();
    assert_eq!(
        f.exchange(&code, &"x".repeat(43), redirect).await.value["error"],
        "invalid_grant"
    );
    assert_eq!(
        f.exchange(&code, &"v".repeat(43), "http://127.0.0.1:49998/callback")
            .await
            .value["error"],
        "invalid_grant"
    );
    let ok = f.exchange(&code, &"v".repeat(43), redirect).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.value);
    let token = ok.value["access_token"].as_str().unwrap();
    assert_eq!(
        f.get("/oauth/userinfo", Some(token)).await.status,
        StatusCode::OK
    );
    assert_eq!(
        f.exchange(&code, &"v".repeat(43), redirect).await.value["error"],
        "invalid_grant"
    );
    assert_eq!(
        f.get("/oauth/userinfo", Some(token)).await.status,
        StatusCode::UNAUTHORIZED,
        "Replayed authorization codes revoke the issued token"
    );
}

#[tokio::test]
async fn browser_denial_preserves_state_and_issuer_without_creating_a_code() {
    let f = Fixture::new();
    let mut request = browser_request();
    request["approve"] = json!(false);
    let result = f.json("/api/cli/authorization", request.clone()).await;
    assert_eq!(result.status, StatusCode::OK);
    let redirect = url::Url::parse(result.value["redirect_uri"].as_str().unwrap()).unwrap();
    let pairs: std::collections::HashMap<_, _> = redirect.query_pairs().into_owned().collect();
    assert_eq!(pairs["error"], "access_denied");
    assert_eq!(pairs["state"], request["state"]);
    assert_eq!(pairs["iss"], "http://localhost:3000");
    assert!(!pairs.contains_key("code"));
}

#[tokio::test]
async fn browser_rejects_redirect_changes_pkce_downgrades_and_unknown_clients() {
    let f = Fixture::new();
    for redirect in [
        "https://example.invalid/callback",
        "http://localhost:49999/callback",
        "http://0.0.0.0:49999/callback",
        "http://127.0.0.1:0/callback",
        "http://127.0.0.1:49999/other",
        "http://127.0.0.1:49999/callback?x=1",
        "http://127.0.0.1:49999/callback#fragment",
        "http://127.0.0.1:49999/callback/../callback",
        "http://127.1:49999/callback",
    ] {
        let mut input = browser_request();
        input["redirect_uri"] = json!(redirect);
        assert_eq!(
            f.json("/api/cli/authorization", input).await.status,
            StatusCode::BAD_REQUEST,
            "{redirect}"
        );
    }
    for (key, value) in [
        ("code_challenge_method", "plain"),
        ("client_id", "unknown"),
        ("scope", "superuser"),
        ("response_type", "token"),
        ("state", "short"),
    ] {
        let mut input = browser_request();
        input[key] = json!(value);
        assert_eq!(
            f.json("/api/cli/authorization", input).await.status,
            StatusCode::BAD_REQUEST
        );
    }
    let mut ipv6 = browser_request();
    ipv6["redirect_uri"] = json!("http://[::1]:42311/callback");
    assert_eq!(
        f.json("/api/cli/authorization", ipv6).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn account_ban_and_browser_revocation_are_rechecked_at_exchange() {
    for ban in [true, false] {
        let f = Fixture::new();
        let device = f.device().await;
        assert_eq!(f.decision(&device, true).await.status, StatusCode::OK);
        if ban {
            f.state
                .conn()
                .unwrap()
                .execute("UPDATE users SET banned=1 WHERE id=?1", [&f.uid])
                .unwrap();
        } else {
            f.state
                .conn()
                .unwrap()
                .execute("DELETE FROM sessions", [])
                .unwrap();
        }
        assert_eq!(f.poll(&device).await.status, StatusCode::UNAUTHORIZED);
        let count: i64 = f
            .state
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM api_tokens", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn employer_eligibility_is_checked_at_consent_exchange_and_token_use() {
    let f = Fixture::new();
    let device = f.device().await;
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE users SET role='employer',employer_paid=1 WHERE id=?1",
            [&f.uid],
        )
        .unwrap();
    assert_eq!(
        f.decision(&device, true).await.status,
        StatusCode::FORBIDDEN
    );
    f.state.conn().unwrap().execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,status,created_at,updated_at) VALUES('job',?1,'Title','Description',300000,'published',?2,?2)",params![f.uid,now()]).unwrap();
    f.state.conn().unwrap().execute("INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,status,created_at) VALUES('paid',?1,'publish','job',30000,'stripe','paid',?2)",params![f.uid,now()]).unwrap();
    assert_eq!(f.decision(&device, true).await.status, StatusCode::OK);
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE jobs SET status='closed'", [])
        .unwrap();
    assert_eq!(f.poll(&device).await.status, StatusCode::FORBIDDEN);
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE jobs SET status='published'", [])
        .unwrap();
    let result = f.poll(&device).await;
    assert_eq!(result.status, StatusCode::OK);
    let token = result.value["access_token"].as_str().unwrap();
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE jobs SET status='closed'", [])
        .unwrap();
    let who = f.get("/oauth/userinfo", Some(token)).await;
    assert_eq!(who.status, StatusCode::OK);
    assert_eq!(who.value["ai_eligibility"]["eligible"], false);
    assert_eq!(who.value["capabilities"]["mcp"], false);
    assert_eq!(
        f.get("/api/profile", Some(token)).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn administrator_credentials_require_recent_passkey_and_keep_their_namespace() {
    let f = Fixture::new();
    f.make_admin(now() - 601);
    let device = f.device_scope("admin").await;
    assert_eq!(
        f.decision(&device, true).await.status,
        StatusCode::FORBIDDEN
    );
    f.make_admin(now());
    assert_eq!(f.decision(&device, true).await.status, StatusCode::OK);
    f.make_admin(now() - 601);
    assert_eq!(f.poll(&device).await.status, StatusCode::FORBIDDEN);
    f.make_admin(now());
    let result = f.poll(&device).await;
    assert_eq!(result.status, StatusCode::OK, "{}", result.value);
    let token = result.value["access_token"].as_str().unwrap();
    assert!(token.starts_with("hf_admin_"));
    let who = f.get("/oauth/userinfo", Some(token)).await;
    assert_eq!(who.value["admin"], true);
    assert_eq!(who.value["user"], Value::Null);
    assert_eq!(
        f.get("/api/profile", Some(token)).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.form(
            "/oauth/revoke",
            &[("client_id", "hireferry-cli"), ("token", token)]
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        f.get("/oauth/userinfo", Some(token)).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn token_expiry_and_bans_take_effect_without_a_client_restart() {
    for ban in [true, false] {
        let f = Fixture::new();
        let device = f.device().await;
        f.decision(&device, true).await;
        let result = f.poll(&device).await;
        let token = result.value["access_token"].as_str().unwrap();
        if ban {
            f.state
                .conn()
                .unwrap()
                .execute("UPDATE users SET banned=1", [])
                .unwrap();
        } else {
            f.state
                .conn()
                .unwrap()
                .execute("UPDATE api_tokens SET expires_at=?1", [now() - 1])
                .unwrap();
        }
        assert_eq!(
            f.get("/oauth/userinfo", Some(token)).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn protocol_errors_are_json_and_never_echo_the_submitted_credentials() {
    let f = Fixture::new();
    let bad = f
        .form(
            "/oauth/token",
            &[
                ("client_id", "hireferry-cli"),
                ("grant_type", "authorization_code"),
                ("code", "credential-sentinel-not-valid"),
                ("code_verifier", "private-verifier"),
            ],
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad.value["error"], "invalid_grant");
    assert!(!bad.value.to_string().contains("sentinel"));
    assert!(!bad.value.to_string().contains("private-verifier"));
    assert_eq!(bad.headers["cache-control"], "no-store");
    assert_eq!(bad.headers["pragma"], "no-cache");
    assert_eq!(
        f.form("/oauth/device/code", &[("client_id", "wrong")])
            .await
            .value["error"],
        "invalid_client"
    );
    assert_eq!(
        f.form(
            "/oauth/device/code",
            &[("client_id", "hireferry-cli"), ("scope", "superuser")]
        )
        .await
        .value["error"],
        "invalid_scope"
    );
    assert_eq!(
        f.form(
            "/oauth/token",
            &[("client_id", "hireferry-cli"), ("grant_type", "password")]
        )
        .await
        .value["error"],
        "unsupported_grant_type"
    );
}

#[tokio::test]
async fn disabled_cli_login_fails_closed_and_configuration_has_bounds() {
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.cli_auth.enabled = false;
    let f = Fixture::with_config(cfg.clone());
    assert_eq!(
        f.form("/oauth/device/code", &[("client_id", "hireferry-cli")])
            .await
            .status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        f.get("/oauth/device", None).await.status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        f.form(
            "/oauth/revoke",
            &[("client_id", "hireferry-cli"), ("token", "already-removed")]
        )
        .await
        .status,
        StatusCode::OK
    );
    cfg.cli_auth.access_token_minutes = 0;
    assert!(cfg.validate().is_err());
    cfg.cli_auth.access_token_minutes = 15;
    cfg.cli_auth.grant_minutes = 31;
    assert!(cfg.validate().is_err());
    cfg.cli_auth.grant_minutes = 10;
    cfg.cli_auth.poll_seconds = 1;
    assert!(cfg.validate().is_err());
}

#[tokio::test]
async fn metadata_names_public_client_protocols_and_migration_is_repeatable() {
    let f = Fixture::new();
    let meta = f.get("/.well-known/oauth-authorization-server", None).await;
    assert_eq!(meta.status, StatusCode::OK);
    assert_eq!(meta.value["issuer"], "http://localhost:3000");
    assert_eq!(
        meta.value["code_challenge_methods_supported"],
        json!(["S256"])
    );
    assert_eq!(
        meta.value["token_endpoint_auth_methods_supported"],
        json!(["none"])
    );
    let device = f.device().await;
    hireferry::db::migrate(&f.state.conn().unwrap()).unwrap();
    assert_eq!(f.decision(&device, true).await.status, StatusCode::OK);
}

#[tokio::test]
async fn public_oauth_ignores_extensions_and_empty_scope_but_rejects_duplicate_fields() {
    let f = Fixture::new();
    let device = f
        .form(
            "/oauth/device/code",
            &[
                ("client_id", "hireferry-cli"),
                ("scope", ""),
                ("extension", "ignored"),
            ],
        )
        .await;
    assert_eq!(device.status, StatusCode::OK);
    let duplicate = f
        .form(
            "/oauth/device/code",
            &[
                ("client_id", "hireferry-cli"),
                ("client_id", "hireferry-cli"),
            ],
        )
        .await;
    assert_eq!(duplicate.value["error"], "invalid_request");
    let mut request = browser_request();
    request["scope"] = json!("");
    let page = f
        .get(
            &format!("/oauth/authorize?{}&extension=ignored", query(&request)),
            None,
        )
        .await;
    assert_eq!(page.status, StatusCode::OK);
    let duplicate = f
        .get(
            &format!(
                "/oauth/authorize?{}&client_id=hireferry-cli",
                query(&request)
            ),
            None,
        )
        .await;
    assert_eq!(duplicate.status, StatusCode::BAD_REQUEST);
    assert_eq!(f.decision(&device.value, true).await.status, StatusCode::OK);
    let result = f
        .form(
            "/oauth/token",
            &[
                ("client_id", "hireferry-cli"),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device.value["device_code"].as_str().unwrap()),
                ("extension", "ignored"),
            ],
        )
        .await;
    assert_eq!(result.status, StatusCode::OK);
    let revoke = f
        .form(
            "/oauth/revoke",
            &[
                ("client_id", "hireferry-cli"),
                ("token", result.value["access_token"].as_str().unwrap()),
                ("token_type_hint", "unknown-extension"),
                ("extension", "ignored"),
            ],
        )
        .await;
    assert_eq!(revoke.status, StatusCode::OK);
    let mut strict = browser_request();
    strict["extension"] = json!("not allowed in consent");
    assert_eq!(
        f.json("/api/cli/authorization", strict).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn basic_login_works_without_ai_eligibility_and_cannot_access_business_apis() {
    for role in ["unassigned", "employer"] {
        let f = Fixture::new();
        f.state
            .conn()
            .unwrap()
            .execute(
                "UPDATE users SET role=?1,employer_paid=0 WHERE id=?2",
                params![role, f.uid],
            )
            .unwrap();
        let result = f.login("").await;
        let token = result["access_token"].as_str().unwrap();
        assert!(token.starts_with("hf_cli_"));
        assert_eq!(result["scope"], "identity");
        let who = f.get("/oauth/userinfo", Some(token)).await;
        assert_eq!(who.status, StatusCode::OK, "{}", who.value);
        assert_eq!(who.value["user"]["role"], role);
        assert_eq!(who.value["ai_eligibility"]["eligible"], false);
        assert_eq!(
            who.value["capabilities"],
            json!({"identity":true,"mcp":false,"admin":false})
        );
        assert!(!who.value.to_string().contains("Private resume sentinel"));
        assert_eq!(
            f.get("/api/profile", Some(token)).await.status,
            StatusCode::FORBIDDEN
        );
        let mcp = f
            .request(
                "POST",
                "/mcp",
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string(),
                "application/json",
                false,
                Some(token),
            )
            .await;
        assert_eq!(mcp.status, StatusCode::FORBIDDEN);
        let business_tokens: i64 = f
            .state
            .conn()
            .unwrap()
            .query_row(
                "SELECT (SELECT count(*) FROM api_tokens)+(SELECT count(*) FROM admin_api_tokens)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(business_tokens, 0);
        let refreshed = f
            .refresh(result["refresh_token"].as_str().unwrap(), None)
            .await;
        assert_eq!(refreshed.status, StatusCode::OK);
        assert!(
            refreshed.value["access_token"]
                .as_str()
                .unwrap()
                .starts_with("hf_cli_")
        );
    }
}

#[tokio::test]
async fn administrator_default_login_is_identity_only_even_with_an_old_passkey_session() {
    let f = Fixture::new();
    f.make_admin(now() - 601);
    let result = f.login("").await;
    let token = result["access_token"].as_str().unwrap();
    assert!(token.starts_with("hf_cli_"));
    let who = f.get("/oauth/userinfo", Some(token)).await;
    assert_eq!(who.status, StatusCode::OK);
    assert_eq!(who.value["admin"], true);
    assert_eq!(
        who.value["capabilities"],
        json!({"identity":true,"mcp":false,"admin":false})
    );
    assert_eq!(
        f.get("/api/admin/overview", Some(token)).await.status,
        StatusCode::FORBIDDEN
    );
    let count: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM admin_api_tokens", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let admin = f.device_scope("admin").await;
    assert_eq!(f.decision(&admin, true).await.status, StatusCode::FORBIDDEN);
    let member = f.device_scope("mcp").await;
    assert_eq!(
        f.decision(&member, true).await.status,
        StatusCode::FORBIDDEN
    );
    let refreshed = f
        .refresh(result["refresh_token"].as_str().unwrap(), None)
        .await;
    assert_eq!(refreshed.status, StatusCode::OK);
    assert_eq!(refreshed.value["scope"], "identity");
}

#[tokio::test]
async fn refresh_rotates_expired_access_without_sliding_expiry_and_reuse_revokes_the_family() {
    let f = Fixture::new();
    let result = f.login("mcp").await;
    let old_access = result["access_token"].as_str().unwrap();
    let old_refresh = result["refresh_token"].as_str().unwrap();
    let deadline: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT expires_at FROM cli_authorizations", [], |r| {
            r.get(0)
        })
        .unwrap();
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE cli_tokens SET expires_at=?1", [now() - 1])
        .unwrap();
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE api_tokens SET expires_at=?1", [now() - 1])
        .unwrap();
    assert_eq!(
        f.get("/oauth/userinfo", Some(old_access)).await.status,
        StatusCode::UNAUTHORIZED
    );
    let next = f.refresh(old_refresh, Some("")).await;
    assert_eq!(next.status, StatusCode::OK, "{}", next.value);
    let new_access = next.value["access_token"].as_str().unwrap();
    let new_refresh = next.value["refresh_token"].as_str().unwrap();
    assert_ne!(old_access, new_access);
    assert_ne!(old_refresh, new_refresh);
    assert_eq!(next.value["scope"], "identity mcp");
    assert_eq!(
        f.get("/oauth/userinfo", Some(new_access)).await.status,
        StatusCode::OK
    );
    let (same_deadline, used): (i64, bool) = f.state.conn().unwrap().query_row(
        "SELECT f.expires_at,r.used_at IS NOT NULL FROM cli_authorizations f JOIN cli_refresh_tokens r ON r.family_id=f.id WHERE r.token_hash=?1", [auth::hash_token(old_refresh)], |r| Ok((r.get(0)?,r.get(1)?))
    ).unwrap();
    assert_eq!(same_deadline, deadline);
    assert!(used);
    let hashes: Vec<String> = f
        .state
        .conn()
        .unwrap()
        .prepare("SELECT token_hash FROM cli_refresh_tokens")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(hashes.len(), 2);
    assert!(hashes.contains(&auth::hash_token(new_refresh)));
    assert!(!hashes.iter().any(|h| h == old_refresh || h == new_refresh));
    assert_eq!(
        f.refresh(old_refresh, None).await.value["error"],
        "invalid_grant"
    );
    assert_eq!(
        f.get("/oauth/userinfo", Some(new_access)).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.get("/api/profile", Some(new_access)).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.refresh(new_refresh, None).await.value["error"],
        "invalid_grant"
    );
    let active: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM api_tokens WHERE revoked=0", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(active, 0);
}

#[tokio::test]
async fn refresh_can_narrow_scope_but_never_expand_or_recover_removed_permission() {
    let f = Fixture::new();
    let basic = f.login("identity").await;
    let refresh = basic["refresh_token"].as_str().unwrap();
    assert_eq!(
        f.refresh(refresh, Some("mcp")).await.value["error"],
        "invalid_scope"
    );
    assert_eq!(
        f.get("/oauth/userinfo", basic["access_token"].as_str())
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        f.refresh(refresh, None).await.status,
        StatusCode::OK,
        "Invalid scope must not consume a refresh token"
    );
    let member = f.login("mcp identity").await;
    let reduced = f
        .refresh(member["refresh_token"].as_str().unwrap(), Some("identity"))
        .await;
    assert_eq!(reduced.status, StatusCode::OK);
    let access = reduced.value["access_token"].as_str().unwrap();
    assert!(access.starts_with("hf_cli_"));
    assert_eq!(reduced.value["scope"], "identity");
    assert_eq!(
        f.get("/api/profile", Some(access)).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.get("/oauth/userinfo", member["access_token"].as_str())
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.refresh(
            reduced.value["refresh_token"].as_str().unwrap(),
            Some("mcp")
        )
        .await
        .value["error"],
        "invalid_scope"
    );
    let conflicting = f
        .form(
            "/oauth/device/code",
            &[("client_id", "hireferry-cli"), ("scope", "mcp admin")],
        )
        .await;
    assert_eq!(conflicting.value["error"], "invalid_scope");
}

#[tokio::test]
async fn refresh_rechecks_bans_and_current_member_eligibility_and_revokes_failed_families() {
    for ban in [false, true] {
        let f = Fixture::new();
        let result = f.login("mcp").await;
        if ban {
            f.state
                .conn()
                .unwrap()
                .execute("UPDATE users SET banned=1", [])
                .unwrap();
        } else {
            f.state
                .conn()
                .unwrap()
                .execute("UPDATE users SET role='employer',employer_paid=0", [])
                .unwrap();
        }
        let denied = f
            .refresh(result["refresh_token"].as_str().unwrap(), None)
            .await;
        assert_eq!(
            denied.status,
            if ban {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::FORBIDDEN
            }
        );
        f.state
            .conn()
            .unwrap()
            .execute("UPDATE users SET banned=0,role='seeker'", [])
            .unwrap();
        assert_eq!(
            f.get("/oauth/userinfo", result["access_token"].as_str())
                .await
                .status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            f.refresh(result["refresh_token"].as_str().unwrap(), None)
                .await
                .value["error"],
            "invalid_grant"
        );
    }
}

#[tokio::test]
async fn competing_refreshes_detect_reuse_and_revoke_the_winning_response() {
    let f = Fixture::new();
    let result = f.login("identity").await;
    let refresh = result["refresh_token"].as_str().unwrap();
    let (a, b) = tokio::join!(f.refresh(refresh, None), f.refresh(refresh, None));
    let (accepted, rejected) = if a.status == StatusCode::OK {
        (a, b)
    } else {
        (b, a)
    };
    assert_eq!(accepted.status, StatusCode::OK);
    assert_eq!(rejected.value["error"], "invalid_grant");
    assert_eq!(
        f.get("/oauth/userinfo", accepted.value["access_token"].as_str())
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.refresh(accepted.value["refresh_token"].as_str().unwrap(), None)
            .await
            .value["error"],
        "invalid_grant"
    );
}

#[tokio::test]
async fn absolute_family_expiry_and_refresh_revocation_prevent_new_tokens() {
    let f = Fixture::new();
    let expired = f.login("identity").await;
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE cli_authorizations SET expires_at=?1", [now() - 1])
        .unwrap();
    assert_eq!(
        f.refresh(expired["refresh_token"].as_str().unwrap(), None)
            .await
            .value["error"],
        "invalid_grant"
    );
    assert_eq!(
        f.get("/oauth/userinfo", expired["access_token"].as_str())
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let result = f.login("identity").await;
    let refresh = result["refresh_token"].as_str().unwrap();
    for _ in 0..2 {
        assert_eq!(
            f.form(
                "/oauth/revoke",
                &[("client_id", "hireferry-cli"), ("token", refresh)]
            )
            .await
            .status,
            StatusCode::OK
        );
    }
    assert_eq!(
        f.refresh(refresh, None).await.value["error"],
        "invalid_grant"
    );
    assert_eq!(
        f.get("/oauth/userinfo", result["access_token"].as_str())
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn existing_member_and_administrator_token_revocation_revoke_rotated_families() {
    for admin in [false, true] {
        let f = Fixture::new();
        if admin {
            f.make_admin(now());
        }
        let result = f.login(if admin { "admin" } else { "mcp" }).await;
        let id: String = f
            .state
            .conn()
            .unwrap()
            .query_row("SELECT token_id FROM cli_tokens", [], |r| r.get(0))
            .unwrap();
        let next = f
            .refresh(result["refresh_token"].as_str().unwrap(), None)
            .await;
        assert_eq!(next.status, StatusCode::OK);
        let route = if admin {
            "/api/admin/tokens"
        } else {
            "/api/tokens"
        };
        let listed = f.get(route, None).await;
        assert_eq!(listed.status, StatusCode::OK);
        assert_eq!(listed.value["items"].as_array().unwrap().len(), 1);
        assert_eq!(
            f.delete(&format!("{route}/{id}")).await.status,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            f.get("/oauth/userinfo", next.value["access_token"].as_str())
                .await
                .status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            f.refresh(next.value["refresh_token"].as_str().unwrap(), None)
                .await
                .value["error"],
            "invalid_grant"
        );
    }
}

#[tokio::test]
async fn browser_connections_are_cookie_only_owned_by_identity_and_revoke_without_ai_eligibility() {
    let f = Fixture::new();
    let result = f.login("mcp").await;
    let (family, token_id): (String, String) = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT family_id,token_id FROM cli_tokens", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    let other = uuid::Uuid::new_v4().to_string();
    f.state.conn().unwrap().execute("INSERT INTO users(id,email,role,created_at,updated_at) VALUES(?1,'other@example.invalid','seeker',?2,?2)", params![other,now()]).unwrap();
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE sessions SET user_id=?1", [&other])
        .unwrap();
    let list = f.get("/api/cli/connections", None).await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.value["items"], json!([]));
    assert_eq!(
        f.delete(&format!("/api/cli/connections/{family}"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.delete(&format!("/api/tokens/{token_id}")).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        f.get("/oauth/userinfo", result["access_token"].as_str())
            .await
            .status,
        StatusCode::OK,
        "Another member's legacy token delete must not revoke this family"
    );
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE sessions SET user_id=?1", [&f.uid])
        .unwrap();
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE users SET role='employer',employer_paid=0 WHERE id=?1",
            [&f.uid],
        )
        .unwrap();
    let list = f.get("/api/cli/connections", None).await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.value["items"][0]["id"], family);
    assert_eq!(list.value["items"][0]["scope"], "identity mcp");
    assert!(!list.value.to_string().contains("hf_"));
    assert_eq!(
        f.get("/api/cli/connections", result["access_token"].as_str())
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let route = format!("/api/cli/connections/{family}");
    assert_eq!(
        f.request(
            "DELETE",
            &route,
            String::new(),
            "application/json",
            false,
            None
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    for _ in 0..2 {
        assert_eq!(f.delete(&route).await.status, StatusCode::NO_CONTENT);
    }
    assert_eq!(
        f.refresh(result["refresh_token"].as_str().unwrap(), None)
            .await
            .value["error"],
        "invalid_grant"
    );
}

#[tokio::test]
async fn disabling_new_login_blocks_refresh_but_keeps_identity_and_management_available() {
    let mut f = Fixture::new();
    let result = f.login("identity").await;
    f.reconfigure(|cfg| cfg.cli_auth.enabled = false);
    assert_eq!(
        f.refresh(result["refresh_token"].as_str().unwrap(), None)
            .await
            .status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        f.get("/oauth/userinfo", result["access_token"].as_str())
            .await
            .status,
        StatusCode::OK
    );
    let list = f.get("/api/cli/connections", None).await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(
        f.get("/oauth/connections", None).await.status,
        StatusCode::OK
    );
    assert_eq!(
        f.delete(&format!(
            "/api/cli/connections/{}",
            list.value["items"][0]["id"].as_str().unwrap()
        ))
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        f.get("/oauth/userinfo", result["access_token"].as_str())
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn configured_client_scopes_lifetimes_and_connection_limit_are_enforced() {
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.cli_auth.client_id = "private-hfy".into();
    cfg.cli_auth.allowed_scopes = vec!["identity".into()];
    cfg.cli_auth.access_token_minutes = 2;
    cfg.cli_auth.refresh_token_days = 2;
    cfg.cli_auth.code_seconds = 60;
    cfg.cli_auth.grant_minutes = 3;
    cfg.cli_auth.poll_seconds = 7;
    cfg.cli_auth.max_active_tokens = 1;
    let f = Fixture::with_config(cfg);
    assert_eq!(
        f.form("/oauth/device/code", &[("client_id", "hireferry-cli")])
            .await
            .value["error"],
        "invalid_client"
    );
    assert_eq!(
        f.form(
            "/oauth/device/code",
            &[("client_id", "private-hfy"), ("scope", "mcp")]
        )
        .await
        .value["error"],
        "invalid_scope"
    );
    let device = f.device_scope("").await;
    assert_eq!(device["expires_in"], 180);
    assert_eq!(device["interval"], 7);
    let detail = f
        .get(
            &format!(
                "/api/cli/authorization?user_code={}",
                device["user_code"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(detail.value["client_id"], "private-hfy");
    assert_eq!(detail.value["token_expires_in"], 120);
    assert_eq!(detail.value["refresh_expires_in"], 172800);
    assert_eq!(f.decision(&device, true).await.status, StatusCode::OK);
    let first = f.poll(&device).await;
    assert!((119..=120).contains(&first.value["expires_in"].as_i64().unwrap()));
    assert!((172799..=172800).contains(&first.value["refresh_expires_in"].as_i64().unwrap()));
    let second = f.device_scope("identity").await;
    assert_eq!(f.decision(&second, true).await.status, StatusCode::OK);
    assert_eq!(f.poll(&second).await.status, StatusCode::CONFLICT);
    f.form(
        "/oauth/revoke",
        &[
            ("client_id", "private-hfy"),
            ("token", first.value["access_token"].as_str().unwrap()),
        ],
    )
    .await;
    assert_eq!(f.poll(&second).await.status, StatusCode::OK);
    let mut oauth = browser_request();
    oauth["client_id"] = json!("private-hfy");
    oauth["scope"] = json!("identity");
    let code = f.browser_code(&oauth).await;
    let seconds: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT expires_at-created_at FROM cli_grants WHERE secret_hash=?1",
            [auth::hash_token(&code)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(seconds, 60);
    let meta = f.get("/.well-known/oauth-authorization-server", None).await;
    assert_eq!(meta.value["client_id"], "private-hfy");
    assert_eq!(meta.value["scopes_supported"], json!(["identity"]));
}

#[tokio::test]
async fn configurable_rate_code_attempt_and_pending_limits_fail_closed() {
    let mut f = Fixture::new();
    f.reconfigure(|cfg| cfg.cli_auth.rate_limit_requests = 1);
    f.device_scope("identity").await;
    assert_eq!(
        f.form("/oauth/device/code", &[("client_id", "hireferry-cli")])
            .await
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
    let mut f = Fixture::new();
    f.reconfigure(|cfg| cfg.cli_auth.max_user_code_attempts = 1);
    assert_eq!(
        f.get("/api/cli/authorization?user_code=BBBB-BBBB", None)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.get("/api/cli/authorization?user_code=BBBB-BBBB", None)
            .await
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
    let mut f = Fixture::new();
    f.reconfigure(|cfg| cfg.cli_auth.max_pending_grants = 1);
    f.device_scope("identity").await;
    assert_eq!(
        f.form("/oauth/device/code", &[("client_id", "hireferry-cli")])
            .await
            .status,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn version_eight_upgrade_preserves_version_six_and_seven_accounts_and_manual_tokens() {
    for old_version in [6, 7] {
        let f = Fixture::new();
        let manual = "hf_manual_existing_token";
        {
            let db = f.state.conn().unwrap();
            db.execute_batch("DROP TRIGGER cli_account_suspended; DROP TABLE cli_refresh_tokens; DROP TABLE cli_tokens; DROP TABLE cli_grants; DROP TABLE cli_authorizations;").unwrap();
            if old_version == 6 {
                db.execute_batch("DROP TRIGGER hg_account_suspended; DROP TABLE hg_sessions; DROP TABLE hg_grants; DROP TABLE hg_requests; DROP TABLE hg_nonces; DROP TABLE hg_limits;").unwrap();
            }
            db.pragma_update(None, "user_version", old_version).unwrap();
            db.execute("INSERT INTO api_tokens(id,user_id,name,token_hash,created_at,expires_at) VALUES('old-manual',?1,'Before migration',?2,?3,?4)", params![f.uid,auth::hash_token(manual),now(),now()+3600]).unwrap();
            hireferry::db::migrate(&db).unwrap();
            let version: i64 = db
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap();
            assert_eq!(version, 16);
            let name: String = db
                .query_row(
                    "SELECT name FROM api_tokens WHERE id='old-manual'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(name, "Before migration");
            let hg_tables: i64 = db.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('hg_requests','hg_grants','hg_sessions','hg_nonces','hg_limits')", [], |r| r.get(0)).unwrap();
            assert_eq!(hg_tables, 5);
            hireferry::db::migrate(&db).unwrap();
        }
        assert_eq!(
            f.get("/api/profile", Some(manual)).await.status,
            StatusCode::OK
        );
        let result = f.login("identity").await;
        f.form(
            "/oauth/revoke",
            &[
                ("client_id", "hireferry-cli"),
                ("token", result["access_token"].as_str().unwrap()),
            ],
        )
        .await;
        assert_eq!(
            f.get("/api/profile", Some(manual)).await.status,
            StatusCode::OK
        );
    }
}

#[test]
fn failed_version_eight_upgrade_rolls_back_cli_schema_and_keeps_humangrant_version_seven() {
    let f = Fixture::new();
    let db = f.state.conn().unwrap();
    db.execute_batch("DROP TRIGGER cli_account_suspended; DROP TABLE cli_refresh_tokens; DROP TABLE cli_tokens; DROP TABLE cli_grants; DROP TABLE cli_authorizations; PRAGMA user_version=7; CREATE TABLE cli_refresh_tokens(sentinel TEXT);").unwrap();
    assert!(hireferry::db::migrate(&db).is_err());
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 7);
    let partial_tables: i64 = db.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('cli_authorizations','cli_grants','cli_tokens')", [], |r| r.get(0)).unwrap();
    assert_eq!(partial_tables, 0);
    let accounts: i64 = db
        .query_row("SELECT count(*) FROM users", [], |r| r.get(0))
        .unwrap();
    assert_eq!(accounts, 1);
}

#[tokio::test]
async fn restoring_a_suspended_account_never_restores_old_identity_credentials() {
    let f = Fixture::new();
    let result = f.login("identity").await;
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id=?1", [&f.uid])
        .unwrap();
    // No token request occurs during suspension; invalidation must persist in
    // the database instead of relying on an online request noticing the ban.
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=0 WHERE id=?1", [&f.uid])
        .unwrap();
    assert_eq!(
        f.get("/oauth/userinfo", result["access_token"].as_str())
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.refresh(result["refresh_token"].as_str().unwrap(), None)
            .await
            .value["error"],
        "invalid_grant"
    );
}

#[tokio::test]
async fn changed_client_id_disables_old_business_capabilities_but_allows_identity_and_revocation() {
    let mut f = Fixture::new();
    let result = f.login("mcp").await;
    f.reconfigure(|cfg| cfg.cli_auth.client_id = "new-public-client".into());
    let who = f
        .get("/oauth/userinfo", result["access_token"].as_str())
        .await;
    assert_eq!(who.status, StatusCode::OK);
    assert_eq!(
        who.value["capabilities"],
        json!({"identity":true,"mcp":false,"admin":false})
    );
    assert_eq!(
        f.get("/api/profile", result["access_token"].as_str())
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.refresh(result["refresh_token"].as_str().unwrap(), None)
            .await
            .value["error"],
        "invalid_grant"
    );
    assert_eq!(
        f.form(
            "/oauth/revoke",
            &[
                ("client_id", "hireferry-cli"),
                ("token", result["refresh_token"].as_str().unwrap())
            ]
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        f.get("/oauth/userinfo", result["access_token"].as_str())
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn ineligible_consent_remains_visible_and_can_be_denied_without_weakening_approval() {
    for (role, admin, scope, recent_required) in [
        ("unassigned", false, "mcp", false),
        ("employer", false, "mcp", false),
        ("seeker", false, "admin", false),
        ("seeker", true, "mcp", false),
        ("seeker", true, "admin", true),
    ] {
        let f = Fixture::new();
        f.state
            .conn()
            .unwrap()
            .execute("UPDATE users SET role=?1 WHERE id=?2", params![role, f.uid])
            .unwrap();
        if admin {
            f.make_admin(now() - 601);
        }
        let device = f.device_scope(scope).await;
        let details = f
            .get(
                &format!(
                    "/api/cli/authorization?user_code={}",
                    device["user_code"].as_str().unwrap()
                ),
                None,
            )
            .await;
        assert_eq!(details.status, StatusCode::OK, "{}", details.value);
        assert_eq!(details.value["can_approve"], false);
        assert!(
            details.value["approval_error"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        );
        assert_eq!(details.value["requires_recent_auth"], recent_required);
        assert_eq!(details.value["scope"], format!("identity {scope}"));
        assert_eq!(details.value["admin"], admin);
        if !admin {
            assert_eq!(details.value["user"]["id"], f.uid);
        }
        assert_eq!(
            f.decision(&device, true).await.status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(f.decision(&device, false).await.status, StatusCode::OK);
        assert_eq!(f.poll(&device).await.value["error"], "access_denied");
        let count: i64 = f
            .state
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM cli_authorizations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn consent_recovery_does_not_mask_invalid_sessions_requests_or_database_failures() {
    let f = Fixture::new();
    let device = f.device_scope("mcp").await;
    let path = format!(
        "/api/cli/authorization?user_code={}",
        device["user_code"].as_str().unwrap()
    );
    assert_eq!(
        f.get("/api/cli/authorization?user_code=invalid", None)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE users SET role='employer',employer_paid=1 WHERE id=?1",
            [&f.uid],
        )
        .unwrap();
    // Corrupt only this isolated test fixture to make the eligibility query
    // fail. Operational failures must not become a misleading consent denial.
    f.state
        .conn()
        .unwrap()
        .execute_batch("DROP TABLE orders;")
        .unwrap();
    assert_eq!(
        f.get(&path, None).await.status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    f.state
        .conn()
        .unwrap()
        .execute("DELETE FROM sessions", [])
        .unwrap();
    assert_eq!(f.get(&path, None).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn cli_and_humangrant_credentials_with_the_same_client_id_remain_separate() {
    let f = Fixture::new();
    let human_token = URL_SAFE_NO_PAD.encode([19u8; 32]);
    let permissions = vec![Permission {
        action: "profile.read".into(),
        resource: "self".into(),
    }];
    let encoded = serde_json::to_string(&permissions).unwrap();
    let human = GrantContext {
        grant_id: "human-grant".into(),
        user_id: f.uid.clone(),
        actor_id: "ed25519:isolated-fixture".into(),
        client_id: "hireferry-cli".into(),
        token_hash: URL_SAFE_NO_PAD.encode(Sha256::digest(human_token.as_bytes())),
        permissions,
        expires_at: now() + 3600,
    };
    {
        let db = f.state.conn().unwrap();
        db.execute("INSERT INTO hg_requests(id,human_link,actor_id,public_key,client_id,purpose,requested_permissions,phase,created_at,status_updated_at,expires_at,requested_grant_expires_at) VALUES('human-request','human-link',?1,'synthetic-key','hireferry-cli','Protocol isolation regression',?2,'succeeded',?3,?3,?4,?4)", params![human.actor_id,encoded,now(),human.expires_at]).unwrap();
        db.execute("INSERT INTO hg_grants(id,request_id,user_id,actor_id,client_id,permissions,created_at,expires_at,audit_id) VALUES(?1,'human-request',?2,?3,'hireferry-cli',?4,?5,?6,'human-audit')", params![human.grant_id,f.uid,human.actor_id,encoded,now(),human.expires_at]).unwrap();
        db.execute("INSERT INTO hg_sessions(request_id,grant_id,token_hash,actor_id,created_at) VALUES('human-request',?1,?2,?3,?4)", params![human.grant_id,human.token_hash,human.actor_id,now()]).unwrap();
        assert!(humangrant::validate_grant(&db, &human).is_ok());
    }
    for path in ["/oauth/userinfo", "/api/profile"] {
        assert_eq!(
            f.get(path, Some(&human_token)).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        f.refresh(&human_token, None).await.value["error"],
        "invalid_grant"
    );
    assert_eq!(
        f.form(
            "/oauth/revoke",
            &[("client_id", "hireferry-cli"), ("token", &human_token)],
        )
        .await
        .status,
        StatusCode::OK
    );
    assert!(humangrant::validate_grant(&f.state.conn().unwrap(), &human).is_ok());

    let cli = f.login("identity").await;
    let access = cli["access_token"].as_str().unwrap();
    let mut wrong_namespace = human.clone();
    wrong_namespace.token_hash = URL_SAFE_NO_PAD.encode(Sha256::digest(access.as_bytes()));
    assert!(humangrant::validate_grant(&f.state.conn().unwrap(), &wrong_namespace).is_err());
    assert_eq!(
        f.get("/v1/session", Some(access)).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.get("/api/humangrant/grants", Some(access)).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.form(
            "/oauth/revoke",
            &[("client_id", "hireferry-cli"), ("token", access)],
        )
        .await
        .status,
        StatusCode::OK
    );
    assert!(humangrant::validate_grant(&f.state.conn().unwrap(), &human).is_ok());

    // Both suspension triggers coexist and revoke their own credential families.
    let active_cli = f.login("identity").await;
    {
        let db = f.state.conn().unwrap();
        db.execute("UPDATE users SET banned=1 WHERE id=?1", [&f.uid])
            .unwrap();
        db.execute("UPDATE users SET banned=0 WHERE id=?1", [&f.uid])
            .unwrap();
        assert!(humangrant::validate_grant(&db, &human).is_err());
    }
    assert_eq!(
        f.get("/oauth/userinfo", active_cli["access_token"].as_str())
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.refresh(active_cli["refresh_token"].as_str().unwrap(), None)
            .await
            .value["error"],
        "invalid_grant"
    );
}

#[tokio::test]
async fn signed_mcp_adapter_preserves_explicit_cli_member_and_admin_scope_boundaries() {
    for scope in ["identity", "mcp", "admin"] {
        let f = Fixture::new();
        if scope == "admin" {
            f.make_admin(now());
        }
        let cli = f.login(scope).await;
        let access = cli["access_token"].as_str().unwrap();
        let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string();
        let response = f
            .request(
                "POST",
                "/mcp",
                body.clone(),
                "application/json",
                false,
                Some(access),
            )
            .await;
        if scope == "identity" {
            assert_eq!(response.status, StatusCode::FORBIDDEN);
        } else {
            assert_eq!(response.status, StatusCode::OK, "{}", response.value);
            let tools = response.value["result"]["tools"].as_array().unwrap();
            let expected = if scope == "admin" {
                "admin_overview"
            } else {
                "get_profile"
            };
            assert!(tools.iter().any(|tool| tool["name"] == expected));
            if scope == "mcp" {
                assert!(
                    tools
                        .iter()
                        .all(|tool| !tool["name"].as_str().unwrap().starts_with("admin_"))
                );
            }
        }
        // The signed protocol must never fall back to the valid Bearer credential.
        let response = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .header("authorization", format!("HumanGrant {access}"))
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
