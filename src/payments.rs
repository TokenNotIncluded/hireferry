//! Hosted, one-time payments. No return URL can mark an order paid.
//!
//! The exact request body is signed/verified before it is parsed. Receipt
//! settlement belongs to commerce so all payment providers use one transaction.
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use rsa::{
    RsaPrivateKey, RsaPublicKey,
    pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey},
    pkcs1v15::{Signature, SigningKey, VerifyingKey},
    pkcs8::{DecodePrivateKey, DecodePublicKey},
    signature::{RandomizedSigner, SignatureEncoding, Verifier},
    traits::PublicKeyParts,
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    AppState,
    error::{AppError, AppResult},
    models::Order,
};

const WAFFO_CHECKOUT_PATH: &str = "/v1/actions/checkout/create-session";
const MAX_GATEWAY_BODY: usize = 128 * 1024;
const STRIPE_TOLERANCE_SECONDS: u64 = 300;
// Waffo retries retain the original timestamp; its last retry is after 31 min.
const WAFFO_TOLERANCE_MILLISECONDS: u64 = 45 * 60 * 1_000;

#[derive(Debug, Clone, Serialize)]
pub struct Checkout {
    pub session_id: String,
    pub url: String,
    pub expires_at: i64,
}

pub(crate) fn checkout_ttl_seconds(provider: &str) -> i64 {
    if provider == "waffo" { 1800 } else { 24 * 3600 }
}

#[derive(Debug, Clone)]
pub struct PaymentReceipt {
    pub provider: String,
    pub event_id: String,
    pub order_id: String,
    // Waffo does not include its checkout session ID in order.completed.
    pub session_id: Option<String>,
    pub payment_id: String,
    pub amount_cents: i64,
    pub currency: String,
    pub live_mode: bool,
}

/// A provider-confirmed refund notification, never a request to issue a refund.
/// Waffo identifies both refund outcomes by the same refund entity ID.
#[derive(Debug, Clone)]
pub struct RefundReceipt {
    pub provider: String,
    pub event_id: String,
    pub refund_id: String,
    pub order_id: String,
    pub provider_order_id: String,
    pub payment_id: String,
    pub amount_cents: i64,
    pub original_amount_cents: Option<i64>,
    pub currency: String,
    pub live_mode: bool,
    pub succeeded: bool,
    pub reason: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/webhooks/stripe", post(stripe_webhook))
        .route("/api/webhooks/waffo", post(waffo_webhook))
        .layer(DefaultBodyLimit::max(MAX_GATEWAY_BODY))
}

pub async fn create_checkout(state: &AppState, order: &Order, email: &str) -> AppResult<Checkout> {
    if order.amount_cents <= 0
        || crate::config::currency_decimals(&order.currency).is_none()
        || order.status != "pending"
    {
        return Err(AppError::bad_request("Invalid payment order"));
    }
    crate::commerce::provider_currency_enabled(state, &order.provider, &order.currency)?;
    // A stable local order ID also supplies a stable provider idempotency key.
    if order.id.is_empty()
        || order.id.len() > 128
        || !order
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(AppError::bad_request("Invalid payment order ID"));
    }
    match order.provider.as_str() {
        "stripe" => stripe_checkout(state, order, email).await,
        "waffo" => waffo_checkout(state, order, email).await,
        _ => Err(AppError::bad_request("Unknown payment provider")),
    }
}

fn return_url(
    public_url: &str,
    order_id: &str,
    provider: &str,
    canceled: bool,
) -> AppResult<String> {
    let mut url = reqwest::Url::parse(public_url)
        .map_err(|_| AppError::unavailable("Configure a valid public_url"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(AppError::unavailable(
            "Payment checkout requires an HTTPS public_url",
        ));
    }
    url.set_path("/billing/return");
    url.set_query(None);
    url.set_fragment(None);
    url.query_pairs_mut()
        .append_pair("order_id", order_id)
        .append_pair("provider", provider);
    if canceled {
        // This hint is for presentation only. Only authenticated callbacks settle orders.
        url.query_pairs_mut().append_pair("canceled", "1");
    }
    Ok(url.to_string())
}

fn product_name(kind: &str) -> &'static str {
    match kind {
        "onboarding" | "recruiter_activation" | "activation" | "recruiter" => {
            "HireFerry recruiter access"
        }
        "donation" => "Voluntary donation to the current site operator",
        "job_edit" | "edit" => "HireFerry job description update",
        _ => "HireFerry job advertisement",
    }
}

