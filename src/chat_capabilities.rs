//! Public discovery for chat clients. No conversation or account data is exposed.
use axum::{Json, Router, extract::State, routing::get};
use serde_json::{Value, json};

use crate::{AppState, chat_stream};

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/chat/capabilities", get(http_capabilities))
}

pub fn capabilities(state: &AppState) -> Value {
    json!({
        "version": 1,
        "formats": ["plain", "markdown", "openui"],
        "default_format": "markdown",
        "markdown": ["headings", "lists", "task_lists", "tables", "links", "blockquotes", "strikethrough", "code", "math"],
        "openui": {
            "library": "hireferry@1",
            "spec_url": "/static/chat-openui.json",
            "source": "https://github.com/thesysdev/open-intelligent-ui",
            "actions": "draft_only",
            "automatic_tool_calls": false
        },
        "transport": {
            "messages": "/api/chat/conversations/{conversation_id}/messages",
            "events": "/api/chat/events",
            "subscribe": "/api/chat/events/stream",
            "input": "/api/chat/conversations/{conversation_id}/stream",
            "input_content_type": "application/x-ndjson",
            "output_content_type": "text/event-stream",
            "resume_header": "Last-Event-ID",
            "mcp": "/mcp"
        },
        "limits": {
            "message_characters": chat_stream::MAX_MESSAGE_CHARS,
            "delta_characters": chat_stream::MAX_DELTA_CHARS,
            "stream_chunks": chat_stream::MAX_STREAM_CHUNKS,
            "active_streams_per_sender": chat_stream::MAX_ACTIVE_STREAMS,
            "stream_idle_seconds": chat_stream::STREAM_IDLE_SECONDS,
            "event_page": 100,
            "retained_events": chat_stream::EVENT_RETENTION
        },
        "welcome": state.cfg.chat.public()["welcome"],
        "authentication": "Use the current browser session or an authorized Bearer token. HumanGrant access is scoped through MCP.",
        "streaming": "Persist next_cursor, not a message ID. Apply deltas once in sequence. On resync_required, reload the conversation snapshot and its event_cursor."
    })
}

async fn http_capabilities(State(state): State<AppState>) -> Json<Value> {
    Json(capabilities(&state))
}
