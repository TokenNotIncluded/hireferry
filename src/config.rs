use crate::error::{AppError, AppResult};
use serde::Deserialize;
use std::collections::BTreeMap;

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
    pub humangrant: HumanGrant,
    pub smtp: Smtp,
    pub payments: Payments,
    pub pricing: Pricing,
    pub limits: Limits,
    pub webhooks: Webhooks,
    pub legal: crate::legal::Legal,
    pub cli_auth: CliAuth,
    pub cli_distribution: CliDistribution,
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
            humangrant: HumanGrant::default(),
            smtp: Smtp::default(),
            payments: Payments::default(),
            pricing: Pricing::default(),
            limits: Limits::default(),
            webhooks: Webhooks::default(),
            legal: crate::legal::Legal::default(),
            cli_auth: CliAuth::default(),
            cli_distribution: CliDistribution::default(),
        }
    }
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CliDistribution {
    pub release_repository: String,
    pub release_tag_prefix: String,
}
impl Default for CliDistribution {
    fn default() -> Self {
        Self {
            release_repository: "TokenNotIncluded/hireferry".into(),
            release_tag_prefix: "cli-v".into(),
        }
    }
}
impl CliDistribution {
    pub fn validate(&self) -> AppResult<()> {
        let parts: Vec<_> = self.release_repository.split('/').collect();
        let safe = |s: &str| {
            !s.is_empty()
                && s.len() <= 100
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
                && !matches!(s, "." | "..")
        };
        if parts.len() != 2
            || !parts.iter().all(|part| safe(part))
            || !safe(&self.release_tag_prefix)
        {
            return Err(AppError::bad_request(
                "Invalid CLI release repository or tag prefix",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CliAuth {
    pub enabled: bool,
    pub client_id: String,
    pub allowed_scopes: Vec<String>,
    pub access_token_minutes: u64,
    pub refresh_token_days: u64,
    pub code_seconds: u64,
    pub grant_minutes: u64,
    pub poll_seconds: u64,
    pub max_active_tokens: u64,
    pub rate_limit_requests: u64,
    pub rate_limit_window_seconds: u64,
    pub max_user_code_attempts: u64,
    pub max_pending_grants: u64,
}
impl Default for CliAuth {
    fn default() -> Self {
        Self {
            enabled: true,
            client_id: "hireferry-cli".into(),
            allowed_scopes: vec!["identity".into(), "mcp".into(), "admin".into()],
            access_token_minutes: 15,
            refresh_token_days: 30,
            code_seconds: 120,
            grant_minutes: 10,
            poll_seconds: 5,
            max_active_tokens: 10,
            rate_limit_requests: 100,
            rate_limit_window_seconds: 300,
            max_user_code_attempts: 30,
            max_pending_grants: 1000,
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
            support_email: "support@lmm.best".into(),
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
/// Local account-to-agent authorization. No third-party identity or secrets.
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HumanGrant {
    pub enabled: bool,
    pub client_ids: Vec<String>,
    pub request_ttl_seconds: i64,
    pub max_grant_seconds: i64,
    pub request_limit_per_hour: i64,
    pub service_limit_per_hour: i64,
}
impl Default for HumanGrant {
    fn default() -> Self {
        Self {
            enabled: true,
            client_ids: vec!["hireferry-web".into(), "hireferry-cli".into()],
            request_ttl_seconds: 600,
            max_grant_seconds: 86_400,
            request_limit_per_hour: 12,
            service_limit_per_hour: 600,
        }
    }
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
    pub reply_to: String,
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
            reply_to: "support@lmm.best".into(),
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
    pub allowed_currencies: Vec<String>,
    pub secret_key: String,
    pub webhook_secret: String,
    pub live_mode: bool,
}
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Waffo {
    pub enabled: bool,
    pub allowed_currencies: Vec<String>,
    pub base_url: String,
    pub merchant_id: String,
    pub store_id: String,
    pub product_id: String,
    pub private_key_pem: String,
    pub webhook_public_key_pem: String,
    pub tax_category: String,
    pub mode: String,
    pub checkout_regions: BTreeMap<String, WaffoRegion>,
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct WaffoRegion {
    pub currencies: Vec<String>,
    pub payment_methods: Vec<String>,
}
impl Default for Waffo {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_currencies: Vec::new(),
            base_url: "https://api.waffo.ai".into(),
            merchant_id: "".into(),
            store_id: "".into(),
            product_id: "".into(),
            private_key_pem: "".into(),
            webhook_public_key_pem: "".into(),
            tax_category: "".into(),
            mode: "test".into(),
            checkout_regions: BTreeMap::from([
                (
                    "global".into(),
                    WaffoRegion {
                        currencies: ["USD", "EUR", "GBP", "HKD", "JPY"]
                            .map(str::to_owned)
                            .into(),
                        payment_methods: ["card", "applepay", "googlepay"]
                            .map(str::to_owned)
                            .into(),
                    },
                ),
                (
                    "china".into(),
                    WaffoRegion {
                        currencies: ["CNY", "USD"].map(str::to_owned).into(),
                        payment_methods: vec!["wechat".into(), "alipay".into()],
                    },
                ),
            ]),
        }
    }
}

impl Waffo {
    pub fn checkout_region(
        &self,
        currency: &str,
        requested: Option<&str>,
    ) -> AppResult<(&str, &WaffoRegion)> {
        let preferred = if currency == "CNY" { "china" } else { "global" };
        let name = requested.unwrap_or(preferred);
        self.checkout_regions
            .get_key_value(name)
            .filter(|(_, region)| region.currencies.iter().any(|code| code == currency))
            .map(|(name, region)| (name.as_str(), region))
            .ok_or_else(|| {
                AppError::bad_request("This Waffo region does not support the order currency")
            })
    }
}
#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pricing {
    pub currency: String,
    pub allowed_currencies: Vec<String>,
    pub onboarding_cents: i64,
    pub onboarding_amounts: BTreeMap<String, i64>,
    pub publish_mode: String,
    pub edit_mode: String,
    pub publish_bps: i64,
    pub edit_bps: i64,
    pub publish_fixed_minor: i64,
    pub edit_fixed_minor: i64,
    pub employer_ai_min_salary_minor: i64,
    pub currency_amounts: BTreeMap<String, CurrencyAmounts>,
    pub max_edits: i64,
}
impl Default for Pricing {
    fn default() -> Self {
        Self {
            currency: "CNY".into(),
            allowed_currencies: Vec::new(),
            onboarding_cents: 10000,
            onboarding_amounts: BTreeMap::new(),
            publish_mode: "salary_percent".into(),
            edit_mode: "salary_percent".into(),
            publish_bps: 1000,
            edit_bps: 100,
            publish_fixed_minor: 30000,
            edit_fixed_minor: 3000,
            employer_ai_min_salary_minor: 300000,
            currency_amounts: BTreeMap::new(),
            max_edits: 3,
        }
    }
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CurrencyAmounts {
    pub publish_fixed_minor: i64,
    pub edit_fixed_minor: i64,
    pub employer_ai_min_salary_minor: i64,
}

// Explicit charge currencies verified against the provider documentation. These
// are integration capabilities, not the currencies an operator has enabled.
// Stripe's ISK/UGX compatibility rules and three-decimal charges need separate
// handling before inclusion; changing a stored currency's exponent is forbidden.
pub const STRIPE_CURRENCIES: &[&str] = &[
    "AED", "ARS", "AUD", "BRL", "CAD", "CHF", "CNY", "COP", "CZK", "DKK", "EUR", "GBP", "HKD",
    "HUF", "IDR", "ILS", "INR", "JPY", "KRW", "MXN", "MYR", "NOK", "NZD", "PHP", "PLN", "RON",
    "SEK", "SGD", "THB", "TWD", "USD", "ZAR",
];
pub const WAFFO_CURRENCIES: &[&str] = &["CNY", "EUR", "GBP", "HKD", "JPY", "USD"];

impl Payments {
    pub fn permits_currency(&self, provider: &str, currency: &str) -> bool {
        let (enabled, configured, implemented) = match provider {
            "stripe" => (
                self.stripe.enabled,
                &self.stripe.allowed_currencies,
                STRIPE_CURRENCIES,
            ),
            "waffo" => (
                self.waffo.enabled,
                &self.waffo.allowed_currencies,
                WAFFO_CURRENCIES,
            ),
            _ => return false,
        };
        enabled
            && implemented.contains(&currency)
            && (configured.is_empty() || configured.iter().any(|value| value == currency))
    }
    pub fn public_currencies(&self) -> BTreeMap<String, Vec<String>> {
        let mut result = BTreeMap::new();
        for (provider, enabled, configured, implemented) in [
            (
                "stripe",
                self.stripe.enabled,
                &self.stripe.allowed_currencies,
                STRIPE_CURRENCIES,
            ),
            (
                "waffo",
                self.waffo.enabled,
                &self.waffo.allowed_currencies,
                WAFFO_CURRENCIES,
            ),
        ] {
            if enabled {
                result.insert(
                    provider.to_owned(),
                    if configured.is_empty() {
                        implemented
                            .iter()
                            .map(|value| (*value).to_owned())
                            .collect()
                    } else {
                        configured.clone()
                    },
                );
            }
        }
        result
    }
}

/// Amount precision is immutable metadata; enabling a currency and selecting a
/// gateway are separate business decisions. This is charge, not payout, precision.
pub fn currency_decimals(currency: &str) -> Option<u32> {
    match currency {
        "JPY" | "KRW" => Some(0),
        _ if STRIPE_CURRENCIES.contains(&currency) => Some(2),
        _ => None,
    }
}

impl Pricing {
    pub fn onboarding_amount(&self, currency: &str) -> AppResult<i64> {
        if currency == self.currency {
            return Ok(self.onboarding_cents);
        }
        self.onboarding_amounts
            .get(currency)
            .copied()
            .ok_or_else(|| AppError::bad_request("Choose a configured activation currency"))
    }

    pub fn currencies(&self) -> Vec<String> {
        if self.allowed_currencies.is_empty() {
            vec![self.currency.clone()]
        } else {
            self.allowed_currencies.clone()
        }
    }
    pub fn permits_currency(&self, currency: &str) -> bool {
        currency_decimals(currency).is_some()
            && self.currencies().iter().any(|value| value == currency)
    }
    pub fn amounts(&self, currency: &str) -> AppResult<CurrencyAmounts> {
        if currency == self.currency {
            return Ok(CurrencyAmounts {
                publish_fixed_minor: self.publish_fixed_minor,
                edit_fixed_minor: self.edit_fixed_minor,
                employer_ai_min_salary_minor: self.employer_ai_min_salary_minor,
            });
        }
        self.currency_amounts.get(currency).cloned().ok_or_else(|| {
            AppError::bad_request("Configure amounts for this currency before using it")
        })
    }
    pub fn public(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).expect("pricing serialization");
        value["allowed_currencies"] = serde_json::json!(self.currencies());
        value["currency_decimals"] = serde_json::json!(currency_decimals(&self.currency));
        let mut activation = vec![
            serde_json::json!({"currency": self.currency, "currency_decimals": currency_decimals(&self.currency), "onboarding_minor": self.onboarding_cents}),
        ];
        activation.extend(self.onboarding_amounts.iter().map(|(currency, amount)| serde_json::json!({"currency": currency, "currency_decimals": currency_decimals(currency), "onboarding_minor": amount})));
        value["activation_currency_options"] = serde_json::json!(activation);
        value["currency_options"] = serde_json::json!(self.currencies().iter().filter_map(|currency| {
            let amounts = self.amounts(currency).ok()?;
            Some(serde_json::json!({"currency":currency,"currency_decimals":currency_decimals(currency),
                "publish_fixed_minor":amounts.publish_fixed_minor,"edit_fixed_minor":amounts.edit_fixed_minor,
                "employer_ai_min_salary_minor":amounts.employer_ai_min_salary_minor}))
        }).collect::<Vec<_>>());
        value
    }
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_job_headcount: i64,
    pub max_pending_orders_per_user: i64,
    pub list_page_size: i64,
    pub list_max_page: i64,
    pub workbench_page_size: i64,
    pub recent_list_limit: i64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_job_headcount: 1_000_000,
            max_pending_orders_per_user: 20,
            list_page_size: 50,
            list_max_page: 2000,
            workbench_page_size: 25,
            recent_list_limit: 200,
        }
    }
}

/// System limits for user-configured outbound webhooks, distinct from payment callbacks.
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Webhooks {
    pub enabled: bool,
    pub max_endpoints: i64,
    pub max_attempts: i64,
    pub max_pending_per_endpoint: i64,
    pub request_timeout_secs: u64,
    pub max_response_bytes: usize,
    pub poll_interval_secs: u64,
    pub delivered_retention_days: i64,
    pub failed_retention_days: i64,
}
impl Default for Webhooks {
    fn default() -> Self {
        Self {
            enabled: true,
            max_endpoints: 5,
            max_attempts: 5,
            max_pending_per_endpoint: 1000,
            request_timeout_secs: 5,
            max_response_bytes: 16384,
            poll_interval_secs: 5,
            delivered_retention_days: 30,
            failed_retention_days: 90,
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
        self.cli_distribution.validate()?;
        let hg = &self.humangrant;
        if !(60..=3600).contains(&hg.request_ttl_seconds)
            || !(60..=86_400).contains(&hg.max_grant_seconds)
            || !(1..=1000).contains(&hg.request_limit_per_hour)
            || !(1..=100_000).contains(&hg.service_limit_per_hour)
            || hg.client_ids.is_empty()
            || hg.client_ids.len() > 64
            || hg.client_ids.iter().enumerate().any(|(index, id)| {
                id.is_empty()
                    || id.len() > 64
                    || !id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
                    || hg.client_ids[..index].contains(id)
            })
        {
            return Err(AppError::bad_request("Invalid HumanGrant configuration"));
        }
        let cli = &self.cli_auth;
        if cli.client_id.is_empty()
            || cli.client_id.len() > 80
            || !cli
                .client_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
            || !cli.allowed_scopes.iter().any(|s| s == "identity")
            || cli
                .allowed_scopes
                .iter()
                .any(|s| !matches!(s.as_str(), "identity" | "mcp" | "admin"))
            || cli.allowed_scopes.len() > 3
            || cli
                .allowed_scopes
                .iter()
                .enumerate()
                .any(|(index, scope)| cli.allowed_scopes[..index].contains(scope))
            || !(1..=1440).contains(&cli.access_token_minutes)
            || !(1..=365).contains(&cli.refresh_token_days)
            || !(30..=600).contains(&cli.code_seconds)
            || !(1..=30).contains(&cli.grant_minutes)
            || !(5..=60).contains(&cli.poll_seconds)
            || !(1..=100).contains(&cli.max_active_tokens)
            || !(1..=10000).contains(&cli.rate_limit_requests)
            || !(10..=3600).contains(&cli.rate_limit_window_seconds)
            || !(1..=1000).contains(&cli.max_user_code_attempts)
            || !(1..=100000).contains(&cli.max_pending_grants)
        {
            return Err(AppError::bad_request(
                "Invalid cli_auth client, scopes, lifetime or rate limit configuration",
            ));
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
        if !self.smtp.reply_to.is_empty()
            && self
                .smtp
                .reply_to
                .parse::<lettre::message::Mailbox>()
                .is_err()
        {
            return Err(AppError::bad_request(
                "SMTP reply_to must be a valid email mailbox",
            ));
        }
        if self.payments.stripe.enabled
            && (self.payments.stripe.secret_key.is_empty()
                || self.payments.stripe.webhook_secret.is_empty())
        {
            return Err(AppError::bad_request("Stripe credentials are required"));
        }
        for (provider, currencies, supported) in [
            (
                "Stripe",
                &self.payments.stripe.allowed_currencies,
                STRIPE_CURRENCIES,
            ),
            (
                "Waffo",
                &self.payments.waffo.allowed_currencies,
                WAFFO_CURRENCIES,
            ),
        ] {
            let unique: std::collections::BTreeSet<_> = currencies.iter().collect();
            if unique.len() != currencies.len()
                || currencies
                    .iter()
                    .any(|currency| !supported.contains(&currency.as_str()))
            {
                return Err(AppError::bad_request(format!(
                    "{provider} allowed_currencies must be unique currency codes supported by this integration"
                )));
            }
        }
        let w = &self.payments.waffo;
        for (region, settings) in &w.checkout_regions {
            let methods = &settings.payment_methods;
            if !matches!(region.as_str(), "global" | "china")
                || settings.currencies.is_empty()
                || methods.is_empty()
                || settings
                    .currencies
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != settings.currencies.len()
                || methods
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != methods.len()
                || methods.iter().any(|method| {
                    method.is_empty()
                        || method.len() > 32
                        || !method.bytes().all(|byte| {
                            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                        })
                })
                || settings
                    .currencies
                    .iter()
                    .any(|code| !WAFFO_CURRENCIES.contains(&code.as_str()))
            {
                return Err(AppError::bad_request(
                    "Invalid Waffo checkout region currencies or payment methods",
                ));
            }
        }
        if w.enabled && w.checkout_regions.is_empty() {
            return Err(AppError::bad_request(
                "Configure at least one Waffo checkout region",
            ));
        }
        if self
            .pricing
            .onboarding_amounts
            .iter()
            .any(|(code, amount)| {
                code == &self.pricing.currency
                    || currency_decimals(code).is_none()
                    || !(1..=100000000).contains(amount)
            })
        {
            return Err(AppError::bad_request(
                "Activation amounts must be positive minor units of an additional supported currency",
            ));
        }
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
            || !(0..=10000).contains(&self.pricing.publish_bps)
            || !(0..=10000).contains(&self.pricing.edit_bps)
            || !(0..=100).contains(&self.pricing.max_edits)
        {
            return Err(AppError::bad_request("Invalid pricing configuration"));
        }
        let p = &self.pricing;
        if !p.permits_currency(&p.currency)
            || !matches!(p.publish_mode.as_str(), "salary_percent" | "fixed" | "free")
            || !matches!(p.edit_mode.as_str(), "salary_percent" | "fixed" | "free")
            || (p.publish_mode == "salary_percent" && p.publish_bps == 0)
            || (p.edit_mode == "salary_percent" && p.edit_bps == 0)
        {
            return Err(AppError::bad_request(
                "Invalid pricing currency or charge mode; use free for a zero charge",
            ));
        }
        let currencies = p.currencies();
        let unique: std::collections::BTreeSet<_> = currencies.iter().collect();
        if unique.len() != currencies.len() {
            return Err(AppError::bad_request(
                "Allowed currencies must be unique supported currency codes",
            ));
        }
        for currency in &currencies {
            if currency_decimals(currency).is_none() {
                return Err(AppError::bad_request(
                    "Unsupported currency; see the documented payment currency set",
                ));
            }
            let amounts = p.amounts(currency)?;
            if !(0..=100_000_000).contains(&amounts.publish_fixed_minor)
                || !(0..=100_000_000).contains(&amounts.edit_fixed_minor)
                || !(0..=1_000_000_000).contains(&amounts.employer_ai_min_salary_minor)
                || (p.publish_mode == "fixed" && amounts.publish_fixed_minor == 0)
                || (p.edit_mode == "fixed" && amounts.edit_fixed_minor == 0)
            {
                return Err(AppError::bad_request(
                    "Invalid per-currency fixed fee or AI salary threshold",
                ));
            }
        }
        if p.currency_amounts
            .keys()
            .any(|currency| currency == &p.currency || !p.permits_currency(currency))
        {
            return Err(AppError::bad_request(
                "Currency amount overrides must belong to an additional allowed currency",
            ));
        }
        let l = &self.limits;
        if !(1..=1_000_000).contains(&l.max_job_headcount)
            || !(1..=1000).contains(&l.max_pending_orders_per_user)
            || !(1..=200).contains(&l.list_page_size)
            || !(1..=100_000).contains(&l.list_max_page)
            || !(1..=100).contains(&l.workbench_page_size)
            || !(1..=1000).contains(&l.recent_list_limit)
        {
            return Err(AppError::bad_request("Invalid application limits"));
        }
        let w = &self.webhooks;
        if !(1..=5).contains(&w.max_endpoints)
            || !(1..=5).contains(&w.max_attempts)
            || !(1..=1000).contains(&w.max_pending_per_endpoint)
            || !(1..=15).contains(&w.request_timeout_secs)
            || !(1024..=65536).contains(&w.max_response_bytes)
            || !(1..=60).contains(&w.poll_interval_secs)
            || !(1..=365).contains(&w.delivered_retention_days)
            || !(1..=365).contains(&w.failed_retention_days)
        {
            return Err(AppError::bad_request("Invalid outbound webhook limits"));
        }
        self.legal.validate()?;
        for kind in ["terms", "privacy"] {
            for language in ["zh", "en"] {
                crate::legal::document(self, kind, language)?;
            }
        }
        Ok(())
    }
}
