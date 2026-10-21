use serde::Serialize;

/// Only fixed, public messages are carried across the output boundary. In
/// particular, reqwest URLs, response bodies and credentials are never attached.
#[derive(Debug, Serialize)]
pub struct Error {
    pub code: &'static str,
    pub message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl Error {
    pub const fn new(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn network(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::new("request_timeout", "The server request timed out.")
        } else {
            Self::new("network_error", "Could not reach the HireFerry server.")
        }
    }

    pub const fn invalid_response() -> Self {
        Self::new(
            "invalid_response",
            "The server returned an invalid response.",
        )
    }

    pub const fn storage() -> Self {
        Self::new(
            "credential_store",
            "Could not use the private credential store.",
        )
    }

    pub const fn signed_out() -> Self {
        Self::new("not_authenticated", "Not signed in. Run hfy auth login.")
    }

    pub const fn timeout() -> Self {
        Self::new(
            "login_timeout",
            "Login timed out. Run hfy auth login again.",
        )
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
