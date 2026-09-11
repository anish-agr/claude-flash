//! The loopback HTTP server. Claude Code's hook events arrive here, and so do the
//! CLI's requests.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use flash_core::duration;
use flash_core::event::{HookEvent, Signal};
use flash_core::http::{self, DEFAULT_LIMITS, Parse, Rejection, Request as HttpRequest};
use flash_core::settings::{INGRESS_PATH, MARKER_HEADER, SESSION_HEADER, session_opted_out};
use serde::Serialize;
use serde_json::json;

use crate::api::{self, Control, Events, Health};
use crate::hub::Hub;
use crate::log;
use crate::runtime::{Request, Shared};

const MAX_CONNECTIONS: usize = 64;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_WAIT_SECS: u64 = 30;

pub struct Server {
    pub listener: TcpListener,
    pub port: u16,
    pub token: String,
    pub requests: Sender<Request>,
    pub hub: Arc<Hub>,
    pub shared: Arc<Shared>,
}

impl Server {
    pub fn spawn(self) -> io::Result<()> {
        let server = Arc::new(self);
        thread::Builder::new().name("http".into()).spawn(move || server.accept()).map(drop)
    }

    fn accept(self: Arc<Self>) {
        let active = Arc::new(AtomicUsize::new(0));
        for stream in self.listener.incoming() {
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(e) => {
                    log!("accepting a connection failed: {e}");
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
            };
            if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                active.fetch_sub(1, Ordering::SeqCst);
                let _ = stream.write_all(&error(503, "too many connections"));
                continue;
            }
            let (server, counter) = (Arc::clone(&self), Arc::clone(&active));
            let spawned =
                thread::Builder::new().name("http-connection".into()).stack_size(256 * 1024).spawn(move || {
                    server.serve(stream);
                    counter.fetch_sub(1, Ordering::SeqCst);
                });
            if spawned.is_err() {
                active.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    fn serve(&self, mut stream: TcpStream) {
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_nodelay(true);
        let response = match read_request(&mut stream) {
            Ok(request) => self.route(&request),
            Err(response) => response,
        };
        let _ = stream.write_all(&response);
        let _ = stream.shutdown(Shutdown::Write);
    }

    fn route(&self, request: &HttpRequest) -> Vec<u8> {
        if let Err(rejection) = http::admit(request, self.port) {
            return error(
                403,
                match rejection {
                    Rejection::FromBrowser => "requests from web pages are refused",
                    Rejection::ForeignHost => "the Host header does not name this machine's loopback address",
                },
            );
        }
        let method = request.method.as_str();
        match request.path() {
            "/v1/health" if method == "GET" => ok(&Health {
                name: api::NAME.to_owned(),
                version: flash_core::VERSION.to_owned(),
                api: api::VERSION,
                pid: std::process::id(),
            }),
            INGRESS_PATH if method == "POST" => self.hook(request),
            path @ ("/v1/status" | "/v1/control" | "/v1/signal" | "/v1/events") => {
                if !self.authorized(request) {
                    return error(401, "missing or incorrect API token");
                }
                match (method, path) {
                    ("GET", "/v1/status") => self.status(),
                    ("POST", "/v1/control") => self.control(request),
                    ("POST", "/v1/signal") => self.signal(request),
                    ("GET", "/v1/events") => self.events(request),
                    _ => error(405, "method not allowed"),
                }
            }
            "/v1/health" | INGRESS_PATH => error(405, "method not allowed"),
            _ => error(404, "not found"),
        }
    }

    fn authorized(&self, request: &HttpRequest) -> bool {
        request.bearer_token().is_some_and(|token| http::constant_time_eq(token.as_bytes(), self.token.as_bytes()))
    }

    fn hook(&self, request: &HttpRequest) -> Vec<u8> {
        if request.header(MARKER_HEADER).is_none() {
            return error(400, "not a Claude Flash hook: the X-Claude-Flash header is missing");
        }
        if request.header(SESSION_HEADER).is_some_and(session_opted_out) {
            self.shared.opted_out.fetch_add(1, Ordering::Relaxed);
            return no_decision();
        }
        match HookEvent::from_slice(&request.body) {
            Ok(event) => {
                let _ = self.requests.send(Request::Hook(event));
                no_decision()
            }
            Err(e) => error(400, &e.to_string()),
        }
    }

    fn signal(&self, request: &HttpRequest) -> Vec<u8> {
        match serde_json::from_slice::<Signal>(&request.body) {
            Ok(signal) => {
                let _ = self.requests.send(Request::Signal(signal));
                http::json_response(202, &json!({ "accepted": true }))
            }
            Err(e) => error(400, &format!("not a valid signal: {e}")),
        }
    }

    fn control(&self, request: &HttpRequest) -> Vec<u8> {
        let control: Control = match serde_json::from_slice(&request.body) {
            Ok(control) => control,
            Err(e) => return error(400, &format!("not a valid control: {e}")),
        };
        if let Control::Pause { duration } = &control
            && let Err(e) = duration::parse(duration)
        {
            return error(400, &e.to_string());
        }
        let (reply, answer) = mpsc::sync_channel(1);
        if self.requests.send(Request::Control(control, Some(reply))).is_err() {
            return error(503, "the agent is shutting down");
        }
        match answer.recv_timeout(REPLY_TIMEOUT) {
            Ok(Ok(status)) => ok(&status),
            Ok(Err(message)) => error(400, &message),
            Err(_) => error(503, "the agent did not respond"),
        }
    }

    fn status(&self) -> Vec<u8> {
        let (reply, answer) = mpsc::sync_channel(1);
        if self.requests.send(Request::Status(reply)).is_err() {
            return error(503, "the agent is shutting down");
        }
        match answer.recv_timeout(REPLY_TIMEOUT) {
            Ok(status) => ok(&status),
            Err(_) => error(503, "the agent did not respond"),
        }
    }

    fn events(&self, request: &HttpRequest) -> Vec<u8> {
        let wait = request.query_param("wait").and_then(|w| w.parse::<u64>().ok()).unwrap_or(25).min(MAX_WAIT_SECS);
        let (records, next) = match request.query_param("since").and_then(|s| s.parse::<u64>().ok()) {
            Some(since) => self.hub.since(since, Duration::from_secs(wait)),
            None => self.hub.recent(20),
        };
        ok(&Events { next, records })
    }
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, Vec<u8>> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 16 * 1024];
    let mut continued = false;
    loop {
        match http::parse(&buf, DEFAULT_LIMITS) {
            Parse::Complete(request, _) => return Ok(request),
            Parse::Invalid(e) => return Err(error(e.status(), &e.to_string())),
            Parse::Incomplete => {}
        }
        if !continued && expects_continue(&buf) {
            continued = true;
            let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err(error(400, "the connection closed before the request was complete")),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return Err(error(408, "timed out waiting for the request")),
        }
    }
}

/// curl, among others, waits for a go-ahead before it sends a large body.
fn expects_continue(buf: &[u8]) -> bool {
    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { return false };
    String::from_utf8_lossy(&buf[..end]).lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("expect") && value.trim().eq_ignore_ascii_case("100-continue")
        })
    })
}

/// A 2xx with an empty body: the hook succeeded and makes no decision, so Claude
/// Code carries on exactly as it would without Claude Flash.
fn no_decision() -> Vec<u8> {
    http::response(200, "application/json", b"")
}

fn ok(value: &impl Serialize) -> Vec<u8> {
    http::response(200, "application/json", &serde_json::to_vec(value).expect("API bodies always serialise"))
}

fn error(status: u16, message: &str) -> Vec<u8> {
    http::json_response(status, &json!({ "error": message }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_expect_continue_only_in_the_head() {
        assert!(expects_continue(b"POST / HTTP/1.1\r\nExpect: 100-continue\r\n\r\n"));
        assert!(!expects_continue(b"POST / HTTP/1.1\r\nExpect: 100-continue\r\n"));
        assert!(!expects_continue(b"POST / HTTP/1.1\r\nHost: x\r\n\r\nExpect: 100-continue"));
    }
}
