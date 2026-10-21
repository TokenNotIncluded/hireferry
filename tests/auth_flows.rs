//! Authentication tests call the real HTTP handlers and SQLite implementation.
//! No external email or Google requests are made.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use hireferry::{AppState, auth, config::Config};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tower::ServiceExt;
use webauthn_authenticator_rs::{WebauthnAuthenticator, softpasskey::SoftPasskey};

struct Client {
    app: Router,
    cookies: BTreeMap<String, String>,
    csrf: Option<String>,
}
impl Client {
    fn new(state: &AppState) -> Self {
        Self {
            app: auth::routes().with_state(state.clone()),
            cookies: BTreeMap::new(),
            csrf: None,
        }
    }
    async fn request(&mut self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "http://localhost:3000");
        if !self.cookies.is_empty() {
            request = request.header(
                header::COOKIE,
                self.cookies
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if let Some(csrf) = &self.csrf {
            request = request.header("x-csrf-token", csrf);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        for value in response.headers().get_all(header::SET_COOKIE).iter() {
            let cookie = value.to_str().unwrap();
            assert!(cookie.contains("HttpOnly"));
            assert!(cookie.contains("SameSite=Lax"));
            let (key, value) = cookie.split(';').next().unwrap().split_once('=').unwrap();
            if value.is_empty() {
                self.cookies.remove(key);
            } else {
                self.cookies.insert(key.to_owned(), value.to_owned());
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if let Some(csrf) = value["csrf"].as_str() {
            self.csrf = Some(csrf.to_owned());
        }
        (status, value)
    }
    async fn sign_in(&mut self, email: &str) -> Value {
        let (status, start) = self
            .request("POST", "/api/auth/email/start", json!({"email":email}))
            .await;
        assert_eq!(status, StatusCode::OK, "{start}");
        let (status, value) = self
            .request(
                "POST",
                "/api/auth/email/verify",
                json!({"email":email,"code":start["dev_code"]}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
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

#[tokio::test]
async fn google_redirect_uses_exact_callback_route_and_rejects_missing_state() {
    let state = AppState::new(Config {
        development: true,
        database: ":memory:".into(),
        public_url: "http://localhost:3000/".into(),
        google: hireferry::config::Google {
            enabled: true,
            client_id: "test-client.apps.googleusercontent.com".into(),
            client_secret: "test-secret-never-sent".into(),
        },
        ..Default::default()
    })
    .unwrap();
    let app = hireferry::app(state.clone());
    let start = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/auth/google/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::SEE_OTHER);
    let authorization =
        url::Url::parse(start.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    let query: BTreeMap<_, _> = authorization.query_pairs().into_owned().collect();
    assert_eq!(authorization.host_str(), Some("accounts.google.com"));
    assert_eq!(
        query["redirect_uri"],
        "http://localhost:3000/auth/google/callback"
    );
    assert!(!query["state"].is_empty());
    assert_eq!(query["code_challenge_method"], "S256");
    let cookie = start
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .find(|value| value.starts_with("hf_google_challenge="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();

    // A real callback route must reject CSRF before exchanging this fake code.
    // The complete app router also detects an accidental static-page fallback.
    let callback = app
        .oneshot(
            Request::builder()
                .uri("/auth/google/callback?code=never-exchange-this-code")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(callback.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(callback.headers()[header::CONTENT_TYPE], "application/json");
    let bytes = callback.into_body().collect().await.unwrap().to_bytes();
    let error: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        error
            .to_string()
            .contains("Google sign-in verification failed")
    );
    assert_eq!(
        state
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM sessions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn email_auth_creates_private_user_hashes_session_and_logs_out_with_csrf() {
    let state = state();
    let mut client = Client::new(&state);
    let value = client.sign_in("seeker@example.test").await;
    assert_eq!(value["user"]["role"], "unassigned");
    assert_eq!(value["user"]["visible"], false);
    assert_eq!(value["user"]["public_email"], "");
    assert!(value["user"].get("google_sub").is_none());
    {
        let db = state.conn().unwrap();
        let stored: String = db
            .query_row("SELECT token_hash FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, auth::hash_token(&client.cookies["hf_session"]));
        assert_ne!(stored, client.cookies["hf_session"]);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM auth_challenges", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    client.csrf = None;
    assert_eq!(
        client
            .request("POST", "/api/auth/logout", json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, session) = client.request("GET", "/api/session", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["user"]["id"], value["user"]["id"]);
    assert_eq!(
        client
            .request("POST", "/api/auth/logout", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    assert!(client.request("GET", "/api/session", json!({})).await.1["user"].is_null());
}

#[tokio::test]
async fn email_code_is_browser_bound_single_use_and_limits_wrong_guesses() {
    let state = state();
    let mut a = Client::new(&state);
    let mut b = Client::new(&state);
    let (_, start) = a
        .request(
            "POST",
            "/api/auth/email/start",
            json!({"email":"bound@example.test"}),
        )
        .await;
    let code = start["dev_code"].as_str().unwrap();
    assert_eq!(
        b.request(
            "POST",
            "/api/auth/email/verify",
            json!({"email":"bound@example.test","code":code})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let wrong = if code == "00000000" {
        "11111111"
    } else {
        "00000000"
    };
    for _ in 0..5 {
        assert_eq!(
            a.request(
                "POST",
                "/api/auth/email/verify",
                json!({"email":"bound@example.test","code":wrong})
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        a.request(
            "POST",
            "/api/auth/email/verify",
            json!({"email":"bound@example.test","code":code})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        state
            .conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );

    let (_, start) = b
        .request(
            "POST",
            "/api/auth/email/start",
            json!({"email":"once@example.test"}),
        )
        .await;
    let payload = json!({"email":"once@example.test","code":start["dev_code"]});
    let previous = b.cookies.clone();
    assert_eq!(
        b.request("POST", "/api/auth/email/verify", payload.clone())
            .await
            .0,
        StatusCode::OK
    );
    b.cookies = previous;
    assert_eq!(
        b.request("POST", "/api/auth/email/verify", payload).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn banned_users_lose_existing_sessions_and_expired_codes_fail() {
    let state = state();
    let mut client = Client::new(&state);
    let user = client.sign_in("suspend@example.test").await;
    state
        .conn()
        .unwrap()
        .execute(
            "UPDATE users SET banned=1 WHERE id=?1",
            [user["user"]["id"].as_str().unwrap()],
        )
        .unwrap();
    assert!(client.request("GET", "/api/session", json!({})).await.1["user"].is_null());
    let (_, start) = client
        .request(
            "POST",
            "/api/auth/email/start",
            json!({"email":"expired@example.test"}),
        )
        .await;
    state
        .conn()
        .unwrap()
        .execute("UPDATE auth_challenges SET expires_at=0", [])
        .unwrap();
    assert_eq!(
        client
            .request(
                "POST",
                "/api/auth/email/verify",
                json!({"email":"expired@example.test","code":start["dev_code"]})
            )
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn bootstrap_password_is_single_use_and_does_not_grant_admin_access() {
    let state = state();
    let mut client = Client::new(&state);
    let secret = auth::generate_bootstrap(&state.conn().unwrap()).unwrap();
    let (status, body) = client
        .request(
            "POST",
            "/api/admin/auth/bootstrap",
            json!({"password":secret}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["bootstrap"], true);
    assert_eq!(body["admin"], false);
    assert_eq!(
        client
            .request("GET", "/api/admin/passkeys", json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .request(
                "POST",
                "/api/admin/auth/bootstrap",
                json!({"password":secret})
            )
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, body) = client
        .request(
            "POST",
            "/api/admin/passkeys/register/start",
            json!({"name":"My passkey"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["options"]["publicKey"]["authenticatorSelection"]["userVerification"],
        "required"
    );
    assert_eq!(body["options"]["publicKey"]["rp"]["id"], "localhost");
    let previous = client.cookies["hf_session"].clone();
    auth::generate_bootstrap(&state.conn().unwrap()).unwrap();
    assert!(client.request("GET", "/api/session", json!({})).await.1["user"].is_null());
    assert_eq!(
        state
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE token_hash=?1",
                [auth::hash_token(&previous)],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn auth_requires_same_origin_and_email_resend_cooldown() {
    let state = state();
    let mut client = Client::new(&state);
    assert_eq!(
        client
            .request(
                "POST",
                "/api/auth/email/start",
                json!({"email":"limited@example.test"})
            )
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        client
            .request(
                "POST",
                "/api/auth/email/start",
                json!({"email":"limited@example.test"})
            )
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    let response = client
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/email/start")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://attacker.example")
                .body(Body::from(
                    json!({"email":"cross-origin@example.test"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        client
            .request("GET", "/api/auth/google/start", json!({}))
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn production_without_smtp_never_exposes_development_code() {
    let state = AppState::new(Config {
        development: false,
        database: ":memory:".into(),
        public_url: "https://hireferry.example".into(),
        webauthn_rp_id: "hireferry.example".into(),
        ..Default::default()
    })
    .unwrap();
    let app = auth::routes().with_state(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/email/start")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://hireferry.example")
                .body(Body::from(
                    json!({"email":"production@example.test"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains("dev_code"));
}

async fn enroll(
    client: &mut Client,
    name: &str,
    key: &mut WebauthnAuthenticator<SoftPasskey>,
) -> Value {
    let (status, body) = client
        .request(
            "POST",
            "/api/admin/passkeys/register/start",
            json!({"name":name}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let options = serde_json::from_value(body["options"].clone()).unwrap();
    // The official testing authenticator signs real protocol data. Its UV flag
    // is simulated: this test does not claim to test a physical sensor.
    let credential = key
        .do_registration(url::Url::parse("http://localhost:3000").unwrap(), options)
        .unwrap();
    let (status, body) = client
        .request(
            "POST",
            "/api/admin/passkeys/register/finish",
            json!({"credential":credential}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["admin"], true);
    body
}

async fn passkey_sign_in(
    client: &mut Client,
    key: &mut WebauthnAuthenticator<SoftPasskey>,
) -> Value {
    let (status, body) = client
        .request("POST", "/api/admin/passkeys/login/start", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let options = serde_json::from_value(body["options"].clone()).unwrap();
    let credential = key
        .do_authentication(url::Url::parse("http://localhost:3000").unwrap(), options)
        .unwrap();
    let (status, body) = client
        .request(
            "POST",
            "/api/admin/passkeys/login/finish",
            json!({"credential":credential}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["admin"], true);
    body
}

#[tokio::test]
async fn real_webauthn_registration_login_multiple_keys_revocation_and_last_key_guard() {
    let state = state();
    let mut setup = Client::new(&state);
    let password = auth::generate_bootstrap(&state.conn().unwrap()).unwrap();
    assert_eq!(
        setup
            .request(
                "POST",
                "/api/admin/auth/bootstrap",
                json!({"password":password})
            )
            .await
            .0,
        StatusCode::OK
    );
    let mut first = WebauthnAuthenticator::new(SoftPasskey::new(true));
    enroll(&mut setup, "First test key", &mut first).await;
    assert!(auth::generate_bootstrap(&state.conn().unwrap()).is_err());
    let (_, list) = setup.request("GET", "/api/admin/passkeys", json!({})).await;
    let first_id = list["passkeys"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        setup
            .request(
                "DELETE",
                &format!("/api/admin/passkeys/{first_id}"),
                json!({})
            )
            .await
            .0,
        StatusCode::CONFLICT
    );
    let mut second = WebauthnAuthenticator::new(SoftPasskey::new(true));
    enroll(&mut setup, "Backup test key", &mut second).await;
    let (_, list) = setup.request("GET", "/api/admin/passkeys", json!({})).await;
    assert_eq!(list["passkeys"].as_array().unwrap().len(), 2);
    let mut a = Client::new(&state);
    let mut b = Client::new(&state);
    passkey_sign_in(&mut a, &mut first).await;
    passkey_sign_in(&mut b, &mut second).await;
    assert_eq!(
        b.request(
            "DELETE",
            &format!("/api/admin/passkeys/{first_id}"),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(a.request("GET", "/api/session", json!({})).await.1["user"].is_null());
    assert_eq!(
        a.request("GET", "/api/admin/passkeys", json!({})).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (_, list) = b.request("GET", "/api/admin/passkeys", json!({})).await;
    assert_eq!(list["passkeys"].as_array().unwrap().len(), 1);
    assert_eq!(list["passkeys"][0]["name"], "Backup test key");
    assert!(
        state
            .conn()
            .unwrap()
            .query_row("SELECT MAX(counter) FROM admin_passkeys", [], |r| r
                .get::<_, i64>(0))
            .unwrap()
            > 0
    );
    assert_eq!(
        setup
            .request(
                "POST",
                "/api/admin/auth/bootstrap",
                json!({"password":password})
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn passkey_signature_replay_and_other_origin_are_rejected() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let state = state();
    let mut client = Client::new(&state);
    let password = auth::generate_bootstrap(&state.conn().unwrap()).unwrap();
    client
        .request(
            "POST",
            "/api/admin/auth/bootstrap",
            json!({"password":password}),
        )
        .await;
    let mut key = WebauthnAuthenticator::new(SoftPasskey::new(true));
    enroll(&mut client, "Test key", &mut key).await;
    let mut login = Client::new(&state);
    let (_, body) = login
        .request("POST", "/api/admin/passkeys/login/start", json!({}))
        .await;
    let credential = key
        .do_authentication(
            url::Url::parse("http://localhost:3000").unwrap(),
            serde_json::from_value(body["options"].clone()).unwrap(),
        )
        .unwrap();
    let payload = json!({"credential":credential});
    let challenge_cookie = login.cookies.clone();
    assert_eq!(
        login
            .request("POST", "/api/admin/passkeys/login/finish", payload.clone())
            .await
            .0,
        StatusCode::OK
    );
    login.cookies = challenge_cookie;
    assert_eq!(
        login
            .request("POST", "/api/admin/passkeys/login/finish", payload)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );

    let (_, body) = login
        .request("POST", "/api/admin/passkeys/login/start", json!({}))
        .await;
    let credential = key
        .do_authentication(
            url::Url::parse("http://localhost:3000").unwrap(),
            serde_json::from_value(body["options"].clone()).unwrap(),
        )
        .unwrap();
    let mut tampered = serde_json::to_value(credential).unwrap();
    let encoded = tampered["response"]["clientDataJSON"].as_str().unwrap();
    let mut client_data: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap();
    client_data["origin"] = json!("https://attacker.example");
    tampered["response"]["clientDataJSON"] =
        json!(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&client_data).unwrap()));
    assert_eq!(
        login
            .request(
                "POST",
                "/api/admin/passkeys/login/finish",
                json!({"credential":tampered})
            )
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn adding_passkeys_requires_recent_authentication() {
    let state = state();
    let mut client = Client::new(&state);
    let password = auth::generate_bootstrap(&state.conn().unwrap()).unwrap();
    client
        .request(
            "POST",
            "/api/admin/auth/bootstrap",
            json!({"password":password}),
        )
        .await;
    let mut key = WebauthnAuthenticator::new(SoftPasskey::new(true));
    enroll(&mut client, "First key", &mut key).await;
    state
        .conn()
        .unwrap()
        .execute("UPDATE sessions SET recent_auth_at=0", [])
        .unwrap();
    assert_eq!(
        client
            .request(
                "POST",
                "/api/admin/passkeys/register/start",
                json!({"name":"Another key"})
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    passkey_sign_in(&mut client, &mut key).await;
    assert_eq!(
        client
            .request(
                "POST",
                "/api/admin/passkeys/register/start",
                json!({"name":"Another key"})
            )
            .await
            .0,
        StatusCode::OK
    );
}
