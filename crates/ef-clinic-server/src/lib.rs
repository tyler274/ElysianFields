//! Headless Curie Clinic M2 vertical-slice HTTP/1.1 + SSE service.
//!
//! Strictly free of rendering engine dependencies (`bevy` is forbidden).
//! The pure HTTP/SSE router and WAL model cross-compile cleanly on `wasm32-unknown-unknown`,
//! while the native TCP listener and filesystem WAL live behind `#[cfg(not(target_arch = "wasm32"))]`.

pub mod http;
pub mod persistence;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
pub mod service;

pub use http::{
    AUTH_MAX_BODY_BYTES, COMMANDS_MAX_BODY_BYTES, HttpMethod, HttpRequest, HttpResponse,
    STRICT_CSP, format_sse_event,
};
#[cfg(not(target_arch = "wasm32"))]
pub use persistence::DurableClinicStore;
pub use persistence::{InMemoryWal, WAL_FILE_NAME, WalEntry, WalError, WalSink, WalTokenKind};
#[cfg(not(target_arch = "wasm32"))]
pub use server::TcpClinicServer;
pub use service::{ClinicService, ClinicServiceError};
