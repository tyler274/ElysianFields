//! Pure HTTP/1.1 request/response framing, strict security headers, and SSE encoding.
//!
//! Cross-compiles cleanly on both native hosts and `wasm32-unknown-unknown`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ef_clinic_domain::chat::ChatStreamEvent;
use ef_core::CachePolicy;
use serde::Serialize;

/// Strict Content Security Policy for Curie Clinic Browser Wasm delivery.
pub const STRICT_CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; font-src 'self'; img-src 'self' data: blob:; connect-src 'self'; frame-ancestors 'none'";

/// Maximum request body size for `/elysianfields/api/auth/*` endpoints (`64 KiB`).
pub const AUTH_MAX_BODY_BYTES: usize = 64 * 1024;

/// Maximum request body size for `/commands` and general API endpoints (`2 MiB`).
pub const COMMANDS_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Supported HTTP methods for the M2 clinic vertical slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
}

impl HttpMethod {
    /// Parses an HTTP method token.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "GET" => Some(Self::Get),
            "HEAD" => Some(Self::Head),
            "POST" => Some(Self::Post),
            "PUT" => Some(Self::Put),
            _ => None,
        }
    }
}

/// Parsed HTTP/1.1 request passed into the headless clinic router.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub target: String,
    /// Header names normalized to lowercase ASCII.
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// Returns a header value using case-insensitive lookup.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Extracts an `Authorization: Bearer <token>` credential if present and non-empty.
    #[must_use]
    pub fn bearer_token(&self) -> Option<&str> {
        let raw = self.header("authorization")?.trim();
        let rest = raw
            .strip_prefix("Bearer ")
            .or_else(|| raw.strip_prefix("bearer "))?;
        let token = rest.trim();
        if token.is_empty() { None } else { Some(token) }
    }

    /// Splits the request target into `(pathname, optional_query)`.
    #[must_use]
    pub fn split_target(&self) -> (&str, Option<&str>) {
        match self.target.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (self.target.as_str(), None),
        }
    }
}

/// Structured HTTP/1.1 response emitted by the headless clinic router.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub reason: &'static str,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Constructs a JSON response carrying strict security headers and `ClinicalNoStore` headers.
    #[must_use]
    pub fn json_clinical<T: Serialize>(status: u16, payload: &T) -> Self {
        let body = serde_json::to_vec(payload)
            .unwrap_or_else(|_| br#"{"error":"serialization_failed"}"#.to_vec());
        let mut resp = Self::base(status, CachePolicy::ClinicalNoStore);
        resp.set_header("Content-Type", "application/json; charset=utf-8");
        resp.body = body;
        resp
    }

    /// Constructs an HTML response for `/elysianfields/` carrying `no-cache`, strict CSP,
    /// and zero `Last-Modified` header.
    #[must_use]
    pub fn html_entry(body: impl Into<Vec<u8>>) -> Self {
        let mut resp = Self::base(200, CachePolicy::EntryHtmlNoCache);
        resp.set_header("Content-Type", "text/html; charset=utf-8");
        resp.body = body.into();
        resp
    }

    /// Constructs a static file asset response carrying strict CSP, explicit cache policy,
    /// and zero `Last-Modified` header.
    #[must_use]
    pub fn static_asset(
        content_type: &'static str,
        cache_policy: CachePolicy,
        body: impl Into<Vec<u8>>,
    ) -> Self {
        let mut resp = Self::base(200, cache_policy);
        resp.set_header("Content-Type", content_type);
        resp.body = body.into();
        resp
    }

    /// Constructs a `429 Too Many Requests` response with `Retry-After` and `ClinicalNoStore` headers.
    #[must_use]
    pub fn too_many_requests(retry_after_secs: u64) -> Self {
        let retry = retry_after_secs.max(1);
        let mut resp = Self::json_clinical(
            429,
            &serde_json::json!({
                "error": "rate_limit_exceeded",
                "retry_after_secs": retry,
            }),
        );
        resp.set_header("Retry-After", retry.to_string());
        resp
    }

    /// Constructs a `308 Permanent Redirect` preserving the requested location.
    #[must_use]
    pub fn permanent_redirect(location: String) -> Self {
        let mut resp = Self::base(308, CachePolicy::ClinicalNoStore);
        resp.set_header("Location", location);
        resp
    }

    /// Constructs a `text/event-stream` SSE response carrying `ClinicalNoStore` headers.
    #[must_use]
    pub fn sse_stream(stream_body: String) -> Self {
        let mut resp = Self::base(200, CachePolicy::ClinicalNoStore);
        resp.set_header("Content-Type", "text/event-stream; charset=utf-8");
        resp.set_header("X-Accel-Buffering", "no");
        resp.body = stream_body.into_bytes();
        resp
    }

    /// Constructs a `404 Not Found` response with `ClinicalNoStore` headers.
    #[must_use]
    pub fn not_found() -> Self {
        Self::json_clinical(404, &serde_json::json!({ "error": "not_found" }))
    }

    /// Constructs a `413 Payload Too Large` response with `ClinicalNoStore` headers.
    #[must_use]
    pub fn payload_too_large(max_bytes: usize) -> Self {
        Self::json_clinical(
            413,
            &serde_json::json!({
                "error": "payload_too_large",
                "max_bytes": max_bytes,
            }),
        )
    }

    /// Constructs a `405 Method Not Allowed` response with `ClinicalNoStore` headers.
    #[must_use]
    pub fn method_not_allowed() -> Self {
        Self::json_clinical(405, &serde_json::json!({ "error": "method_not_allowed" }))
    }

    /// Constructs a `400 Bad Request` response with `ClinicalNoStore` headers.
    #[must_use]
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::json_clinical(
            400,
            &serde_json::json!({
                "error": "bad_request",
                "detail": message.into(),
            }),
        )
    }

    fn base(status: u16, cache_policy: CachePolicy) -> Self {
        let mut headers = vec![
            ("Content-Security-Policy".to_owned(), STRICT_CSP.to_owned()),
            ("X-Frame-Options".to_owned(), "DENY".to_owned()),
            ("X-Content-Type-Options".to_owned(), "nosniff".to_owned()),
            ("Referrer-Policy".to_owned(), "no-referrer".to_owned()),
            (
                "Cache-Control".to_owned(),
                cache_policy.cache_control_header().to_owned(),
            ),
        ];
        if let Some(cdn) = cache_policy.cdn_cache_control_header() {
            headers.push(("CDN-Cache-Control".to_owned(), cdn.to_owned()));
            headers.push(("Surrogate-Control".to_owned(), cdn.to_owned()));
        }
        Self {
            status,
            reason: status_reason(status),
            headers,
            body: Vec::new(),
        }
    }

    /// Sets or replaces a response header.
    pub fn set_header(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();
        if let Some((_, existing)) = self
            .headers
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case(&name))
        {
            *existing = value;
        } else {
            self.headers.push((name, value));
        }
    }

    /// Looks up a response header case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Serializes this response into an HTTP/1.1 wire payload.
    #[must_use]
    pub fn to_http1_bytes(&self, suppress_body: bool) -> Vec<u8> {
        let mut head = format!("HTTP/1.1 {} {}\r\n", self.status, self.reason);
        for (name, value) in &self.headers {
            let _ = write!(head, "{name}: {value}\r\n");
        }
        let _ = write!(
            head,
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            self.body.len()
        );
        let mut out = head.into_bytes();
        if !suppress_body {
            out.extend_from_slice(&self.body);
        }
        out
    }
}

