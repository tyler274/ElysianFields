//! Typed error foundations preserving underlying causes across `ef-core`.

use thiserror::Error;

/// Validation errors for typed domain identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdError {
    #[error("{kind} must not be empty or whitespace-only")]
    Empty { kind: &'static str },
    #[error("{kind} must not contain control characters")]
    ControlCharacter { kind: &'static str },
}

/// Errors returned when validating `/elysianfields` subpath URLs or loopback overrides.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SubpathError {
    #[error("path '{path}' is outside the /elysianfields mount")]
    OutsideMount { path: String },
    #[error("path contains parent directory traversal")]
    PathTraversal,
    #[error("external or protocol-relative URLs are not allowed for mounted paths")]
    ExternalUrlNotAllowed,
    #[error("path contains invalid control characters or whitespace")]
    InvalidCharacters,
    #[error("api override must use http:// or https:// to a loopback host without userinfo")]
    NonLoopbackOverrideRejected,
}

/// Top-level error enum for `ef-core` operations preserving typed error causes.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid identifier: {0}")]
    Id(#[from] IdError),
    #[error("invalid subpath: {0}")]
    Subpath(#[from] SubpathError),
    #[error("json serialization failed")]
    Json(#[from] serde_json::Error),
}
