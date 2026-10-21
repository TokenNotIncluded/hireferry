use crate::{
    callback, defaults,
    error::{Error, Result},
    server::Server,
    storage::{Credential, Store},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use clap::Args;
use rand::{RngCore, rngs::OsRng};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::IsTerminal,
    process::{Command, Stdio},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    time::{Instant, sleep_until, timeout_at},
};
use url::Url;

#[derive(Args)]
pub struct LoginArgs {
    /// Sign in using a short code on another device.
    #[arg(long, conflicts_with = "oauth")]
    pub device: bool,
    /// Sign in in a browser with a local callback and PKCE.
    #[arg(long, conflicts_with = "device")]
    pub oauth: bool,
    /// Print the sign-in link without opening a browser.
    #[arg(long)]
    pub no_browser: bool,
    /// Maximum time to wait for login, in seconds.
    #[arg(long, value_parser = clap::value_parser!(u64).range(10..=1800))]
    pub timeout: Option<u64>,
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
    token_type: String,
    expires_in: u64,
    scope: String,
    refresh_token: String,
    refresh_expires_in: u64,
}

impl Token {
    fn credential(self, server: &Server, expected_scope: &str) -> Result<Credential> {
        if self.access_token.is_empty()
            || self.access_token.len() > 4096
            || !self.token_type.eq_ignore_ascii_case("bearer")
            || self.scope != expected_scope
            || self.refresh_token.is_empty()
            || self.refresh_token.len() > 4096
            || self.refresh_expires_in == 0
            || self.refresh_expires_in > defaults::MAX_TOKEN_LIFETIME_SECONDS
            || self.expires_in == 0
            || self.expires_in > defaults::MAX_TOKEN_LIFETIME_SECONDS
        {
            return Err(Error::invalid_response());
        }
        Ok(Credential {
            server: server.origin.clone(),
            client_id: server.client_id.clone(),
            account: None,
            scope: self.scope,
            refresh_token: self.refresh_token,
            refresh_expires_at: crate::now().saturating_add(self.refresh_expires_in as i64),
            refresh_pending: false,
            access_token: self.access_token,
            expires_at: crate::now().saturating_add(self.expires_in as i64),
        })
    }
}

#[derive(Deserialize)]
struct DeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
}

pub async fn login(server: &Server, args: &LoginArgs, auth_timeout: u64) -> Result<Credential> {
    let deadline = Instant::now() + Duration::from_secs(args.timeout.unwrap_or(auth_timeout));
    let browser = args.oauth || (!args.device && !args.no_browser && desktop_available());
    tokio::select! {
        result = async {
            if browser { oauth(server, args, deadline).await }
            else { device(server, args, deadline).await }
        } => result,
        _ = tokio::signal::ctrl_c() => Err(Error::new("cancelled", "Login cancelled.")),
    }
}

fn desktop_available() -> bool {
    if !std::io::stdin().is_terminal()
        || !std::io::stderr().is_terminal()
        || std::env::var_os("SSH_CONNECTION").is_some()
        || std::env::var_os("SSH_TTY").is_some()
    {
        return false;
    }
    cfg!(any(target_os = "windows", target_os = "macos"))
        || std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

async fn device(server: &Server, args: &LoginArgs, deadline: Instant) -> Result<Credential> {
    let code: DeviceCode = timeout_at(
        deadline,
        server.form(
            "/oauth/device/code",
            &[
                ("client_id", server.client_id.as_str()),
                ("scope", server.scope.as_str()),
            ],
        ),
    )
    .await
    .map_err(|_| Error::timeout())??;
    if code.device_code.is_empty()
        || code.device_code.len() > 4096
        || code.user_code.is_empty()
        || code.user_code.len() > 32
        || !code
            .user_code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b' '))
        || code.expires_in == 0
        || code.expires_in > 1800
    {
        return Err(Error::invalid_response());
    }
    let verification = server.same_origin_url(&code.verification_uri, "/oauth/device")?;
    if verification.query().is_some() {
        return Err(Error::invalid_response());
    }
    let link = if let Some(complete) = &code.verification_uri_complete {
        let complete = server.same_origin_url(complete, "/oauth/device")?;
        let query: Vec<_> = complete.query_pairs().collect();
        if query.len() != 1 || query[0].0 != "user_code" || query[0].1 != code.user_code {
            return Err(Error::invalid_response());
        }
        complete
    } else {
        verification
    };
    eprintln!(
        "Open this HireFerry page:\n{link}\nEnter or confirm this code: {}\nCode expires in {} seconds. This command will wait up to {} seconds.\nApprove only if the code matches this terminal.",
        code.user_code,
        code.expires_in,
        code.expires_in
            .min(deadline.saturating_duration_since(Instant::now()).as_secs())
    );
    if !args.no_browser && desktop_available() && !open_browser(link.as_str()).await {
        eprintln!("Could not open a browser. Open the link above on a trusted device.");
    }
    let deadline = deadline.min(Instant::now() + Duration::from_secs(code.expires_in));
    let mut poll = PollInterval::new(code.interval, server.poll_interval_seconds)?;
    loop {
        wait_interval(poll.seconds, deadline).await?;
        let fields = [
            ("client_id", server.client_id.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", code.device_code.as_str()),
        ];
        let request = server.form::<Token>("/oauth/token", &fields);
        match timeout_at(deadline, request)
            .await
            .map_err(|_| Error::timeout())?
        {
            Ok(token) => return token.credential(server, &server.scope),
            Err(error) if error.code == "authorization_pending" => {}
            Err(error) if error.code == "slow_down" => poll.slow_down(),
            Err(error) if error.code == "request_timeout" => poll.connection_timeout(),
            Err(error) => return Err(error),
        }
    }
}