/// Extracts a single query parameter value from a raw URL query string.
#[must_use]
pub fn query_param<'a>(query: Option<&'a str>, key: &str) -> Option<&'a str> {
    let query = query?;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=')
            && k == key
        {
            return Some(v);
        }
    }
    None
}

/// Formats a [`ChatStreamEvent`] into a standard Server-Sent Events (`text/event-stream`) frame.
#[must_use]
pub fn format_sse_event(event: &ChatStreamEvent) -> String {
    match event {
        ChatStreamEvent::KeepAlive => ": keep-alive\n\n".to_owned(),
        ChatStreamEvent::DeltaChunk { event_id, .. } => {
            format_sse_frame(*event_id, "delta_chunk", event)
        }
        ChatStreamEvent::StepRecorded { event_id, .. } => {
            format_sse_frame(*event_id, "step_recorded", event)
        }
        ChatStreamEvent::Completed { event_id, .. } => {
            format_sse_frame(*event_id, "completed", event)
        }
        ChatStreamEvent::Cancelled { event_id, .. } => {
            format_sse_frame(*event_id, "cancelled", event)
        }
        ChatStreamEvent::CouldntFinish { event_id, .. } => {
            format_sse_frame(*event_id, "couldnt_finish", event)
        }
        ChatStreamEvent::ResyncRequired { event_id, .. } => {
            format_sse_frame(*event_id, "resync", event)
        }
    }
}

/// Returns the event ID of a [`ChatStreamEvent`], if it carries one.
#[must_use]
pub const fn stream_event_id(event: &ChatStreamEvent) -> Option<u64> {
    match event {
        ChatStreamEvent::DeltaChunk { event_id, .. }
        | ChatStreamEvent::StepRecorded { event_id, .. }
        | ChatStreamEvent::Completed { event_id, .. }
        | ChatStreamEvent::Cancelled { event_id, .. }
        | ChatStreamEvent::CouldntFinish { event_id, .. }
        | ChatStreamEvent::ResyncRequired { event_id, .. } => Some(*event_id),
        ChatStreamEvent::KeepAlive => None,
    }
}

fn format_sse_frame(event_id: u64, event_name: &str, event: &ChatStreamEvent) -> String {
    let json = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_owned());
    format!("id: {event_id}\nevent: {event_name}\ndata: {json}\n\n")
}

const fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}
