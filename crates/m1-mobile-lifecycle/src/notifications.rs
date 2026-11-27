//! Typed push notification payload validation and tap-to-open routing.
//!
//! Handles both cold-start launch from a closed application state and warm
//! navigation while active through one unified routing rule.

use std::fmt;

use crate::state_restore::{LaunchMode, RecordId, RecordIdError, ScreenRoute};

const MAX_NOTIFICATION_ID_LEN: usize = 128;

/// Validated notification identifier delivered by FCM or APNs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NotificationId(String);

/// Validation failure for a [`NotificationId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationIdError {
    Empty,
    TooLong { len: usize, max: usize },
    InvalidCharacter { byte: u8 },
}

impl fmt::Display for NotificationIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "notification ID must not be empty"),
            Self::TooLong { len, max } => {
                write!(f, "notification ID length {len} exceeds maximum {max}")
            }
            Self::InvalidCharacter { byte } => {
                write!(f, "notification ID contains invalid byte 0x{byte:02x}")
            }
        }
    }
}

impl std::error::Error for NotificationIdError {}

impl NotificationId {
    /// Parses and validates a notification ID (`[A-Za-z0-9._-]`, 1..=128 bytes).
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, NotificationIdError> {
        let raw = raw.as_ref();
        if raw.is_empty() {
            return Err(NotificationIdError::Empty);
        }
        if raw.len() > MAX_NOTIFICATION_ID_LEN {
            return Err(NotificationIdError::TooLong {
                len: raw.len(),
                max: MAX_NOTIFICATION_ID_LEN,
            });
        }
        for &byte in raw.as_bytes() {
            let valid =
                byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte == b'.';
            if !valid {
                return Err(NotificationIdError::InvalidCharacter { byte });
            }
        }
        Ok(Self(raw.to_owned()))
    }

    /// Returns the validated notification ID string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Target destination embedded in a push notification payload.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NotificationDestination {
    Conversation { record_id: RecordId },
    RecordDetail { record_id: RecordId },
    VitalsOverview,
}

/// Validated notification tap payload delivered from Android FCM or iOS APNs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationPayload {
    pub id: NotificationId,
    pub destination: NotificationDestination,
    pub launch_mode: LaunchMode,
}

/// Wire kind discriminator parsed once at the native bridge boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationKind {
    Conversation,
    RecordDetail,
    VitalsOverview,
}

/// Errors produced when parsing a notification payload at the platform boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationError {
    InvalidId(NotificationIdError),
    MissingTargetRecordId { kind: NotificationKind },
    InvalidTargetRecordId(RecordIdError),
}

impl fmt::Display for NotificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(err) => write!(f, "invalid notification ID: {err}"),
            Self::MissingTargetRecordId { kind } => {
                write!(f, "notification kind {kind:?} requires a target record ID")
            }
            Self::InvalidTargetRecordId(err) => {
                write!(f, "invalid notification target record ID: {err}")
            }
        }
    }
}

impl std::error::Error for NotificationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidId(err) => Some(err),
            Self::InvalidTargetRecordId(err) => Some(err),
            Self::MissingTargetRecordId { .. } => None,
        }
    }
}

impl NotificationPayload {
    /// Parses typed platform bridge fields into a validated [`NotificationPayload`].
    pub fn parse_from_bridge(
        raw_id: &str,
        kind: NotificationKind,
        raw_target_record_id: Option<&str>,
        launch_mode: LaunchMode,
    ) -> Result<Self, NotificationError> {
        let id = NotificationId::parse(raw_id).map_err(NotificationError::InvalidId)?;
        let destination = match kind {
            NotificationKind::Conversation => {
                let raw = raw_target_record_id
                    .ok_or(NotificationError::MissingTargetRecordId { kind })?;
                let record_id =
                    RecordId::parse(raw).map_err(NotificationError::InvalidTargetRecordId)?;
                NotificationDestination::Conversation { record_id }
            }
            NotificationKind::RecordDetail => {
                let raw = raw_target_record_id
                    .ok_or(NotificationError::MissingTargetRecordId { kind })?;
                let record_id =
                    RecordId::parse(raw).map_err(NotificationError::InvalidTargetRecordId)?;
                NotificationDestination::RecordDetail { record_id }
            }
            NotificationKind::VitalsOverview => NotificationDestination::VitalsOverview,
        };

        Ok(Self {
            id,
            destination,
            launch_mode,
        })
    }
}

/// Routed screen update derived from a tapped push notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRouteResult {
    pub screen: ScreenRoute,
    pub active_record_id: Option<RecordId>,
}

/// Single owner mapping notification taps to mobile screen routes.
pub struct NotificationRouter;

impl NotificationRouter {
    /// Resolves a notification payload directly to its target screen route and
    /// active record context.
    #[must_use]
    pub fn route(payload: &NotificationPayload) -> NotificationRouteResult {
        match &payload.destination {
            NotificationDestination::Conversation { record_id } => NotificationRouteResult {
                screen: ScreenRoute::Conversation {
                    record_id: record_id.clone(),
                },
                active_record_id: Some(record_id.clone()),
            },
            NotificationDestination::RecordDetail { record_id } => NotificationRouteResult {
                screen: ScreenRoute::RecordDetail {
                    record_id: record_id.clone(),
                },
                active_record_id: Some(record_id.clone()),
            },
            NotificationDestination::VitalsOverview => NotificationRouteResult {
                screen: ScreenRoute::VitalsOverview,
                active_record_id: None,
            },
        }
    }
}
