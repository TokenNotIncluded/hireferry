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
    pub team: Team,
    pub feedback: Feedback,
    pub maintenance: Maintenance,
    pub chat: Chat,
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
            team: Team::default(),
            feedback: Feedback::default(),
            maintenance: Maintenance::default(),
            chat: Chat::default(),
        }
    }
}
/// An absent field keeps built-in localized copy; an empty prompt list hides it.
#[derive(Clone, Default, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Chat {
    pub welcome: BTreeMap<String, ChatWelcome>,
}
#[derive(Clone, Default, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChatWelcome {
    pub title: Option<String>,
    pub title_en: Option<String>,
    pub body_md: Option<String>,
    pub body_md_en: Option<String>,
    pub prompts: Option<Vec<ChatPrompt>>,
}
#[derive(Clone, Default, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChatPrompt {
    pub label: String,
    pub body: String,
    pub label_en: Option<String>,
    pub body_en: Option<String>,
}
impl Chat {
    pub fn validate(&self) -> AppResult<()> {
        for (audience, welcome) in &self.welcome {
            if !matches!(
                audience.as_str(),
                "platform_seeker" | "platform_employer" | "matched" | "staff"
            ) {
                return Err(AppError::bad_request("Unknown chat welcome audience"));
            }
            let check = |value: &str, max: usize| -> AppResult<()> {
                if value.chars().count() > max
                    || value
                        .chars()
                        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
                {
                    return Err(AppError::bad_request(
                        "Chat welcome text is too long or contains invalid characters",
                    ));
                }
                Ok(())
            };
            for title in [&welcome.title, &welcome.title_en].into_iter().flatten() {
                check(title, 160)?;
            }
            for body in [&welcome.body_md, &welcome.body_md_en]
                .into_iter()
                .flatten()
            {
                check(body, 4000)?;
            }
            if let Some(prompts) = &welcome.prompts {
                if prompts.len() > 6 {
                    return Err(AppError::bad_request(
                        "At most six chat welcome prompts are allowed",
                    ));
                }
                for prompt in prompts {
                    if prompt.label.trim().is_empty() || prompt.body.trim().is_empty() {
                        return Err(AppError::bad_request(
                            "Chat welcome prompts need a label and message",
                        ));
                    }
                    check(&prompt.label, 80)?;
                    check(&prompt.body, 2000)?;
                    if let Some(label) = &prompt.label_en {
                        check(label, 80)?;
                    }
                    if let Some(body) = &prompt.body_en {
                        check(body, 2000)?;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn public(&self) -> serde_json::Value {
        let mut value = serde_json::json!(self);
        for (audience, welcome) in &self.welcome {
            if let Some(body) = &welcome.body_md {
                value["welcome"][audience]["body_html"] =
                    serde_json::json!(crate::safety::markdown(body));
            }
            if let Some(body) = &welcome.body_md_en {
                value["welcome"][audience]["body_html_en"] =
                    serde_json::json!(crate::safety::markdown(body));
            }
        }
        value
    }
}
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Team {
    pub invitation_ttl_hours: i64,
    pub ai_token_ttl_days: i64,
    pub max_members: i64,
    pub max_pending_invitations: i64,
    pub invitations_per_hour: i64,
    pub resend_cooldown_seconds: i64,
}
impl Default for Team {
    fn default() -> Self {
        Self {
            invitation_ttl_hours: 72,
            ai_token_ttl_days: 30,
            max_members: 200,
            max_pending_invitations: 100,
            invitations_per_hour: 30,
            resend_cooldown_seconds: 60,
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
    pub production: Option<WaffoProduction>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaffoProduction {
    pub merchant_id: String,
    pub store_id: String,
    pub product_id: String,
    pub private_key_pem: String,
    pub webhook_public_key_pem: String,
    pub tax_category: String,
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
            production: None,
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
    pub fn for_mode(&self, mode: &str) -> AppResult<Self> {
        if !matches!(mode, "test" | "prod") {
            return Err(AppError::bad_request("Choose test or prod payment mode"));
        }
        let mut selected = self.clone();
        selected.production = None;
        if mode == "prod" {
            if let Some(p) = &self.production {
                selected.merchant_id = p.merchant_id.clone();
                selected.store_id = p.store_id.clone();
                selected.product_id = p.product_id.clone();
                selected.private_key_pem = p.private_key_pem.clone();
                selected.webhook_public_key_pem = p.webhook_public_key_pem.clone();
                selected.tax_category = p.tax_category.clone();
            } else if self.mode != "prod" {
                return Err(AppError::unavailable(
                    "Production Waffo credentials are not configured",
                ));
            }
        } else if self.mode == "prod" {
            return Err(AppError::unavailable(
                "Test Waffo credentials are not configured",
            ));
        }
        selected.mode = mode.into();
        if [
            &selected.merchant_id,
            &selected.store_id,
            &selected.product_id,
            &selected.private_key_pem,
            &selected.webhook_public_key_pem,
            &selected.tax_category,
        ]
        .iter()
        .any(|v| v.trim().is_empty())
        {
            return Err(AppError::unavailable("Waffo credentials are incomplete"));
        }
        use rsa::pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey};
        use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
        rsa::RsaPrivateKey::from_pkcs8_pem(&selected.private_key_pem)
            .or_else(|_| rsa::RsaPrivateKey::from_pkcs1_pem(&selected.private_key_pem))
            .map_err(|_| AppError::bad_request("Invalid Waffo private key"))?;
        rsa::RsaPublicKey::from_public_key_pem(&selected.webhook_public_key_pem)
            .or_else(|_| rsa::RsaPublicKey::from_pkcs1_pem(&selected.webhook_public_key_pem))
            .map_err(|_| AppError::bad_request("Invalid Waffo webhook public key"))?;
        Ok(selected)
    }

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
    pub onboarding_discount_percent: i64,
    pub onboarding_amounts: BTreeMap<String, i64>,
    pub publish_mode: String,
    pub edit_mode: String,
    pub publish_bps: i64,
    pub publish_discount_percent: i64,
    pub edit_bps: i64,
    pub edit_discount_percent: i64,
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
            onboarding_discount_percent: 0,
            onboarding_amounts: BTreeMap::new(),
            publish_mode: "salary_percent".into(),
            edit_mode: "salary_percent".into(),
            publish_bps: 1000,
            publish_discount_percent: 0,
            edit_bps: 100,
            edit_discount_percent: 0,
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
    pub publish_discount_percent: i64,
    pub edit_fixed_minor: i64,
    pub edit_discount_percent: i64,
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
    pub fn onboarding_original_amount(&self, currency: &str) -> AppResult<i64> {
        if currency == self.currency {
            return Ok(self.onboarding_cents);
        }
        self.onboarding_amounts
            .get(currency)
            .copied()
            .ok_or_else(|| AppError::bad_request("Choose a configured activation currency"))
    }
    pub fn discount_amount(original: i64, percent: i64) -> AppResult<i64> {
        if !(0..=100).contains(&percent) || original < 0 {
            return Err(AppError::bad_request(
                "Discount percent must be between 0 and 100",
            ));
        }
        if percent == 100 {
            return Ok(0);
        }
        let value = original
            .checked_mul(100 - percent)
            .and_then(|v| v.checked_add(50))
            .map(|v| v / 100)
            .ok_or_else(|| AppError::bad_request("Discounted amount is too large"))?;
        if original > 0 && value == 0 {
            return Err(AppError::bad_request(
                "Discounted amount must be at least one minor unit unless discount is 100%",
            ));
        }
        Ok(value)
    }
    pub fn onboarding_amount(&self, currency: &str) -> AppResult<i64> {
        Self::discount_amount(
            self.onboarding_original_amount(currency)?,
            self.onboarding_discount_percent,
        )
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
                publish_discount_percent: self.publish_discount_percent,
                edit_fixed_minor: self.edit_fixed_minor,
                edit_discount_percent: self.edit_discount_percent,
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
        value["onboarding_original_minor"] = serde_json::json!(self.onboarding_cents);
        value["onboarding_minor"] = serde_json::json!(
            self.onboarding_amount(&self.currency)
                .unwrap_or(self.onboarding_cents)
        );
        value["onboarding_discount_percent"] = serde_json::json!(self.onboarding_discount_percent);
        let mut activation = vec![
            serde_json::json!({"currency": self.currency, "currency_decimals": currency_decimals(&self.currency), "onboarding_minor": self.onboarding_amount(&self.currency).unwrap_or(self.onboarding_cents), "onboarding_original_minor": self.onboarding_cents, "onboarding_discount_percent": self.onboarding_discount_percent}),
        ];
        activation.extend(self.onboarding_amounts.iter().map(|(currency, amount)| serde_json::json!({"currency": currency, "currency_decimals": currency_decimals(currency), "onboarding_minor": self.onboarding_amount(currency).ok(), "onboarding_original_minor": amount, "onboarding_discount_percent": self.onboarding_discount_percent})));
        value["activation_currency_options"] = serde_json::json!(activation);
        value["currency_options"] = serde_json::json!(self.currencies().iter().filter_map(|currency| {
            let amounts = self.amounts(currency).ok()?;
            Some(serde_json::json!({"currency":currency,"currency_decimals":currency_decimals(currency),
                "publish_fixed_minor":amounts.publish_fixed_minor,"publish_discount_percent":amounts.publish_discount_percent,"edit_fixed_minor":amounts.edit_fixed_minor,"edit_discount_percent":amounts.edit_discount_percent,
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

/// Small, short-lived user reports; never used as a permanent support archive.
#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Feedback {
    pub enabled: bool,
    /// Unresolved reports; the original config key remains compatible.
    pub retention_days: i64,
    pub resolved_retention_days: i64,
    pub duplicate_window_seconds: i64,
    pub max_records: i64,
    pub max_per_account_per_hour: i64,
    pub max_guest_per_hour: i64,
}
impl Default for Feedback {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 180,
            resolved_retention_days: 30,
            duplicate_window_seconds: 600,
            max_records: 10_000,
            max_per_account_per_hour: 5,
            max_guest_per_hour: 30,
        }
    }
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Maintenance {
    pub interval_minutes: u64,
    pub batch_size: i64,
    pub expired_credentials_grace_days: i64,
    pub human_grant_history_days: i64,
    pub staff_invitation_history_days: i64,
}
impl Default for Maintenance {
    fn default() -> Self {
        Self {
            interval_minutes: 60,
            batch_size: crate::maintenance::DEFAULT_BATCH_SIZE,
            expired_credentials_grace_days: 30,
            human_grant_history_days: 90,
            staff_invitation_history_days: 30,
        }
    }
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
        self.chat.validate()?;
        let team = &self.team;
        if !(1..=720).contains(&team.invitation_ttl_hours)
            || !(1..=365).contains(&team.ai_token_ttl_days)
            || !(1..=1000).contains(&team.max_members)
            || !(1..=1000).contains(&team.max_pending_invitations)
            || !(1..=1000).contains(&team.invitations_per_hour)
            || !(30..=3600).contains(&team.resend_cooldown_seconds)
        {
            return Err(AppError::bad_request(
                "Invalid team invitation or member limits",
            ));
        }
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
        if w.production.is_some() {
            if w.mode != "test" {
                return Err(AppError::bad_request(
                    "Keep base Waffo credentials in test mode when configuring production",
                ));
            }
            w.for_mode("prod")?;
            w.for_mode("test")?;
        }
        if !(0..=100000000).contains(&self.pricing.onboarding_cents)
            || !(0..=10000).contains(&self.pricing.publish_bps)
            || !(0..=100).contains(&self.pricing.onboarding_discount_percent)
            || !(0..=100).contains(&self.pricing.publish_discount_percent)
            || !(0..=10000).contains(&self.pricing.edit_bps)
            || !(0..=100).contains(&self.pricing.edit_discount_percent)
            || !(0..=100).contains(&self.pricing.max_edits)
        {
            return Err(AppError::bad_request("Invalid pricing configuration"));
        }
        let p = &self.pricing;
        if !p.permits_currency(&p.currency)
            || !matches!(p.publish_mode.as_str(), "salary_percent" | "fixed" | "free")
            || !matches!(p.edit_mode.as_str(), "salary_percent" | "fixed" | "free")
            || (p.publish_mode == "salary_percent" && p.publish_bps == 0)
            || (p.publish_discount_percent > 0 && p.publish_mode == "free")
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
                || !(0..=100).contains(&amounts.publish_discount_percent)
                || !(0..=100).contains(&amounts.edit_discount_percent)
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
        let f = &self.feedback;
        if !(1..=3650).contains(&f.retention_days)
            || !(1..=3650).contains(&f.resolved_retention_days)
            || !(1..=86_400).contains(&f.duplicate_window_seconds)
            || !(100..=100_000).contains(&f.max_records)
            || !(1..=100).contains(&f.max_per_account_per_hour)
            || !(1..=1000).contains(&f.max_guest_per_hour)
        {
            return Err(AppError::bad_request(
                "Invalid feedback retention or rate limits",
            ));
        }
        let m = &self.maintenance;
        if !(15..=1440).contains(&m.interval_minutes)
            || !(1..=1000).contains(&m.batch_size)
            || !(1..=365).contains(&m.expired_credentials_grace_days)
            || !(1..=365).contains(&m.human_grant_history_days)
            || !(1..=365).contains(&m.staff_invitation_history_days)
        {
            return Err(AppError::bad_request(
                "Invalid housekeeping intervals or retention",
            ));
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
