//! Pure HTTP/1.1 framing and security headers for `/elysianfields/mobile-api/...`.
//!
//! Cross-compiles cleanly on both native hosts and `wasm32-unknown-unknown`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ef_core::CachePolicy;
use serde::Serialize;

/// Strict CSP header returned on all Curie Mobile API responses.
pub const MOBILE_STRICT_CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; font-src 'self'; img-src 'self' data: blob:; connect-src 'self'; frame-ancestors 'none'";

/// Maximum request body cap for mobile API endpoints (`2 MiB`).
pub const MOBILE_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Supported HTTP methods for the Curie Mobile HTTP API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MobileHttpMethod {
    Get,
    Post,
}

impl MobileHttpMethod {
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "GET" => Some(Self::Get),
            "POST" => Some(Self::Post),
            _ => None,
        }
    }

    /// Returns the canonical uppercase HTTP method token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// Parsed HTTP/1.1 request routed by [`crate::MobileService`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileHttpRequest {
    pub method: MobileHttpMethod,
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl MobileHttpRequest {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    #[must_use]
    pub fn bearer_token(&self) -> Option<&str> {
        let raw = self.header("authorization")?.trim();
        let rest = raw
            .strip_prefix("Bearer ")
            .or_else(|| raw.strip_prefix("bearer "))?;
        let tok = rest.trim();
        if tok.is_empty() { None } else { Some(tok) }
    }

    #[must_use]
    pub fn split_target(&self) -> (&str, Option<&str>) {
        match self.target.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (self.target.as_str(), None),
        }
    }
}

/// Structured HTTP/1.1 response carrying strict security & `ClinicalNoStore` headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileHttpResponse {
    pub status: u16,
    pub reason: &'static str,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl MobileHttpResponse {
    #[must_use]
    pub fn json<T: Serialize>(status: u16, payload: &T) -> Self {
        let body = serde_json::to_vec(payload)
            .unwrap_or_else(|_| br#"{"error":"serialization_failed"}"#.to_vec());
        let policy = CachePolicy::ClinicalNoStore;
        let mut headers = vec![
            (
                "Content-Type".to_owned(),
                "application/json; charset=utf-8".to_owned(),
            ),
            (
                "Content-Security-Policy".to_owned(),
                MOBILE_STRICT_CSP.to_owned(),
            ),
            ("X-Frame-Options".to_owned(), "DENY".to_owned()),
            ("X-Content-Type-Options".to_owned(), "nosniff".to_owned()),
            ("Referrer-Policy".to_owned(), "no-referrer".to_owned()),
            (
                "Cache-Control".to_owned(),
                policy.cache_control_header().to_owned(),
            ),
        ];
        if let Some(cdn) = policy.cdn_cache_control_header() {
            headers.push(("CDN-Cache-Control".to_owned(), cdn.to_owned()));
            headers.push(("Surrogate-Control".to_owned(), cdn.to_owned()));
        }
        Self {
            status,
            reason: status_reason(status),
            headers,
            body,
        }
    }

    #[must_use]
    pub fn not_found() -> Self {
        Self::json(404, &serde_json::json!({ "error": "not_found" }))
    }

    #[must_use]
    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::json(
            400,
            &serde_json::json!({ "error": "bad_request", "detail": detail.into() }),
        )
    }

    #[must_use]
    pub fn method_not_allowed() -> Self {
        Self::json(405, &serde_json::json!({ "error": "method_not_allowed" }))
    }

    /// Constructs a zero-PHI Prometheus exposition (`0.0.4`) response carrying `ClinicalNoStore` headers.
    #[must_use]
    pub fn prometheus_metrics(body: String) -> Self {
        let policy = CachePolicy::ClinicalNoStore;
        let mut headers = vec![
            (
                "Content-Type".to_owned(),
                "text/plain; version=0.0.4; charset=utf-8".to_owned(),
            ),
            (
                "Content-Security-Policy".to_owned(),
                MOBILE_STRICT_CSP.to_owned(),
            ),
            ("X-Frame-Options".to_owned(), "DENY".to_owned()),
            ("X-Content-Type-Options".to_owned(), "nosniff".to_owned()),
            ("Referrer-Policy".to_owned(), "no-referrer".to_owned()),
            (
                "Cache-Control".to_owned(),
                policy.cache_control_header().to_owned(),
            ),
        ];
        if let Some(cdn) = policy.cdn_cache_control_header() {
            headers.push(("CDN-Cache-Control".to_owned(), cdn.to_owned()));
            headers.push(("Surrogate-Control".to_owned(), cdn.to_owned()));
        }
        Self {
            status: 200,
            reason: status_reason(200),
            headers,
            body: body.into_bytes(),
        }
    }

    /// Sets or replaces a response header case-insensitively.
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

    /// Looks up a response header value case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    #[must_use]
    pub fn to_http1_bytes(&self) -> Vec<u8> {
        let mut head = format!("HTTP/1.1 {} {}\r\n", self.status, self.reason);
        for (k, v) in &self.headers {
            let _ = write!(head, "{k}: {v}\r\n");
        }
        let _ = write!(
            head,
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            self.body.len()
        );
        let mut out = head.into_bytes();
        out.extend_from_slice(&self.body);
        out
    }
}

const fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}
