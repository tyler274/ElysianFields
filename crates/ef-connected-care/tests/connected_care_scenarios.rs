use ef_connected_care::{
    AccountIncarnation, AuditEventKind, BridgeError, ConfidentialityTag, ConnectedCareBridge,
    ConnectedCareState, ConsentAcknowledgement, ConsentRecordId, CreateInvitationRequest,
    DateOfBirth, DisconnectActor, DobVerificationOutcome, EnvelopeItem, InviteCredential,
    MAX_WRONG_DOB_ATTEMPTS, MailDeliveryOutcome, MobileAccountId, PairKey, Part2WrittenConsent,
    TransferStatus, TransferSubmission, VERBATIM_PART2_REDISCLOSURE_NOTICE,
};
use ef_core::{IdempotencyKey, PatientId, PhysicianId, PracticeId};

fn pair(practice: &str, patient: &str) -> PairKey {
    PairKey::new(
        PracticeId::new(practice).expect("valid practice id"),
        PatientId::new(patient).expect("valid patient id"),
    )
}

fn mobile_account(id: &str) -> MobileAccountId {
    MobileAccountId::new(id).expect("valid mobile account id")
}

fn physician(id: &str) -> PhysicianId {
    PhysicianId::new(id).expect("valid physician id")
}

fn dob(y: u16, m: u8, d: u8) -> DateOfBirth {
    DateOfBirth::new(y, m, d).expect("valid dob")
}

fn standard_item(id: &str, summary: &str) -> EnvelopeItem {
    EnvelopeItem {
        item_id: id.to_owned(),
        source_version: 1,
        confidentiality: ConfidentialityTag::Standard,
        payload_sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            .to_owned(),
        verbatim_payload: summary.to_owned(),
        part2_notice: None,
    }
}

