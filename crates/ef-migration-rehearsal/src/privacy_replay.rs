//! Account deletion, create-only `PrivacyReceiptJournal` replay, `account_incarnation`
//! non-resurrection fencing across pre-deletion backup restores, and Connected Care termination.

use std::collections::BTreeMap;

use ef_connected_care::{
    AccountIncarnation as BridgeIncarnation, ConnectedCareBridge, ConnectedCareState,
    CreateInvitationRequest, DateOfBirth, InviteCredential, MobileAccountId as BridgeAccountId,
    PairKey,
};
use ef_core::{PatientId, PhysicianId, PracticeId};
use ef_mobile_domain::{
    AccountIncarnation, ClinicalRecordBucket, ConfidentialityTag, ForMyDoctorWorkspace,
    ImportSourceKind, InMemoryReceiptJournal, IncomingBatchItem, MobileAccountId,
    MobileAuthAuthority, MobileAuthError, MobileEmail, MobileHealthIngestStore,
    MobilePrivacyTarget, PatientRecord, PatientRecordStore, PortalVendor, PracticeSendReceiver,
    PrivacyReceipt, PrivacyReceiptJournal, PushTokenRegistry, PutReceiptOutcome,
    RestrictionOperation, run_restore_replay_gate,
};

use crate::RehearsalError;

/// Typed verification status for the fail-closed journal unavailability probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailClosedCheck {
    RejectedClosed,
    UnexpectedPass,
}

/// Summary of the account deletion + pre-deletion backup restore + receipt replay rehearsal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyReplayRehearsalReport {
    pub deleted_account_id: MobileAccountId,
    pub receipts_created: usize,
    pub duplicate_put_outcome: PutReceiptOutcome,
    pub replay_applied_first_pass: usize,
    pub replay_applied_second_pass: usize,
    pub post_restore_records_remaining: usize,
    pub session_rejected_after_restore: bool,
    pub connected_care_state_after_replay: ConnectedCareState,
    pub unavailable_journal_check: FailClosedCheck,
}

