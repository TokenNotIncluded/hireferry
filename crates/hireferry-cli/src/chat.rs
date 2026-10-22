//! Native conversation APIs. Streaming commands emit one JSON object per line.
use crate::{
    business_token,
    error::{Error, Result},
    server::Server,
    storage::Store,
};
use clap::Subcommand;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Subcommand)]
pub enum ChatCommand {
    /// Show supported formats, welcome text and streaming endpoints.
    Capabilities,
    /// List your recent accessible conversations.
    List,
    /// Read one conversation, including current participants and writable status.
    Get { conversation_id: String },
    /// Open your platform conversation (administrators must supply --user-id).
    Open {
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Read recent messages, or page from a saved message cursor.
    Messages {
        conversation_id: String,
        #[arg(long, conflicts_with = "before", value_parser = clap::value_parser!(i64).range(0..))]
        after: Option<i64>,
        #[arg(long, conflicts_with = "after", value_parser = clap::value_parser!(i64).range(1..))]
        before: Option<i64>,
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(i64).range(1..=100))]
        limit: i64,
    },
    /// Send one message; reuse --request-id only for identical retries.
    Send {
        conversation_id: String,
        body: String,
        #[arg(long)]
        request_id: String,
        #[arg(long, default_value = "markdown", value_parser = ["plain", "markdown", "openui"])]
        format: String,
    },
    /// Advance your read cursor to an existing message in this conversation.
    Read {
        conversation_id: String,
        #[arg(value_parser = clap::value_parser!(i64).range(0..))]
        message_id: i64,
    },
    /// Subscribe to SSE and emit compact JSON lines; reconnect from the last event cursor.
    Watch {
        #[arg(long)]
        conversation_id: Option<String>,
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(i64).range(0..))]
        after: i64,
        /// Exit after the first data event; useful for scripts and diagnostics.
        #[arg(long)]
        once: bool,
    },
    /// Upload NDJSON records from stdin as they arrive; print server acknowledgements immediately.
    Stream { conversation_id: String },
}

pub async fn run(command: ChatCommand, server: &Server, store: &Store) -> Result<Value> {
    if let ChatCommand::Watch {
        conversation_id,
        after,
        once,
    } = command
    {
        return watch(server, store, conversation_id.as_deref(), after, once).await;
    }
    let token = business_token(server, store).await?;
    let value = match command {
        ChatCommand::Capabilities => {
            server
                .chat_get(&token, "/api/chat/capabilities", &[])
                .await?
        }
        ChatCommand::List => {
            server
                .chat_get(&token, "/api/chat/conversations", &[])
                .await?
        }
        ChatCommand::Get { conversation_id } => {
            server
                .chat_get(
                    &token,
                    &Server::conversation_path(&conversation_id, None)?,
                    &[],
                )
                .await?
        }
        ChatCommand::Open { user_id } => {
            let body = user_id.map(|id| json!({"user_id":id})).unwrap_or(json!({}));
            server
                .chat_post(&token, "/api/chat/platform", &body)
                .await?
        }
        ChatCommand::Messages {
            conversation_id,
            after,
            before,
            limit,
        } => {
            let mut query = vec![("limit", limit.to_string())];
            if let Some(after) = after {
                query.push(("after", after.to_string()));
            }
            if let Some(before) = before {
                query.push(("before", before.to_string()));
            }
            server
                .chat_get(
                    &token,
                    &Server::conversation_path(&conversation_id, Some("messages"))?,
                    &query,
                )
                .await?
        }
        ChatCommand::Send {
            conversation_id,
            body,
            request_id,
            format,
        } => {
            server
                .chat_post(
                    &token,
                    &Server::conversation_path(&conversation_id, Some("messages"))?,
                    &json!({"body":body,"client_id":request_id,"format":format}),
                )
                .await?
        }
        ChatCommand::Read {
            conversation_id,
            message_id,
        } => {
            server
                .chat_post(
                    &token,
                    &Server::conversation_path(&conversation_id, Some("read"))?,
                    &json!({"message_id":message_id}),
                )
                .await?
        }
        ChatCommand::Stream { conversation_id } => {
            let input = tokio_util::io::ReaderStream::with_capacity(
                tokio::io::stdin().take(1024 * 1024 + 1),
                4096,
            );
            let mut response = server
                .chat_upload(&token, &conversation_id, reqwest::Body::wrap_stream(input))
                .await?;
            let mut lines = Lines::default();
            let mut terminal = false;
            while let Some(chunk) = response.chunk().await.map_err(Error::network)? {
                for line in lines.push(&chunk)? {
                    if line.trim().is_empty() {
                        continue;
                    }
                    terminal = upload_ack(&line).await?;
                }
            }
            if let Some(line) = lines.finish()? {
                terminal = upload_ack(&line).await?;
            }
            if !terminal {
                return Err(Error::new(
                    "chat_stream_incomplete",
                    "The upload ended before a finish or cancel acknowledgement. Resume the same message using its saved sequence.",
                ));
            }
            return Ok(Value::Null);
        }
        ChatCommand::Watch { .. } => unreachable!(),
    };
    Ok(json!({"server":server.origin,"chat":value}))
}

