//! Zero-side-effect shadow mode, pending-job drain/handoff across Clinic & Mobile,
//! and clean rollback to pre-cutover state with SHA-256 parity verification.

use ef_clinic_domain::chat::{
    ChatEngine, CommandEnvelope, CommandPayload, DeliveryMode, RetryBackoffMode,
    SubmitMessagePayload, WORKER_LEASE_SECS,
};
use ef_clinic_workflows::replies::{MessageReplyEngine, ReviewExecutionState};
use ef_core::{ChatId, IdempotencyKey, PhysicianId, WorkspaceId};
use ef_mobile_domain::{
    AccountFencedHealthBatch, AccountIncarnation, HealthAdapterPlatform, HealthMetricKind,
    HealthSampleEntry, HealthTransactionalOutbox, MobileAccountId, MobileHealthIngestStore,
};
use ef_patient_bank::sha256_hex;
use serde::{Deserialize, Serialize};

use crate::RehearsalError;
use crate::export_import::{DomainWriterFence, StorageDomain, WriterLeaseToken};

/// Execution mode enforcing hard suppression of outbound side effects during shadow runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Shadow rehearsal: ZERO outbound side effects permitted; every effect is intercepted & audited.
    ShadowZeroOutbound,
    /// Approved live cutover: outbound effects permitted.
    LiveCutover,
}

/// Typed outbound effect categories gated by [`OutboundEffectSink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundEffectKind {
    EhrWrite,
    SmsDispatch,
    EmailDispatch,
    ApnsPush,
    FcmPush,
}

/// Content-free audit entry capturing an intercepted or dispatched outbound effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundEffectAudit {
    pub kind: OutboundEffectKind,
    pub correlation_id: String,
    pub suppressed_by_shadow: bool,
}

/// Typed sink guaranteeing **0** outbound EHR/SMS/email/push dispatches during `ShadowZeroOutbound`.
#[derive(Debug, Clone)]
pub struct OutboundEffectSink {
    mode: ExecutionMode,
    dispatched_count: u64,
    suppressed_count: u64,
    audit_log: Vec<OutboundEffectAudit>,
}

impl OutboundEffectSink {
    #[must_use]
    pub const fn new(mode: ExecutionMode) -> Self {
        Self {
            mode,
            dispatched_count: 0,
            suppressed_count: 0,
            audit_log: Vec::new(),
        }
    }

    /// Attempts an outbound effect; strictly suppresses in `ShadowZeroOutbound` mode.
    pub fn emit(&mut self, kind: OutboundEffectKind, correlation_id: impl Into<String>) {
        let suppressed = self.mode == ExecutionMode::ShadowZeroOutbound;
        if suppressed {
            self.suppressed_count = self.suppressed_count.saturating_add(1);
        } else {
            self.dispatched_count = self.dispatched_count.saturating_add(1);
        }
        self.audit_log.push(OutboundEffectAudit {
            kind,
            correlation_id: correlation_id.into(),
            suppressed_by_shadow: suppressed,
        });
    }

    #[must_use]
    pub const fn dispatched_count(&self) -> u64 {
        self.dispatched_count
    }

    #[must_use]
    pub const fn suppressed_count(&self) -> u64 {
        self.suppressed_count
    }

    #[must_use]
    pub fn audit_log(&self) -> &[OutboundEffectAudit] {
        &self.audit_log
    }
}

/// Content-free differential parity audit entry comparing reference vs replacement behavior in shadow mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowDiffAudit {
    pub route: String,
    pub reference_status: u16,
    pub replacement_status: u16,
    pub reference_digest: String,
    pub replacement_digest: String,
    pub parity_matched: bool,
}

