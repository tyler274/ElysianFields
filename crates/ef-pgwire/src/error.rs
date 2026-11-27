//! Typed errors for `ef-pgwire` preserving underlying I/O and server causes.

use thiserror::Error;

/// Errors returned by PostgreSQL URL parsing, authentication, framing, or query execution.
#[derive(Debug, Error)]
pub enum PgError {
    #[error("invalid PostgreSQL connection URL: {detail}")]
    InvalidUrl { detail: String },

    #[error("PostgreSQL socket I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("PostgreSQL wire protocol error: {detail}")]
    Protocol { detail: String },

    #[error("PostgreSQL authentication failed: {detail}")]
    Auth { detail: String },

    #[error("PostgreSQL server error [{code}] ({severity}): {message}")]
    Server {
        severity: String,
        code: String,
        message: String,
        detail: Option<String>,
    },

    #[error("column index {index} out of bounds for row with {count} column(s)")]
    ColumnOutOfBounds { index: usize, count: usize },

    #[error("unexpected NULL value in column {column}")]
    UnexpectedNull { column: usize },

    #[error("column {column} conversion to {expected} failed: {detail}")]
    TypeConversion {
        column: usize,
        expected: &'static str,
        detail: String,
    },
}