async fn upload_ack(line: &str) -> Result<bool> {
    let value: Value = serde_json::from_str(line).map_err(|_| Error::invalid_response())?;
    if !matches!(value["type"].as_str(), Some("ack" | "error")) {
        return Err(Error::invalid_response());
    }
    emit(&value).await?;
    if value["type"] == "error" {
        return Err(Error::new(
            "chat_stream_rejected",
            "The server rejected a streaming record. Check the preceding JSON acknowledgement and resume only with the expected sequence.",
        ));
    }
    Ok(matches!(
        value["status"].as_str(),
        Some("complete" | "completed" | "cancelled")
    ))
}

async fn watch(
    server: &Server,
    store: &Store,
    conversation: Option<&str>,
    mut cursor: i64,
    once: bool,
) -> Result<Value> {
    let mut retries = 0u32;
    let mut auth_retries = 0u32;
    loop {
        let token = business_token(server, store).await?;
        let connection = server.chat_events(&token, conversation, cursor).await;
        let mut response = match connection {
            Ok(response) => response,
            Err(error) if error.code == "not_authenticated" && auth_retries == 0 => {
                auth_retries += 1;
                // Recheck userinfo/refresh through the same locked credential
                // path. A revoked refresh grant fails there and stops retries.
                continue;
            }
            Err(error)
                if matches!(error.code, "request_timeout" | "network_error") && retries < 5 =>
            {
                retries += 1;
                tokio::time::sleep(Duration::from_secs((1u64 << retries).min(10))).await;
                continue;
            }
            Err(error) => return Err(error),
        };
        let mut decoder = SseDecoder::default();
        'connection: loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    for event in decoder.push(&chunk)? {
                        if let Some(id) = event.id {
                            if id < cursor {
                                return Err(Error::invalid_response());
                            }
                            cursor = id;
                        }
                        let data: Value = serde_json::from_str(&event.data)
                            .map_err(|_| Error::invalid_response())?;
                        emit(&json!({"cursor":cursor,"event":event.name,"data":data})).await?;
                        retries = 0;
                        if matches!(event.name.as_str(), "error" | "auth_error") {
                            if data["status"] == 401 && auth_retries == 0 {
                                auth_retries += 1;
                                break 'connection;
                            }
                            return Err(Error::new(
                                "chat_stream_closed",
                                "The server closed this subscription. Verify your current conversation access before reconnecting.",
                            ));
                        }
                        auth_retries = 0;
                        if event.name == "resync" || data["resync_required"] == true {
                            return Err(Error::new(
                                "chat_resync_required",
                                "The event history was trimmed. Reload recent messages and restart from the supplied event cursor.",
                            ));
                        }
                        if once {
                            return Ok(Value::Null);
                        }
                    }
                }
                Ok(None) => break,
                Err(error) if error.is_timeout() => break,
                Err(error) => {
                    retries += 1;
                    if retries > 5 {
                        return Err(Error::network(error));
                    }
                    break;
                }
            }
        }
        // Avoid tight reconnect loops when a proxy closes an idle stream.
        tokio::time::sleep(Duration::from_secs(1u64 << retries.min(3))).await;
    }
}