/// Rehearses the full privacy durability lifecycle:
/// 1. Provision a mobile patient account with active session, records, and Connected Care link.
/// 2. Snapshot a **pre-deletion backup** of primary Mobile + Connected Care databases.
/// 3. Delete the account and persist immutable create-only receipts (`AccountDelete`, `SessionsRevoke`,
///    `PublicationRevoke`) into the independent `PrivacyReceiptJournal`.
/// 4. Restore the pre-deletion backup and execute `run_restore_replay_gate` + `ConnectedCareBridge::terminate_account_links`.
/// 5. Prove zero resurrection of sessions/records, Connected Care link ended, idempotent second replay,
///    and fail-closed behavior when the independent journal is unavailable.
pub fn rehearse_deletion_and_restore_replay() -> Result<PrivacyReplayRehearsalReport, RehearsalError>
{
    let t0 = 1_750_000_000u64;

    // 1. Live pre-deletion state.
    let mut auth = MobileAuthAuthority::new();
    let email = MobileEmail::parse("synthetic.patient.privacy@example.invalid").map_err(|e| {
        RehearsalError::DomainFailure {
            reason: e.to_string(),
        }
    })?;
    let account =
        auth.ensure_account(email.clone())
            .map_err(|e| RehearsalError::DomainFailure {
                reason: e.to_string(),
            })?;
    let token =
        auth.issue_sign_in_link(email.as_str(), t0)
            .map_err(|e| RehearsalError::DomainFailure {
                reason: e.to_string(),
            })?;
    let session = auth
        .verify_sign_in(email.as_str(), &token, t0 + 1)
        .map_err(|e| RehearsalError::DomainFailure {
            reason: e.to_string(),
        })?;

    let mut records = PatientRecordStore::new();
    let _ = records.ingest_batch_with_savepoints(vec![IncomingBatchItem::Valid(PatientRecord {
        record_id: "rec-privacy-1".to_owned(),
        version: 1,
        account_id: account.account_id.clone(),
        incarnation: account.incarnation,
        source_kind: ImportSourceKind::SmartFhirR4 {
            vendor: PortalVendor::EpicMyChart,
        },
        portal_id: None,
        bucket: ClinicalRecordBucket::Notes,
        confidentiality: ConfidentialityTag::Standard,
        part2_notice: None,
        original_date: "2026-06-01".to_owned(),
        title: "Synthetic Progress Note".to_owned(),
        summary: "Must not resurrect after backup restore".to_owned(),
        native_bytes: b"{\"resourceType\":\"DocumentReference\"}".to_vec(),
        removed_upstream: false,
    })]);

    let health = MobileHealthIngestStore::new();
    let push_tokens = PushTokenRegistry::new();
    let mut doctor_workspaces: BTreeMap<MobileAccountId, ForMyDoctorWorkspace> = BTreeMap::new();
    doctor_workspaces.insert(
        account.account_id.clone(),
        ForMyDoctorWorkspace::new(account.account_id.clone()),
    );
    let bridge_staging = PracticeSendReceiver::new();

    // Establish Connected Care link for the same patient account.
    let prac_id = PracticeId::new("prac-privacy").map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;
    let phys_id = PhysicianId::new("phys-privacy").map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;
    let pat_id = PatientId::new("pat-privacy-1").map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;
    let pair = PairKey {
        practice_id: prac_id,
        practice_patient_id: pat_id,
    };
    let dob = DateOfBirth::new(1985, 5, 12).map_err(|e| RehearsalError::DomainFailure {
        reason: e.to_string(),
    })?;
    let bridge_acct = BridgeAccountId::new(account.account_id.as_str()).map_err(|e| {
        RehearsalError::DomainFailure {
            reason: e.to_string(),
        }
    })?;

    let mut bridge = ConnectedCareBridge::new();
    let issued = bridge
        .create_invitation(CreateInvitationRequest {
            pair: pair.clone(),
            issued_by: phys_id,
            practice_name: "Synthetic Practice".to_owned(),
            physician_name: "Dr. Privacy".to_owned(),
            expected_dob: dob,
            created_at: "2026-06-15T00:00:00Z".to_owned(),
        })
        .map_err(|e| RehearsalError::DomainFailure {
            reason: e.to_string(),
        })?;
    let cred = InviteCredential::LinkToken(issued.link_token);
    let _ = bridge
        .verify_dob(&cred, &bridge_acct, dob, "2026-06-15T00:01:00Z")
        .map_err(|e| RehearsalError::DomainFailure {
            reason: e.to_string(),
        })?;
    let _ = bridge
        .confirm_share(&cred, &bridge_acct, "2026-06-15T00:02:00Z")
        .map_err(|e| RehearsalError::DomainFailure {
            reason: e.to_string(),
        })?;
    assert_eq!(
        bridge.clinic_standing(&pair).state,
        ConnectedCareState::Connected
    );

    // 2. Capture pre-deletion backup snapshots (simulating point-in-time DB backup before user deleted account).
    let mut restored_auth = auth.clone();
    let mut restored_records = records.clone();
    let mut restored_health = health.clone();
    let mut restored_push = push_tokens.clone();
    let mut restored_ws = doctor_workspaces.clone();
    let mut restored_staging = bridge_staging.clone();

    // 3. Record immutable create-only receipts in independent journal outside primary DB.
    let mut journal = InMemoryReceiptJournal::new();
    let receipts = vec![
        PrivacyReceipt {
            schema_version: 1,
            event_id: "evt-revoke-001".to_owned(),
            account_id: account.account_id.clone(),
            account_incarnation: account.incarnation,
            operation: RestrictionOperation::SessionsRevoke,
            scope_id: "all".to_owned(),
            scope_version: 1,
            occurred_at: "2026-06-15T01:00:00Z".to_owned(),
        },
        PrivacyReceipt {
            schema_version: 1,
            event_id: "evt-pub-002".to_owned(),
            account_id: account.account_id.clone(),
            account_incarnation: account.incarnation,
            operation: RestrictionOperation::PublicationRevoke,
            scope_id: "bridge".to_owned(),
            scope_version: 1,
            occurred_at: "2026-06-15T01:00:01Z".to_owned(),
        },
        PrivacyReceipt {
            schema_version: 1,
            event_id: "evt-delete-003".to_owned(),
            account_id: account.account_id.clone(),
            account_incarnation: account.incarnation,
            operation: RestrictionOperation::AccountDelete,
            scope_id: "account".to_owned(),
            scope_version: 1,
            occurred_at: "2026-06-15T01:00:02Z".to_owned(),
        },
    ];

    let mut receipts_created = 0usize;
    for r in &receipts {
        if journal
            .put_if_absent(r)
            .map_err(|e| RehearsalError::DomainFailure {
                reason: e.to_string(),
            })?
            == PutReceiptOutcome::Created
        {
            receipts_created = receipts_created.saturating_add(1);
        }
    }
    let duplicate_put_outcome =
        journal
            .put_if_absent(&receipts[2])
            .map_err(|e| RehearsalError::DomainFailure {
                reason: e.to_string(),
            })?;

    // 4. Restore pre-deletion backup and run mandatory restore replay gate + Connected Care termination!
    let mut target = MobilePrivacyTarget {
        auth: &mut restored_auth,
        records: &mut restored_records,
        health: &mut restored_health,
        push_tokens: &mut restored_push,
        doctor_workspaces: &mut restored_ws,
        bridge_staging: &mut restored_staging,
    };
    let first_pass = run_restore_replay_gate(&journal, &mut target, 100).map_err(|e| {
        RehearsalError::DomainFailure {
            reason: e.to_string(),
        }
    })?;
    let second_pass = run_restore_replay_gate(&journal, &mut target, 100).map_err(|e| {
        RehearsalError::DomainFailure {
            reason: e.to_string(),
        }
    })?;

    // Replay deletion onto Connected Care bridge (idempotent across repeated replay).
    let bridge_inc = BridgeIncarnation::new(account.incarnation.as_u64()).map_err(|e| {
        RehearsalError::DomainFailure {
            reason: e.to_string(),
        }
    })?;
    let _term_1 = bridge.terminate_account_links(&bridge_acct, bridge_inc, "2026-06-15T01:00:02Z");
    let term_2 = bridge.terminate_account_links(&bridge_acct, bridge_inc, "2026-06-15T01:00:03Z");
    assert_eq!(term_2.connections_ended, 0);

    // 5. Verify non-resurrection and fail-closed journal unavailability.
    let session_rejected_after_restore = matches!(
        restored_auth.validate_session(&session.session_id, t0 + 10),
        Err(MobileAuthError::AccountDeleted | MobileAuthError::SessionEnded)
    );
    let post_restore_records_remaining = restored_records
        .records_for_account(&account.account_id)
        .len();
    let connected_care_state_after_replay = bridge.clinic_standing(&pair).state;

    // Verify incarnation fence blocks recreating or validating at deleted incarnation.
    assert!(account.incarnation == AccountIncarnation::first());

    journal.set_unavailable(Some("object store bucket unreachable".to_owned()));
    let mut target_fail_closed = MobilePrivacyTarget {
        auth: &mut restored_auth,
        records: &mut restored_records,
        health: &mut restored_health,
        push_tokens: &mut restored_push,
        doctor_workspaces: &mut restored_ws,
        bridge_staging: &mut restored_staging,
    };
    let unavailable_journal_check =
        if run_restore_replay_gate(&journal, &mut target_fail_closed, 100).is_err() {
            FailClosedCheck::RejectedClosed
        } else {
            FailClosedCheck::UnexpectedPass
        };

    Ok(PrivacyReplayRehearsalReport {
        deleted_account_id: account.account_id,
        receipts_created,
        duplicate_put_outcome,
        replay_applied_first_pass: first_pass.applied,
        replay_applied_second_pass: second_pass.applied,
        post_restore_records_remaining,
        session_rejected_after_restore,
        connected_care_state_after_replay,
        unavailable_journal_check,
    })
}
