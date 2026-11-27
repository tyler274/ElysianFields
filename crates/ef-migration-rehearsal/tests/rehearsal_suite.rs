//! End-to-end M4 Vicissitude operational rehearsal test suite:
//! 1. Full 468-patient logical export/import with mid-flight crash, idempotent resume, SHA-256 parity, and panel preservation.
//! 2. Single-writer domain fencing rejecting split-brain concurrent/stale writes across Clinic, Mobile, and Connected Care.
//! 3. Account deletion + create-only `PrivacyReceiptJournal` replay + `account_incarnation` non-resurrection + Connected Care termination.
//! 4. Zero-side-effect shadow mode (`0` outbound EHR/SMS/email/push dispatches) + differential parity audit.
//! 5. Pending-job drain/handoff across Clinic chat, reply reviews, and Mobile health outbox + clean rollback verification.

use ef_connected_care::ConnectedCareState;
use ef_migration_rehearsal::RehearsalError;
use ef_migration_rehearsal::export_import::{
    DomainLeaseSet, DomainWriterFence, ImportCheckpoint, MigrationTargetStores, StorageDomain,
    build_468_patient_logical_export, run_fenced_logical_import, verify_migration_parity,
};
use ef_migration_rehearsal::privacy_replay::{
    FailClosedCheck, rehearse_deletion_and_restore_replay,
};
use ef_migration_rehearsal::shadow_and_handoff::{
    CutoverRollbackSnapshot, ExecutionMode, OutboundEffectKind, OutboundEffectSink,
    ShadowDiffAudit, execute_clean_rollback, rehearse_pending_job_drain_and_handoff,
};
use ef_mobile_domain::PutReceiptOutcome;
use ef_patient_bank::COHORT_TOTAL_PATIENTS;

#[test]
fn m4_rehearsal_468_patient_logical_export_import_with_mid_flight_crash_and_resume() {
    let export = build_468_patient_logical_export().expect("build 468-patient logical export");
    assert_eq!(export.records.len(), COHORT_TOTAL_PATIENTS);
    assert!(export.total_resource_rows > 5_000);

    let mut fence = DomainWriterFence::new();
    let leases = DomainLeaseSet {
        clinic: fence.acquire_exclusive(StorageDomain::CurieClinic, "elysian-migrator"),
        mobile: fence.acquire_exclusive(StorageDomain::CurieMobile, "elysian-migrator"),
        connected_care: fence.acquire_exclusive(StorageDomain::ConnectedCare, "elysian-migrator"),
    };

    let mut target = MigrationTargetStores::default();
    let mut checkpoint = ImportCheckpoint::default();

    // Pass 1: Crash mid-flight after committing exactly 200 of 468 patients.
    let crash_err = run_fenced_logical_import(
        &export,
        &fence,
        &leases,
        &mut target,
        &mut checkpoint,
        Some(200),
    )
    .expect_err("simulated mid-flight crash must halt at 200");
    assert_eq!(
        crash_err,
        RehearsalError::SimulatedMidFlightCrash {
            committed_count: 200
        }
    );
    assert_eq!(checkpoint.committed_patient_keys.len(), 200);

    // Pass 2: Resume from checkpoint -> skips 200 already-committed patients idempotently & finishes all 468!
    run_fenced_logical_import(&export, &fence, &leases, &mut target, &mut checkpoint, None)
        .expect("resumed import must succeed cleanly");

    // Pass 3: Re-run full import idempotently -> skips all 468 with zero duplicate rows!
    run_fenced_logical_import(&export, &fence, &leases, &mut target, &mut checkpoint, None)
        .expect("idempotent re-run must succeed");

    let parity = verify_migration_parity(&export, &target, &checkpoint).expect("verify parity");
    assert!(parity.checksum_parity_matched);
    assert_eq!(parity.expected_patients, 468);
    assert_eq!(parity.imported_patients, 468);
    assert_eq!(parity.panel_assignments_verified, 468);
    assert_eq!(parity.connected_care_links_verified, 47);
    assert_eq!(parity.expected_resource_rows, parity.imported_resource_rows);
    assert_eq!(parity.duplicates_skipped, 200 + 468);
    assert_eq!(parity.resume_cycles, 3);
}