pub(crate) async fn emit(value: &Value) -> Result<()> {
    let mut output = tokio::io::stdout();
    let mut bytes = serde_json::to_vec(value).map_err(|_| Error::invalid_response())?;
    bytes.push(b'\n');
    output
        .write_all(&bytes)
        .await
        .map_err(|_| Error::new("output_closed", "The output pipe was closed."))?;
    output
        .flush()
        .await
        .map_err(|_| Error::new("output_closed", "The output pipe was closed."))
}

const MAX_FRAME: usize = 512 * 1024;
#[derive(Default)]
struct Lines {
    bytes: Vec<u8>,
}
impl Lines {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        let mut lines = Vec::new();
        for piece in bytes.split_inclusive(|byte| *byte == b'\n') {
            if self.bytes.len() + piece.len() > MAX_FRAME {
                return Err(Error::invalid_response());
            }
            self.bytes.extend_from_slice(piece);
            if piece.last() == Some(&b'\n') {
                self.bytes.pop();
                if self.bytes.last() == Some(&b'\r') {
                    self.bytes.pop();
                }
                lines.push(
                    String::from_utf8(std::mem::take(&mut self.bytes))
                        .map_err(|_| Error::invalid_response())?,
                );
            }
        }
        Ok(lines)
    }
    fn finish(&mut self) -> Result<Option<String>> {
        if self.bytes.is_empty() {
            return Ok(None);
        }
        String::from_utf8(std::mem::take(&mut self.bytes))
            .map(Some)
            .map_err(|_| Error::invalid_response())
    }
}

#[derive(Default)]
struct SseDecoder {
    lines: Lines,
    name: String,
    id: Option<i64>,
    data: String,
}
struct Event {
    name: String,
    id: Option<i64>,
    data: String,
}
impl SseDecoder {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        for line in self.lines.push(bytes)? {
            if line.is_empty() {
                if !self.data.is_empty() {
                    self.data.pop();
                    events.push(Event {
                        name: if self.name.is_empty() {
                            "message".into()
                        } else {
                            std::mem::take(&mut self.name)
                        },
                        id: self.id.take(),
                        data: std::mem::take(&mut self.data),
                    });
                }
                self.name.clear();
                self.id = None;
            } else if !line.starts_with(':') {
                let (field, value) = line.split_once(':').unwrap_or((&line, ""));
                let value = value.strip_prefix(' ').unwrap_or(value);
                match field {
                    "data" => {
                        if self.data.len() + value.len() + 1 > MAX_FRAME {
                            return Err(Error::invalid_response());
                        }
                        self.data.push_str(value);
                        self.data.push('\n');
                    }
                    "id" => {
                        self.id = Some(
                            value
                                .parse::<i64>()
                                .ok()
                                .filter(|id| *id >= 0)
                                .ok_or_else(Error::invalid_response)?,
                        );
                    }
                    "event" if value.len() <= 128 => self.name = value.to_owned(),
                    "event" => return Err(Error::invalid_response()),
                    _ => {}
                }
            }
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sse_handles_utf8_crlf_and_frame_boundaries_without_loss() {
        let bytes = "id: 19\r\nevent: message.delta\r\ndata: {\"delta\":\"聘渡\"}\r\n\r\n: heartbeat\n\nid: 20\ndata: {\"done\":true}\n\n".as_bytes();
        let mut decoder = SseDecoder::default();
        let mut result = Vec::new();
        for byte in bytes {
            result.extend(decoder.push(&[*byte]).unwrap());
        }
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, Some(19));
        assert_eq!(result[0].name, "message.delta");
        assert_eq!(
            serde_json::from_str::<Value>(&result[0].data).unwrap()["delta"],
            "聘渡"
        );
        assert_eq!(result[1].name, "message");
        assert_eq!(result[1].id, Some(20));
    }
    #[test]
    fn streaming_parsers_bound_unterminated_and_multiline_input() {
        assert!(Lines::default().push(&vec![b'x'; MAX_FRAME + 1]).is_err());
        let mut decoder = SseDecoder::default();
        for _ in 0..128 {
            decoder
                .push(format!("data: {}\n", "x".repeat(4000)).as_bytes())
                .unwrap();
        }
        assert!(
            decoder
                .push(format!("data: {}\n", "x".repeat(20000)).as_bytes())
                .is_err()
        );
        assert!(SseDecoder::default().push(b"id: -1\ndata: {}\n\n").is_err());
    }
}
