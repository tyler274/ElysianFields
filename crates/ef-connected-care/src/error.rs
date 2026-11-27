//! Typed errors for the Connected Care bridge preserving root causes.

use ef_core::IdError;
use thiserror::Error;

use crate::types::{
    ConnectedCareState, ConnectionId, InvitationId, MobileAccountId, PairKey, TransferId,
};

/// Domain errors returned by Connected Care state transitions and transfer guards.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BridgeError {
    #[error("invalid identifier: {0}")]
    InvalidId(#[from] IdError),

    #[error("invalid date of birth: {year:04}-{month:02}-{day:02}")]
    InvalidDateOfBirth { year: u16, month: u8, day: u8 },

    #[error("account incarnation must be >= 1, got {0}")]
    InvalidAccountIncarnation(u64),

    #[error("empty field '{field}' is not permitted")]
    EmptyField { field: &'static str },

    #[error("pair ({practice}, {patient}) already has state {state:?}; cannot open another invitation", practice = pair.practice_id, patient = pair.practice_patient_id)]
    CannotInviteInState {
        pair: PairKey,
        state: ConnectedCareState,
    },

    #[error("no open invitation exists for pair ({practice}, {patient})", practice = pair.practice_id, patient = pair.practice_patient_id)]
    NoOpenInvitation { pair: PairKey },

    #[error("invitation '{invitation_id}' not found or credential is no longer valid")]
    InvalidOrRevokedCredential { invitation_id: String },

    #[error(
        "invitation '{invitation_id}' is permanently locked after 5 failed date-of-birth attempts; cancel and issue a fresh invitation"
    )]
    InvitationLocked { invitation_id: InvitationId },

    #[error("invitation '{invitation_id}' has already been cancelled and can never connect")]
    InvitationCancelled { invitation_id: InvitationId },

    #[error(
        "date of birth must be verified by account '{account_id}' before sharing consent can be granted"
    )]
    DobVerificationRequired { account_id: MobileAccountId },

    #[error("connection '{connection_id}' is not active (current state: {state:?})")]
    ConnectionNotActive {
        connection_id: ConnectionId,
        state: ConnectedCareState,
    },

    #[error("sender account '{sender}' is not authorized for connection '{connection_id}'")]
    UnauthorizedSender {
        connection_id: ConnectionId,
        sender: MobileAccountId,
    },

    #[error("practice '{practice_id}' is not authorized to act on connection '{connection_id}'")]
    UnauthorizedPractice {
        connection_id: ConnectionId,
        practice_id: String,
    },

    #[error("transfer envelope must contain at least one curated item")]
    EmptyTransferEnvelope,

    #[error(
        "42 CFR Part 2 written consent is required before transferring SUD records or counseling notes"
    )]
    Part2WrittenConsentRequired,

    #[error("42 CFR Part 2 transfer consent does not match sender account or destination practice")]
    Part2ConsentScopeMismatch,

    #[error(
        "42 CFR Part 2 item '{item_id}' is missing the verbatim 42 CFR § 2.32(a) prohibition on redisclosure notice"
    )]
    Part2VerbatimNoticeMissing { item_id: String },

    #[error("transfer '{transfer_id}' not found")]
    TransferNotFound { transfer_id: TransferId },

    #[error("transfer '{transfer_id}' cannot transition from {from} to {to}")]
    InvalidTransferTransition {
        transfer_id: TransferId,
        from: &'static str,
        to: &'static str,
    },

    #[error(
        "mobile account '{account_id}' has been deleted (incarnation {deleted_incarnation}) and cannot establish connections or transfers"
    )]
    AccountDeleted {
        account_id: MobileAccountId,
        deleted_incarnation: u64,
    },
}

/// Structural invariant violation detected by [`crate::bridge::ConnectedCareBridge::audit_all_invariants`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InvariantViolation {
    #[error("AtMostOneOpen violated for pair ({practice}, {patient}): {open_count} open invitations", practice = pair.practice_id, patient = pair.practice_patient_id)]
    MultipleOpenInvitations { pair: PairKey, open_count: usize },

    #[error("SameConnectionState violated for pair ({practice}, {patient}): clinic={clinic:?}, mobile={mobile:?}", practice = pair.practice_id, patient = pair.practice_patient_id)]
    DriftedConnectionState {
        pair: PairKey,
        clinic: ConnectedCareState,
        mobile: ConnectedCareState,
    },

    #[error(
        "LockedAfterFiveWrongDob violated on invitation '{invitation_id}': attempts={attempts}, locked={is_locked}"
    )]
    LockoutStateMismatch {
        invitation_id: InvitationId,
        attempts: u8,
        is_locked: bool,
    },

    #[error(
        "CancelledNeverConnects violated: cancelled invitation '{invitation_id}' retains active credential lookup"
    )]
    CancelledCredentialStillActive { invitation_id: InvitationId },

    #[error(
        "ConnectionEndStopsUncertain violated: ended connection '{connection_id}' still has live non-halted transfer '{transfer_id}'"
    )]
    LiveTransferAfterDisconnect {
        connection_id: ConnectionId,
        transfer_id: TransferId,
    },
}
