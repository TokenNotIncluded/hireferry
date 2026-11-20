//! A bounded stdio transport bridge; authorization stays on the HireFerry server.
use crate::{
    business_token,
    chat::emit,
    error::{Error, Result},
    server::Server,
    storage::Store,
};
use clap::Subcommand;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};

#[derive(Subcommand)]
pub enum McpCommand {
    /// Forward newline-delimited MCP JSON-RPC over the authenticated HTTP connection.
    Serve,
    /// Print an Agent client configuration without embedding any credentials.
    Config,
}

pub async fn run(command: McpCommand, server: &Server, store: &Store) -> Result<Value> {
    match command {
        McpCommand::Config => Ok(
            json!({"mcpServers":{"hireferry":{"command":"hfy","args":["--server",server.origin,"--client-id",server.client_id,"--config-dir",store.directory(),"mcp","serve"]}}}),
        ),
        McpCommand::Serve => {
            let mut input = BufReader::new(tokio::io::stdin());
            while let Some(line) = bounded_line(&mut input).await? {
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let request: Value = match serde_json::from_slice(&line) {
                    Ok(request) => request,
                    Err(_) => {
                        emit(&json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Invalid JSON"}})).await?;
                        continue;
                    }
                };
                if !request.is_object()
                    || request["jsonrpc"] != "2.0"
                    || !request["method"].is_string()
                    || request
                        .get("id")
                        .is_some_and(|id| !(id.is_null() || id.is_string() || id.is_number()))
                {
                    emit(&json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid JSON-RPC request"}})).await?;
                    continue;
                }
                let result = match business_token(server, store).await {
                    Ok(token) => server.mcp_rpc(&token, &request).await,
                    Err(error) => Err(error),
                };
                match result {
                    Ok(Some(response)) => emit(&response).await?,
                    Ok(None) => {},
                    Err(error) if request.get("id").is_some() => emit(&json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":error.message,"data":{"code":error.code}}})).await?,
                    Err(error) => eprintln!("MCP notification failed [{}]: {}", error.code, error.message),
                }
            }
            Ok(Value::Null)
        }
    }
}

async fn bounded_line(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<Option<Vec<u8>>> {
    const MAX_REQUEST: usize = 512 * 1024;
    let mut line = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .await
            .map_err(|_| Error::new("input_closed", "Could not read the MCP input pipe."))?;
        if available.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let end = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|position| position + 1);
        let count = end.unwrap_or(available.len());
        if line.len() + count > MAX_REQUEST {
            return Err(Error::new(
                "mcp_request_too_large",
                "An MCP request exceeded 512 KiB.",
            ));
        }
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(Some(line));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn request_reader_handles_unterminated_eof_and_caps_memory() {
        let mut input = BufReader::with_capacity(3, &b"{\"id\":1}\n{\"id\":2}"[..]);
        assert_eq!(
            bounded_line(&mut input).await.unwrap().unwrap(),
            b"{\"id\":1}\n"
        );
        assert_eq!(
            bounded_line(&mut input).await.unwrap().unwrap(),
            b"{\"id\":2}"
        );
        assert!(bounded_line(&mut input).await.unwrap().is_none());
        let bytes = vec![b'x'; 512 * 1024 + 1];
        assert_eq!(
            bounded_line(&mut BufReader::new(bytes.as_slice()))
                .await
                .unwrap_err()
                .code,
            "mcp_request_too_large"
        );
    }
}
