use crate::{AppError, AppResult, AppState, config::Config, safety};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path as FilePath};

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Legal {
    pub values: BTreeMap<String, String>,
    pub document_files: BTreeMap<String, String>,
    #[serde(skip)]
    documents: BTreeMap<String, String>,
}

impl Legal {
    pub fn load_documents(&mut self, directory: &FilePath) -> AppResult<()> {
        for (key, path) in &self.document_files {
            if default_document(key).is_none() {
                return Err(AppError::bad_request("Unknown legal document key"));
            }
            let text = std::fs::read_to_string(directory.join(path)).map_err(AppError::internal)?;
            safety::bounded_text(&text, 200_000, "Legal document")?;
            self.documents.insert(key.clone(), text);
        }
        Ok(())
    }

    pub fn validate(&self) -> AppResult<()> {
        for (key, value) in &self.values {
            if key.is_empty() || !key.bytes().all(|c| c.is_ascii_lowercase() || c == b'_') {
                return Err(AppError::bad_request("Invalid legal variable name"));
            }
            safety::bounded_text(value, 2_000, "Legal variable")?;
            if value.contains(['\n', '\r']) {
                return Err(AppError::bad_request(
                    "Legal variables must be single-line text",
                ));
            }
        }
        Ok(())
    }
}

fn default_document(key: &str) -> Option<&'static str> {
    match key {
        "terms.zh" => Some(include_str!("../legal/terms.zh.md")),
        "terms.en" => Some(include_str!("../legal/terms.en.md")),
        "privacy.zh" => Some(include_str!("../legal/privacy.zh.md")),
        "privacy.en" => Some(include_str!("../legal/privacy.en.md")),
        _ => None,
    }
}

fn markdown_text(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '(' | ')' | '#' | '!' | '|' => {
                escaped.push('\\');
                escaped.push(character);
            }
            _ => escaped.push(character),
        }
    }
    escaped
}

fn substitute(template: &str, values: &BTreeMap<String, String>) -> AppResult<String> {
    let mut output = String::new();
    let mut remaining = template;
    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let variable = &remaining[start + 2..];
        let end = variable
            .find("}}")
            .ok_or_else(|| AppError::bad_request("Unclosed legal variable"))?;
        output.push_str(
            values
                .get(&variable[..end])
                .ok_or_else(|| AppError::bad_request("Unknown legal variable"))?,
        );
        remaining = &variable[end + 2..];
    }
    output.push_str(remaining);
    Ok(output)
}

fn money(amount: i64, currency: &str) -> AppResult<String> {
    let value = crate::payments::minor_to_decimal(amount, currency)
        .ok_or_else(|| AppError::bad_request("Invalid legal pricing amount or currency"))?;
    Ok(format!("{currency} {value}"))
}

fn percent(bps: i64) -> String {
    let whole = bps / 100;
    let remainder = bps % 100;
    if remainder == 0 {
        format!("{whole}%")
    } else if remainder % 10 == 0 {
        format!("{whole}.{}%", remainder / 10)
    } else {
        format!("{whole}.{remainder:02}%")
    }
}

fn fee_rule(mode: &str, bps: i64, original: bool, english: bool) -> AppResult<String> {
    match (mode, english) {
        ("free", false) => Ok("免费，无需付款".into()),
        ("free", true) => Ok("free, with no payment required".into()),
        ("fixed", false) => Ok("按所选币种收取下表列明的固定金额".into()),
        ("fixed", true) => Ok("the fixed amount listed below for the selected currency".into()),
        ("salary_percent", false) => Ok(format!(
            "{}固定月薪底薪的 {}",
            if original { "原" } else { "所填" },
            percent(bps)
        )),
        ("salary_percent", true) => Ok(format!(
            "{} of the {}fixed base monthly salary",
            percent(bps),
            if original { "original " } else { "stated " }
        )),
        _ => Err(AppError::bad_request("Invalid legal fee mode")),
    }
}

