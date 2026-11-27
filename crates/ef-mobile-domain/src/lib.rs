//! Headless Curie Mobile patient-owned domain crate (M3 Increment 2).
//!
//! Strictly product-isolated: ZERO dependencies on `bevy` and ZERO imports of `ef-clinic-*`.

pub mod account;
pub mod for_my_doctor;
pub mod health;
pub mod privacy;
pub mod records;

pub use account::{
    AccountIncarnation, AccountScopedDraft, ConnectivityState, MobileAccountId,
    MobileAccountRecord, MobileAuthAuthority, MobileAuthError, MobileClientScreen,
    MobileDeviceAuthState, MobileEmail, MobileSessionId, MobileSessionRecord, OwedSignOut,
    SignInInvariantViolation, VerifiedMobileSession,
};
pub use for_my_doctor::{
    DoctorNote, DoctorSendRecord, ForMyDoctorError, ForMyDoctorWorkspace, FrozenSendSnapshot,
    FrozenSharedRecord, NoteSaveState, Part2ConsentConfirmation, PracticeLinkStatus,
    PracticeSendReceiver, SEND_IRREVERSIBLE_DISCLOSURE, SendDeliveryStatus, SendTransportOutcome,
    SharedInvariantViolation,
};
pub use health::{
    AccountFencedHealthBatch, DevicePushRegistration, HealthAdapterPlatform, HealthMetricKind,
    HealthSampleEntry, HealthSyncError, HealthTransactionalOutbox, HealthUploadReceipt,
    MobileHealthIngestStore, PushPlatform, PushTokenRegistry,
};
#[cfg(not(target_arch = "wasm32"))]
pub use privacy::FileReceiptJournal;
pub use privacy::{
    InMemoryReceiptJournal, JournalError, MobilePrivacyTarget, PortablePatientExport,
    PrivacyReceipt, PrivacyReceiptJournal, PutReceiptOutcome, RestrictionJobError,
    RestrictionJobReport, RestrictionOperation, apply_privacy_receipt, run_restore_replay_gate,
};
pub use records::{
    ClinicalRecordBucket, ConfidentialityTag, FailedImportItem, ImportSourceKind,
    IncomingBatchItem, PART2_REDISCLOSURE_NOTICE, PatientRecord, PatientRecordStore,
    PkceS256Challenge, PortalConnection, PortalLinkState, PortalVendor, RecordImportError,
    SavepointBatchReport, SmartHealthLinkPayload, decode_base64url, encode_base64url,
};
