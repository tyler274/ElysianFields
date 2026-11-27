#![cfg(not(target_arch = "wasm32"))]
//! Live full-stack Bevy `0.20.1` Curie Mobile E2E integration test driving
//! [`CurieMobileClientPlugin`] through [`LiveHttpMobileTransport`] over real TCP
//! against [`TcpMobileServer`] (`ef-mobile-server`).

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use ef_connected_care::{
    ConfidentialityTag as BridgeConfidentialityTag, ConnectedCareState, ConsentAcknowledgement,
    ConsentRecordId, CreateInvitationRequest, DateOfBirth, DobVerificationOutcome, EnvelopeItem,
    InviteCredential, MobileAccountId as BridgeMobileAccountId, PairKey, Part2WrittenConsent,
    VERBATIM_PART2_REDISCLOSURE_NOTICE,
};
use ef_core::{IdempotencyKey, PatientId, PhysicianId, PracticeId};
use ef_mobile_client::{
    ConnectedCareStatusResult, CurieMobileClientPlugin, LiveHttpMobileTransport,
    MobileClientCommand, MobileClientCommandQueue, MobileClientState, MobileClientTransport,
    MobileTab, MobileTransportResource,
};
use ef_mobile_domain::{
    AccountFencedHealthBatch, ClinicalRecordBucket, ConfidentialityTag, ConnectivityState,
    FileReceiptJournal, HealthAdapterPlatform, HealthMetricKind, HealthSampleEntry,
    ImportSourceKind, IncomingBatchItem, MobileClientScreen, PatientRecord,
};
use ef_mobile_server::{MobileService, TcpMobileServer};

fn fresh_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "ef-mobile-live-fullstack-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn build_live_mobile_app(addr: SocketAddr) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, CurieMobileClientPlugin))
        .insert_resource(MobileTransportResource {
            transport: Box::new(LiveHttpMobileTransport::new(addr)),
        });
    app
}

fn dispatch(app: &mut App, cmd: MobileClientCommand) {
    app.world_mut()
        .resource_mut::<MobileClientCommandQueue>()
        .push(cmd);
    app.update();
}

