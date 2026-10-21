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
    values.insert(
        "activation_price".into(),
        format!("CNY {:.2}", cfg.pricing.onboarding_cents as f64 / 100.0),
    );
    values.insert(
        "publish_percent".into(),
        format!("{}%", cfg.pricing.publish_bps as f64 / 100.0),
    );
    values.insert(
        "edit_percent".into(),
        format!("{}%", cfg.pricing.edit_bps as f64 / 100.0),
    );
    values.insert("max_edits".into(), cfg.pricing.max_edits.to_string());
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
