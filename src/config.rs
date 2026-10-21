use crate::error::{AppError, AppResult};
use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub bind: String,
    pub public_url: String,
    pub database: String,
    pub development: bool,
    pub session_hours: u64,
    pub webauthn_rp_id: String,
    pub site: Site,
    pub google: Google,
    pub smtp: Smtp,
    pub payments: Payments,
    pub pricing: Pricing,
    pub legal: crate::legal::Legal,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:3000".into(),
            public_url: "http://localhost:3000".into(),
            database: "data/hireferry.sqlite3".into(),
            development: false,
            session_hours: 24,
            webauthn_rp_id: "localhost".into(),
            site: Site::default(),
            google: Google::default(),
            smtp: Smtp::default(),
            payments: Payments::default(),
            pricing: Pricing::default(),
            legal: crate::legal::Legal::default(),
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Site {
    pub name: String,
    pub support_email: String,
    pub default_language: String,
    pub source_url: String,
}
impl Default for Site {
    fn default() -> Self {
        Self {
            name: "HireFerry · 聘渡".into(),
            support_email: "".into(),
            default_language: "zh".into(),
            source_url: "https://github.com/TokenNotIncluded/hireferry".into(),
        }
    }
}
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Google {
    pub enabled: bool,
    pub client_id: String,
    pub client_secret: String,
}
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Smtp {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
    pub starttls: bool,
}
impl Default for Smtp {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "".into(),
            port: 587,
            username: "".into(),
            password: "".into(),
            from: "".into(),
            starttls: true,
        }
    }
}
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Payments {
    pub stripe: Stripe,
    pub waffo: Waffo,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Stripe {
    pub enabled: bool,
    pub secret_key: String,
    pub webhook_secret: String,
    pub live_mode: bool,
}
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Waffo {
    pub enabled: bool,
    pub base_url: String,
    pub merchant_id: String,
    pub store_id: String,
    pub product_id: String,
    pub private_key_pem: String,
    pub webhook_public_key_pem: String,
    pub tax_category: String,
    pub mode: String,
}
impl Default for Waffo {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: "https://api.waffo.ai".into(),
            merchant_id: "".into(),
            store_id: "".into(),
            product_id: "".into(),
            private_key_pem: "".into(),
            webhook_public_key_pem: "".into(),
            tax_category: "".into(),
            mode: "test".into(),
        }
    }
}
#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pricing {
    pub onboarding_cents: i64,
    pub publish_bps: i64,
    pub edit_bps: i64,
    pub max_edits: i64,
}
impl Default for Pricing {
    fn default() -> Self {
        Self {
            onboarding_cents: 10000,
            publish_bps: 1000,
            edit_bps: 100,
            max_edits: 3,
        }
    }
}
impl Config {
    pub fn load(path: &std::path::Path) -> AppResult<Self> {
        let text = std::fs::read_to_string(path).map_err(AppError::internal)?;
        let mut cfg: Self = toml::from_str(&text)
            .map_err(|e| AppError::bad_request(format!("Invalid TOML: {e}")))?;
        cfg.legal
            .load_documents(path.parent().unwrap_or_else(|| std::path::Path::new(".")))?;
        cfg.validate()?;
        Ok(cfg)
    }
    pub fn validate(&self) -> AppResult<()> {
        self.legal.validate()?;
        for kind in ["terms", "privacy"] {
            for language in ["zh", "en"] {
                crate::legal::document(self, kind, language)?;
            }
        }
        let url = url::Url::parse(&self.public_url).map_err(AppError::bad_request)?;
        let bind: std::net::SocketAddr = self.bind.parse().map_err(AppError::bad_request)?;
        if url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(AppError::bad_request(
                "public_url must be an origin without a path, query or credentials",
            ));
        }
        if self.development {
            if !bind.ip().is_loopback()
                || !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
            {
                return Err(AppError::bad_request(
                    "development requires loopback bind and localhost public_url",
                ));
            }
        } else if url.scheme() != "https" {
            return Err(AppError::bad_request(
                "Production public_url must use HTTPS",
            ));
        }
        if url.host_str() != Some(self.webauthn_rp_id.as_str()) {
            return Err(AppError::bad_request(
                "webauthn_rp_id must equal public_url host",
            ));
        }
        if !(1..=168).contains(&self.session_hours) {
            return Err(AppError::bad_request(
                "session_hours must be between 1 and 168",
            ));
        }
        if self.google.enabled
            && (self.google.client_id.is_empty() || self.google.client_secret.is_empty())
        {
            return Err(AppError::bad_request("Google credentials are required"));
        }
        if self.smtp.enabled && (self.smtp.host.is_empty() || !self.smtp.from.contains('@')) {
            return Err(AppError::bad_request(
                "SMTP host and from address are required",
            ));
        }
        if self.payments.stripe.enabled
            && (self.payments.stripe.secret_key.is_empty()
                || self.payments.stripe.webhook_secret.is_empty())
        {
            return Err(AppError::bad_request("Stripe credentials are required"));
        }
        let w = &self.payments.waffo;
        if w.enabled
            && ([
                &w.merchant_id,
                &w.store_id,
                &w.product_id,
                &w.private_key_pem,
                &w.webhook_public_key_pem,
                &w.tax_category,
            ]
            .iter()
            .any(|v| v.is_empty())
                || !matches!(w.mode.as_str(), "test" | "prod"))
        {
            return Err(AppError::bad_request(
                "Waffo merchant, product, keys, tax_category and test/prod mode are required",
            ));
        }
        if !(0..=100000000).contains(&self.pricing.onboarding_cents)
            || !(1..=10000).contains(&self.pricing.publish_bps)
            || !(1..=10000).contains(&self.pricing.edit_bps)
            || !(0..=100).contains(&self.pricing.max_edits)
        {
            return Err(AppError::bad_request("Invalid pricing configuration"));
        }
        Ok(())
    }
}
