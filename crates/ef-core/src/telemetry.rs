//! Shared zero-PHI HTTP telemetry, `X-Request-Id` correlation, route template redaction,
//! and graceful shutdown primitives across all Elysian Fields HTTP servers.
//!
//! Zero external dependencies beyond workspace `serde`/`serde_json`/`thiserror`;
//! cross-compiles cleanly on both native hosts and `wasm32-unknown-unknown`.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Canonical HTTP header name for end-to-end request correlation.
pub const REQUEST_ID_HEADER: &str = "X-Request-Id";

/// Environment variable opting into single-line JSON access logs on `stderr` (`1`).
pub const ACCESS_LOG_JSON_ENV: &str = "EF_ACCESS_LOG_JSON";

/// Default bounded capacity for in-memory [`AccessLogRingBuffer`] inspection.
pub const DEFAULT_ACCESS_LOG_CAPACITY: usize = 256;

static NEXT_REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);

/// Validated HTTP correlation identifier propagated via `X-Request-Id`.
///
/// Restricted to ASCII `[A-Za-z0-9._-]{1,64}` to prevent header injection, CRLF splitting,
/// quote smuggling, or unbounded log growth.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct HttpRequestId(String);

/// Module-scoped alias matching the telemetry specification while avoiding collision
/// with [`crate::id::RequestId`] (the numeric `u64` stale-response fence).
pub type RequestId = HttpRequestId;

impl HttpRequestId {
    /// Maximum permitted ASCII byte length for an HTTP request correlation ID.
    pub const MAX_LEN: usize = 64;

    /// Parses and validates a raw `X-Request-Id` header value (`[A-Za-z0-9._-]{1,64}`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.is_empty() || raw.len() > Self::MAX_LEN {
            return None;
        }
        if raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            Some(Self(raw.to_owned()))
        } else {
            None
        }
    }

    /// Preserves a valid incoming `X-Request-Id` verbatim, or generates a deterministic
    /// monotonic `ef-<prefix>-<seq>` identifier when missing or invalid.
    #[must_use]
    pub fn parse_or_generate(incoming: Option<&str>, service_prefix: &str) -> Self {
        if let Some(valid) = incoming.and_then(Self::parse) {
            return valid;
        }
        let clean_prefix: String = service_prefix
            .bytes()
            .filter(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            .take(24)
            .map(char::from)
            .collect();
        let prefix = if clean_prefix.is_empty() {
            "svc"
        } else {
            clean_prefix.as_str()
        };
        let seq = NEXT_REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
        Self(format!("ef-{prefix}-{seq}"))
    }

    /// Returns the validated ASCII request ID slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for HttpRequestId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HttpRequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for HttpRequestId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| {
            serde::de::Error::custom(
                "HttpRequestId must match ASCII [A-Za-z0-9._-]{1,64} without control characters",
            )
        })
    }
}

/// Sanitized HTTP route template guaranteed free of query strings and dynamic entity IDs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ZeroPhiRouteTemplate(String);

impl ZeroPhiRouteTemplate {
    /// Normalizes a raw HTTP request target into a zero-PHI route template.
    #[must_use]
    pub fn from_target(raw_target: &str) -> Self {
        Self(normalize_route_template(raw_target))
    }