fn currency_fee_table(cfg: &Config, english: bool) -> AppResult<String> {
    let p = &cfg.pricing;
    let mut table = if english {
        "| Currency | Publication | Each description update | Employer AI base-pay threshold |\n| --- | --- | --- | --- |\n".to_owned()
    } else {
        "| 币种 | 发布 | 每次描述修改 | 招聘方 AI 连接底薪门槛 |\n| --- | --- | --- | --- |\n"
            .to_owned()
    };
    for currency in p.currencies() {
        let amounts = p.amounts(&currency)?;
        let fee = |mode: &str, bps: i64, fixed: i64| -> AppResult<String> {
            match mode {
                "free" => Ok(if english { "Free" } else { "免费" }.into()),
                "fixed" => money(fixed, &currency),
                "salary_percent" => Ok(if english {
                    format!("{} of base pay", percent(bps))
                } else {
                    format!("底薪的 {}", percent(bps))
                }),
                _ => Err(AppError::bad_request("Invalid legal fee mode")),
            }
        };
        let publication = fee(&p.publish_mode, p.publish_bps, amounts.publish_fixed_minor)?;
        let edit = fee(&p.edit_mode, p.edit_bps, amounts.edit_fixed_minor)?;
        let threshold = if amounts.employer_ai_min_salary_minor == 0 {
            if english {
                "No salary threshold"
            } else {
                "不设薪资门槛"
            }
            .into()
        } else {
            money(amounts.employer_ai_min_salary_minor, &currency)?
        };
        table.push_str(&format!(
            "| {currency} | {publication} | {edit} | {threshold} |\n"
        ));
    }
    Ok(table)
}

pub fn document(cfg: &Config, kind: &str, language: &str) -> AppResult<Value> {
    let key = format!("{kind}.{language}");
    let default =
        default_document(&key).ok_or_else(|| AppError::not_found("Legal document not found"))?;
    let mut settings: BTreeMap<String, String> =
        serde_json::from_str(include_str!("../legal/defaults.json")).map_err(AppError::internal)?;
    settings.extend(cfg.legal.values.clone());
    let version = settings.get("version").cloned().unwrap_or_default();
    let effective_date = settings.get("effective_date").cloned().unwrap_or_default();
    let mut values: BTreeMap<_, _> = settings
        .into_iter()
        .map(|(key, value)| (key, markdown_text(&value)))
        .collect();
    values.insert("public_url".into(), markdown_text(&cfg.public_url));
    values.insert("site_name".into(), markdown_text(&cfg.site.name));
    let english = language == "en";
    let activation_price = money(cfg.pricing.onboarding_cents, &cfg.pricing.currency)?;
    values.insert("activation_price".into(), activation_price.clone());
    values.insert(
        "activation_fee".into(),
        match (cfg.pricing.onboarding_cents == 0, english) {
            (true, false) => "免费开通".into(),
            (true, true) => "Free activation".into(),
            (false, false) => format!("一次性支付 {activation_price}"),
            (false, true) => format!("A one-time {activation_price} charge"),
        },
    );
    values.insert("publish_percent".into(), percent(cfg.pricing.publish_bps));
    values.insert("edit_percent".into(), percent(cfg.pricing.edit_bps));
    values.insert(
        "publish_fee_rule".into(),
        fee_rule(
            &cfg.pricing.publish_mode,
            cfg.pricing.publish_bps,
            false,
            english,
        )?,
    );
    values.insert(
        "edit_fee_rule".into(),
        fee_rule(&cfg.pricing.edit_mode, cfg.pricing.edit_bps, true, english)?,
    );
    values.insert(
        "currency_fee_table".into(),
        currency_fee_table(cfg, english)?,
    );
    values.insert(
        "default_currency".into(),
        markdown_text(&cfg.pricing.currency),
    );
    values.insert(
        "available_currencies".into(),
        markdown_text(&cfg.pricing.currencies().join(", ")),
    );
    values.insert("max_edits".into(), cfg.pricing.max_edits.to_string());
    values.insert(
        "feedback_retention_days".into(),
        cfg.feedback.retention_days.to_string(),
    );
    values.insert(
        "feedback_max_records".into(),
        cfg.feedback.max_records.to_string(),
    );
    let contact = if cfg.site.support_email.is_empty() {
        if language == "en" {
            "the site support contact".into()
        } else {
            "网站客服".into()
        }
    } else {
        let email = safety::email(&cfg.site.support_email)?;
        format!(
            "[{}](mailto:{})",
            markdown_text(&email),
            markdown_text(&email)
        )
    };
    values.insert("support_contact".into(), contact);
    let template = cfg
        .legal
        .documents
        .get(&key)
        .map(String::as_str)
        .unwrap_or(default);
    let markdown = substitute(template, &values)?;
    Ok(
        json!({"html":safety::legal_markdown(&markdown), "version":version, "effective_date":effective_date}),
    )
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/legal/{kind}", get(get_document))
}

#[derive(Deserialize)]
struct Language {
    #[serde(default = "default_language")]
    lang: String,
}
fn default_language() -> String {
    "zh".into()
}

async fn get_document(
    State(state): State<AppState>,
    Path(kind): Path<String>,
    Query(language): Query<Language>,
) -> AppResult<Json<Value>> {
    Ok(Json(document(&state.cfg, &kind, &language.lang)?))
}