impl ShadowDiffAudit {
    #[must_use]
    pub fn compare(
        route: impl Into<String>,
        reference_status: u16,
        replacement_status: u16,
        reference_payload: &[u8],
        replacement_payload: &[u8],
    ) -> Self {
        let reference_digest = sha256_hex(reference_payload);
        let replacement_digest = sha256_hex(replacement_payload);
        let parity_matched =
            reference_status == replacement_status && reference_digest == replacement_digest;
        Self {
            route: route.into(),
            reference_status,
            replacement_status,
            reference_digest,
            replacement_digest,
            parity_matched,
        }
    }
}

/// Summary of the pending-job drain and handoff rehearsal across Clinic chat, reply review, and Mobile outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDrainHandoffReport {
    pub chat_work_handed_off: bool,
    pub reply_reviews_recovered_to_woken: usize,
    pub mobile_outbox_uploaded_once: usize,
    pub mobile_outbox_duplicate_retries_ignored: usize,
}

/// Rehearses pending-job drain & zero-duplicate handoff across cutover:
/// - In-flight Clinic chat turn lease expires cleanly and hands off to the replacement worker without duplication.
/// - In-flight Clinic reply review recovers cleanly from `Running -> Woken` on worker handoff.
/// - Pending Curie Mobile health transactional outbox drains idempotently even if retried after cutover.
pub fn rehearse_pending_job_drain_and_handoff() -> Result<JobDrainHandoffReport, RehearsalError> {
    let ws = WorkspaceId::new("pw-drain-1").map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;
    let chat = ChatId::new("chat-drain-1").map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;
    let phys = PhysicianId::new("phys-drain-1").map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;

    // 1. Clinic chat worker lease handoff.
    let mut chat_engine = ChatEngine::new(RetryBackoffMode::DeterministicTest);
    let acc = chat_engine
        .execute_command(
            CommandEnvelope {
                idempotency_key: IdempotencyKey::new("idem-drain-1").map_err(|e| {
                    RehearsalError::DomainFailure {
                        reason: e.to_string(),
                    }
                })?,
                command: CommandPayload::MessageSubmit(SubmitMessagePayload {
                    patient_workspace_id: ws.clone(),
                    chat_id: chat,
                    text: "Summarize pending labs".to_owned(),
                    physician_time_zone: "UTC".to_owned(),
                    delivery: Some(DeliveryMode::SendNow),
                }),
            },
            1_000,
        )
        .map_err(|e| RehearsalError::DomainFailure {
            reason: e.to_string(),
        })?;
    let _legacy_claim = chat_engine.claim_due_work("legacy-worker", 1_000);
    // Replacement worker cannot claim while legacy lease is still active (no split-brain execution).
    assert!(
        chat_engine
            .claim_due_work("elysian-worker", 1_010)
            .is_none()
    );
    // After 60s lease expiry, replacement worker reclaims cleanly.
    let handed_off = chat_engine
        .claim_due_work("elysian-worker", 1_000 + WORKER_LEASE_SECS)
        .is_some_and(|(id, _)| id == acc.work_item_id);

    // 2. Clinic reply review interrupted during handoff recovers Running -> Woken cleanly.
    let mut replies = MessageReplyEngine::new();
    replies.register_conversation("conv-drain-1", ws.clone(), true);
    replies
        .receive_patient_message("conv-drain-1", "pm-drain-1", "Follow-up question")
        .map_err(|e| RehearsalError::DomainFailure {
            reason: e.to_string(),
        })?;
    let _ = replies
        .start_review("conv-drain-1")
        .map_err(|e| RehearsalError::DomainFailure {
            reason: e.to_string(),
        })?;
    let reply_reviews_recovered_to_woken = replies.recover_interrupted_reviews_on_restart();
    let card = replies
        .derive_reply_card(&phys, "conv-drain-1", |_, _| true)
        .map_err(|e| RehearsalError::DomainFailure {
            reason: e.to_string(),
        })?;
    assert_eq!(card.review_state, ReviewExecutionState::Woken);

    // 3. Mobile health outbox idempotent drain across cutover retry.
    let acct = MobileAccountId::new("mob-drain-1").map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;
    let inc = AccountIncarnation::first();
    let batch = AccountFencedHealthBatch {
        account_id: acct.clone(),
        account_incarnation: inc,
        idempotency_key: "health-batch-drain-1".to_owned(),
        samples: vec![HealthSampleEntry {
            sample_id: "sample-1".to_owned(),
            metric: HealthMetricKind::RestingHeartRate,
            value_milli: 64_000,
            unit: "bpm".to_owned(),
            recorded_at_secs: 1_750_000_000,
            platform: HealthAdapterPlatform::IosHealthKit,
        }],
    };
    let mut outbox = HealthTransactionalOutbox::new();
    let _ = outbox
        .enqueue(batch.clone())
        .map_err(|e| RehearsalError::DomainFailure {
            reason: e.to_string(),
        })?;

    let mut ingest_store = MobileHealthIngestStore::new();
    let receipt_1 = ingest_store
        .ingest_batch(&acct, batch.clone())
        .map_err(|e| RehearsalError::DomainFailure {
            reason: e.to_string(),
        })?;
    // Duplicate retry after cutover handoff is strictly deduplicated (0 duplicate samples!).
    let receipt_2 = ingest_store
        .ingest_batch(&acct, batch.clone())
        .map_err(|e| RehearsalError::DomainFailure {
            reason: e.to_string(),
        })?;
    outbox.mark_uploaded(&acct, &batch.idempotency_key);

    Ok(JobDrainHandoffReport {
        chat_work_handed_off: handed_off,
        reply_reviews_recovered_to_woken,
        mobile_outbox_uploaded_once: receipt_1.stored_count,
        mobile_outbox_duplicate_retries_ignored: usize::from(receipt_2.deduplicated),
    })
}

