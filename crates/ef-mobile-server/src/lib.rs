//! Headless Curie Mobile HTTP/1.1 + WAL service (M3 Increment 2).
//!
//! Strictly product-isolated: ZERO `bevy` dependencies and ZERO `ef-clinic-*` imports.

pub mod http;
pub mod persistence;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
pub mod service;

pub use http::{
    MOBILE_MAX_BODY_BYTES, MOBILE_STRICT_CSP, MobileHttpMethod, MobileHttpRequest,
    MobileHttpResponse,
};
#[cfg(not(target_arch = "wasm32"))]
pub use persistence::{DurableMobileStore, PgMobileStore};
pub use persistence::{
    InMemoryMobileWal, MOBILE_WAL_FILE_NAME, MobileWalEntry, MobileWalError, MobileWalSink,
};
#[cfg(not(target_arch = "wasm32"))]
pub use server::TcpMobileServer;
pub use service::{
    EXPECTED_MIGRATION_VERSION, MOBILE_API_PREFIX, MobileService, MobileServiceError,
};
