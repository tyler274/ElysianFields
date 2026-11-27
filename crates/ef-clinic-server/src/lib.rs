//! Headless Curie Clinic HTTP/1.1 + SSE service.
//!
//! Strictly free of rendering engine dependencies (`bevy` is forbidden).
//! The pure HTTP/SSE router, rate limiter, workflows, Connected Care bridge, and WAL model
//! cross-compile cleanly on `wasm32-unknown-unknown`, while the native TCP listener,
//! static file server, and filesystem WAL live behind `#[cfg(not(target_arch = "wasm32"))]`.

pub mod http;
pub mod persistence;
pub mod rate_limit;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
pub mod service;
#[cfg(not(target_arch = "wasm32"))]
pub mod static_files;
pub mod workflows;

pub use http::{
    AUTH_MAX_BODY_BYTES, COMMANDS_MAX_BODY_BYTES, HttpMethod, HttpRequest, HttpResponse,
    STRICT_CSP, format_sse_event,
};
#[cfg(not(target_arch = "wasm32"))]
pub use persistence::DurableClinicStore;
pub use persistence::{
    InMemoryWal, WAL_FILE_NAME, WalAutoMatchSignal, WalEntry, WalError, WalMoveDestination,
    WalSink, WalTokenKind,
};
pub use rate_limit::{IpSlidingWindowLimiter, RateLimitConfig, RateLimitDecision};
#[cfg(not(target_arch = "wasm32"))]
pub use server::TcpClinicServer;
pub use service::{ClinicService, ClinicServiceError};