/// Pre-cutover snapshot supporting clean, zero-data-loss rollback with SHA-256 verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutoverRollbackSnapshot {
    pub snapshot_id: String,
    pub canonical_state_bytes: Vec<u8>,
    pub state_sha256: String,
    pub prior_holder_id: String,
}

impl CutoverRollbackSnapshot {
    /// Captures a canonical pre-cutover state snapshot and computes its SHA-256 digest.
    #[must_use]
    pub fn capture(
        snapshot_id: impl Into<String>,
        prior_holder_id: impl Into<String>,
        canonical_state_bytes: Vec<u8>,
    ) -> Self {
        let state_sha256 = sha256_hex(&canonical_state_bytes);
        Self {
            snapshot_id: snapshot_id.into(),
            canonical_state_bytes,
            state_sha256,
            prior_holder_id: prior_holder_id.into(),
        }
    }
}

/// Result of executing a clean rollback back to the prior writer and verified snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackVerificationReport {
    pub restored_lease: WriterLeaseToken,
    pub restored_sha256: String,
    pub state_parity_verified: bool,
}

/// Executes rollback: re-acquires exclusive writer fence for the prior system (invalidating the
/// aborted candidate's lease), restores `active_state_bytes`, and verifies exact SHA-256 parity.
pub fn execute_clean_rollback(
    domain: StorageDomain,
    fence: &mut DomainWriterFence,
    snapshot: &CutoverRollbackSnapshot,
    active_state_bytes: &mut Vec<u8>,
) -> Result<RollbackVerificationReport, RehearsalError> {
    let restored_lease = fence.acquire_exclusive(domain, snapshot.prior_holder_id.clone());
    active_state_bytes.clear();
    active_state_bytes.extend_from_slice(&snapshot.canonical_state_bytes);
    let restored_sha256 = sha256_hex(active_state_bytes);
    if restored_sha256 != snapshot.state_sha256 {
        return Err(RehearsalError::RollbackVerificationFailed {
            reason: format!(
                "digest mismatch after restore: expected {}, actual {restored_sha256}",
                snapshot.state_sha256
            ),
        });
    }
    Ok(RollbackVerificationReport {
        restored_lease,
        restored_sha256,
        state_parity_verified: true,
    })
}
