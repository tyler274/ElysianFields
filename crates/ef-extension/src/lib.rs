//! Headless protocol, security verifier, memory-only session store, declared named read
//! validator, and billing redactor for the Elysian Fields Chrome MV3 extension (`EF-EXT-0001`).

pub mod jobs;
pub mod manifest;
pub mod redaction;
pub mod session;
pub mod sha256;

pub use jobs::{DeclaredReadOperation, JobValidationError, LeasedReadJob};
pub use manifest::{
    ExtensionAuditReport, ExtensionAuditViolation, MV3_POPUP_MAX_BYTES,
    MV3_POPUP_STRICT_UNCOMPRESSED_BYTES, POPUP_REQUIRED_LOCALES, POPUP_RTL_LOCALES,
    PopupParityReport, audit_extension_package, audit_popup_parity,
};
pub use redaction::{
    ExtensionOutbox, OutboxEntry, OutboxError, OutboxStatus, RedactedPayload, RedactionReport,
    redact_and_receipt, verify_payload_receipt,
};
pub use session::{
    ChromeStorageArea, MemoryOnlySessionStore, PairingState, SessionCredentials, SessionError,
};
pub use sha256::{sha256_bytes, sha256_hex};
