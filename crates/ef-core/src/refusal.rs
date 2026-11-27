//! Structured authentication and authorization refusal codes.

use serde::{Deserialize, Serialize};

/// Canonical machine-readable refusal reason returned by auth/session endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCode {
    NoSession,
    SessionEnded,
    AccessRemoved,
}

impl RefusalCode {
    /// Returns the HTTP status code mandated for this refusal on session verification.
    #[must_use]
    pub const fn http_status_code(self) -> u16 {
        match self {
            Self::NoSession | Self::SessionEnded => 401,
            Self::AccessRemoved => 403,
        }
    }

    /// Canonical wire string representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoSession => "no_session",
            Self::SessionEnded => "session_ended",
            Self::AccessRemoved => "access_removed",
        }
    }
}

/// Wire refusal payload paired with `Cache-Control: no-store`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredRefusal {
    pub detail: String,
    pub reason: RefusalCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<RefusalCode>,
}

impl StructuredRefusal {
    /// Refusal for missing or blank bearer credentials.
    #[must_use]
    pub fn no_session() -> Self {
        Self {
            detail: "Sign in to read the practice.".to_owned(),
            reason: RefusalCode::NoSession,
            code: None,
        }
    }

    /// Refusal for expired, signed-out, unknown, or replayed credentials.
    #[must_use]
    pub fn session_ended() -> Self {
        Self {
            detail: "Sign in to read the practice.".to_owned(),
            reason: RefusalCode::SessionEnded,
            code: None,
        }
    }

    /// Refusal for deactivated accounts or revoked practice access.
    #[must_use]
    pub fn access_removed() -> Self {
        Self {
            detail: "Access removed.".to_owned(),
            reason: RefusalCode::AccessRemoved,
            code: Some(RefusalCode::AccessRemoved),
        }
    }
}
