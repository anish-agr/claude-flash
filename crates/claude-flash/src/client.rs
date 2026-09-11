//! A client for the agent's local API.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use flash_core::settings::{HOOK_VERSION, INGRESS_PATH, MARKER_HEADER, SESSION_HEADER};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::api::{self, ApiError, Control, Events, Health, Status};

#[derive(Debug)]
pub enum ClientError {
    /// Nothing is listening on the port.
    NotRunning,
    /// Something answered, but not the way the agent does.
    Protocol(String),
    /// The agent refused the request.
    Api { status: u16, message: String },
    Io(io::Error),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::NotRunning => f.write_str("the agent is not running"),
            ClientError::Protocol(why) => write!(f, "unexpected response from the agent port: {why}"),
            ClientError::Api { status, message } => write!(f, "the agent refused the request ({status}): {message}"),
            ClientError::Io(e) => write!(f, "could not talk to the agent: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<io::Error> for ClientError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset => ClientError::NotRunning,
            _ => ClientError::Io(e),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Client {
    port: u16,
    token: Option<String>,
}

const CONNECT_TIMEOUT: Duration = Duration::from_millis(800);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

impl Client {
    pub fn new(port: u16, token: Option<String>) -> Client {
        Client { port, token }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn health(&self) -> Result<Health, ClientError> {
        let health: Health = self.json("GET", "/v1/health", None, REQUEST_TIMEOUT)?;
        if health.name != api::NAME {
            return Err(ClientError::Protocol(format!("port {} is used by another program", self.port)));
        }
        Ok(health)
    }

    pub fn status(&self) -> Result<Status, ClientError> {
        self.json("GET", "/v1/status", None, REQUEST_TIMEOUT)
    }

    pub fn control(&self, control: &Control) -> Result<Status, ClientError> {
        let body = serde_json::to_vec(control).expect("controls always serialise");
        self.json("POST", "/v1/control", Some(&body), REQUEST_TIMEOUT)
    }

    pub fn signal(&self, signal: &Value) -> Result<(), ClientError> {
        self.json::<Value>("POST", "/v1/signal", Some(signal.to_string().as_bytes()), REQUEST_TIMEOUT).map(drop)
    }

    /// Records written after `since`, waiting up to `wait` for one. Without `since`,
    /// the most recent few.
    pub fn events(&self, since: Option<u64>, wait: Duration) -> Result<Events, ClientError> {
        let path = match since {
            Some(since) => format!("/v1/events?since={since}&wait={}", wait.as_secs()),
            None => "/v1/events".to_owned(),
        };
        self.json("GET", &path, None, wait + REQUEST_TIMEOUT)
    }

    /// Delivers a hook payload the way Claude Code's HTTP hook does.
    pub fn forward_hook(&self, payload: &[u8], session: &str) -> Result<(), ClientError> {
        let headers = [(MARKER_HEADER, HOOK_VERSION), (SESSION_HEADER, session)];
        let (status, body) = self.send("POST", INGRESS_PATH, Some(payload), &headers, REQUEST_TIMEOUT)?;
        if (200..300).contains(&status) { Ok(()) } else { Err(api_error(status, &body)) }
    }

    fn json<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<T, ClientError> {
        let (status, bytes) = self.send(method, path, body, &[], timeout)?;
        if !(200..300).contains(&status) {
            return Err(api_error(status, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<(u16, Vec<u8>), ClientError> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
        let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(|e| match e.kind() {
            // Windows retries a refused loopback connection for a couple of seconds
            // before reporting it, so a timeout here means the same thing.
            io::ErrorKind::TimedOut => ClientError::NotRunning,
            _ => ClientError::from(e),
        })?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let body = body.unwrap_or_default();
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nUser-Agent: flash/{}\r\nConnection: close\r\n",
            self.port,
            flash_core::VERSION
        );
        if let Some(token) = &self.token {
            head.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        if method != "GET" {
            head.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", body.len()));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes())?;
        stream.write_all(body)?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        parse_response(&response)
    }
}

fn api_error(status: u16, body: &[u8]) -> ClientError {
    let message = serde_json::from_slice::<ApiError>(body)
        .map(|e| e.error)
        .unwrap_or_else(|_| String::from_utf8_lossy(body).trim().to_owned());
    ClientError::Api { status, message }
}

fn parse_response(bytes: &[u8]) -> Result<(u16, Vec<u8>), ClientError> {
    let incomplete = || ClientError::Protocol("incomplete HTTP response".into());
    let end = bytes.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(incomplete)?;
    let head = std::str::from_utf8(&bytes[..end]).map_err(|_| incomplete())?;
    let status_line = head.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.") {
        return Err(ClientError::Protocol("not an HTTP response".into()));
    }
    let status = status_line.split(' ').nth(1).and_then(|s| s.parse().ok()).ok_or_else(incomplete)?;
    Ok((status, bytes[end + 4..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_and_body() {
        let (status, body) = parse_response(b"HTTP/1.1 202 Accepted\r\nContent-Length: 2\r\n\r\n{}").unwrap();
        assert_eq!((status, body.as_slice()), (202, b"{}".as_slice()));
        assert!(matches!(parse_response(b"SSH-2.0-OpenSSH\r\n\r\n"), Err(ClientError::Protocol(_))));
        assert!(matches!(parse_response(b"HTTP/1.1 200 OK\r\n"), Err(ClientError::Protocol(_))));
    }

    #[test]
    fn a_closed_port_reads_as_not_running() {
        // Bind and drop to find a port that is almost certainly free.
        let port = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        assert!(matches!(Client::new(port, None).health(), Err(ClientError::NotRunning)));
    }
}
