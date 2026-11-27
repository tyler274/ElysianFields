//! Headless Connected Care bridge crate (`EF-CONN-0001`, `EF-CONN-0002`).
//!
//! Sole authorized cross-product bridge between a patient's standalone Curie
//! Mobile account and a medical practice in Curie Clinic. Holds zero clinical
//! chart dependencies from either product and zero rendering dependencies.

pub mod bridge;
pub mod envelope;
pub mod error;
pub mod types;

pub use bridge::{
    AccountTerminationSummary, AuditEvent, AuditEventKind, ConnectedCareBridge,
    CreateInvitationRequest, InvariantAuditReport, IssuedInvitation, MAX_WRONG_DOB_ATTEMPTS,
};
pub use envelope::{
    ConsentAcknowledgement, EnvelopeItem, Part2WrittenConsent, TransferEnvelope, TransferStatus,
    TransferSubmission, VERBATIM_PART2_REDISCLOSURE_NOTICE, validate_transfer_consent,
};
pub use error::{BridgeError, InvariantViolation};
pub use types::{
    AccountIncarnation, ConfidentialityTag, ConnectedCareState, ConnectionId, ConnectionStanding,
    ConsentRecordId, DateOfBirth, DisconnectActor, DobVerificationOutcome, InvitationId,
    InviteChallenge, InviteCredential, InviteShortCode, InviteToken, MailDeliveryOutcome,
    MobileAccountId, PairKey, PracticePreview, TransferId,
};