async fn stripe_checkout(state: &AppState, order: &Order, email: &str) -> AppResult<Checkout> {
    let cfg = &state.cfg.payments.stripe;
    if !cfg.enabled || cfg.secret_key.is_empty() || cfg.webhook_secret.is_empty() {
        return Err(AppError::unavailable("Stripe is not configured"));
    }
    let expected_prefix = if cfg.live_mode {
        "sk_live_"
    } else {
        "sk_test_"
    };
    let restricted_prefix = if cfg.live_mode {
        "rk_live_"
    } else {
        "rk_test_"
    };
    if !cfg.secret_key.starts_with(expected_prefix)
        && !cfg.secret_key.starts_with(restricted_prefix)
    {
        return Err(AppError::unavailable(
            "Stripe key does not match the configured payment environment",
        ));
    }
    let destination = return_url(&state.cfg.public_url, &order.id, "stripe", false)?;
    let canceled = return_url(&state.cfg.public_url, &order.id, "stripe", true)?;
    let checkout_product_name = if order.kind == "donation" {
        format!(
            "Voluntary donation to the {} site operator",
            state.cfg.site.name
        )
    } else {
        product_name(&order.kind).to_string()
    };
    let fields = [
        ("mode", "payment".to_string()),
        (
            "line_items[0][price_data][currency]",
            order.currency.to_ascii_lowercase(),
        ),
        (
            "line_items[0][price_data][unit_amount]",
            order.amount_cents.to_string(),
        ),
        (
            "line_items[0][price_data][product_data][name]",
            checkout_product_name,
        ),
        ("line_items[0][quantity]", "1".to_string()),
        ("client_reference_id", order.id.clone()),
        ("metadata[hireferry_order_id]", order.id.clone()),
        ("customer_email", email.to_string()),
        ("success_url", destination),
        ("cancel_url", canceled),
        ("phone_number_collection[enabled]", "false".to_string()),
    ];
    let response = state
        .client
        .post("https://api.stripe.com/v1/checkout/sessions")
        .bearer_auth(&cfg.secret_key)
        .header(
            "Idempotency-Key",
            format!("hireferry-checkout-{}", order.id),
        )
        .timeout(Duration::from_secs(20))
        .form(&fields)
        .send()
        .await
        .map_err(|_| AppError::unavailable("Stripe request failed; retry this order"))?;
    let data = gateway_json(response, "Stripe").await?;
    if data.get("livemode").and_then(Value::as_bool) != Some(cfg.live_mode) {
        return Err(AppError::unavailable(
            "Stripe returned the wrong payment environment",
        ));
    }
    let session_id = required_string(&data, "id")?;
    let url = required_text(&data, "url", 8192)?;
    let expires_at = data["expires_at"]
        .as_i64()
        .filter(|timestamp| *timestamp > 0)
        .ok_or_else(|| AppError::unavailable("Stripe returned no checkout expiration"))?;
    validate_checkout_url(&url, "checkout.stripe.com")?;
    Ok(Checkout {
        session_id,
        url,
        expires_at,
    })
}