/// Proves `EF-CONN-0001` / `SCN-M3-0012` and all 11 formal `Invitation` invariants.
#[test]
fn scn_m3_0012_invitation_lifecycle_all_11_invariants() {
    let mut bridge = ConnectedCareBridge::new();
    let p1_pair = pair("prac-1", "pat-100");
    let patient_account = mobile_account("mob-acct-1");
    let expected_dob = dob(1985, 6, 15);

    // Initial standing agrees across Curie Clinic and Curie Mobile (`SameConnectionState`).
    assert_eq!(
        bridge.clinic_standing(&p1_pair).state,
        ConnectedCareState::NotInvited
    );
    assert_eq!(
        bridge.mobile_standing(&p1_pair).state,
        ConnectedCareState::NotInvited
    );

    // 1. Create open invitation; second open invitation must fail (`AtMostOneOpen`).
    let first_invite = bridge
        .create_invitation(CreateInvitationRequest {
            pair: p1_pair.clone(),
            issued_by: physician("doc-1"),
            practice_name: "Harbor Synthetic Clinic".to_owned(),
            physician_name: "Dr. Synthetic One".to_owned(),
            expected_dob,
            created_at: "2026-01-01T10:00:00Z".to_owned(),
        })
        .expect("first invite created");

    let duplicate_err = bridge
        .create_invitation(CreateInvitationRequest {
            pair: p1_pair.clone(),
            issued_by: physician("doc-1"),
            practice_name: "Harbor Synthetic Clinic".to_owned(),
            physician_name: "Dr. Synthetic One".to_owned(),
            expected_dob,
            created_at: "2026-01-01T10:01:00Z".to_owned(),
        })
        .expect_err("AtMostOneOpen must reject second open invite");
    assert!(matches!(
        duplicate_err,
        BridgeError::CannotInviteInState {
            state: ConnectedCareState::Invited,
            ..
        }
    ));

    // 2. Failed mail leaves invitation open (`FailedMailLeavesOpen`).
    bridge
        .record_mail_delivery(
            &first_invite.invitation_id,
            MailDeliveryOutcome::Failed {
                failed_at: "2026-01-01T10:01:30Z".to_owned(),
                reason: "temporary smtp bounce".to_owned(),
            },
        )
        .expect("failed mail recorded");
    assert_eq!(
        bridge.clinic_standing(&p1_pair).state,
        ConnectedCareState::Invited
    );

    // 3. Resend unlocked invite keeps single open invite & keeps earlier link/code valid (`ResendNeverSplitsOrUnlocks`).
    let resent = bridge
        .resend_invitation(&p1_pair, "2026-01-01T10:02:00Z")
        .expect("resend unlocked invite");
    assert_eq!(resent.invitation_id, first_invite.invitation_id);

    let old_link = InviteCredential::LinkToken(first_invite.link_token.clone());
    let old_code = InviteCredential::ShortCode(first_invite.short_code.clone());
    let new_code = InviteCredential::ShortCode(resent.short_code.clone());

    // Neutral challenge before DOB match reveals zero practice/physician details (`PreviewOnlyAfterDob`).
    let challenge = bridge
        .lookup_challenge(&old_link)
        .expect("old link remains valid after resend");
    assert_eq!(challenge.invitation_id, first_invite.invitation_id);
    assert_eq!(challenge.remaining_attempts, MAX_WRONG_DOB_ATTEMPTS);

    // 4. Submit 4 wrong DOB guesses across old & new credentials -> no preview leaked, counts down cleanly.
    for attempt in 1..=4_u8 {
        let cred = if attempt % 2 == 1 {
            &old_code
        } else {
            &new_code
        };
        let outcome = bridge
            .verify_dob(
                cred,
                &patient_account,
                dob(1985, 6, attempt),
                format!("2026-01-01T10:03:0{attempt}Z"),
            )
            .expect("wrong dob returns mismatch");
        assert_eq!(
            outcome,
            DobVerificationOutcome::Mismatch {
                remaining_attempts: MAX_WRONG_DOB_ATTEMPTS - attempt
            }
        );
    }

    // 5th wrong DOB locks permanently (`LockedAfterFiveWrongDob`).
    let locked_outcome = bridge
        .verify_dob(
            &old_link,
            &patient_account,
            dob(1985, 6, 20),
            "2026-01-01T10:03:05Z",
        )
        .expect("5th wrong dob locks invite");
    assert_eq!(locked_outcome, DobVerificationOutcome::LockedOut);
    assert_eq!(
        bridge.clinic_standing(&p1_pair).state,
        ConnectedCareState::InvitedLocked
    );
    assert_eq!(
        bridge.mobile_standing(&p1_pair).state,
        ConnectedCareState::InvitedLocked
    );

    // Even the exact correct DOB fails once locked, and resend MUST NOT unlock!
    let post_lock_dob = bridge
        .verify_dob(
            &old_link,
            &patient_account,
            expected_dob,
            "2026-01-01T10:03:06Z",
        )
        .expect_err("locked invite rejects correct DOB");
    assert!(matches!(
        post_lock_dob,
        BridgeError::InvitationLocked { .. }
    ));

    let post_lock_resend = bridge
        .resend_invitation(&p1_pair, "2026-01-01T10:03:07Z")
        .expect_err("ResendNeverSplitsOrUnlocks forbids resending locked invite");
    assert!(matches!(
        post_lock_resend,
        BridgeError::InvitationLocked { .. }
    ));

    // 5. Cancel locked invite -> immediately invalidates all prior links/codes (`CancelledNeverConnects`).
    let cancelled_id = bridge
        .cancel_invitation(&p1_pair, "2026-01-01T10:04:00Z")
        .expect("cancel locked invite");
    assert_eq!(cancelled_id, first_invite.invitation_id);
    assert_eq!(
        bridge.clinic_standing(&p1_pair).state,
        ConnectedCareState::NotInvited
    );

    for revoked in [&old_link, &old_code, &new_code] {
        assert!(matches!(
            bridge.lookup_challenge(revoked),
            Err(BridgeError::InvalidOrRevokedCredential { .. })
        ));
        assert!(matches!(
            bridge.confirm_share(revoked, &patient_account, "2026-01-01T10:04:05Z"),
            Err(BridgeError::InvalidOrRevokedCredential { .. })
        ));
    }

    // 6. Issue a fresh invitation after cancelling the locked one.
    let fresh = bridge
        .create_invitation(CreateInvitationRequest {
            pair: p1_pair.clone(),
            issued_by: physician("doc-1"),
            practice_name: "Harbor Synthetic Clinic".to_owned(),
            physician_name: "Dr. Synthetic One".to_owned(),
            expected_dob,
            created_at: "2026-01-01T10:05:00Z".to_owned(),
        })
        .expect("fresh invite created");
    let fresh_cred = InviteCredential::ShortCode(fresh.short_code.clone());

    // Cannot tap Share before DOB verification (`DobVerifiedOnlyAfterMatch`).
    assert!(matches!(
        bridge.confirm_share(&fresh_cred, &patient_account, "2026-01-01T10:05:10Z"),
        Err(BridgeError::DobVerificationRequired { .. })
    ));

    // 7. Enter matching DOB -> reveals preview ONLY now, still grants ZERO consent (`OnlyShareGrantsConsent`).
    let verified = bridge
        .verify_dob(
            &fresh_cred,
            &patient_account,
            expected_dob,
            "2026-01-01T10:05:20Z",
        )
        .expect("matching dob verifies");
    match verified {
        DobVerificationOutcome::Verified { preview } => {
            assert_eq!(preview.practice_name, "Harbor Synthetic Clinic");
            assert_eq!(preview.physician_name, "Dr. Synthetic One");
        }
        other => panic!("expected Verified preview, got {other:?}"),
    }

    // Navigate away without tapping Share -> invite stays open, zero data transferred.
    bridge
        .leave_preview_without_sharing(&fresh_cred, &patient_account, "2026-01-01T10:05:30Z")
        .expect("leave preview recorded");
    assert_eq!(
        bridge.clinic_standing(&p1_pair).state,
        ConnectedCareState::Invited
    );
    assert!(bridge.delivered_transfers_for_pair(&p1_pair).is_empty());

    // 8. Return and tap Share -> connects exact signed-in standalone account (`ConnectsOnlyOnOpenShare`, `ShareConnectsSignedInAccount`).
    let connected = bridge
        .confirm_share(&fresh_cred, &patient_account, "2026-01-01T10:06:00Z")
        .expect("explicit Share connects");
    assert_eq!(connected.state, ConnectedCareState::Connected);
    assert_eq!(connected.connected_account, Some(patient_account));
    assert_eq!(
        bridge.clinic_standing(&p1_pair),
        bridge.mobile_standing(&p1_pair)
    );
    assert!(
        bridge.delivered_transfers_for_pair(&p1_pair).is_empty(),
        "connecting must never auto-transfer historical To send items"
    );

    // Structural invariant audit passes across the entire store.
    let audit = bridge
        .audit_all_invariants()
        .expect("all 11 Invitation invariants hold");
    assert_eq!(audit.open_invitations, 0);
    assert_eq!(audit.active_connections, 1);

    // Verify metadata-only audit log never leaks DOB values.
    let serialized_audit = serde_json::to_string(bridge.audit_log()).expect("audit serializes");
    assert!(!serialized_audit.contains("1985"));
}

