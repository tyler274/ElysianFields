//! M4 Vicissitude operational rehearsal harness for Elysian Fields.
//!
//! Strictly headless (zero `bevy` dependency) and exercises 100% synthetic data
//! (468-patient `ef-patient-bank` cohort) across:
//! - Logical export & import with SHA-256 parity and panel preservation
//! - Single-writer domain fencing preventing split-brain Clinic/Mobile writes
//! - Mid-flight interruption & idempotent checkpoint resume
//! - Account deletion + independent create-only `PrivacyReceiptJournal` replay
//!   + `account_incarnation` non-resurrection fencing + Connected Care termination
//! - Zero-side-effect shadow mode with differential parity auditing
//! - Pending-job drain/handoff and clean rollback to pre-cutover snapshot

pub mod export_import;
pub mod privacy_replay;
pub mod shadow_and_handoff;

use thiserror::Error;

/// Typed errors returned by M4 migration, fencing, shadow, and rollback rehearsals.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RehearsalError {
    #[error(
        "split-brain write rejected on domain '{domain}': active writer '{active_holder}' (epoch {active_epoch}), caller '{caller_holder}' (epoch {caller_epoch})"
    )]
    SplitBrainWriteRejected {
        domain: &'static str,
        active_holder: String,
        active_epoch: u64,
        caller_holder: String,
        caller_epoch: u64,
    },
    #[error("simulated mid-flight crash after committing {committed_count} patient(s)")]
    SimulatedMidFlightCrash { committed_count: usize },
    #[error("checksum parity mismatch on '{subject}': expected {expected}, actual {actual}")]
    ChecksumMismatch {
        subject: String,
        expected: String,
        actual: String,
    },
    #[error("rollback verification failed: {reason}")]
    RollbackVerificationFailed { reason: String },
    #[error("domain operation failed: {reason}")]
    DomainFailure { reason: String },
}
