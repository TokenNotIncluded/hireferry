use axum::{
    Router,
    extract::Query,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;

use crate::AppState;

const DEFAULT_LABEL: &str = "HireFerry";
const DEFAULT_MESSAGE: &str = "AI talent";
const MAX_TEXT_CHARS: usize = 28;

#[derive(Debug, Deserialize, Default)]
struct BadgeQuery {
    label: Option<String>,
    message: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/badge.svg", get(badge))
}

async fn badge(Query(query): Query<BadgeQuery>) -> Response {
    let label = display_text(
        query.label.as_deref().unwrap_or(DEFAULT_LABEL),
        DEFAULT_LABEL,
    );
    let message = display_text(
        query.message.as_deref().unwrap_or(DEFAULT_MESSAGE),
        DEFAULT_MESSAGE,
    );
    let aria = format!("{label}: {message}");
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="44" viewBox="0 0 320 44" role="img" aria-label="{aria}">
  <title>{aria}</title>
  <defs>
    <linearGradient id="ferry-badge-accent" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#ffd59a"/>
      <stop offset="1" stop-color="#f2a7a0"/>
    </linearGradient>
  </defs>
  <rect width="320" height="44" rx="12" fill="#141b2b"/>
  <path d="M12 27h22l-6 8H18zM20 9v17h-8zM24 12l9 14h-9z" fill="#8de0cf"/>
  <circle cx="38" cy="10" r="2" fill="#8de0cf" opacity=".85"/>
  <text x="51" y="28" fill="#f8fafc" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" font-size="16" font-weight="700" letter-spacing=".1">{label}</text>
  <path d="M157 8h151a8 8 0 0 1 8 8v12a8 8 0 0 1-8 8H157z" fill="url(#ferry-badge-accent)"/>
  <circle cx="174" cy="22" r="5" fill="#141b2b" opacity=".9"/>
  <path d="m171.5 22 2 2 3.8-4.2" fill="none" stroke="#ffd59a" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
  <text x="187" y="28" fill="#141b2b" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" font-size="15" font-weight="650">{message}</text>
</svg>"##,
        aria = xml_escape(&aria),
        label = xml_escape(&label),
        message = xml_escape(&message),
    );
    let mut response = svg.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300, stale-while-revalidate=86400"),
    );
    response
}

fn display_text(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return fallback.to_owned();
    }
    let mut text: String = value.chars().take(MAX_TEXT_CHARS).collect();
    if value.chars().count() > MAX_TEXT_CHARS {
        text.pop();
        text.push('…');
    }
    text
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::{display_text, xml_escape};

    #[test]
    fn badge_text_is_bounded_and_defaults_empty_values() {
        assert_eq!(display_text("", "Label"), "Label");
        assert_eq!(
            display_text("abcdefghijklmnopqrstuvwxyz123", "Label"),
            "abcdefghijklmnopqrstuvwxyz1…"
        );
    }

    #[test]
    fn badge_text_is_xml_escaped() {
        assert_eq!(xml_escape("<Hire & Ferry>"), "&lt;Hire &amp; Ferry&gt;");
    }
}