/// Proves `EF-CONN-0002` / `SCN-M3-0013`: multi-practice independence, ending
/// connection during an uncertain send, preserving prior Sent history, and rejoining.
#[test]
fn scn_m3_0013_multi_practice_independence_disconnect_during_uncertain_send_and_rejoin() {
    let mut bridge = ConnectedCareBridge::new();
    let p1_pair = pair("prac-1", "pat-p1");
    let p2_pair = pair("prac-2", "pat-p2");
    let account = mobile_account("mob-patient-p");
    let patient_dob = dob(1990, 3, 22);

    // Connect Patient P to both Practice 1 and Practice 2.
    let conn1 = connect_pair(&mut bridge, &p1_pair, &account, patient_dob, "Practice One");
    let conn2 = connect_pair(&mut bridge, &p2_pair, &account, patient_dob, "Practice Two");
    assert_eq!(bridge.active_connections_for_account(&account).len(), 2);

    // Prior confirmed send to Practice 1 is delivered and must survive disconnect.
    let delivered_send = bridge
        .submit_transfer(TransferSubmission {
            idempotency_key: IdempotencyKey::new("send-p1-1").expect("key"),
            connection_id: conn1.clone(),
            sender_account: account.clone(),
            sent_at: "2026-02-01T09:00:00Z".to_owned(),
            items: vec![standard_item("rec-1", "Synthetic baseline vitals summary")],
            part2_consent: None,
        })
        .expect("staged send 1");
    let _ = bridge
        .confirm_practice_receipt(&delivered_send.transfer_id, "2026-02-01T09:00:05Z")
        .expect("delivered send 1");

    // Second send to Practice 1 enters Uncertain state (simulated timeout).
    let uncertain_send = bridge
        .submit_transfer(TransferSubmission {
            idempotency_key: IdempotencyKey::new("send-p1-2").expect("key"),
            connection_id: conn1.clone(),
            sender_account: account.clone(),
            sent_at: "2026-02-01T09:05:00Z".to_owned(),
            items: vec![standard_item("rec-2", "Synthetic follow-up note")],
            part2_consent: None,
        })
        .expect("staged send 2");
    let _ = bridge
        .mark_transfer_uncertain(
            &uncertain_send.transfer_id,
            "upstream timeout",
            "2026-02-01T09:05:30Z",
        )
        .expect("marked uncertain");

    // End connection with Practice 1 from Curie Mobile while send 2 is Uncertain.
    let ended = bridge
        .end_connection(
            &conn1,
            DisconnectActor::Patient {
                account_id: account.clone(),
            },
            "2026-02-01T09:06:00Z",
        )
        .expect("end connection 1");
    assert_eq!(ended.state, ConnectedCareState::Ended);
    assert_eq!(
        bridge.clinic_standing(&p1_pair).state,
        ConnectedCareState::Ended
    );

    // Concurrent disconnect by Practice 1 resolves idempotently to Ended.
    let idempotent_end = bridge
        .end_connection(
            &conn1,
            DisconnectActor::PracticeMember {
                practice_id: p1_pair.practice_id.clone(),
                physician_id: physician("doc-1"),
            },
            "2026-02-01T09:06:01Z",
        )
        .expect("concurrent disconnect is idempotent");
    assert_eq!(idempotent_end.state, ConnectedCareState::Ended);

    // Uncertain send is halted immediately (`ConnectionEndStopsUncertain`), while prior Delivered send stays intact.
    assert!(matches!(
        bridge
            .transfer(&uncertain_send.transfer_id)
            .expect("transfer exists")
            .status,
        TransferStatus::HaltedOnConnectionEnd { .. }
    ));
    assert!(matches!(
        bridge.confirm_practice_receipt(&uncertain_send.transfer_id, "2026-02-01T09:06:10Z"),
        Err(BridgeError::ConnectionNotActive {
            state: ConnectedCareState::Ended,
            ..
        })
    ));
    assert_eq!(bridge.delivered_transfers_for_pair(&p1_pair).len(), 1);

    // Practice 2 stays completely unaffected (`PracticesStayIndependent`) and continues receiving sends.
    assert_eq!(
        bridge.clinic_standing(&p2_pair).state,
        ConnectedCareState::Connected
    );
    let p2_send = bridge
        .submit_transfer(TransferSubmission {
            idempotency_key: IdempotencyKey::new("send-p2-1").expect("key"),
            connection_id: conn2,
            sender_account: account.clone(),
            sent_at: "2026-02-01T09:07:00Z".to_owned(),
            items: vec![standard_item(
                "rec-p2",
                "Synthetic care note for Practice 2",
            )],
            part2_consent: None,
        })
        .expect("Practice 2 continues receiving sends");
    let _ = bridge
        .confirm_practice_receipt(&p2_send.transfer_id, "2026-02-01T09:07:02Z")
        .expect("Practice 2 confirms receipt");
    assert_eq!(bridge.delivered_transfers_for_pair(&p2_pair).len(), 1);

    // Rejoining Practice 1 requires a fresh invitation + full DOB + explicit Share consent.
    let rejoined_conn = connect_pair(&mut bridge, &p1_pair, &account, patient_dob, "Practice One");
    assert_ne!(rejoined_conn, conn1);
    assert_eq!(
        bridge.clinic_standing(&p1_pair).state,
        ConnectedCareState::Connected
    );
    assert_eq!(bridge.active_connections_for_account(&account).len(), 2);

    let _ = bridge
        .audit_all_invariants()
        .expect("all invariants hold after disconnect and rejoin");
}

