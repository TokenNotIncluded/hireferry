use crate::{
    defaults,
    error::{Error, Result},
    settings::Settings,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::time::Duration;
use url::{Host, Url};

#[derive(Clone)]
pub struct Server {
    pub origin: String,
    pub client_id: String,
    pub scope: String,
    pub poll_interval_seconds: u64,
    client: reqwest::Client,
}

impl Server {
    pub fn new(input: &str) -> Result<Self> {
        Self::with_settings(&Settings {
            server: input.to_owned(),
            ..Settings::default()
        })
    }

    pub fn with_settings(settings: &Settings) -> Result<Self> {
        let url = Url::parse(&settings.server).map_err(|_| invalid_server())?;
        let loopback = match url.host() {
            Some(Host::Domain("localhost")) => true,
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port() == Some(0)
        {
            return Err(invalid_server());
        }
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(
                settings.request_timeout_seconds.min(10),
            ))
            .timeout(Duration::from_secs(settings.request_timeout_seconds))
            .user_agent(concat!("hireferry-cli/", env!("CARGO_PKG_VERSION")));
        if loopback {
            builder = builder.no_proxy();
        }
        let client = builder
            .build()
            .map_err(|_| Error::new("client_error", "Could not start the HTTP client."))?;
        Ok(Self {
            origin: url.origin().ascii_serialization(),
            client_id: settings.client_id.clone(),
            scope: match settings.scope.as_str() {
                "mcp" => "identity mcp",
                "admin" => "identity admin",
                _ => "identity",
            }
            .to_owned(),
            poll_interval_seconds: settings.poll_interval_seconds,
            client,
        })
    }

    pub fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }

    pub fn same_origin_url(&self, input: &str, path: &str) -> Result<Url> {
        let url = Url::parse(input).map_err(|_| Error::invalid_response())?;
        if url.origin().ascii_serialization() != self.origin
            || url.path() != path
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::invalid_response());
        }
        Ok(url)
    }

    pub async fn form<T: DeserializeOwned>(
        &self,
        path: &str,
        fields: &[(&str, &str)],
    ) -> Result<T> {
        let response = self
            .client
            .post(self.endpoint(path))
            .form(fields)
            .send()
            .await
            .map_err(Error::network)?;
        decode(response).await
    }

    pub async fn userinfo(&self, token: &str) -> Result<UserInfo> {
        let response = self
            .client
            .get(self.endpoint("/oauth/userinfo"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(Error::network)?;
        let user: UserInfo = decode(response).await?;
        if !matches!(
            user.scope.as_str(),
            "identity" | "identity mcp" | "identity admin"
        ) || user.admin == user.user.is_some()
            || user
                .user
                .as_ref()
                .is_some_and(|identity| identity.id.is_empty() || identity.id.len() > 256)
            || (user.scope == "identity admin" && !user.admin)
            || (user.scope == "identity mcp" && user.admin)
            || user.expires_at <= crate::now()
        {
            return Err(Error::invalid_response());
        }
        Ok(user)
    }

    pub async fn revoke(&self, token: &str) -> Result<()> {
        let response = self
            .client
            .post(self.endpoint("/oauth/revoke"))
            .form(&[("client_id", self.client_id.as_str()), ("token", token)])
            .send()
            .await
            .map_err(Error::network)?;
        if response.status().is_success() {
            Ok(())
        } else {
            let _: serde_json::Value = decode(response).await?;
            Err(Error::invalid_response())
        }
    }
}

fn invalid_server() -> Error {
    Error::new(
        "invalid_server",
        "Use an HTTPS server origin, or HTTP on loopback for local development. Do not include a path, query or credentials.",
    )
}

#[derive(Deserialize)]
struct OAuthError {
    error: String,
}

async fn decode<T: DeserializeOwned>(mut response: reqwest::Response) -> Result<T> {
    let status = response.status();
    if status.is_redirection() {
        return Err(Error::new(
            "unexpected_redirect",
            "The server redirected an API request.",
        ));
    }
    if response
        .content_length()
        .is_some_and(|n| n > defaults::MAX_RESPONSE_BYTES as u64)
    {
        return Err(Error::invalid_response());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(Error::network)? {
        if bytes.len() + chunk.len() > defaults::MAX_RESPONSE_BYTES {
            return Err(Error::invalid_response());
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        if let Ok(body) = serde_json::from_slice::<OAuthError>(&bytes) {
            if let Some(error) = known_protocol_error(&body.error) {
                return Err(error);
            }
        }
        return Err(match status.as_u16() {
            401 => Error::signed_out(),
            403 => Error::new(
                "access_denied",
                "This account is not allowed to use this connection.",
            ),
            404 => Error::new(
                "unsupported_server",
                "This server does not support CLI login. Update HireFerry on the server.",
            ),
            429 => Error::new("rate_limited", "Too many requests. Try again later."),
            _ => Error::new("server_error", "The server could not complete the request."),
        });
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::invalid_response())
}

pub fn protocol_error(code: &str) -> Error {
    known_protocol_error(code)
        .unwrap_or_else(|| Error::new("server_error", "The server could not complete the request."))
}

fn known_protocol_error(code: &str) -> Option<Error> {
    Some(match code {
        "authorization_pending" => Error::new("authorization_pending", "Waiting for approval."),
        "slow_down" => Error::new(
            "slow_down",
            "The server requested a longer polling interval.",
        ),
        "access_denied" => Error::new(
            "access_denied",
            "The login request was denied or the account is not eligible.",
        ),
        "expired_token" => Error::new(
            "expired_token",
            "The login code expired. Start login again.",
        ),
        "invalid_grant" => Error::new(
            "invalid_grant",
            "The login code is invalid, expired or already used.",
        ),
        "invalid_client" => Error::new(
            "invalid_client",
            "The server does not accept this CLI client.",
        ),
        "invalid_scope" => Error::new(
            "invalid_scope",
            "The server does not accept the requested permissions.",
        ),
        "invalid_token" => Error::signed_out(),
        "temporarily_unavailable" => Error::new(
            "temporarily_unavailable",
            "CLI login is temporarily unavailable.",
        ),
        "invalid_request" => {
            Error::new("invalid_request", "The server rejected the login request.")
        }
        _ => return None,
    })
}

// Only these identity fields can reach stdout; unknown server fields (including
// any token, internal configuration or session fields) are discarded.
#[derive(Deserialize, Serialize)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: String,
}

#[derive(Deserialize, Serialize)]
pub struct UserInfo {
    #[serde(default)]
    pub ai_eligibility: Option<Eligibility>,
    #[serde(default)]
    pub capabilities: Capabilities,
    pub admin: bool,
    pub user: Option<Identity>,
    pub scope: String,
    pub expires_at: i64,
}

#[derive(Default, Deserialize, Serialize)]
pub struct Capabilities {
    pub identity: bool,
    pub mcp: bool,
    pub admin: bool,
}
#[derive(Deserialize, Serialize)]
pub struct Eligibility {
    pub eligible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_salary_cents: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn mock_response(status: &str, body: &str, extra_header: &str) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra_header}Connection: close\r\n\r\n{body}",
            body.len()
        );
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await.unwrap();
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        Server::new(&format!("http://{address}")).unwrap()
    }

    #[test]
    fn server_origins_are_normalized_and_transport_is_checked() {
        assert_eq!(
            Server::new("https://HIREferry.lmm.best:443/")
                .unwrap()
                .origin,
            "https://hireferry.lmm.best"
        );
        for invalid in [
            "http://example.com",
            "https://example.com/api",
            "https://name:password@example.com",
            "https://example.com/?token=x",
            "https://example.com/#x",
            "file:///etc/passwd",
            "http://localhost:0",
        ] {
            assert!(Server::new(invalid).is_err(), "{invalid}");
        }
        for valid in [
            "http://127.0.0.1:3000",
            "http://[::1]:3000",
            "http://localhost:3000",
        ] {
            assert!(Server::new(valid).is_ok(), "{valid}");
        }
    }

    #[test]
    fn errors_never_echo_untrusted_descriptions() {
        let error = protocol_error("secret-token-value");
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("secret-token-value")
        );
    }

    #[tokio::test]
    async fn server_error_bodies_and_redirects_do_not_leak_credentials() {
        let server = mock_response(
            "400 Bad Request",
            r#"{"error":"access_denied","error_description":"secret-access-token"}"#,
            "",
        )
        .await;
        let error = server
            .form::<serde_json::Value>("/oauth/token", &[])
            .await
            .unwrap_err();
        assert_eq!(error.code, "access_denied");
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("secret-access-token")
        );
        let server = mock_response(
            "302 Found",
            "",
            "Location: https://invalid.example/?token=secret-access-token\r\n",
        )
        .await;
        let error = server
            .form::<serde_json::Value>("/oauth/token", &[])
            .await
            .unwrap_err();
        assert_eq!(error.code, "unexpected_redirect");
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("secret-access-token")
        );
    }

    #[tokio::test]
    async fn identity_output_discards_unknown_credential_fields() {
        let body = format!(
            r#"{{"admin":false,"user":{{"id":"member","name":"User","email":"user@example.test","role":"seeker","access_token":"secret-access-token"}},"scope":"identity","expires_at":{},"access_token":"secret-access-token"}}"#,
            crate::now() + 600
        );
        let server = mock_response("200 OK", &body, "").await;
        let identity = server.userinfo("secret-access-token").await.unwrap();
        let output = serde_json::to_string(&identity).unwrap();
        assert!(!output.contains("secret-access-token"));
        assert!(!output.contains("access_token"));
    }

    #[tokio::test]
    async fn human_error_messages_keep_http_authentication_semantics() {
        let server = mock_response(
            "401 Unauthorized",
            r#"{"error":"Client token expired or revoked: secret-access-token"}"#,
            "",
        )
        .await;
        let error = server.userinfo("secret-access-token").await.err().unwrap();
        assert_eq!(error.code, "not_authenticated");
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("secret-access-token")
        );

        let server = mock_response(
            "403 Forbidden",
            r#"{"error":"This account cannot access this resource"}"#,
            "",
        )
        .await;
        let error = server.userinfo("secret-access-token").await.err().unwrap();
        assert_eq!(error.code, "access_denied");
    }

    #[tokio::test]
    async fn administrator_can_have_basic_identity_without_admin_capability() {
        let body = format!(
            r#"{{"admin":true,"user":null,"scope":"identity","expires_at":{},"capabilities":{{"identity":true,"mcp":false,"admin":false}}}}"#,
            crate::now() + 600,
        );
        let server = mock_response("200 OK", &body, "").await;
        let identity = server.userinfo("secret-access-token").await.unwrap();
        assert!(identity.admin);
        assert_eq!(identity.scope, "identity");
        assert!(!identity.capabilities.admin);
    }
}
