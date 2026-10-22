//! Public installers use deployment configuration; no credential is embedded.
use crate::AppState;
use axum::{Router, extract::State, response::IntoResponse, routing::get};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/install.sh", get(shell))
        .route("/install.ps1", get(powershell))
}
fn render(state: &AppState, source: &str) -> String {
    source
        .replace(
            "{{REPOSITORY}}",
            &state.cfg.cli_distribution.release_repository,
        )
        .replace(
            "{{TAG_PREFIX}}",
            &state.cfg.cli_distribution.release_tag_prefix,
        )
        .replace("{{SERVER}}", &state.cfg.public_url)
}
async fn shell(State(state): State<AppState>) -> impl IntoResponse {
    (
        [
            ("content-type", "text/plain; charset=utf-8"),
            ("cache-control", "no-cache"),
        ],
        render(&state, include_str!("../static/install.sh")),
    )
}
async fn powershell(State(state): State<AppState>) -> impl IntoResponse {
    (
        [
            ("content-type", "text/plain; charset=utf-8"),
            ("cache-control", "no-cache"),
        ],
        render(&state, include_str!("../static/install.ps1")),
    )
}