/// Proves 42 CFR Part 2 written consent + verbatim § 2.32(a) notice enforcement,
/// idempotent retries, and audit trail logging.
#[test]
fn part2_written_consent_and_verbatim_notice_gate_with_audit_trail() {
    let mut bridge = ConnectedCareBridge::new();
    let p1_pair = pair("prac-1", "pat-part2");
    let account = mobile_account("mob-part2");
    let conn1 = connect_pair(
        &mut bridge,
        &p1_pair,
        &account,
        dob(1982, 11, 4),
        "Practice One",
    );

    let part2_without_notice = EnvelopeItem {
        item_id: "sud-1".to_owned(),
        source_version: 1,
        confidentiality: ConfidentialityTag::Part2Sud,
        payload_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_owned(),
        verbatim_payload: "Synthetic Part 2 SUD record".to_owned(),
        part2_notice: None,
    };

    // Missing verbatim § 2.32(a) notice fails closed.
    let err = bridge
        .submit_transfer(TransferSubmission {
            idempotency_key: IdempotencyKey::new("p2-fail-1").expect("key"),
            connection_id: conn1.clone(),
            sender_account: account.clone(),
            sent_at: "2026-03-01T12:00:00Z".to_owned(),
            items: vec![part2_without_notice],
            part2_consent: None,
        })
        .expect_err("missing Part 2 notice must fail");
    assert!(matches!(
        err,
        BridgeError::Part2VerbatimNoticeMissing { .. }
    ));

    let part2_with_notice = EnvelopeItem {
        item_id: "sud-notes-1".to_owned(),
        source_version: 2,
        confidentiality: ConfidentialityTag::Part2SudCounselingNotes,
        payload_sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .to_owned(),
        verbatim_payload: "Synthetic Part 2 counseling note".to_owned(),
        part2_notice: Some(VERBATIM_PART2_REDISCLOSURE_NOTICE.to_owned()),
    };

    // Missing explicit written consent fails closed and records audit denial.
    let err = bridge
        .submit_transfer(TransferSubmission {
            idempotency_key: IdempotencyKey::new("p2-fail-2").expect("key"),
            connection_id: conn1.clone(),
            sender_account: account.clone(),
            sent_at: "2026-03-01T12:01:00Z".to_owned(),
            items: vec![part2_with_notice.clone()],
            part2_consent: None,
        })
        .expect_err("missing Part 2 written consent must fail");
    assert_eq!(err, BridgeError::Part2WrittenConsentRequired);
    assert!(
        bridge
            .audit_log()
            .iter()
            .any(|ev| matches!(ev.kind, AuditEventKind::TransferRejectedPart2 { .. }))
    );

    // Consent scoped to a different practice fails closed.
    let wrong_scope_consent = Part2WrittenConsent {
        consent_id: ConsentRecordId::new("consent-wrong").expect("id"),
        patient_account: account.clone(),
        practice_id: PracticeId::new("prac-other").expect("id"),
        granted_at: "2026-03-01T12:02:00Z".to_owned(),
        scope_description: "SUD counseling note disclosure".to_owned(),
        notice_acknowledged: ConsentAcknowledgement::Acknowledged,
    };
    let err = bridge
        .submit_transfer(TransferSubmission {
            idempotency_key: IdempotencyKey::new("p2-fail-3").expect("key"),
            connection_id: conn1.clone(),
            sender_account: account.clone(),
            sent_at: "2026-03-01T12:02:05Z".to_owned(),
            items: vec![part2_with_notice.clone()],
            part2_consent: Some(wrong_scope_consent),
        })
        .expect_err("wrong practice consent scope must fail");
    assert_eq!(err, BridgeError::Part2ConsentScopeMismatch);

    // Valid written consent + verbatim § 2.32(a) notice succeeds and deduplicates idempotently.
    let valid_consent = Part2WrittenConsent {
        consent_id: ConsentRecordId::new("consent-p2-ok").expect("id"),
        patient_account: account.clone(),
        practice_id: p1_pair.practice_id.clone(),
        granted_at: "2026-03-01T12:03:00Z".to_owned(),
        scope_description:
            "Explicit patient-directed disclosure of SUD counseling note to Practice One".to_owned(),
        notice_acknowledged: ConsentAcknowledgement::Acknowledged,
    };
    let submission = TransferSubmission {
        idempotency_key: IdempotencyKey::new("p2-ok-1").expect("key"),
        connection_id: conn1,
        sender_account: account,
        sent_at: "2026-03-01T12:03:05Z".to_owned(),
        items: vec![part2_with_notice],
        part2_consent: Some(valid_consent),
    };
    let first = bridge
        .submit_transfer(submission.clone())
        .expect("valid Part 2 transfer stages");
    let retry = bridge
        .submit_transfer(submission)
        .expect("idempotent retry returns identical frozen transfer");
    assert_eq!(first.transfer_id, retry.transfer_id);
    assert_eq!(
        first.part2_consent_id,
        Some(ConsentRecordId::new("consent-p2-ok").expect("id"))
    );
}