    /// Returns the normalized route template string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ZeroPhiRouteTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Strips query strings/fragments and redacts dynamic entity/session/token path segments
/// into `:id` so raw patient, workspace, conversation, or session identifiers never enter logs.
#[must_use]
pub fn normalize_route_template(raw_target: &str) -> String {
    let without_fragment = raw_target.split_once('#').map_or(raw_target, |(p, _)| p);
    let path_only = without_fragment
        .split_once('?')
        .map_or(without_fragment, |(p, _)| p);

    if path_only.is_empty() || path_only == "/" {
        return "/".to_owned();
    }

    let trailing_slash = path_only.ends_with('/') && path_only.len() > 1;
    let raw_segments: Vec<&str> = path_only.split('/').filter(|s| !s.is_empty()).collect();
    if raw_segments.is_empty() {
        return "/".to_owned();
    }

    let mut normalized = Vec::with_capacity(raw_segments.len());
    let mut prev_raw: Option<&str> = None;
    let mut prev_prev_raw: Option<&str> = None;

    for seg in raw_segments {
        let should_redact = is_contextual_dynamic_id(prev_prev_raw, prev_raw, seg)
            || is_intrinsic_dynamic_segment(seg);
        if should_redact {
            normalized.push(":id");
        } else {
            normalized.push(seg);
        }
        prev_prev_raw = prev_raw;
        prev_raw = Some(seg);
    }

    let mut out = String::with_capacity(path_only.len() + 1);
    out.push('/');
    out.push_str(&normalized.join("/"));
    if trailing_slash {
        out.push('/');
    }
    out
}

fn is_contextual_dynamic_id(prev_prev: Option<&str>, prev: Option<&str>, seg: &str) -> bool {
    match prev {
        Some(
            "patients" | "workspaces" | "chats" | "citations" | "sources" | "replies"
            | "invitations",
        ) => true,
        Some("patient-workspaces") => seg != "events",
        Some("drafts") => true,
        _ => prev_prev == Some("drafts"),
    }
}

fn is_intrinsic_dynamic_segment(seg: &str) -> bool {
    if seg.contains('@') || is_uuid_like(seg) {
        return true;
    }
    if seg.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    if has_dynamic_id_prefix(seg) {
        return true;
    }
    if seg.len() >= 24 && !has_static_asset_extension(seg) {
        let has_digit = seg.bytes().any(|b| b.is_ascii_digit());
        let is_hex = seg.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
        if has_digit || is_hex {
            return true;
        }
    }
    false
}

fn has_dynamic_id_prefix(seg: &str) -> bool {
    let lower = seg.to_ascii_lowercase();
    const PREFIXES: &[&str] = &[
        "ws-", "pw-", "pat-", "chat-", "sess-", "tok-", "inv-", "conv-", "run-", "cmd-", "src-",
        "usr-", "acct-", "turn-", "msg-",
    ];
    PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix) && lower.len() > prefix.len())
}

