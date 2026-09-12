//! A deliberately small HTTP/1.1 server core for the local agent API.
//!
//! The agent only ever talks to Claude Code's hook runner and its own CLI over
//! loopback, so this implements exactly that: one request per connection, bounded
//! sizes, and the checks that keep a web page from reaching the API.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_head: usize,
    pub max_body: usize,
}

pub const DEFAULT_LIMITS: Limits = Limits { max_head: 16 * 1024, max_body: 512 * 1024 };

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn path(&self) -> &str {
        self.target.split_once('?').map_or(self.target.as_str(), |(p, _)| p)
    }

    pub fn query_param(&self, key: &str) -> Option<&str> {
        let (_, query) = self.target.split_once('?')?;
        query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (k == key).then_some(v)
        })
    }

    pub fn bearer_token(&self) -> Option<&str> {
        let value = self.header("authorization")?;
        let (scheme, token) = value.split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    HeadTooLarge,
    BodyTooLarge,
    Malformed(&'static str),
    UnsupportedTransferEncoding,
}

impl HttpError {
    pub const fn status(&self) -> u16 {
        match self {
            HttpError::HeadTooLarge => 431,
            HttpError::BodyTooLarge => 413,
            HttpError::Malformed(_) => 400,
            HttpError::UnsupportedTransferEncoding => 501,
        }
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpError::HeadTooLarge => f.write_str("request head too large"),
            HttpError::BodyTooLarge => f.write_str("request body too large"),
            HttpError::Malformed(why) => write!(f, "malformed request: {why}"),
            HttpError::UnsupportedTransferEncoding => f.write_str("unsupported transfer encoding"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parse {
    /// More bytes are needed.
    Incomplete,
    /// A full request, and how many bytes of the buffer it used.
    Complete(Request, usize),
    Invalid(HttpError),
}

/// Parses one request from the start of `buf`.
pub fn parse(buf: &[u8], limits: Limits) -> Parse {
    let Some(head_end) = find(buf, b"\r\n\r\n") else {
        return if buf.len() > limits.max_head { Parse::Invalid(HttpError::HeadTooLarge) } else { Parse::Incomplete };
    };
    if head_end > limits.max_head {
        return Parse::Invalid(HttpError::HeadTooLarge);
    }
    let Ok(head) = std::str::from_utf8(&buf[..head_end]) else {
        return Parse::Invalid(HttpError::Malformed("head is not UTF-8"));
    };
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Parse::Invalid(HttpError::Malformed("bad request line"));
    };
    if !version.starts_with("HTTP/1.") || method.is_empty() || !target.starts_with('/') {
        return Parse::Invalid(HttpError::Malformed("bad request line"));
    }
    let mut headers = Vec::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Parse::Invalid(HttpError::Malformed("bad header line"));
        };
        if name.is_empty() || name.bytes().any(|b| b.is_ascii_whitespace()) {
            return Parse::Invalid(HttpError::Malformed("bad header name"));
        }
        headers.push((name.to_owned(), value.trim().to_owned()));
    }
    let mut request = Request { method: method.to_owned(), target: target.to_owned(), headers, body: Vec::new() };
    let body_start = head_end + 4;
    let rest = &buf[body_start..];

    if let Some(te) = request.header("transfer-encoding") {
        if !te.eq_ignore_ascii_case("chunked") {
            return Parse::Invalid(HttpError::UnsupportedTransferEncoding);
        }
        return match dechunk(rest, limits.max_body) {
            Ok(Some((body, used))) => {
                request.body = body;
                Parse::Complete(request, body_start + used)
            }
            Ok(None) => Parse::Incomplete,
            Err(e) => Parse::Invalid(e),
        };
    }
    let length = match request.header("content-length") {
        None => 0,
        Some(v) => match v.parse::<usize>() {
            Ok(n) => n,
            Err(_) => return Parse::Invalid(HttpError::Malformed("bad content-length")),
        },
    };
    if length > limits.max_body {
        return Parse::Invalid(HttpError::BodyTooLarge);
    }
    if rest.len() < length {
        return Parse::Incomplete;
    }
    request.body = rest[..length].to_vec();
    Parse::Complete(request, body_start + length)
}

fn dechunk(mut buf: &[u8], max_body: usize) -> Result<Option<(Vec<u8>, usize)>, HttpError> {
    let total = buf.len();
    let mut body = Vec::new();
    loop {
        let Some(line_end) = find(buf, b"\r\n") else { return Ok(None) };
        let size_text = std::str::from_utf8(&buf[..line_end]).map_err(|_| HttpError::Malformed("bad chunk size"))?;
        let size_text = size_text.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| HttpError::Malformed("bad chunk size"))?;
        buf = &buf[line_end + 2..];
        if size == 0 {
            // Optional trailers, then the blank line that ends the message.
            let Some(end) = find(buf, b"\r\n") else { return Ok(None) };
            if end != 0 {
                let Some(trailer_end) = find(buf, b"\r\n\r\n") else { return Ok(None) };
                buf = &buf[trailer_end + 4..];
            } else {
                buf = &buf[2..];
            }
            return Ok(Some((body, total - buf.len())));
        }
        // Compared by subtraction: the size came from the client and can be anything
        // up to usize::MAX, so adding to it could overflow.
        if size > max_body.saturating_sub(body.len()) {
            return Err(HttpError::BodyTooLarge);
        }
        if buf.len() < size + 2 {
            return Ok(None);
        }
        if &buf[size..size + 2] != b"\r\n" {
            return Err(HttpError::Malformed("chunk not terminated"));
        }
        body.extend_from_slice(&buf[..size]);
        buf = &buf[size + 2..];
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

/// Serialises a complete response. Every response closes the connection and forbids
/// caching and content sniffing.
pub fn response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        reason(status),
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

pub fn json_response(status: u16, value: &serde_json::Value) -> Vec<u8> {
    response(status, "application/json", value.to_string().as_bytes())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// The Host header is not a loopback name for our port: a DNS-rebinding attempt,
    /// or a misdirected request.
    ForeignHost,
    /// The request carries browser-only headers, so a web page sent it.
    FromBrowser,
}

/// Admits only requests that a local, non-browser client could have sent.
///
/// Binding to loopback keeps the network out, but not the browser on this machine:
/// any page can POST to `127.0.0.1`. Browsers always attach `Origin` to cross-origin
/// POSTs and `Sec-Fetch-*` to every request, and cannot suppress either, while
/// command-line clients send neither. DNS rebinding is closed by requiring the
/// `Host` header to name loopback rather than an attacker's domain.
pub fn admit(request: &Request, port: u16, loopback_only: bool) -> Result<(), Rejection> {
    if request.header("origin").is_some()
        || request.headers.iter().any(|(k, _)| k.to_ascii_lowercase().starts_with("sec-fetch-"))
    {
        return Err(Rejection::FromBrowser);
    }
    // An agent that listens beyond loopback is addressed by whatever name the client
    // used, so the Host header settles nothing. The token guards every route there,
    // including the hook endpoint, and browsers are already out.
    if !loopback_only {
        return Ok(());
    }
    let host = request.header("host").ok_or(Rejection::ForeignHost)?;
    let allowed = [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")];
    if allowed.iter().any(|a| a.eq_ignore_ascii_case(host)) { Ok(()) } else { Err(Rejection::ForeignHost) }
}

/// Compares secrets in time independent of where they first differ.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_huge_chunk_size_is_refused_rather_than_overflowing() {
        let request = b"POST /x HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffff\r\nabc";
        assert_eq!(parse(request, DEFAULT_LIMITS), Parse::Invalid(HttpError::BodyTooLarge));
    }

    fn complete(raw: &[u8]) -> Request {
        match parse(raw, DEFAULT_LIMITS) {
            Parse::Complete(r, used) => {
                assert_eq!(used, raw.len());
                r
            }
            other => panic!("expected a complete request, got {other:?}"),
        }
    }

    #[test]
    fn parses_content_length_body() {
        let r = complete(b"POST /v1/hooks/claude-code HTTP/1.1\r\nHost: 127.0.0.1:47823\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}");
        assert_eq!(r.method, "POST");
        assert_eq!(r.path(), "/v1/hooks/claude-code");
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_eq!(r.body, b"{}");
    }

    #[test]
    fn reports_incomplete_until_the_body_arrives() {
        let raw = b"POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\nabc";
        assert_eq!(parse(raw, DEFAULT_LIMITS), Parse::Incomplete);
        assert_eq!(parse(b"GET / HTTP/1.1\r\nHost: x", DEFAULT_LIMITS), Parse::Incomplete);
    }

    #[test]
    fn decodes_chunked_bodies_with_extensions_and_trailers() {
        let r = complete(b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n4;ext=1\r\nWiki\r\n5\r\npedia\r\n0\r\nX-Trailer: y\r\n\r\n");
        assert_eq!(r.body, b"Wikipedia");
        let r = complete(b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n");
        assert_eq!(r.body, b"abc");
    }

    #[test]
    fn enforces_limits() {
        let limits = Limits { max_head: 64, max_body: 4 };
        assert_eq!(parse(&[b'a'; 100], limits), Parse::Invalid(HttpError::HeadTooLarge));
        assert_eq!(
            parse(b"POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\n", limits),
            Parse::Invalid(HttpError::BodyTooLarge)
        );
        assert_eq!(
            parse(b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n9\r\n123456789\r\n0\r\n\r\n", limits),
            Parse::Invalid(HttpError::BodyTooLarge)
        );
    }

    #[test]
    fn rejects_malformed_requests() {
        for raw in [
            &b"GARBAGE\r\n\r\n"[..],
            b"GET noslash HTTP/1.1\r\n\r\n",
            b"GET / FTP/1.0\r\n\r\n",
            b"GET / HTTP/1.1\r\nno colon here\r\n\r\n",
            b"GET / HTTP/1.1\r\nBad Name: x\r\n\r\n",
            b"POST / HTTP/1.1\r\nContent-Length: -1\r\n\r\n",
        ] {
            assert!(matches!(parse(raw, DEFAULT_LIMITS), Parse::Invalid(_)), "{:?}", String::from_utf8_lossy(raw));
        }
        assert_eq!(
            parse(b"POST / HTTP/1.1\r\nTransfer-Encoding: gzip\r\n\r\n", DEFAULT_LIMITS),
            Parse::Invalid(HttpError::UnsupportedTransferEncoding)
        );
    }

    fn with_headers(headers: &[(&str, &str)]) -> Request {
        Request {
            method: "POST".into(),
            target: "/v1/signal?x=1&kind=done".into(),
            headers: headers.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
            body: vec![],
        }
    }

    #[test]
    fn admits_command_line_clients_on_loopback_names() {
        for host in ["127.0.0.1:47823", "localhost:47823", "LOCALHOST:47823", "[::1]:47823"] {
            assert_eq!(admit(&with_headers(&[("Host", host)]), 47823, true), Ok(()), "{host}");
        }
    }

    #[test]
    fn an_agent_beyond_loopback_answers_to_any_name_but_never_to_a_browser() {
        // It is addressed by whatever name the other machine used, so the Host header
        // decides nothing there; the token guards every route instead.
        assert_eq!(admit(&with_headers(&[("Host", "192.168.1.5:47823")]), 47823, false), Ok(()));
        assert_eq!(admit(&with_headers(&[("Host", "desktop.local:47823")]), 47823, false), Ok(()));
        assert_eq!(admit(&with_headers(&[]), 47823, false), Ok(()));
        assert_eq!(
            admit(&with_headers(&[("Host", "192.168.1.5:47823"), ("Origin", "https://evil.example")]), 47823, false),
            Err(Rejection::FromBrowser)
        );
    }

    #[test]
    fn refuses_rebinding_and_browsers() {
        assert_eq!(admit(&with_headers(&[("Host", "evil.example:47823")]), 47823, true), Err(Rejection::ForeignHost));
        assert_eq!(admit(&with_headers(&[("Host", "127.0.0.1:1")]), 47823, true), Err(Rejection::ForeignHost));
        assert_eq!(admit(&with_headers(&[]), 47823, true), Err(Rejection::ForeignHost));
        assert_eq!(
            admit(&with_headers(&[("Host", "127.0.0.1:47823"), ("Origin", "https://evil.example")]), 47823, true),
            Err(Rejection::FromBrowser)
        );
        assert_eq!(
            admit(&with_headers(&[("Host", "127.0.0.1:47823"), ("Sec-Fetch-Mode", "no-cors")]), 47823, true),
            Err(Rejection::FromBrowser)
        );
    }

    #[test]
    fn reads_query_and_bearer() {
        let r = with_headers(&[("Authorization", "Bearer abc123 ")]);
        assert_eq!(r.query_param("kind"), Some("done"));
        assert_eq!(r.query_param("missing"), None);
        assert_eq!(r.bearer_token(), Some("abc123"));
        assert_eq!(with_headers(&[("Authorization", "Basic abc")]).bearer_token(), None);
    }

    #[test]
    fn responses_close_and_forbid_sniffing() {
        let text = String::from_utf8(response(200, "application/json", b"{}")).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Length: 2\r\n") && text.contains("Connection: close\r\n"));
        assert!(text.contains("X-Content-Type-Options: nosniff\r\n") && text.ends_with("\r\n\r\n{}"));
    }

    #[test]
    fn constant_time_eq_is_correct() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
