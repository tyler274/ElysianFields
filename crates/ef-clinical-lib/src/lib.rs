//! Headless clinical reference library nightly pipeline, 100 ms shared PubMed refresher,
//! and shared execution guardrails (`EF-HARNESS-0002`, `EF-HARNESS-0003`).
//!
//! Zero rendering dependencies (`bevy` forbidden); zero product crate dependencies.

pub mod guardrails;
pub mod hash;
pub mod nightly;
pub mod pubmed;

pub use guardrails::{
    ALLOWED_OPS_ATTRIBUTES, CutOffReason, ExecutionQuotaTracker, FrozenSkillPackage,
    GuardrailEvent, GuardrailTerminalStatus, JsonPropertyType, ModelRolePolicy,
    OPERATIONAL_LOG_BODY, OPS_TELEMETRY_QUEUE_SIZE, OpsRecordKind, OpsTelemetryPipeline,
    ReasoningEffort, RolePolicyError, SanitizedOpsRecord, SkillCatalogEntry, SkillViewError,
    ToolSchemaSpec, ToolValidationError,
};
pub use hash::{sha256_bytes, sha256_hex};
pub use nightly::{
    ActivationHandle, ArticleId, ArticleState, CONSECUTIVE_FAILED_OR_IDLE_HALT_LIMIT,
    CandidateVersionId, CandidateVersionRecord, ClinicalLibraryNightly, ConceptQuery, FailureCode,
    NightOutcome, NightRunId, NightlyError, OperatorAlert, PipelineHaltState,
    REPEATED_FAILURE_CODE_HALT_LIMIT, ReaderVersion, RecoveryTrigger, SlotHolderStatus, SlotLease,
    UNREADABLE_HOLDER_ALERT_THRESHOLD,
};
pub use pubmed::{
    PUBMED_MIN_SPACING_MS, PubMedError, PubMedRecordPayload, PubMedRefreshDecision,
    PubMedRefresher, PubMedStatus, PubMedTransportOutcome, SharedPubMedRateLimiter,
    StoredPubMedEntry,
};