/// Proves account deletion & repeated post-restore privacy journal replay
/// idempotently terminates all links and scrubs staged bridge payloads.
#[test]
fn account_deletion_and_restore_replay_terminates_links_idempotently() {
    let mut bridge = ConnectedCareBridge::new();
    let p1_pair = pair("prac-1", "pat-del-1");
    let p2_pair = pair("prac-2", "pat-del-2");
    let p3_pair = pair("prac-3", "pat-del-3");
    let account = mobile_account("mob-delete-me");
    let patient_dob = dob(1978, 9, 12);

    let conn1 = connect_pair(&mut bridge, &p1_pair, &account, patient_dob, "Practice One");
    let _conn2 = connect_pair(&mut bridge, &p2_pair, &account, patient_dob, "Practice Two");

    // Stage an in-flight transfer on Practice 1.
    let staged = bridge
        .submit_transfer(TransferSubmission {
            idempotency_key: IdempotencyKey::new("staged-before-delete").expect("key"),
            connection_id: conn1,
            sender_account: account.clone(),
            sent_at: "2026-04-01T08:00:00Z".to_owned(),
            items: vec![standard_item("rec-staged", "Synthetic staged note")],
            part2_consent: None,
        })
        .expect("staged transfer");

    // Also open & DOB-verify an invite with Practice 3 (not yet shared).
    let inv3 = bridge
        .create_invitation(CreateInvitationRequest {
            pair: p3_pair.clone(),
            issued_by: physician("doc-3"),
            practice_name: "Practice Three".to_owned(),
            physician_name: "Dr. Three".to_owned(),
            expected_dob: patient_dob,
            created_at: "2026-04-01T08:01:00Z".to_owned(),
        })
        .expect("invite 3");
    let cred3 = InviteCredential::LinkToken(inv3.link_token);
    let _ = bridge
        .verify_dob(&cred3, &account, patient_dob, "2026-04-01T08:02:00Z")
        .expect("dob verified for invite 3");

    // First deletion replay terminates both connections, cancels invite 3, and scrubs staged transfer.
    let inc = AccountIncarnation::new(1).expect("incarnation 1");
    let summary1 = bridge.terminate_account_links(&account, inc, "2026-04-01T08:05:00Z");
    assert_eq!(summary1.connections_ended, 2);
    assert_eq!(summary1.invitations_cancelled, 1);
    assert_eq!(summary1.transfers_scrubbed, 1);

    assert_eq!(
        bridge.clinic_standing(&p1_pair).state,
        ConnectedCareState::Ended
    );
    assert_eq!(
        bridge.clinic_standing(&p2_pair).state,
        ConnectedCareState::Ended
    );
    assert_eq!(
        bridge.clinic_standing(&p3_pair).state,
        ConnectedCareState::NotInvited
    );

    let scrubbed = bridge
        .transfer(&staged.transfer_id)
        .expect("transfer record");
    assert!(scrubbed.items.is_empty());
    assert!(matches!(
        scrubbed.status,
        TransferStatus::ScrubbedOnAccountDeletion { .. }
    ));

    // Replaying the deletion receipt again is strictly idempotent.
    let summary2 = bridge.terminate_account_links(&account, inc, "2026-04-01T08:10:00Z");
    assert_eq!(summary2.connections_ended, 0);
    assert_eq!(summary2.invitations_cancelled, 0);
    assert_eq!(summary2.transfers_scrubbed, 0);

    // Deleted account cannot connect to any new invitation (`AccountDeleted`).
    let inv4 = bridge
        .create_invitation(CreateInvitationRequest {
            pair: p3_pair,
            issued_by: physician("doc-3"),
            practice_name: "Practice Three".to_owned(),
            physician_name: "Dr. Three".to_owned(),
            expected_dob: patient_dob,
            created_at: "2026-04-01T08:11:00Z".to_owned(),
        })
        .expect("invite 4");
    let err = bridge
        .verify_dob(
            &InviteCredential::LinkToken(inv4.link_token),
            &account,
            patient_dob,
            "2026-04-01T08:12:00Z",
        )
        .expect_err("deleted account cannot verify or connect");
    assert!(matches!(err, BridgeError::AccountDeleted { .. }));

    let _ = bridge
        .audit_all_invariants()
        .expect("invariants hold after account deletion replay");
}