struct PollInterval {
    seconds: u64,
}
impl PollInterval {
    fn new(interval: Option<u64>, minimum: u64) -> Result<Self> {
        let seconds = interval.unwrap_or(defaults::POLL_INTERVAL_SECONDS);
        if !(1..=1800).contains(&seconds) {
            return Err(Error::invalid_response());
        }
        Ok(Self {
            seconds: seconds.max(minimum),
        })
    }
    fn slow_down(&mut self) {
        self.seconds = self.seconds.saturating_add(5);
    }
    fn connection_timeout(&mut self) {
        self.seconds = self.seconds.saturating_mul(2);
    }
}

async fn wait_interval(seconds: u64, deadline: Instant) -> Result<()> {
    let next = Instant::now()
        .checked_add(Duration::from_secs(seconds))
        .unwrap_or(deadline);
    sleep_until(next.min(deadline)).await;
    if Instant::now() >= deadline {
        Err(Error::timeout())
    } else {
        Ok(())
    }
}

async fn oauth(server: &Server, args: &LoginArgs, deadline: Instant) -> Result<Credential> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| callback::listener_error())?;
    let address = listener
        .local_addr()
        .map_err(|_| callback::listener_error())?;
    let redirect = format!("http://{address}/callback");
    let verifier = random_secret();
    let state = random_secret();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url =
        Url::parse(&server.endpoint("/oauth/authorize")).map_err(|_| Error::invalid_response())?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", server.client_id.as_str()),
        ("response_type", "code"),
        ("scope", server.scope.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]);
    eprintln!("Approve this HireFerry login in your browser:\n{url}");
    if !args.no_browser && !open_browser(url.as_str()).await {
        eprintln!(
            "Could not open a browser. Open the link above on this computer, or use hfy auth login --device on a remote machine."
        );
    }
    let code = callback::wait_for_code(listener, &state, &server.origin, deadline).await?;
    let token: Token = timeout_at(
        deadline,
        server.form(
            "/oauth/token",
            &[
                ("client_id", server.client_id.as_str()),
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", &redirect),
                ("code_verifier", &verifier),
            ],
        ),
    )
    .await
    .map_err(|_| Error::timeout())??;
    token.credential(server, &server.scope)
}

fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

async fn open_browser(url: &str) -> bool {
    let url = url.to_owned();
    tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut c = Command::new("rundll32.exe");
            c.arg("url.dll,FileProtocolHandler");
            c
        };
        #[cfg(target_os = "macos")]
        let mut command = Command::new("open");
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let mut command = Command::new("xdg-open");
        command
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    })
    .await
    .unwrap_or(false)
}

/// Call only while holding the Store lock. Mark an exchange before sending it:
/// after a dropped response the previous refresh token must never be replayed.
pub async fn refresh(server: &Server, store: &Store, credential: &mut Credential) -> Result<()> {
    if credential.refresh_pending {
        return Err(refresh_uncertain());
    }
    if credential.refresh_expires_at <= crate::now() {
        return Err(Error::new(
            "session_expired",
            "The saved session expired. Run hfy auth login.",
        ));
    }
    credential.refresh_pending = true;
    store.save(credential)?;
    let fields = [
        ("grant_type", "refresh_token"),
        ("client_id", server.client_id.as_str()),
        ("refresh_token", credential.refresh_token.as_str()),
    ];
    let token: Token = match server.form("/oauth/token", &fields).await {
        Ok(token) => token,
        Err(error)
            if matches!(
                error.code,
                "invalid_grant" | "not_authenticated" | "access_denied" | "expired_token"
            ) =>
        {
            return Err(Error::new(
                "session_revoked",
                "This session is expired or revoked. Run hfy auth login.",
            ));
        }
        Err(_) => return Err(refresh_uncertain()),
    };
    let mut next = token.credential(server, &credential.scope)?;
    next.account = credential.account.clone();
    // A refresh may shorten its lifetime, but cannot silently make it sliding.
    next.refresh_expires_at = next.refresh_expires_at.min(credential.refresh_expires_at);
    if let Err(error) = store.save(&next) {
        let _ = server.revoke(&next.refresh_token).await;
        return Err(error);
    }
    *credential = next;
    Ok(())
}

pub fn refresh_uncertain() -> Error {
    Error::new(
        "refresh_uncertain",
        "A previous token refresh did not finish safely. Its old token will not be retried. Run hfy auth login to authorize a new session.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_keeps_slow_down_and_timeout_backoff() {
        let mut interval = PollInterval::new(None, 1).unwrap();
        assert_eq!(interval.seconds, 5);
        interval.slow_down();
        assert_eq!(interval.seconds, 10);
        interval.slow_down();
        assert_eq!(interval.seconds, 15);
        interval.connection_timeout();
        assert_eq!(interval.seconds, 30);
        assert!(PollInterval::new(Some(0), 1).is_err());
    }

    #[test]
    fn pkce_uses_url_safe_unpadded_random_secrets() {
        let first = random_secret();
        let second = random_secret();
        assert_ne!(first, second);
        assert_eq!(first.len(), 43);
        assert!(
            first
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        );
    }
}