fn is_uuid_like(seg: &str) -> bool {
    if seg.len() != 36 {
        return false;
    }
    let bytes = seg.as_bytes();
    for (idx, b) in bytes.iter().enumerate() {
        if matches!(idx, 8 | 13 | 18 | 23) {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn has_static_asset_extension(seg: &str) -> bool {
    let lower = seg.to_ascii_lowercase();
    [
        ".html",
        ".htm",
        ".js",
        ".mjs",
        ".css",
        ".wasm",
        ".json",
        ".webmanifest",
        ".svg",
        ".png",
        ".woff2",
        ".txt",
        ".xml",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

/// Typed violation returned by [`assert_zero_phi`] when an access log line carries
/// malformed JSON, forbidden PHI/secret keys, query strings, or email-like tokens.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TelemetryPhiViolation {
    #[error("access log line must be a single line without CR/LF characters")]
    MultilineLog,
    #[error("access log line is not valid JSON: {detail}")]
    InvalidJson { detail: String },
    #[error("access log JSON root must be an object")]
    NonObjectRoot,
    #[error("forbidden PHI or secret field `{field}` present in access log")]
    ForbiddenField { field: String },
    #[error("forbidden query string delimiter `?` present in access log")]
    QueryStringPresent,
    #[error("forbidden email-like marker `@` present in access log")]
    EmailMarkerPresent,
}

/// Forbidden JSON field names that must never appear in zero-PHI structured access logs.
const FORBIDDEN_PHI_FIELDS: &[&str] = &[
    "email",
    "dob",
    "prompt",
    "note",
    "token",
    "authorization",
    "password",
    "patient_name",
];

/// Verifies that a serialized JSON access log line contains zero PHI/secret keys,
/// zero query strings, and zero email markers.
pub fn assert_zero_phi(json_line: &str) -> Result<(), TelemetryPhiViolation> {
    if json_line.contains('\n') || json_line.contains('\r') {
        return Err(TelemetryPhiViolation::MultilineLog);
    }
    let value: serde_json::Value =
        serde_json::from_str(json_line).map_err(|err| TelemetryPhiViolation::InvalidJson {
            detail: err.to_string(),
        })?;
    let Some(map) = value.as_object() else {
        return Err(TelemetryPhiViolation::NonObjectRoot);
    };
    check_object_zero_phi(map)?;
    if json_line.contains('?') {
        return Err(TelemetryPhiViolation::QueryStringPresent);
    }
    if json_line.contains('@') {
        return Err(TelemetryPhiViolation::EmailMarkerPresent);
    }
    Ok(())
}

fn check_object_zero_phi(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), TelemetryPhiViolation> {
    for (key, val) in map {
        let lower = key.to_ascii_lowercase();
        if FORBIDDEN_PHI_FIELDS.iter().any(|f| *f == lower) {
            return Err(TelemetryPhiViolation::ForbiddenField { field: key.clone() });
        }
        if let Some(nested) = val.as_object() {
            check_object_zero_phi(nested)?;
        }
    }
    Ok(())
}

/// Zero-PHI structured JSON access log event emitted by all Elysian Fields HTTP servers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZeroPhiAccessEvent {
    pub timestamp_secs: u64,
    pub service: String,
    pub request_id: HttpRequestId,
    pub method: String,
    pub route_template: ZeroPhiRouteTemplate,
    pub status: u16,
    pub latency_us: u64,
}

impl ZeroPhiAccessEvent {
    /// Serializes this access event into a compact single-line JSON record.
    #[must_use]
    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|_| "{\"error\":\"access_event_serialization_failed\"}".to_owned())
    }

    /// Validates that this event's serialized JSON satisfies all zero-PHI invariants.
    pub fn assert_zero_phi(&self) -> Result<(), TelemetryPhiViolation> {
        assert_zero_phi(&self.to_json_line())
    }
}

/// Lightweight cloneable shutdown signal for deterministic graceful server drain.
#[derive(Debug, Clone, Default)]
pub struct ShutdownSignal {
    triggered: Arc<AtomicBool>,
}

impl ShutdownSignal {
    /// Creates a new untriggered shutdown signal.
    #[must_use]
    pub fn new() -> Self {
        Self {
            triggered: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Triggers graceful shutdown for every server listening on this signal.
    pub fn trigger(&self) {
        self.triggered.store(true, Ordering::SeqCst);
    }

    /// Returns `true` once [`Self::trigger`] has been called.
    #[must_use]
    pub fn is_triggered(&self) -> bool {
        self.triggered.load(Ordering::SeqCst)
    }
}

/// Bounded thread-safe ring buffer of [`ZeroPhiAccessEvent`] records with opt-in
/// `EF_ACCESS_LOG_JSON=1` single-line JSON emission to `stderr`.
#[derive(Debug, Clone)]
pub struct AccessLogRingBuffer {
    capacity: usize,
    emit_stderr: bool,
    events: Arc<Mutex<VecDeque<ZeroPhiAccessEvent>>>,
}

impl AccessLogRingBuffer {
    /// Creates an access log sink reading `EF_ACCESS_LOG_JSON=1` from the environment
    /// on native targets (always quiet on `wasm32`).
    #[must_use]
    pub fn from_env(capacity: usize) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let emit_stderr = std::env::var(ACCESS_LOG_JSON_ENV).as_deref() == Ok("1");
        #[cfg(target_arch = "wasm32")]
        let emit_stderr = false;
        Self::new(capacity, emit_stderr)
    }

    /// Creates an access log sink with explicit capacity and `stderr` emission flag.
    #[must_use]
    pub fn new(capacity: usize, emit_stderr: bool) -> Self {
        let bounded = capacity.max(1);
        Self {
            capacity: bounded,
            emit_stderr,
            events: Arc::new(Mutex::new(VecDeque::with_capacity(bounded.min(64)))),
        }
    }

    /// Records a [`ZeroPhiAccessEvent`], emitting compact JSON to `stderr` when enabled.
    pub fn record(&self, event: ZeroPhiAccessEvent) {
        if self.emit_stderr {
            #[cfg(not(target_arch = "wasm32"))]
            {
                eprintln!("{}", event.to_json_line());
            }
        }
        if let Ok(mut guard) = self.events.lock() {
            if guard.len() >= self.capacity {
                let _ = guard.pop_front();
            }
            guard.push_back(event);
        }
    }

    /// Returns a chronological snapshot of retained access events.
    #[must_use]
    pub fn snapshot(&self) -> Vec<ZeroPhiAccessEvent> {
        self.events
            .lock()
            .map(|guard| guard.iter().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_validates_ascii_and_generates_deterministic_fallback() {
        let valid = HttpRequestId::parse_or_generate(Some("ef-compose-trace-42.a_1"), "clinic");
        assert_eq!(valid.as_str(), "ef-compose-trace-42.a_1");

        // Reject CRLF, spaces, quotes, control chars, empty, and >64 chars.
        let crlf = HttpRequestId::parse_or_generate(Some("bad\r\nX-Injected: 1"), "clinic");
        assert!(crlf.as_str().starts_with("ef-clinic-"));

        let spaces = HttpRequestId::parse_or_generate(Some("bad id"), "mobile");
        assert!(spaces.as_str().starts_with("ef-mobile-"));

        let quotes = HttpRequestId::parse_or_generate(Some("bad\"quote"), "public");
        assert!(quotes.as_str().starts_with("ef-public-"));

        let too_long = "a".repeat(65);
        assert!(HttpRequestId::parse(&too_long).is_none());
        let generated = HttpRequestId::parse_or_generate(Some(&too_long), "clinic");
        assert!(generated.as_str().starts_with("ef-clinic-"));

        let missing_a = RequestId::parse_or_generate(None, "clinic");
        let missing_b = RequestId::parse_or_generate(None, "clinic");
        assert_ne!(missing_a, missing_b);
    }

    #[test]
    fn route_template_strips_query_and_redacts_dynamic_ids() {
        assert_eq!(normalize_route_template("/"), "/");
        assert_eq!(
            normalize_route_template("/dei/search?q=alice@example.com&state=CA"),
            "/dei/search"
        );
        assert_eq!(
            normalize_route_template(
                "/elysianfields/api/workspaces/ws-patient-99/chats/550e8400-e29b-41d4-a716-446655440000/commands?token=secret"
            ),
            "/elysianfields/api/workspaces/:id/chats/:id/commands"
        );
        assert_eq!(
            normalize_route_template("/elysianfields/api/patients/12345/memory"),
            "/elysianfields/api/patients/:id/memory"
        );
        assert_eq!(
            normalize_route_template("/elysianfields/api/patient-workspaces/events"),
            "/elysianfields/api/patient-workspaces/events"
        );
        assert_eq!(
            normalize_route_template("/elysianfields/api/patient-workspaces/pw-42"),
            "/elysianfields/api/patient-workspaces/:id"
        );
        assert_eq!(
            normalize_route_template("/elysianfields/mobile/manifest.webmanifest"),
            "/elysianfields/mobile/manifest.webmanifest"
        );
        assert_eq!(
            normalize_route_template(
                "/elysianfields/api/connected-care/invitations/inv-abc123/cancel"
            ),
            "/elysianfields/api/connected-care/invitations/:id/cancel"
        );
    }

    #[test]
    fn zero_phi_access_event_passes_and_checker_rejects_phi_fields_and_queries() {
        let event = ZeroPhiAccessEvent {
            timestamp_secs: 1_760_000_000,
            service: "ef-clinic-server".to_owned(),
            request_id: HttpRequestId::parse_or_generate(Some("req-100"), "clinic"),
            method: "POST".to_owned(),
            route_template: ZeroPhiRouteTemplate::from_target(
                "/elysianfields/api/workspaces/ws-1/chats/chat-9/commands?email=doc@clinic.org",
            ),
            status: 200,
            latency_us: 420,
        };
        let line = event.to_json_line();
        assert_eq!(event.assert_zero_phi(), Ok(()));
        assert_eq!(assert_zero_phi(&line), Ok(()));
        assert!(line.contains("\"/elysianfields/api/workspaces/:id/chats/:id/commands\""));

        assert_eq!(
            assert_zero_phi(r#"{"email":"doc@clinic.org","status":200}"#),
            Err(TelemetryPhiViolation::ForbiddenField {
                field: "email".to_owned()
            })
        );
        assert_eq!(
            assert_zero_phi(r#"{"patient_name":"Jane Doe","status":200}"#),
            Err(TelemetryPhiViolation::ForbiddenField {
                field: "patient_name".to_owned()
            })
        );
        assert_eq!(
            assert_zero_phi(r#"{"route_template":"/dei?q=test","status":200}"#),
            Err(TelemetryPhiViolation::QueryStringPresent)
        );
    }

    #[test]
    fn shutdown_signal_and_ring_buffer_work_deterministically() {
        let sig = ShutdownSignal::new();
        let clone = sig.clone();
        assert!(!sig.is_triggered());
        clone.trigger();
        assert!(sig.is_triggered());

        let ring = AccessLogRingBuffer::new(2, false);
        for status in [200u16, 404, 503] {
            ring.record(ZeroPhiAccessEvent {
                timestamp_secs: 1,
                service: "ef-test".to_owned(),
                request_id: HttpRequestId::parse_or_generate(None, "test"),
                method: "GET".to_owned(),
                route_template: ZeroPhiRouteTemplate::from_target("/healthz"),
                status,
                latency_us: 10,
            });
        }
        let snap = ring.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].status, 404);
        assert_eq!(snap[1].status, 503);
    }
}