#[test]
fn closed_schema_json_serialization_matches_m3_contract() {
    assert_eq!(
        serde_json::to_string(&ConnectedCareState::NotInvited).expect("json"),
        "\"NotInvited\""
    );
    assert_eq!(
        serde_json::to_string(&ConnectedCareState::Invited).expect("json"),
        "\"Invited\""
    );
    assert_eq!(
        serde_json::to_string(&ConnectedCareState::InvitedLocked).expect("json"),
        "\"InvitedLocked\""
    );
    assert_eq!(
        serde_json::to_string(&ConnectedCareState::Connected).expect("json"),
        "\"Connected\""
    );
    assert_eq!(
        serde_json::to_string(&ConnectedCareState::Ended).expect("json"),
        "\"Ended\""
    );
    assert_eq!(
        serde_json::to_string(&ConfidentialityTag::Standard).expect("json"),
        "\"Standard\""
    );
    assert_eq!(
        serde_json::to_string(&ConfidentialityTag::Part2Sud).expect("json"),
        "\"Part2Sud\""
    );
    assert_eq!(
        serde_json::to_string(&ConfidentialityTag::Part2SudCounselingNotes).expect("json"),
        "\"Part2SudCounselingNotes\""
    );
}

#[test]
fn bridge_crate_manifest_stays_headless_and_product_isolated() {
    let manifest = include_str!("../Cargo.toml");
    assert!(!manifest.contains("bevy"));
    assert!(!manifest.contains("ef-clinic-"));
    assert!(!manifest.contains("ef-mobile-"));
}

fn connect_pair(
    bridge: &mut ConnectedCareBridge,
    pair: &PairKey,
    account: &MobileAccountId,
    patient_dob: DateOfBirth,
    practice_name: &str,
) -> ef_connected_care::ConnectionId {
    let issued = bridge
        .create_invitation(CreateInvitationRequest {
            pair: pair.clone(),
            issued_by: physician("doc-helper"),
            practice_name: practice_name.to_owned(),
            physician_name: "Dr. Synthetic Helper".to_owned(),
            expected_dob: patient_dob,
            created_at: "2026-01-10T08:00:00Z".to_owned(),
        })
        .expect("create invite");
    let cred = InviteCredential::LinkToken(issued.link_token);
    let verified = bridge
        .verify_dob(&cred, account, patient_dob, "2026-01-10T08:01:00Z")
        .expect("verify dob");
    assert!(matches!(verified, DobVerificationOutcome::Verified { .. }));
    let standing = bridge
        .confirm_share(&cred, account, "2026-01-10T08:02:00Z")
        .expect("confirm share");
    standing
        .active_connection_id
        .expect("active connection id present")
}
