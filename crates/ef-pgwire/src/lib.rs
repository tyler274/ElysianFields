//! Pure-Rust PostgreSQL Wire Protocol v3.0 client (`ef-pgwire`).
//!
//! Strictly headless and product-neutral (zero `bevy`, zero `ef-clinic-*`, zero `ef-mobile-*`,
//! zero third-party networking/crypto crates). The URL parser, SCRAM-SHA-256 cryptography,
//! wire framing, and typed row/param models cross-compile cleanly on `wasm32-unknown-unknown`,
//! while TCP socket execution lives behind `#[cfg(not(target_arch = "wasm32"))]`.

#[cfg(not(target_arch = "wasm32"))]
pub mod client;
pub mod config;
pub mod crypto;
pub mod error;
pub mod protocol;
pub mod types;

#[cfg(not(target_arch = "wasm32"))]
pub use client::PgClient;
pub use config::PgConnectionConfig;
pub use error::PgError;
pub use types::{PgColumn, PgParam, PgRow};
