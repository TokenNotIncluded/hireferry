//! All client defaults live here. Server policy can further reduce these limits.
pub const SERVER: &str = "https://hireferry.lmm.best";
pub const CLIENT_ID: &str = "hireferry-cli";
pub const SCOPE: &str = "identity";
pub const AUTH_TIMEOUT_SECONDS: u64 = 600;
pub const REQUEST_TIMEOUT_SECONDS: u64 = 20;
pub const POLL_INTERVAL_SECONDS: u64 = 5;
pub const REFRESH_EARLY_SECONDS: i64 = 30;
pub const MAX_TOKEN_LIFETIME_SECONDS: u64 = 366 * 86400;
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