#[test]
fn live_mobile_fullstack_sign_in_owed_sign_out_health_connected_care_part2_and_non_resurrection() {
    let db_dir = fresh_temp_dir("db");
    let backup_dir = fresh_temp_dir("backup");
    let journal_dir = fresh_temp_dir("journal");

    // 1. Boot durable live `TcpMobileServer` on 127.0.0.1:0 with an independent create-only journal.
    let journal1 = FileReceiptJournal::open(&journal_dir).expect("open file receipt journal");
    let mut svc1 = MobileService::open_durable(&db_dir, Box::new(journal1), 1_760_000_000, false)
        .expect("open durable mobile service");

    let practice_id = PracticeId::new("practice-live-mob-1").expect("practice id");
    let patient_id = PatientId::new("patient-live-mob-1").expect("patient id");
    let physician_id = PhysicianId::new("phys-live-mob-1").expect("physician id");
    let pair_key = PairKey::new(practice_id.clone(), patient_id);

    let issued_invite = svc1
        .connected_care
        .create_invitation(CreateInvitationRequest {
            pair: pair_key.clone(),
            issued_by: physician_id,
            practice_name: "Synthetic Live Mobile Clinic".to_owned(),
            physician_name: "Dr. Live Bridge".to_owned(),
            expected_dob: DateOfBirth::new(1988, 6, 14).expect("valid dob"),
            created_at: "2026-01-12T09:00:00Z".to_owned(),
        })
        .expect("create connected care invitation");

    let srv1 = TcpMobileServer::bind("127.0.0.1:0", svc1).expect("bind mobile server");
    let addr1 = srv1.local_addr();

    let probe1 = LiveHttpMobileTransport::new(addr1);
    let (ready_status, ready_body) = probe1.readyz().expect("readyz");
    assert_eq!(ready_status, 200);
    assert_eq!(ready_body["ready"], true);

    let mut app = build_live_mobile_app(addr1);

    // 2. Live sign-in through Bevy `CurieMobileClientPlugin` + `LiveHttpMobileTransport`.
    dispatch(
        &mut app,
        MobileClientCommand::SignIn {
            email: "live.patient@example.invalid".to_owned(),
        },
    );

    let (acct_id, incarnation, first_session) = match app
        .world()
        .resource::<MobileClientState>()
        .auth_device
        .screen()
    {
        MobileClientScreen::SignedIn {
            account_id,
            incarnation,
            session_id,
            ..
        } => (account_id.clone(), *incarnation, session_id.clone()),
        other => panic!("expected SignedIn after live sign-in, got {other:?}"),
    };
    assert!(probe1.is_session_live(&first_session, 1_760_000_000));

    // 3. Offline owed sign-out -> immediate local wipe -> online batch flush revokes server session.
    dispatch(
        &mut app,
        MobileClientCommand::EditLocalDraft {
            text: "Offline draft cleared on sign-out".to_owned(),
        },
    );
    dispatch(
        &mut app,
        MobileClientCommand::SetConnectivity(ConnectivityState::Offline),
    );
    dispatch(
        &mut app,
        MobileClientCommand::SignOut {
            device_id: Some("ios-live-1".to_owned()),
        },
    );
    {
        let st = app.world().resource::<MobileClientState>();
        assert_eq!(st.auth_device.screen(), &MobileClientScreen::EnterYourEmail);
        assert_eq!(st.auth_device.owed_sign_outs().len(), 1);
    }
    // Server session is still live while device was offline.
    assert!(probe1.is_session_live(&first_session, 1_760_000_000));

    // Connectivity returns -> flush owed sign-outs over HTTP/1.1.
    dispatch(
        &mut app,
        MobileClientCommand::SetConnectivity(ConnectivityState::Online),
    );
    dispatch(&mut app, MobileClientCommand::FlushOwedSignOuts);
    {
        let st = app.world().resource::<MobileClientState>();
        assert!(st.auth_device.owed_sign_outs().is_empty());
        assert!(st.errors.is_empty(), "unexpected errors: {:?}", st.errors);
    }
    assert!(
        !probe1.is_session_live(&first_session, 1_760_000_000),
        "flushed owed sign-out must revoke server session"
    );

    // 4. Re-sign-in, ingest records with per-record savepoints, and sync HealthKit outbox over TCP.
    dispatch(
        &mut app,
        MobileClientCommand::SignIn {
            email: "live.patient@example.invalid".to_owned(),
        },
    );
    let active_session = match app
        .world()
        .resource::<MobileClientState>()
        .auth_device
        .screen()
    {
        MobileClientScreen::SignedIn { session_id, .. } => session_id.clone(),
        other => panic!("expected SignedIn after re-sign-in, got {other:?}"),
    };

    dispatch(
        &mut app,
        MobileClientCommand::IngestRecords(vec![
            IncomingBatchItem::Valid(PatientRecord {
                record_id: "live-rec-1".to_owned(),
                version: 1,
                account_id: acct_id.clone(),
                incarnation,
                source_kind: ImportSourceKind::SmartHealthLink,
                portal_id: None,
                bucket: ClinicalRecordBucket::Results,
                confidentiality: ConfidentialityTag::Standard,
                part2_notice: None,
                original_date: "2026-01-12".to_owned(),
                title: "Synthetic Live BMP".to_owned(),
                summary: "Creatinine 0.98 mg/dL".to_owned(),
                native_bytes: br#"{"resourceType":"Observation"}"#.to_vec(),
                removed_upstream: false,
            }),
            IncomingBatchItem::Malformed {
                external_id: "live-bad-2".to_owned(),
                reason: "malformed XML attachment".to_owned(),
            },
        ]),
    );
    dispatch(&mut app, MobileClientCommand::RefreshRecords);
    {
        let st = app.world().resource::<MobileClientState>();
        let report = st.last_ingest_report.as_ref().expect("ingest report");
        assert_eq!(report.committed_record_ids, vec!["live-rec-1"]);
        assert_eq!(report.failed_items.len(), 1);
        assert_eq!(st.visible_records.len(), 1);
        assert_eq!(st.visible_records[0].record_id, "live-rec-1");
    }

    dispatch(
        &mut app,
        MobileClientCommand::QueueHealthBatch(AccountFencedHealthBatch {
            account_id: acct_id.clone(),
            account_incarnation: incarnation,
            idempotency_key: "live-hk-batch-1".to_owned(),
            samples: vec![
                HealthSampleEntry {
                    sample_id: "live-s-steps".to_owned(),
                    metric: HealthMetricKind::Steps,
                    value_milli: 9_150_000,
                    unit: "count".to_owned(),
                    recorded_at_secs: 1_760_000_010,
                    platform: HealthAdapterPlatform::IosHealthKit,
                },
                HealthSampleEntry {
                    sample_id: "live-s-hr".to_owned(),
                    metric: HealthMetricKind::RestingHeartRate,
                    value_milli: 62_000,
                    unit: "bpm".to_owned(),
                    recorded_at_secs: 1_760_000_020,
                    platform: HealthAdapterPlatform::IosHealthKit,
                },
            ],
        }),
    );
    dispatch(
        &mut app,
        MobileClientCommand::SyncHealthOutboxAndRefreshTrends,
    );
    {
        let st = app.world().resource::<MobileClientState>();
        assert_eq!(st.outbox.total_pending(), 0);
        assert_eq!(st.visible_health_trends.len(), 2);
        assert!(st.errors.is_empty(), "unexpected errors: {:?}", st.errors);
    }

    // 5. Connected Care: lookup (zero preview leak) -> wrong DOB -> valid DOB -> explicit Share.
    let credential = InviteCredential::ShortCode(issued_invite.short_code);
    dispatch(
        &mut app,
        MobileClientCommand::ConnectedCareLookup {
            credential: credential.clone(),
        },
    );
    {
        let st = app.world().resource::<MobileClientState>();
        let challenge = st.last_invite_challenge.as_ref().expect("invite challenge");
        assert_eq!(challenge.remaining_attempts, 5);
    }

    dispatch(
        &mut app,
        MobileClientCommand::ConnectedCareVerifyDob {
            credential: credential.clone(),
            dob: DateOfBirth::new(1980, 1, 1).expect("wrong dob"),
        },
    );
    assert_eq!(
        app.world().resource::<MobileClientState>().last_dob_outcome,
        Some(DobVerificationOutcome::Mismatch {
            remaining_attempts: 4
        })
    );

    dispatch(
        &mut app,
        MobileClientCommand::ConnectedCareVerifyDob {
            credential: credential.clone(),
            dob: DateOfBirth::new(1988, 6, 14).expect("true dob"),
        },
    );
    match app
        .world()
        .resource::<MobileClientState>()
        .last_dob_outcome
        .as_ref()
    {
        Some(DobVerificationOutcome::Verified { preview }) => {
            assert_eq!(preview.practice_name, "Synthetic Live Mobile Clinic");
        }
        other => panic!("expected Verified preview, got {other:?}"),
    }

    // Before Share, standing is still Invited (`OnlyShareGrantsConsent`).
    match probe1
        .connected_care_status(&active_session, Some(&pair_key), 1_760_000_000)
        .expect("status before share")
    {
        ConnectedCareStatusResult::Pair(standing) => {
            assert_eq!(standing.state, ConnectedCareState::Invited);
        }
        other => panic!("expected pair standing, got {other:?}"),
    }

    dispatch(
        &mut app,
        MobileClientCommand::ConnectedCareShare { credential },
    );
    let connection_id = {
        let st = app.world().resource::<MobileClientState>();
        let standing = st
            .last_connection_standing
            .as_ref()
            .expect("connection standing");
        assert_eq!(standing.state, ConnectedCareState::Connected);
        standing
            .active_connection_id
            .clone()
            .expect("active connection id")
    };

    // 6. "For my doctor" Part 2 consent-gated send: blocked without written consent, accepted with consent.
    dispatch(
        &mut app,
        MobileClientCommand::SwitchTab(MobileTab::ForMyDoctor),
    );
    let part2_item = EnvelopeItem {
        item_id: "live-part2-item-1".to_owned(),
        source_version: 1,
        confidentiality: BridgeConfidentialityTag::Part2Sud,
        payload_sha256: "c".repeat(64),
        verbatim_payload: "Synthetic Part 2 SUD progress summary".to_owned(),
        part2_notice: Some(VERBATIM_PART2_REDISCLOSURE_NOTICE.to_owned()),
    };

    dispatch(
        &mut app,
        MobileClientCommand::SendForMyDoctor {
            idempotency_key: IdempotencyKey::new("live-fmd-part2-1").expect("key"),
            connection_id: connection_id.clone(),
            items: vec![part2_item.clone()],
            part2_consent: None,
        },
    );
    {
        let st = app.world().resource::<MobileClientState>();
        assert!(
            st.sent_envelopes.is_empty(),
            "Part 2 send without written consent must fail closed"
        );
        assert!(
            st.errors.iter().any(|e| e.contains("403")),
            "expected 403 consent refusal, got {:?}",
            st.errors
        );
    }

    let consent = Part2WrittenConsent {
        consent_id: ConsentRecordId::new("live-consent-part2-1").expect("consent id"),
        patient_account: BridgeMobileAccountId::new(acct_id.as_str()).expect("bridge acct"),
        practice_id,
        granted_at: "2026-01-12T09:15:00Z".to_owned(),
        scope_description: "Share SUD summary with Synthetic Live Mobile Clinic".to_owned(),
        notice_acknowledged: ConsentAcknowledgement::Acknowledged,
    };
    dispatch(
        &mut app,
        MobileClientCommand::SendForMyDoctor {
            idempotency_key: IdempotencyKey::new("live-fmd-part2-1").expect("key"),
            connection_id: connection_id.clone(),
            items: vec![part2_item],
            part2_consent: Some(consent),
        },
    );
    {
        let st = app.world().resource::<MobileClientState>();
        assert_eq!(st.sent_envelopes.len(), 1);
        assert_eq!(
            st.sent_envelopes[0]
                .part2_consent_id
                .as_ref()
                .map(|c| c.as_str()),
            Some("live-consent-part2-1")
        );
    }

    // Disconnect cleanly over HTTP/1.1 and verify `Ended` standing.
    dispatch(
        &mut app,
        MobileClientCommand::ConnectedCareDisconnect { connection_id },
    );
    assert_eq!(
        app.world()
            .resource::<MobileClientState>()
            .last_connection_standing
            .as_ref()
            .map(|s| s.state),
        Some(ConnectedCareState::Ended)
    );

    // 7. Snapshot pre-deletion WAL backup BEFORE permanent account deletion!
    fs::copy(
        db_dir.join("mobile-wal.jsonl"),
        backup_dir.join("mobile-wal.jsonl"),
    )
    .expect("snapshot pre-deletion WAL backup");

    // 8. Delete account through Bevy -> writes immutable receipt to independent journal & wipes client state.
    dispatch(
        &mut app,
        MobileClientCommand::DeleteAccount {
            event_id: "evt-live-fullstack-del-1".to_owned(),
            occurred_at: "2026-01-12T09:30:00Z".to_owned(),
        },
    );
    {
        let st = app.world().resource::<MobileClientState>();
        let receipt = st.last_delete_receipt.as_ref().expect("delete receipt");
        assert!(receipt.deleted);
        assert_eq!(receipt.receipt_event_id, "evt-live-fullstack-del-1");
        assert!(st.visible_records.is_empty());
        assert!(st.visible_health_trends.is_empty());
        assert!(st.sent_envelopes.is_empty());
        assert_eq!(st.auth_device.screen(), &MobileClientScreen::EnterYourEmail);
    }
    srv1.shutdown();

    // 9. Boot fresh server from PRE-DELETION backup with mandatory restore-replay gate!
    let journal2 = FileReceiptJournal::open(&journal_dir).expect("reopen independent journal");
    let svc2 = MobileService::open_durable(&backup_dir, Box::new(journal2), 1_760_000_500, true)
        .expect("open restored backup");
    let srv2 = TcpMobileServer::bind("127.0.0.1:0", svc2).expect("bind restored server");
    let addr2 = srv2.local_addr();

    let probe2 = LiveHttpMobileTransport::new(addr2);
    let (pre_replay_status, pre_replay_body) = probe2.readyz().expect("readyz before replay");
    assert_eq!(pre_replay_status, 503);
    assert_eq!(pre_replay_body["ready"], false);

    // Execute mandatory privacy-receipt replay gate before opening traffic.
    {
        let svc_arc = srv2.service();
        let report = svc_arc
            .lock()
            .expect("lock restored service")
            .execute_restore_replay_gate(1_000)
            .expect("restore replay gate succeeds");
        assert_eq!(report.applied, 1);
    }

    let (post_replay_status, post_replay_body) = probe2.readyz().expect("readyz after replay");
    assert_eq!(post_replay_status, 200);
    assert_eq!(post_replay_body["ready"], true);

    // 10. Non-resurrection proof: old session is dead and deleted email cannot re-sign-in via Bevy client!
    assert!(!probe2.is_session_live(&active_session, 1_760_000_500));

    let mut restored_app = build_live_mobile_app(addr2);
    dispatch(
        &mut restored_app,
        MobileClientCommand::SignIn {
            email: "live.patient@example.invalid".to_owned(),
        },
    );
    {
        let st = restored_app.world().resource::<MobileClientState>();
        assert_eq!(st.auth_device.screen(), &MobileClientScreen::EnterYourEmail);
        assert!(
            st.errors
                .iter()
                .any(|e| e.to_ascii_lowercase().contains("deleted")),
            "expected AccountDeleted refusal on restored backup, got {:?}",
            st.errors
        );
    }

    srv2.shutdown();
    let _ = fs::remove_dir_all(&db_dir);
    let _ = fs::remove_dir_all(&backup_dir);
    let _ = fs::remove_dir_all(&journal_dir);
}