#[test]
fn m4_rehearsal_single_writer_domain_fence_rejects_split_brain_writers() {
    let mut fence = DomainWriterFence::new();
    let legacy_clinic = fence.acquire_exclusive(StorageDomain::CurieClinic, "legacy-clinic");
    let legacy_mobile = fence.acquire_exclusive(StorageDomain::CurieMobile, "legacy-mobile");
    let legacy_bridge = fence.acquire_exclusive(StorageDomain::ConnectedCare, "legacy-bridge");

    assert!(fence.verify_writer(&legacy_clinic).is_ok());
    assert!(fence.verify_writer(&legacy_mobile).is_ok());

    // Cutover transfers Clinic writer ownership to Elysian Fields (epoch 1 -> epoch 2).
    let elysian_clinic = fence.acquire_exclusive(StorageDomain::CurieClinic, "elysian-clinic");
    assert_eq!(elysian_clinic.epoch, 2);
    assert!(fence.verify_writer(&elysian_clinic).is_ok());

    // Legacy Clinic writer attempting a split-brain write is rejected fail-closed!
    assert_eq!(
        fence.verify_writer(&legacy_clinic),
        Err(RehearsalError::SplitBrainWriteRejected {
            domain: "curie_clinic",
            active_holder: "elysian-clinic".to_owned(),
            active_epoch: 2,
            caller_holder: "legacy-clinic".to_owned(),
            caller_epoch: 1,
        })
    );
    // Mobile and Connected Care fences remain independent per domain.
    assert!(fence.verify_writer(&legacy_mobile).is_ok());
    assert!(fence.verify_writer(&legacy_bridge).is_ok());
}

#[test]
fn m4_rehearsal_account_deletion_privacy_receipt_replay_and_non_resurrection() {
    let report = rehearse_deletion_and_restore_replay().expect("privacy replay rehearsal");
    assert_eq!(report.receipts_created, 3);
    assert_eq!(
        report.duplicate_put_outcome,
        PutReceiptOutcome::AlreadyPresent
    );
    assert_eq!(report.replay_applied_first_pass, 3);
    assert_eq!(report.replay_applied_second_pass, 3);
    assert_eq!(report.post_restore_records_remaining, 0);
    assert!(report.session_rejected_after_restore);
    assert_eq!(
        report.connected_care_state_after_replay,
        ConnectedCareState::Ended
    );
    assert_eq!(
        report.unavailable_journal_check,
        FailClosedCheck::RejectedClosed
    );
}

#[test]
fn m4_rehearsal_shadow_mode_suppresses_all_outbound_effects_and_verifies_diff_parity() {
    let mut sink = OutboundEffectSink::new(ExecutionMode::ShadowZeroOutbound);
    for (idx, kind) in [
        OutboundEffectKind::EhrWrite,
        OutboundEffectKind::SmsDispatch,
        OutboundEffectKind::EmailDispatch,
        OutboundEffectKind::ApnsPush,
        OutboundEffectKind::FcmPush,
    ]
    .into_iter()
    .enumerate()
    {
        sink.emit(kind, format!("shadow-corr-{idx}"));
    }

    // Strict shadow invariant: ZERO outbound dispatches; 100% intercepted & audited.
    assert_eq!(sink.dispatched_count(), 0);
    assert_eq!(sink.suppressed_count(), 5);
    assert!(sink.audit_log().iter().all(|a| a.suppressed_by_shadow));

    let diff_ok = ShadowDiffAudit::compare(
        "/elysianfields/api/patients",
        200,
        200,
        br#"{"patients":["syn-pt-0001"]}"#,
        br#"{"patients":["syn-pt-0001"]}"#,
    );
    assert!(diff_ok.parity_matched);

    let diff_mismatch = ShadowDiffAudit::compare(
        "/elysianfields/api/patients/other",
        404,
        200,
        br#"{"error":"not_found"}"#,
        br#"{"patient":"leaked"}"#,
    );
    assert!(!diff_mismatch.parity_matched);
}

#[test]
fn m4_rehearsal_pending_job_drain_handoff_and_clean_rollback() {
    let drain = rehearse_pending_job_drain_and_handoff().expect("job drain rehearsal");
    assert!(drain.chat_work_handed_off);
    assert_eq!(drain.reply_reviews_recovered_to_woken, 1);
    assert_eq!(drain.mobile_outbox_uploaded_once, 1);
    assert_eq!(drain.mobile_outbox_duplicate_retries_ignored, 1);

    // Rehearse clean rollback with writer-fence revocation and exact SHA-256 parity.
    let mut fence = DomainWriterFence::new();
    let _legacy = fence.acquire_exclusive(StorageDomain::CurieClinic, "legacy-clinic");
    let pre_cutover = CutoverRollbackSnapshot::capture(
        "snap-pre-cutover-001",
        "legacy-clinic",
        b"canonical-pre-cutover-state-468-patients".to_vec(),
    );

    let candidate_lease = fence.acquire_exclusive(StorageDomain::CurieClinic, "elysian-clinic");
    let mut live_state = b"partial-aborted-cutover-state".to_vec();

    let rollback = execute_clean_rollback(
        StorageDomain::CurieClinic,
        &mut fence,
        &pre_cutover,
        &mut live_state,
    )
    .expect("clean rollback");
    assert!(rollback.state_parity_verified);
    assert_eq!(rollback.restored_sha256, pre_cutover.state_sha256);
    assert_eq!(live_state, pre_cutover.canonical_state_bytes);
    // Aborted candidate writer is now fenced out!
    assert!(matches!(
        fence.verify_writer(&candidate_lease),
        Err(RehearsalError::SplitBrainWriteRejected { .. })
    ));
    assert!(fence.verify_writer(&rollback.restored_lease).is_ok());
}
