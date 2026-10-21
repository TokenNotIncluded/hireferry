use crate::{
    error::{Error, Result},
    server::protocol_error,
};
use std::{collections::BTreeMap, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{Instant, timeout_at},
};
use url::Url;

const MAX_HEADERS: usize = 8192;

pub async fn wait_for_code(
    listener: TcpListener,
    state: &str,
    issuer: &str,
    deadline: Instant,
) -> Result<String> {
    let host = listener
        .local_addr()
        .map_err(|_| listener_error())?
        .to_string();
    loop {
        let (mut connection, peer) = timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| Error::timeout())?
            .map_err(|_| listener_error())?;
        if !peer.ip().is_loopback() {
            continue;
        }
        let read_deadline = deadline.min(Instant::now() + Duration::from_secs(2));
        let request = timeout_at(read_deadline, read_headers(&mut connection)).await;
        let outcome = match request {
            Ok(Ok(request)) => parse_callback(&request, &host, state, issuer),
            _ => None,
        };
        let (status, message) = match &outcome {
            Some(Ok(_)) => (
                "200 OK",
                "Return to the terminal to finish HireFerry sign-in.",
            ),
            Some(Err(_)) => (
                "200 OK",
                "Sign-in was not approved. Return to the terminal.",
            ),
            None => (
                "400 Bad Request",
                "This is not the expected HireFerry sign-in response.",
            ),
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{message}",
            message.len()
        );
        let _ = timeout_at(
            deadline.min(Instant::now() + Duration::from_secs(1)),
            connection.write_all(response.as_bytes()),
        )
        .await;
        if let Some(outcome) = outcome {
            return outcome;
        }
    }
}

async fn read_headers(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        let count = stream.read(&mut buffer).await?;
        if count == 0 || bytes.len() + count > MAX_HEADERS {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            return String::from_utf8(bytes).map_err(|_| std::io::ErrorKind::InvalidData.into());
        }
    }
}

fn parse_callback(
    request: &str,
    expected_host: &str,
    state: &str,
    issuer: &str,
) -> Option<Result<String>> {
    let mut lines = request.split("\r\n");
    let mut line = lines.next()?.split_whitespace();
    if line.next()? != "GET" {
        return None;
    }
    let target = line.next()?;
    if !matches!(line.next()?, "HTTP/1.1" | "HTTP/1.0")
        || line.next().is_some()
        || !target.starts_with('/')
        || target.starts_with("//")
        || target.split('?').next()? != "/callback"
    {
        return None;
    }
    let mut host = None;
    for line in lines.take_while(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("host") {
            if host.is_some() {
                return None;
            }
            host = Some(value.trim());
        }
        if name.eq_ignore_ascii_case("transfer-encoding")
            || (name.eq_ignore_ascii_case("content-length") && value.trim() != "0")
        {
            return None;
        }
    }
    if host? != expected_host {
        return None;
    }
    let url = Url::parse(&format!("http://{expected_host}{target}")).ok()?;
    if url.path() != "/callback" || url.fragment().is_some() {
        return None;
    }
    let mut query = BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if query.insert(key.into_owned(), value.into_owned()).is_some() {
            return None;
        }
    }
    if !equal_state(query.get("state")?, state) || query.get("iss")? != issuer {
        return None;
    }
    match (query.get("code"), query.get("error")) {
        (Some(code), None) if !code.is_empty() && code.len() <= 4096 => Some(Ok(code.clone())),
        (None, Some(error)) => Some(Err(protocol_error(error))),
        _ => None,
    }
}

fn equal_state(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

pub fn listener_error() -> Error {
    Error::new(
        "callback_listener",
        "Could not open the local login callback. Try hfy auth login --device.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(query: &str) -> String {
        format!("GET /callback?{query} HTTP/1.1\r\nHost: 127.0.0.1:12345\r\n\r\n")
    }

    #[test]
    fn callback_requires_state_issuer_and_one_value_per_field() {
        let valid = request("code=code&state=random&iss=https%3A%2F%2Fexample.com");
        let parse = |request: &str| {
            parse_callback(request, "127.0.0.1:12345", "random", "https://example.com")
        };
        assert_eq!(parse(&valid).unwrap().unwrap(), "code");
        for bad in [
            valid.replace("random", "wrong"),
            valid.replace("example.com", "attacker.com"),
            valid.replace("code=code", "code=code&code=other"),
            valid.replace("state=random", "state=random&state=wrong"),
            valid.replace("/callback?", "/other?"),
            valid.replace("/callback?", "/other/../callback?"),
            valid.replace("GET ", "POST "),
            valid.replace("Host: 127.0.0.1:12345", "Host: attacker.example"),
            valid.replace("code=code&", ""),
            valid.replace("code=code", "code=code&error=access_denied"),
            valid.replace("iss=https%3A%2F%2Fexample.com", ""),
        ] {
            assert!(parse(&bad).is_none(), "{bad}");
        }
        let denied = request("error=access_denied&state=random&iss=https%3A%2F%2Fexample.com");
        assert_eq!(parse(&denied).unwrap().unwrap_err().code, "access_denied");
    }

    #[tokio::test]
    async fn an_invalid_connection_does_not_consume_the_callback() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let waiter = tokio::spawn(wait_for_code(
            listener,
            "random",
            "https://example.com",
            Instant::now() + Duration::from_secs(5),
        ));
        let mut bad = TcpStream::connect(address).await.unwrap();
        bad.write_all(format!("GET /callback?code=x&state=wrong&iss=https%3A%2F%2Fexample.com HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes()).await.unwrap();
        let mut response = String::new();
        bad.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 400"));
        let mut valid = TcpStream::connect(address).await.unwrap();
        valid.write_all(format!("GET /callback?code=good&state=random&iss=https%3A%2F%2Fexample.com HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes()).await.unwrap();
        assert_eq!(waiter.await.unwrap().unwrap(), "good");
    }
}
