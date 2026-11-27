//! Closed domain enums, validated identifiers, and value types for Connected Care.

use std::fmt;

use ef_core::{IdError, PatientId, PhysicianId, PracticeId};
use serde::{Deserialize, Serialize};

use crate::error::BridgeError;

macro_rules! define_bridge_id {
    ($(#[$meta:meta])* $name:ident, $label:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
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

define_bridge_id!(
    /// Standalone Curie Mobile patient account identifier (distinct from practice `PatientId`).
    MobileAccountId,
    "MobileAccountId"
);
define_bridge_id!(
    /// Practice invitation identifier.
    InvitationId,
    "InvitationId"
);
define_bridge_id!(
    /// Signed/opaque `/invite` link token emailed to a patient.
    InviteToken,
    "InviteToken"
);
define_bridge_id!(
    /// Short manual-entry invitation code emailed to a patient.
    InviteShortCode,
    "InviteShortCode"
);
define_bridge_id!(
    /// Established Connected Care connection identifier.
    ConnectionId,
    "ConnectionId"
);
define_bridge_id!(
    /// Consent-gated cross-product transfer envelope identifier.
    TransferId,
    "TransferId"
);
define_bridge_id!(
    /// Explicit 42 CFR Part 2 written consent record identifier.
    ConsentRecordId,
    "ConsentRecordId"
);

/// Monotonic incarnation fence matching the M3 `PrivacyReceipt` contract (`>= 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountIncarnation(u64);

impl AccountIncarnation {
    pub fn new(value: u64) -> Result<Self, BridgeError> {
        if value == 0 {
            return Err(BridgeError::InvalidAccountIncarnation(value));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Canonical closed enum matching `contracts/m3-boundaries-and-states.schema.json#/$defs/ConnectedCareState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConnectedCareState {
    NotInvited,
    Invited,
    InvitedLocked,
    Connected,
    Ended,
}

/// Canonical closed enum matching `contracts/m3-boundaries-and-states.schema.json#/$defs/ConfidentialityTag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConfidentialityTag {
    Standard,
    Part2Sud,
    Part2SudCounselingNotes,
}

impl ConfidentialityTag {
    #[must_use]
    pub const fn requires_part2_written_consent(self) -> bool {
        matches!(self, Self::Part2Sud | Self::Part2SudCounselingNotes)
    }
}

/// Canonical single-owner key for a practice-patient connection pair.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PairKey {
    pub practice_id: PracticeId,
    pub practice_patient_id: PatientId,
}

impl PairKey {
    #[must_use]
    pub const fn new(practice_id: PracticeId, practice_patient_id: PatientId) -> Self {
        Self {
            practice_id,
            practice_patient_id,
        }
    }
}

/// Validated calendar date of birth used for invitation verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DateOfBirth {
    year: u16,
    month: u8,
    day: u8,
}

impl DateOfBirth {
    pub fn new(year: u16, month: u8, day: u8) -> Result<Self, BridgeError> {
        let max_day = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100))
                    || year.is_multiple_of(400);
                if leap { 29 } else { 28 }
            }
            _ => {
                return Err(BridgeError::InvalidDateOfBirth { year, month, day });
            }
        };
        if !(1900..=2100).contains(&year) || day == 0 || day > max_day {
            return Err(BridgeError::InvalidDateOfBirth { year, month, day });
        }
        Ok(Self { year, month, day })
    }

    #[must_use]
    pub const fn year(self) -> u16 {
        self.year
    }

    #[must_use]
    pub const fn month(self) -> u8 {
        self.month
    }

    #[must_use]
    pub const fn day(self) -> u8 {
        self.day
    }
}

/// Credential presented by a patient via `/invite` link or manual short code.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InviteCredential {
    LinkToken(InviteToken),
    ShortCode(InviteShortCode),
}

/// Practice details revealed ONLY after date-of-birth verification succeeds (`PreviewOnlyAfterDob`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PracticePreview {
    pub invitation_id: InvitationId,
    pub practice_id: PracticeId,
    pub practice_name: String,
    pub physician_name: String,
    pub sharing_disclosure: String,
}

/// Neutral challenge returned before DOB match (leaks zero practice or physician details).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteChallenge {
    pub invitation_id: InvitationId,
    pub remaining_attempts: u8,
}

/// Outcome of submitting a date-of-birth verification attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DobVerificationOutcome {
    Verified { preview: PracticePreview },
    Mismatch { remaining_attempts: u8 },
    LockedOut,
}

/// Outbound invitation email delivery outcome (`FailedMailLeavesOpen`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MailDeliveryOutcome {
    Delivered { delivered_at: String },
    Failed { failed_at: String, reason: String },
}

/// Actor initiating a connection end or cancellation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisconnectActor {
    Patient {
        account_id: MobileAccountId,
    },
    PracticeMember {
        practice_id: PracticeId,
        physician_id: PhysicianId,
    },
    AccountDeletionReplay {
        account_id: MobileAccountId,
        account_incarnation: AccountIncarnation,
    },
}

/// Summary returned to both Curie Clinic and Curie Mobile from the single owner (`SameConnectionState`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionStanding {
    pub pair: PairKey,
    pub state: ConnectedCareState,
    pub active_invitation_id: Option<InvitationId>,
    pub active_connection_id: Option<ConnectionId>,
    pub connected_account: Option<MobileAccountId>,
}
