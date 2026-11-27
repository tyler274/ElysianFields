//! Validated typed identifiers preventing cross-entity string mix-ups.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::IdError;

macro_rules! define_string_id {
    ($(#[$meta:meta])* $name:ident, $label:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Parses and validates a non-empty identifier without control characters.
            pub fn new(raw: impl Into<String>) -> Result<Self, IdError> {
                let value = raw.into();
                if value.trim().is_empty() {
                    return Err(IdError::Empty { kind: $label });
                }
                if value.chars().any(char::is_control) {
                    return Err(IdError::ControlCharacter { kind: $label });
                }
                Ok(Self(value))
            }

            /// Returns the underlying string slice.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let raw = String::deserialize(deserializer)?;
                Self::new(raw).map_err(serde::de::Error::custom)
            }
        }
    };
}

define_string_id!(
    /// Authenticated session bearer identifier.
    SessionId,
    "SessionId"
);
define_string_id!(
    /// Clinical practice tenant identifier.
    PracticeId,
    "PracticeId"
);
define_string_id!(
    /// Authenticated physician account identifier.
    PhysicianId,
    "PhysicianId"
);
define_string_id!(
    /// Patient identity identifier.
    PatientId,
    "PatientId"
);
define_string_id!(
    /// Practice-scoped patient workspace identifier.
    WorkspaceId,
    "WorkspaceId"
);
define_string_id!(
    /// Conversation thread identifier within a patient workspace.
    ChatId,
    "ChatId"
);
define_string_id!(
    /// Clinical conversation turn identifier.
    TurnId,
    "TurnId"
);
define_string_id!(
    /// Inline citation identifier.
    CitationId,
    "CitationId"
);
define_string_id!(
    /// Durable command identifier.
    CommandId,
    "CommandId"
);
define_string_id!(
    /// Durable work item identifier for an exchange execution.
    WorkItemId,
    "WorkItemId"
);
define_string_id!(
    /// Run identifier fencing `work.resume` and `Try again` transitions.
    RunId,
    "RunId"
);
define_string_id!(
    /// Completed assistant response identifier.
    ResponseId,
    "ResponseId"
);
define_string_id!(
    /// Execution attempt identifier.
    AttemptId,
    "AttemptId"
);
define_string_id!(
    /// Cited clinical source key scoped to a patient workspace.
    SourceKey,
    "SourceKey"
);
define_string_id!(
    /// Client-supplied idempotency key for `POST /commands`.
    IdempotencyKey,
    "IdempotencyKey"
);
define_string_id!(
    /// Browser tab or window identifier for cross-tab session coordination.
    TabId,
    "TabId"
);

/// Monotonic numeric request identifier used for stale-response fencing (e.g., citation panel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(u64);

impl RequestId {
    /// Creates a `RequestId` from a numeric value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the next monotonic `RequestId`.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// Returns the raw monotonic value.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