pub fn waffo_mode(state: &AppState, db: &rusqlite::Connection) -> AppResult<String> {
    use rusqlite::OptionalExtension;
    Ok(db
        .query_row(
            "SELECT mode FROM payment_settings WHERE provider='waffo'",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_else(|| state.cfg.payments.waffo.mode.clone()))
}

async fn waffo_checkout(state: &AppState, order: &Order, email: &str) -> AppResult<Checkout> {
    let cfg = state
        .cfg
        .payments
        .waffo
        .for_mode(if order.live_mode { "prod" } else { "test" })?;
    if !cfg.enabled
        || cfg.merchant_id.is_empty()
        || cfg.store_id.is_empty()
        || cfg.product_id.is_empty()
        || cfg.private_key_pem.is_empty()
        || cfg.webhook_public_key_pem.is_empty()
        || cfg.tax_category.is_empty()
        || !matches!(cfg.mode.as_str(), "test" | "prod")
    {
        return Err(AppError::unavailable("Waffo Pancake is not configured"));
    }
    let mut endpoint = reqwest::Url::parse(&cfg.base_url)
        .map_err(|_| AppError::unavailable("Invalid Waffo API base URL"))?;
    if endpoint.scheme() != "https"
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || !matches!(endpoint.path(), "" | "/")
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(AppError::unavailable(
            "Waffo API base URL must be an HTTPS origin",
        ));
    }
    endpoint.set_path(WAFFO_CHECKOUT_PATH);
    let body = serde_json::to_vec(&waffo_checkout_body(state, order, email)?)
        .map_err(AppError::internal)?;
    let timestamp = unix_milliseconds() / 1_000;
    let signature = sign_waffo_request(&cfg.private_key_pem, timestamp, &body)
        .map_err(|_| AppError::unavailable("Waffo signing key is invalid"))?;
    let response = state
        .client
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("X-Merchant-Id", &cfg.merchant_id)
        .header("X-Timestamp", timestamp.to_string())
        .header("X-Signature", signature)
        .header(
            "X-Idempotency-Key",
            format!("hireferry-checkout-{}", order.id),
        )
        .timeout(Duration::from_secs(20))
        .body(body)
        .send()
        .await
        .map_err(|_| AppError::unavailable("Waffo request failed; retry this order"))?;
    let response = gateway_json(response, "Waffo").await?;
    if response
        .get("errors")
        .and_then(Value::as_array)
        .is_some_and(|v| !v.is_empty())
    {
        return Err(AppError::unavailable("Waffo rejected the checkout request"));
    }
    let data = response
        .get("data")
        .ok_or_else(|| AppError::unavailable("Invalid Waffo response"))?;
    let session_id = required_string(data, "sessionId")?;
    let mut url = required_text(data, "checkoutUrl", 8192)?;
    let host = reqwest::Url::parse(&url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    if !matches!(host.as_str(), "checkout.waffo.ai" | "pancake.waffo.ai") {
        return Err(AppError::unavailable("Untrusted checkout URL"));
    }
    validate_checkout_url(&url, &host)?;
    if cfg.mode == "test" {
        let mut parsed = reqwest::Url::parse(&url).map_err(AppError::internal)?;
        parsed.query_pairs_mut().append_pair("test", "true");
        url = parsed.into();
    }
    // A retried idempotent request may return the original session. Its lifetime
    // must stay anchored to the first attempt, not to this HTTP response.
    let expires_at = order
        .checkout_expires_at
        .ok_or_else(|| AppError::unavailable("Missing checkout expiration"))?;
    Ok(Checkout {
        session_id,
        url,
        expires_at,
    })
}

fn waffo_checkout_body(state: &AppState, order: &Order, email: &str) -> AppResult<Value> {
    let cfg = state
        .cfg
        .payments
        .waffo
        .for_mode(if order.live_mode { "prod" } else { "test" })?;
    let snapshot: Value = serde_json::from_str(&order.snapshot_json).map_err(AppError::internal)?;
    let (_, region) = cfg.checkout_region(&order.currency, snapshot["payment_region"].as_str())?;
    Ok(json!({
        "productId": cfg.product_id,
        "currency": order.currency,
        "buyerEmail": email,
        "priceSnapshot": {
            "amount": minor_to_decimal(order.amount_cents, &order.currency).ok_or_else(|| AppError::bad_request("Unsupported payment currency"))?,
            "taxIncluded": true,
            "taxCategory": cfg.tax_category,
        },
        "orderMerchantExternalId": order.id,
        "metadata": { "hireferry_order_id": order.id },
        "successUrl": return_url(&state.cfg.public_url, &order.id, "waffo", false)?,
        "expiresInSeconds": checkout_ttl_seconds("waffo"),
        "includePaymentMethods": region.payment_methods,
    }))
}

async fn gateway_json(mut response: reqwest::Response, provider: &str) -> AppResult<Value> {
    if !response.status().is_success() {
        // Do not expose upstream error bodies, which can contain billing data.
        return Err(AppError::unavailable(format!(
            "{provider} checkout was not accepted (HTTP {})",
            response.status().as_u16()
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AppError::unavailable("Payment response was interrupted"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_GATEWAY_BODY {
            return Err(AppError::unavailable("Payment response was too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| AppError::unavailable("Invalid payment response"))
}

fn validate_checkout_url(value: &str, expected_host: &str) -> AppResult<()> {
    let url =
        reqwest::Url::parse(value).map_err(|_| AppError::unavailable("Invalid checkout URL"))?;
    if url.scheme() != "https"
        || url.host_str() != Some(expected_host)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
    {
        return Err(AppError::unavailable("Untrusted checkout URL"));
    }
    Ok(())
}

async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<StatusCode> {
    let cfg = &state.cfg.payments.stripe;
    if !cfg.enabled || cfg.webhook_secret.is_empty() {
        return Err(AppError::unavailable("Stripe webhooks are not configured"));
    }
    let signature = single_header(&headers, "stripe-signature")?;
    if !verify_stripe_signature(
        &body,
        signature,
        &cfg.webhook_secret,
        unix_milliseconds() / 1_000,
    ) {
        return Err(AppError::unauthorized("Invalid payment signature"));
    }
    let event: Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::bad_request("Invalid payment event"))?;
    if let Some(receipt) = stripe_receipt(&event, cfg.live_mode)? {
        crate::commerce::settle_payment(&state, receipt).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn waffo_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<StatusCode> {
    let event: Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::bad_request("Invalid payment event"))?;
    let mode = event["mode"]
        .as_str()
        .ok_or_else(|| AppError::bad_request("Missing payment mode"))?;
    let cfg = state
        .cfg
        .payments
        .waffo
        .for_mode(mode)
        .map_err(|_| AppError::bad_request("Incorrect or unconfigured payment environment"))?;
    if !cfg.enabled
        || cfg.webhook_public_key_pem.is_empty()
        || cfg.store_id.is_empty()
        || !matches!(cfg.mode.as_str(), "test" | "prod")
    {
        return Err(AppError::unavailable("Waffo webhooks are not configured"));
    }
    let signature = single_header(&headers, "x-waffo-signature")?;
    if !verify_waffo_signature(
        &body,
        signature,
        &cfg.webhook_public_key_pem,
        unix_milliseconds(),
    ) {
        return Err(AppError::unauthorized("Invalid payment signature"));
    }
    if let Some(receipt) = waffo_receipt(&event, &cfg.mode, &cfg.store_id)? {
        crate::commerce::settle_payment(&state, receipt).await?;
    } else if let Some(receipt) = waffo_refund(&event, &cfg.mode, &cfg.store_id)? {
        crate::commerce::settle_refund(&state, receipt).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

fn stripe_receipt(event: &Value, live_mode: bool) -> AppResult<Option<PaymentReceipt>> {
    if event.get("livemode").and_then(Value::as_bool) != Some(live_mode) {
        return Err(AppError::bad_request("Incorrect payment environment"));
    }
    if !matches!(
        event.get("type").and_then(Value::as_str),
        Some("checkout.session.completed" | "checkout.session.async_payment_succeeded")
    ) {
        return Ok(None);
    }
    let data = event
        .pointer("/data/object")
        .ok_or_else(|| AppError::bad_request("Missing checkout session"))?;
    if data.get("payment_status").and_then(Value::as_str) != Some("paid") {
        // An asynchronously funded session can complete while still unpaid.
        return Ok(None);
    }
    if data.get("mode").and_then(Value::as_str) != Some("payment") {
        return Err(AppError::bad_request("Expected a one-time payment"));
    }
    let reference = data.get("client_reference_id").and_then(Value::as_str);
    let metadata = data
        .pointer("/metadata/hireferry_order_id")
        .and_then(Value::as_str);
    if reference.is_some() && metadata.is_some() && reference != metadata {
        return Err(AppError::bad_request("Payment order references disagree"));
    }
    let order_id = reference
        .or(metadata)
        .filter(|s| valid_id(s))
        .ok_or_else(|| AppError::bad_request("Missing payment order reference"))?
        .to_string();
    let amount_cents = data
        .get("amount_total")
        .and_then(Value::as_i64)
        .filter(|n| *n > 0)
        .ok_or_else(|| AppError::bad_request("Invalid payment amount"))?;
    let currency = required_string(data, "currency")?.to_ascii_uppercase();
    if crate::config::currency_decimals(&currency).is_none() {
        return Err(AppError::bad_request("Unexpected payment currency"));
    }
    let payment_id = data
        .get("payment_intent")
        .and_then(Value::as_str)
        .or_else(|| data.pointer("/payment_intent/id").and_then(Value::as_str))
        .filter(|s| valid_id(s))
        .ok_or_else(|| AppError::bad_request("Missing payment ID"))?
        .to_string();
    Ok(Some(PaymentReceipt {
        provider: "stripe".into(),
        event_id: required_string(event, "id")?,
        order_id,
        session_id: Some(required_string(data, "id")?),
        payment_id,
        amount_cents,
        currency,
        live_mode,
    }))
}

fn waffo_receipt(event: &Value, mode: &str, store_id: &str) -> AppResult<Option<PaymentReceipt>> {
    if event.get("mode").and_then(Value::as_str) != Some(mode)
        || event.get("storeId").and_then(Value::as_str) != Some(store_id)
    {
        return Err(AppError::bad_request(
            "Incorrect payment environment or store",
        ));
    }
    if event.get("eventType").and_then(Value::as_str) != Some("order.completed") {
        return Ok(None);
    }
    let data = event
        .get("data")
        .ok_or_else(|| AppError::bad_request("Missing payment data"))?;
    if data.get("orderStatus").and_then(Value::as_str) != Some("completed")
        || data.get("paymentStatus").and_then(Value::as_str) != Some("succeeded")
    {
        return Err(AppError::bad_request("Payment is not completed"));
    }
    let currency = required_string(data, "currency")?;
    if crate::config::currency_decimals(&currency).is_none() {
        return Err(AppError::bad_request("Unexpected payment currency"));
    }
    // Waffo's legacy amount falls back to the list price if the payment channel
    // reported no charged amount. Only chargedAmount proves what was collected;
    // accepting that fallback could grant a paid service without a known charge.
    let charged = data
        .get("chargedAmount")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad_request("Missing charged amount"))?;
    let amount_cents = decimal_to_minor(charged, &currency)
        .filter(|n| *n > 0)
        .ok_or_else(|| AppError::bad_request("Invalid charged amount"))?;
    let order_id = required_string(data, "orderMerchantExternalId")?;
    if let Some(metadata_id) = data
        .pointer("/orderMetadata/hireferry_order_id")
        .and_then(Value::as_str)
        && metadata_id != order_id
    {
        return Err(AppError::bad_request("Payment order references disagree"));
    }
    Ok(Some(PaymentReceipt {
        provider: "waffo".into(),
        event_id: required_string(event, "id")?,
        order_id,
        session_id: None,
        payment_id: required_string(data, "paymentId")?,
        amount_cents,
        currency,
        live_mode: mode == "prod",
    }))
}

fn waffo_refund(event: &Value, mode: &str, store_id: &str) -> AppResult<Option<RefundReceipt>> {
    if event.get("mode").and_then(Value::as_str) != Some(mode)
        || event.get("storeId").and_then(Value::as_str) != Some(store_id)
    {
        return Err(AppError::bad_request(
            "Incorrect refund environment or store",
        ));
    }
    let succeeded = match event.get("eventType").and_then(Value::as_str) {
        Some("refund.succeeded") => true,
        Some("refund.failed") => false,
        _ => return Ok(None),
    };
    let data = event
        .get("data")
        .ok_or_else(|| AppError::bad_request("Missing refund data"))?;
    let expected_status = if succeeded { "succeeded" } else { "failed" };
    if data.get("refundStatus").and_then(Value::as_str) != Some(expected_status)
        || data.get("paymentStatus").and_then(Value::as_str) != Some("succeeded")
        || data.get("orderStatus").and_then(Value::as_str) != Some("completed")
    {
        return Err(AppError::bad_request(
            "Refund outcome or original payment is inconsistent",
        ));
    }
    let refund_id = required_string(event, "id")?;
    let event_id = required_string(event, "eventId")?;
    if event_id != refund_id {
        return Err(AppError::bad_request("Refund identifiers disagree"));
    }
    let order_id = required_string(data, "orderMerchantExternalId")?;
    if let Some(metadata_id) = data.pointer("/orderMetadata/hireferry_order_id")
        && metadata_id.as_str() != Some(order_id.as_str())
    {
        return Err(AppError::bad_request("Refund order references disagree"));
    }
    let currency = required_string(data, "currency")?;
    if crate::config::currency_decimals(&currency).is_none() {
        return Err(AppError::bad_request("Unexpected refund currency"));
    }
    // Waffo's amount is the legacy refund amount; total/originalPayment.total
    // describe the original list price and must never be used as a refund.
    let amount_cents = data
        .get("refundedAmount")
        .or_else(|| data.get("amount"))
        .and_then(Value::as_str)
        .and_then(|value| decimal_to_minor(value, &currency))
        .filter(|n| *n > 0)
        .ok_or_else(|| AppError::bad_request("Missing or invalid refund amount"))?;
    let original_amount_cents = data
        .get("originalChargedAmount")
        .map(|value| {
            value
                .as_str()
                .and_then(|value| decimal_to_minor(value, &currency))
                .filter(|n| *n > 0)
                .ok_or_else(|| AppError::bad_request("Invalid original charged amount"))
        })
        .transpose()?;
    let reason = match data.get("refundReason") {
        None => String::new(),
        Some(Value::String(reason)) => {
            crate::safety::bounded_text(reason, 2000, "Refund reason")?;
            reason.clone()
        }
        Some(_) => return Err(AppError::bad_request("Invalid refund reason")),
    };
    Ok(Some(RefundReceipt {
        provider: "waffo".into(),
        event_id,
        refund_id,
        order_id,
        provider_order_id: required_string(data, "orderId")?,
        payment_id: required_string(data, "paymentId")?,
        amount_cents,
        original_amount_cents,
        currency,
        live_mode: mode == "prod",
        succeeded,
        reason,
    }))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn required_string(value: &Value, key: &str) -> AppResult<String> {
    required_text(value, key, 256)
}

fn required_text(value: &Value, key: &str, limit: usize) -> AppResult<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= limit && !s.chars().any(char::is_control))
        .map(str::to_string)
        .ok_or_else(|| AppError::bad_request(format!("Missing or invalid {key}")))
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> AppResult<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::unauthorized("Missing payment signature"))?;
    if values.next().is_some() || value.len() > 4096 {
        return Err(AppError::unauthorized("Ambiguous payment signature"));
    }
    Ok(value)
}

fn signature_parts(header: &str) -> Option<(u64, Vec<&str>)> {
    if header.len() > 4096 {
        return None;
    }
    let mut timestamp = None;
    let mut signatures = Vec::new();
    for part in header.split(',') {
        let (key, value) = part.trim().split_once('=')?;
        match key {
            "t" => {
                if timestamp.is_some()
                    || value.is_empty()
                    || !value.bytes().all(|b| b.is_ascii_digit())
                {
                    return None;
                }
                timestamp = Some(value.parse().ok()?);
            }
            "v1" if !value.is_empty() => signatures.push(value),
            _ => {}
        }
    }
    (!signatures.is_empty()).then_some((timestamp?, signatures))
}

fn verify_stripe_signature(body: &[u8], header: &str, secret: &str, now_seconds: u64) -> bool {
    let Some((timestamp, signatures)) = signature_parts(header) else {
        return false;
    };
    if secret.is_empty() || now_seconds.abs_diff(timestamp) > STRIPE_TOLERANCE_SECONDS {
        return false;
    }
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    // HMAC's verification compares in constant time. Multiple v1 values occur
    // during webhook-secret rotation.
    signatures
        .iter()
        .any(|s| hex::decode(s).is_ok_and(|bytes| mac.clone().verify_slice(&bytes).is_ok()))
}

fn sign_waffo_request(private_pem: &str, timestamp: u64, body: &[u8]) -> Result<String, ()> {
    let key = RsaPrivateKey::from_pkcs8_pem(private_pem)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(private_pem))
        .map_err(|_| ())?;
    if key.n().bits() < 2048 {
        return Err(());
    }
    let canonical = format!(
        "POST\n{WAFFO_CHECKOUT_PATH}\n{timestamp}\n{}",
        STANDARD.encode(Sha256::digest(body))
    );
    // Randomized signing applies RSA blinding while retaining the required
    // deterministic PKCS#1 v1.5 wire signature format.
    let signature = SigningKey::<Sha256>::new(key)
        .sign_with_rng(&mut rsa::rand_core::OsRng, canonical.as_bytes());
    Ok(STANDARD.encode(signature.to_bytes()))
}

fn verify_waffo_signature(
    body: &[u8],
    header: &str,
    public_pem: &str,
    now_milliseconds: u64,
) -> bool {
    let Some((timestamp, signatures)) = signature_parts(header) else {
        return false;
    };
    if now_milliseconds.abs_diff(timestamp) > WAFFO_TOLERANCE_MILLISECONDS {
        return false;
    }
    let Ok(key) = RsaPublicKey::from_public_key_pem(public_pem)
        .or_else(|_| RsaPublicKey::from_pkcs1_pem(public_pem))
    else {
        return false;
    };
    if key.n().bits() < 2048 {
        return false;
    }
    let key = VerifyingKey::<Sha256>::new(key);
    let mut signed = timestamp.to_string().into_bytes();
    signed.push(b'.');
    signed.extend_from_slice(body);
    signatures.iter().any(|value| {
        let Ok(bytes) = STANDARD.decode(value) else {
            return false;
        };
        let Ok(signature) = Signature::try_from(bytes.as_slice()) else {
            return false;
        };
        key.verify(&signed, &signature).is_ok()
    })
}

fn unix_milliseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn minor_to_decimal(amount: i64, currency: &str) -> Option<String> {
    let decimals = crate::config::currency_decimals(currency)?;
    if amount < 0 {
        return None;
    }
    if decimals == 0 {
        return Some(amount.to_string());
    }
    let scale = 10_i64.checked_pow(decimals)?;
    Some(format!(
        "{}.{:0width$}",
        amount / scale,
        amount % scale,
        width = decimals as usize
    ))
}

pub fn decimal_to_minor(value: &str, currency: &str) -> Option<i64> {
    let decimals = crate::config::currency_decimals(currency)? as usize;
    if value.is_empty() || value.len() > 24 {
        return None;
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (value.contains('.') && fraction.is_empty())
    {
        return None;
    }
    // Providers may pad display values with zeros. Never round away money.
    if fraction.len() > decimals && !fraction[decimals..].bytes().all(|digit| digit == b'0') {
        return None;
    }
    let fraction = &fraction[..fraction.len().min(decimals)];
    let scale = 10_i64.checked_pow(decimals as u32)?;
    let whole = whole.parse::<i64>().ok()?.checked_mul(scale)?;
    let fractional = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<i64>()
            .ok()?
            .checked_mul(10_i64.checked_pow((decimals - fraction.len()) as u32)?)?
    };
    whole.checked_add(fractional)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::signature::Signer;
    use std::sync::OnceLock;

    fn test_key() -> &'static RsaPrivateKey {
        static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
        KEY.get_or_init(|| RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap())
    }

    fn stripe_header(body: &[u8], timestamp: u64, secret: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{timestamp}.").as_bytes());
        mac.update(body);
        format!(
            "t={timestamp},v1={}",
            hex::encode(mac.finalize().into_bytes())
        )
    }

    #[test]
    fn money_is_exact_and_rejects_ambiguous_inputs() {
        for (text, cents) in [
            ("0", 0),
            ("0.01", 1),
            ("30.1", 3010),
            ("300.00", 30000),
            ("600", 60000),
        ] {
            assert_eq!(decimal_to_minor(text, "CNY"), Some(cents));
            assert_eq!(
                decimal_to_minor(&minor_to_decimal(cents, "CNY").unwrap(), "CNY"),
                Some(cents)
            );
        }
        for text in [
            "",
            "-1",
            "+1",
            " 1",
            "1 ",
            "1e2",
            "NaN",
            "1,000",
            "1.001",
            ".5",
            "1.",
            "1.2.3",
            "92233720368547758.08",
            "999999999999999999999999",
        ] {
            assert_eq!(decimal_to_minor(text, "CNY"), None, "{text}");
        }
    }

    #[test]
    fn stripe_signature_rejects_tampering_old_events_and_wrong_keys() {
        let body = br#"{"id":"evt_test","amount_total":30000}"#;
        let now = 1_800_000_000;
        let header = stripe_header(body, now, "whsec_test");
        assert!(verify_stripe_signature(body, &header, "whsec_test", now));
        assert!(!verify_stripe_signature(b"{}", &header, "whsec_test", now));
        assert!(!verify_stripe_signature(body, &header, "wrong", now));
        assert!(!verify_stripe_signature(
            body,
            &header,
            "whsec_test",
            now + 301
        ));
        assert!(!verify_stripe_signature(
            body,
            &header,
            "whsec_test",
            now - 301
        ));
        assert!(!verify_stripe_signature(
            body,
            &format!("t={now},{header}"),
            "whsec_test",
            now
        ));
        assert!(verify_stripe_signature(
            body,
            &format!("v1=0000,{header}"),
            "whsec_test",
            now
        ));
        assert!(!verify_stripe_signature(
            body,
            &header.replace("v1=", "v0="),
            "whsec_test",
            now
        ));
    }

    #[test]
    fn waffo_signature_accepts_real_retry_window_and_rejects_mutation() {
        let body = br#"{"eventType":"order.completed"}"#;
        let timestamp = 1_800_000_000_000_u64;
        let mut signed = format!("{timestamp}.").into_bytes();
        signed.extend_from_slice(body);
        let signature = SigningKey::<Sha256>::new(test_key().clone()).sign(&signed);
        let header = format!("t={timestamp},v1={}", STANDARD.encode(signature.to_bytes()));
        let public = test_key()
            .to_public_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        assert!(verify_waffo_signature(body, &header, &public, timestamp));
        assert!(verify_waffo_signature(
            body,
            &header,
            &public,
            timestamp + 32 * 60 * 1_000
        ));
        assert!(!verify_waffo_signature(
            body,
            &header,
            &public,
            timestamp + 46 * 60 * 1_000
        ));
        assert!(!verify_waffo_signature(b"{}", &header, &public, timestamp));
        assert!(!verify_waffo_signature(
            body,
            &header,
            "not a key",
            timestamp
        ));
        assert!(!verify_waffo_signature(
            body,
            &header,
            &public,
            timestamp / 1_000
        ));
    }

    #[test]
    fn waffo_request_signature_covers_path_timestamp_and_exact_bytes() {
        let key = test_key().to_pkcs8_pem(LineEnding::LF).unwrap();
        let body = br#"{"currency":"CNY","amount":"300.00"}"#;
        let signature = sign_waffo_request(&key, 1_800_000_000, body).unwrap();
        let signature =
            Signature::try_from(STANDARD.decode(signature).unwrap().as_slice()).unwrap();
        let key = VerifyingKey::<Sha256>::new(test_key().to_public_key());
        let canonical = format!(
            "POST\n/v1/actions/checkout/create-session\n1800000000\n{}",
            STANDARD.encode(Sha256::digest(body))
        );
        assert!(key.verify(canonical.as_bytes(), &signature).is_ok());
        assert!(
            key.verify(
                canonical.replace("1800000000", "1800000001").as_bytes(),
                &signature
            )
            .is_err()
        );
        assert!(
            key.verify(
                canonical
                    .replace("create-session", "create-store")
                    .as_bytes(),
                &signature
            )
            .is_err()
        );
    }

    fn stripe_event() -> Value {
        json!({"id":"evt_one","livemode":false,"type":"checkout.session.completed","data":{"object":{
            "id":"cs_one","mode":"payment","payment_status":"paid","client_reference_id":"order_one",
            "metadata":{"hireferry_order_id":"order_one"},"payment_intent":"pi_one","amount_total":30000,"currency":"cny"
        }}})
    }

    #[test]
    fn stripe_requires_paid_one_time_supported_currency_and_consistent_order() {
        let event = stripe_event();
        let receipt = stripe_receipt(&event, false).unwrap().unwrap();
        assert_eq!(receipt.amount_cents, 30000);
        assert_eq!(receipt.session_id.as_deref(), Some("cs_one"));
        assert!(stripe_receipt(&event, true).is_err());
        let mut invalid = event.clone();
        invalid["data"]["object"]["metadata"]["hireferry_order_id"] = json!("another_order");
        assert!(stripe_receipt(&invalid, false).is_err());
        invalid = event.clone();
        invalid["data"]["object"]["currency"] = json!("invalid");
        assert!(stripe_receipt(&invalid, false).is_err());
        invalid = event;
        invalid["data"]["object"]["payment_status"] = json!("unpaid");
        assert!(stripe_receipt(&invalid, false).unwrap().is_none());
    }

    #[test]
    fn waffo_uses_charged_amount_and_checks_store_and_environment() {
        let mut event = json!({"id":"PAY_one","eventType":"order.completed","mode":"test","storeId":"STO_one","data":{
            "orderStatus":"completed","paymentStatus":"succeeded","orderMerchantExternalId":"order_one",
            "orderMetadata":{"hireferry_order_id":"order_one"},"currency":"CNY","chargedAmount":"300.00",
            "amount":"300.00","listPrice":{"total":"999.00"},"paymentId":"PAY_one"
        }});
        let receipt = waffo_receipt(&event, "test", "STO_one").unwrap().unwrap();
        assert_eq!(receipt.amount_cents, 30000);
        assert!(receipt.session_id.is_none());
        assert!(waffo_receipt(&event, "prod", "STO_one").is_err());
        assert!(waffo_receipt(&event, "test", "STO_other").is_err());
        event["data"]["chargedAmount"] = json!("300.001");
        assert!(waffo_receipt(&event, "test", "STO_one").is_err());
        event["data"]
            .as_object_mut()
            .unwrap()
            .remove("chargedAmount");
        // The legacy amount remains present, but can describe a list price
        // rather than money collected when chargedAmount is absent.
        assert!(waffo_receipt(&event, "test", "STO_one").is_err());
        event["data"].as_object_mut().unwrap().remove("amount");
        assert!(waffo_receipt(&event, "test", "STO_one").is_err());
        for invalid in [Value::Null, json!("0.00"), json!(300)] {
            event["data"]["chargedAmount"] = invalid;
            event["data"]["amount"] = json!("300.00");
            assert!(waffo_receipt(&event, "test", "STO_one").is_err());
        }
        event["data"]["chargedAmount"] = json!("250.00");
        assert_eq!(
            waffo_receipt(&event, "test", "STO_one")
                .unwrap()
                .unwrap()
                .amount_cents,
            25000
        );
    }

    #[test]
    fn checkout_redirects_cannot_leave_the_verified_gateway() {
        assert!(
            validate_checkout_url(
                "https://checkout.stripe.com/c/pay/cs_test#fragment",
                "checkout.stripe.com"
            )
            .is_ok()
        );
        for url in [
            "http://checkout.stripe.com/pay",
            "https://checkout.stripe.com.evil.test/pay",
            "https://checkout.stripe.com@evil.test/pay",
            "javascript:alert(1)",
            "https://checkout.stripe.com:8443/pay",
        ] {
            assert!(
                validate_checkout_url(url, "checkout.stripe.com").is_err(),
                "{url}"
            );
        }
    }

    #[test]
    fn payment_returns_use_canonical_billing_route_and_only_status_hints() {
        for provider in ["stripe", "waffo"] {
            assert_eq!(
                return_url("https://hireferry.lmm.best", "order_one", provider, false).unwrap(),
                format!(
                    "https://hireferry.lmm.best/billing/return?order_id=order_one&provider={provider}"
                )
            );
        }
        assert_eq!(
            return_url("https://hireferry.lmm.best", "order_one", "stripe", true).unwrap(),
            "https://hireferry.lmm.best/billing/return?order_id=order_one&provider=stripe&canceled=1"
        );

        // Building a root route discards stale config paths, queries and fragments.
        // References are encoded instead of being able to inject success flags.
        let raw = return_url(
            "https://hireferry.lmm.best/old/path?paid=true#fragment",
            "order&paid=true/中文",
            "waffo",
            false,
        )
        .unwrap();
        let url = reqwest::Url::parse(&raw).unwrap();
        assert_eq!(
            url.origin().ascii_serialization(),
            "https://hireferry.lmm.best"
        );
        assert_eq!(url.path(), "/billing/return");
        assert!(url.fragment().is_none());
        assert_eq!(
            url.query_pairs().into_owned().collect::<Vec<_>>(),
            vec![
                ("order_id".to_owned(), "order&paid=true/中文".to_owned()),
                ("provider".to_owned(), "waffo".to_owned()),
            ]
        );
        for invalid in [
            "http://hireferry.lmm.best",
            "https://user:secret@hireferry.lmm.best",
            "javascript:alert(1)",
            "not a URL",
        ] {
            assert!(return_url(invalid, "order_one", "stripe", false).is_err());
        }
    }
}
