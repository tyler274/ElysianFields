//! M3 Increment 2 acceptance tests proving `SCN-M3-0007` through `SCN-M3-0011`
//! across `ef-mobile-domain`, `ef-ui-catalog`, and the Bevy `0.20.1` `CurieMobileClientPlugin`.

use std::collections::BTreeMap;

use bevy::prelude::*;
use ef_core::PracticeId;
use ef_mobile_client::{
    CurieMobileClientPlugin, LocalMobileTransport, MobileApiEndpoint, MobileClientCommand,
    MobileClientCommandQueue, MobileClientState, MobileTab, MobileTransportResource,
    MobileVisibility,
};
use ef_mobile_domain::{
    AccountFencedHealthBatch, AccountIncarnation, ClinicalRecordBucket, ConfidentialityTag,
    ConnectivityState, DevicePushRegistration, ForMyDoctorError, ForMyDoctorWorkspace,
    HealthAdapterPlatform, HealthMetricKind, HealthSampleEntry, ImportSourceKind,
    InMemoryReceiptJournal, IncomingBatchItem, MobileAccountId, MobileAuthAuthority,
    MobileClientScreen, MobileDeviceAuthState, MobileHealthIngestStore, MobilePrivacyTarget,
    NoteSaveState, PART2_REDISCLOSURE_NOTICE, Part2ConsentConfirmation, PatientRecord,
    PatientRecordStore, PkceS256Challenge, PortablePatientExport, PortalLinkState, PortalVendor,
    PracticeLinkStatus, PracticeSendReceiver, PrivacyReceipt, PrivacyReceiptJournal, PushPlatform,
    PushTokenRegistry, PutReceiptOutcome, RecordImportError, RestrictionJobError,
    RestrictionOperation, SEND_IRREVERSIBLE_DISCLOSURE, SendDeliveryStatus, SendTransportOutcome,
    SmartHealthLinkPayload, encode_base64url, run_restore_replay_gate,
};
use ef_ui_catalog::{ComposerSurface, Locale, UiLocale};

fn build_mobile_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, CurieMobileClientPlugin))
        .insert_resource(MobileTransportResource {
            transport: Box::new(LocalMobileTransport::new()),
        });
    app
}

fn push_cmd(app: &mut App, cmd: MobileClientCommand) {
    app.world_mut()
        .resource_mut::<MobileClientCommandQueue>()
        .push(cmd);
}

#[test]
fn scn_m3_0007_mobile_sign_in_offline_owed_sign_out_flush_and_handoff_isolation() {
    let mut authority = MobileAuthAuthority::new();
    let mut device = MobileDeviceAuthState::new();
    let now = 1_760_000_000;

    // 1. Account A signs in and writes a local draft.
    let _ = device
        .submit_email("patient.a@example.invalid")
        .expect("email A");
    let tok_a = authority
        .issue_sign_in_link("patient.a@example.invalid", now)
        .expect("link A");
    let sess_a = authority
        .verify_sign_in("patient.a@example.invalid", &tok_a, now)
        .expect("verify A");
    device.complete_sign_in(sess_a.clone());
    device
        .set_local_draft("Private Account A draft note")
        .expect("draft A");
    device
        .check_all_6_invariants(&authority, now)
        .expect("all 6 invariants hold signed in");

    // 2. Device goes offline and signs out -> clears credentials/draft immediately, queues `owes`.
    device.set_connectivity(ConnectivityState::Offline);
    let signed_out_id = device
        .sign_out(&mut authority)
        .expect("signed out session A");
    assert_eq!(signed_out_id, sess_a.session_id);
    assert_eq!(device.screen(), &MobileClientScreen::EnterYourEmail);
    assert_eq!(device.owed_sign_outs().len(), 1);
    assert!(authority.is_session_live(&sess_a.session_id, now));
    device
        .check_all_6_invariants(&authority, now)
        .expect("all 6 invariants hold offline with owed sign-out");

    // 3. Connectivity returns -> owed sign-out flushes and ends session on server (`OwedSessionsEventuallyEnd`).
    device.set_connectivity(ConnectivityState::Online);
    let flushed = device.flush_owed_sign_outs(&mut authority);
    assert_eq!(flushed, 1);
    assert!(device.owed_sign_outs().is_empty());
    assert!(!authority.is_session_live(&sess_a.session_id, now));
    device
        .check_all_6_invariants(&authority, now)
        .expect("all 6 invariants hold after online flush");

    // 4. Device handoff: Account B signs in cleanly with zero Account A drafts or records.
    let tok_b = authority
        .issue_sign_in_link("patient.b@example.invalid", now + 10)
        .expect("link B");
    let sess_b = authority
        .verify_sign_in("patient.b@example.invalid", &tok_b, now + 10)
        .expect("verify B");
    device.complete_sign_in(sess_b.clone());
    match device.screen() {
        MobileClientScreen::SignedIn {
            account_id,
            local_draft,
            ..
        } => {
            assert_eq!(account_id, &sess_b.account_id);
            assert!(local_draft.is_none());
        }
        other => panic!("expected SignedIn for Account B, got {other:?}"),
    }
    device
        .check_all_6_invariants(&authority, now + 10)
        .expect("all 6 invariants hold after handoff");

    // 5. Expiry path -> SessionEnded holds zero credentials/drafts and returns cleanly to EnterYourEmail (`NoDeadlock`).
    authority.expire_session(&sess_b.session_id, now + 20);
    device.handle_session_ended();
    assert_eq!(device.screen(), &MobileClientScreen::SessionEnded);
    device
        .check_all_6_invariants(&authority, now + 20)
        .expect("all 6 invariants hold on SessionEnded");
    device.return_to_sign_in();
    assert_eq!(device.screen(), &MobileClientScreen::EnterYourEmail);
}

#[test]
fn scn_m3_0008_portal_pkce_shl_per_record_savepoints_and_part2_boundaries() {
    let mut store = PatientRecordStore::new();
    let acct = MobileAccountId::new("mob-acct-1").expect("acct");
    let now = 1_760_000_000;

    // 1. Single-use PKCE S256 portal link + replay rejection.
    let challenge = PkceS256Challenge::new(
        "state-pkce-1",
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        vec!["patient/Observation.rs".to_owned()],
    )
    .expect("valid PKCE");
    store
        .begin_portal_oauth(PortalVendor::EpicMyChart, challenge)
        .expect("begin oauth");
    let conn = store
        .complete_portal_oauth(
            "state-pkce-1",
            "portal-epic-1",
            acct.clone(),
            PortalVendor::EpicMyChart,
            "Synthetic Health System MyChart",
            now,
        )
        .expect("complete oauth");
    assert_eq!(conn.state, PortalLinkState::Connected);
    assert_eq!(
        store.complete_portal_oauth(
            "state-pkce-1",
            "portal-epic-dup",
            acct.clone(),
            PortalVendor::EpicMyChart,
            "Replay",
            now
        ),
        Err(RecordImportError::PkceStateReplayedOrUnknown)
    );

    // 2. Valid `shlink:/` parses cleanly; insecure `http://` and invalid-key URIs fail before fetch.
    let valid_key_32 = encode_base64url(&[0x42u8; 32]);
    let valid_shl = SmartHealthLinkPayload {
        url: "https://records.example.org/shl/manifest.json".to_owned(),
        key: valid_key_32.clone(),
        flag: Some("L".to_owned()),
        exp: Some(now + 3_600),
        label: Some("Synthetic Discharge Summary".to_owned()),
    };
    let parsed =
        SmartHealthLinkPayload::parse_uri(&valid_shl.to_shl_uri(), now).expect("valid shlink:/");
    assert_eq!(parsed, valid_shl);

    let insecure_shl = SmartHealthLinkPayload {
        url: "http://records.example.org/shl/manifest.json".to_owned(),
        key: valid_key_32,
        flag: None,
        exp: None,
        label: None,
    };
    assert_eq!(
        SmartHealthLinkPayload::parse_uri(&insecure_shl.to_shl_uri(), now),
        Err(RecordImportError::InsecureShlDestination)
    );

    let short_key_shl = SmartHealthLinkPayload {
        url: "https://records.example.org/shl/manifest.json".to_owned(),
        key: encode_base64url(&[0x11u8; 16]),
        flag: None,
        exp: None,
        label: None,
    };
    assert_eq!(
        SmartHealthLinkPayload::parse_uri(&short_key_shl.to_shl_uri(), now),
        Err(RecordImportError::InvalidShlKeyLength { actual_bytes: 16 })
    );

    // 3. Mixed batch ingestion with per-record savepoints + 42 CFR Part 2 boundary enforcement.
    let report = store.ingest_batch_with_savepoints(vec![
        IncomingBatchItem::Valid(PatientRecord {
            record_id: "rec-fhir-1".to_owned(),
            version: 1,
            account_id: acct.clone(),
            incarnation: AccountIncarnation::first(),
            source_kind: ImportSourceKind::SmartFhirR4 {
                vendor: PortalVendor::EpicMyChart,
            },
            portal_id: Some("portal-epic-1".to_owned()),
            bucket: ClinicalRecordBucket::Results,
            confidentiality: ConfidentialityTag::Standard,
            part2_notice: None,
            original_date: "2026-01-10".to_owned(),
            title: "Synthetic CMP Lab".to_owned(),
            summary: "Creatinine 1.0 mg/dL".to_owned(),
            native_bytes: br#"{"resourceType":"Observation"}"#.to_vec(),
            removed_upstream: false,
        }),
        IncomingBatchItem::Malformed {
            external_id: "bad-resource-2".to_owned(),
            reason: "corrupted C-CDA nonXMLBody attachment".to_owned(),
        },
        IncomingBatchItem::Valid(PatientRecord {
            record_id: "rec-part2-3".to_owned(),
            version: 1,
            account_id: acct.clone(),
            incarnation: AccountIncarnation::first(),
            source_kind: ImportSourceKind::KipuBehavioralHealth,
            portal_id: None,
            bucket: ClinicalRecordBucket::Notes,
            confidentiality: ConfidentialityTag::Part2Sud,
            part2_notice: None,
            original_date: "2026-01-11".to_owned(),
            title: "Synthetic SUD Intake Note".to_owned(),
            summary: "42 CFR Part 2 protected intake".to_owned(),
            native_bytes: b"<ClinicalDocument/>".to_vec(),
            removed_upstream: false,
        }),
    ]);

    assert_eq!(
        report.committed_record_ids,
        vec!["rec-fhir-1", "rec-part2-3"]
    );
    assert_eq!(report.failed_items.len(), 1);
    assert_eq!(report.failed_items[0].external_id, "bad-resource-2");

    let part2_rec = store
        .get_record(&acct, "rec-part2-3")
        .expect("part2 stored");
    assert_eq!(
        part2_rec.part2_notice.as_deref(),
        Some(PART2_REDISCLOSURE_NOTICE)
    );
    assert!(!part2_rec.is_general_share_eligible());

    // 4. Disconnecting portal preserves previously imported records; kill-switch blocks new links only.
    store
        .disconnect_portal("portal-epic-1")
        .expect("disconnect portal");
    assert_eq!(
        store.portal("portal-epic-1").expect("portal").state,
        PortalLinkState::Disconnected
    );
    assert_eq!(store.records_for_account(&acct).len(), 2);

    store.set_vendor_kill_switch(PortalVendor::OracleHealth, true);
    let blocked_challenge = PkceS256Challenge::new(
        "state-oracle-1",
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        vec![],
    )
    .expect("pkce");
    assert_eq!(
        store.begin_portal_oauth(PortalVendor::OracleHealth, blocked_challenge),
        Err(RecordImportError::VendorKillSwitchActive {
            vendor: PortalVendor::OracleHealth
        })
    );
}

#[test]
fn scn_m3_0009_health_outbox_account_fencing_bevy_ui_isolation_and_push_revocation() {
    let mut app = build_mobile_app();
    assert_eq!(
        MobileApiEndpoint::HealthTrends.path(),
        "/elysianfields/mobile-api/health/trends"
    );

    // 1. Account A signs in, registers APNs push token, goes offline, and queues health samples.
    push_cmd(
        &mut app,
        MobileClientCommand::SignIn {
            email: "health.a@example.invalid".to_owned(),
        },
    );
    app.update();

    let (acct_a, inc_a) = match app
        .world()
        .resource::<MobileClientState>()
        .auth_device
        .screen()
    {
        MobileClientScreen::SignedIn {
            account_id,
            incarnation,
            ..
        } => (account_id.clone(), *incarnation),
        other => panic!("expected SignedIn A, got {other:?}"),
    };

    app.world_mut()
        .resource_mut::<MobileClientState>()
        .push_registry
        .register_or_rotate(DevicePushRegistration {
            device_id: "ios-dev-1".to_owned(),
            account_id: acct_a.clone(),
            platform: PushPlatform::IosApns,
            token: "apns-tok-1".to_owned(),
            proactive_enabled: true,
        });

    push_cmd(
        &mut app,
        MobileClientCommand::SetConnectivity(ConnectivityState::Offline),
    );
    let batch_a = AccountFencedHealthBatch {
        account_id: acct_a.clone(),
        account_incarnation: inc_a,
        idempotency_key: "hk-batch-a-1".to_owned(),
        samples: vec![
            HealthSampleEntry {
                sample_id: "s-steps".to_owned(),
                metric: HealthMetricKind::Steps,
                value_milli: 8_420_000,
                unit: "count".to_owned(),
                recorded_at_secs: 1_760_000_010,
                platform: HealthAdapterPlatform::IosHealthKit,
            },
            HealthSampleEntry {
                sample_id: "s-hr".to_owned(),
                metric: HealthMetricKind::RestingHeartRate,
                value_milli: 64_000,
                unit: "bpm".to_owned(),
                recorded_at_secs: 1_760_000_020,
                platform: HealthAdapterPlatform::IosHealthKit,
            },
        ],
    };
    push_cmd(
        &mut app,
        MobileClientCommand::QueueHealthBatch(batch_a.clone()),
    );
    push_cmd(
        &mut app,
        MobileClientCommand::SignOut {
            device_id: Some("ios-dev-1".to_owned()),
        },
    );
    app.update();

    // Push token revoked on sign-out, outbox preserves Account A's offline batch durably.
    {
        let st = app.world().resource::<MobileClientState>();
        assert!(st.push_registry.active_for_account(&acct_a).is_empty());
        assert_eq!(st.outbox.total_pending(), 1);
    }

    // 2. Network returns; Account B signs in and runs sync -> ZERO Account A samples uploaded or visible!
    push_cmd(
        &mut app,
        MobileClientCommand::SetConnectivity(ConnectivityState::Online),
    );
    push_cmd(&mut app, MobileClientCommand::FlushOwedSignOuts);
    push_cmd(
        &mut app,
        MobileClientCommand::SignIn {
            email: "health.b@example.invalid".to_owned(),
        },
    );
    push_cmd(
        &mut app,
        MobileClientCommand::SyncHealthOutboxAndRefreshTrends,
    );
    app.update();

    {
        let st = app.world().resource::<MobileClientState>();
        assert!(st.visible_health_trends.is_empty());
        assert_eq!(st.outbox.total_pending(), 1);
    }

    // 3. Account A signs back in and syncs idempotently -> exact 2 samples uploaded, zero duplication.
    push_cmd(&mut app, MobileClientCommand::SignOut { device_id: None });
    push_cmd(
        &mut app,
        MobileClientCommand::SignIn {
            email: "health.a@example.invalid".to_owned(),
        },
    );
    push_cmd(
        &mut app,
        MobileClientCommand::SyncHealthOutboxAndRefreshTrends,
    );
    app.update();
    // Re-enqueuing the already-uploaded idempotency key is a no-op.
    push_cmd(&mut app, MobileClientCommand::QueueHealthBatch(batch_a));
    push_cmd(
        &mut app,
        MobileClientCommand::SyncHealthOutboxAndRefreshTrends,
    );
    app.update();

    {
        let st = app.world().resource::<MobileClientState>();
        assert_eq!(st.outbox.total_pending(), 0);
        assert_eq!(st.visible_health_trends.len(), 2);
        assert!(st.errors.is_empty());
    }

    // 4. Verify background pause skips rendering (`0` frames drawn while hidden) & soft-keyboard resize + RTL locale.
    push_cmd(
        &mut app,
        MobileClientCommand::SwitchTab(MobileTab::HealthTrends),
    );
    push_cmd(&mut app, MobileClientCommand::SetSoftKeyboardInsetPx(300.0));
    app.world_mut().resource_mut::<UiLocale>().current = Locale::Ar;
    app.update();
    assert!(
        (app.world()
            .resource::<ComposerSurface>()
            .visible_height_above_keyboard(844.0)
            - 544.0)
            .abs()
            < 0.01
    );

    let frames_before_bg = app.world().resource::<MobileClientState>().frames_drawn;
    push_cmd(
        &mut app,
        MobileClientCommand::SetVisibility(MobileVisibility::BackgroundPaused),
    );
    for _ in 0..5 {
        app.update();
    }
    let st = app.world().resource::<MobileClientState>();
    assert_eq!(st.frames_drawn, frames_before_bg);
    assert_eq!(st.frames_skipped_background, 5);
}

#[test]
fn scn_m3_0010_for_my_doctor_13_invariants_and_part2_written_consent_gate() {
    let acct = MobileAccountId::new("mob-acct-share").expect("acct");
    let practice_1 = PracticeId::new("practice-1").expect("practice-1");
    let mut records = PatientRecordStore::new();
    let mut receiver = PracticeSendReceiver::new();
    let mut fmd = ForMyDoctorWorkspace::new(acct.clone());

    let make_rec = |id: &str, conf: ConfidentialityTag, summary: &str| PatientRecord {
        record_id: id.to_owned(),
        version: 1,
        account_id: acct.clone(),
        incarnation: AccountIncarnation::first(),
        source_kind: ImportSourceKind::SmartHealthLink,
        portal_id: None,
        bucket: ClinicalRecordBucket::Results,
        confidentiality: conf,
        part2_notice: None,
        original_date: "2026-01-12".to_owned(),
        title: format!("Record {id}"),
        summary: summary.to_owned(),
        native_bytes: b"{}".to_vec(),
        removed_upstream: false,
    };

    let _ = records.ingest_batch_with_savepoints(vec![
        IncomingBatchItem::Valid(make_rec(
            "R1",
            ConfidentialityTag::Standard,
            "Original R1 v1",
        )),
        IncomingBatchItem::Valid(make_rec("R2", ConfidentialityTag::Standard, "Linked R2 v1")),
        IncomingBatchItem::Valid(make_rec(
            "R3",
            ConfidentialityTag::Part2Sud,
            "Part 2 SUD R3",
        )),
        IncomingBatchItem::Valid(make_rec("R4", ConfidentialityTag::Standard, "Removable R4")),
        IncomingBatchItem::Valid(make_rec(
            "R5",
            ConfidentialityTag::Part2SudCounselingNotes,
            "SUD Counseling Note R5",
        )),
    ]);

    // Mark R1 and R4 BEFORE connecting (`JoinAndLeaveSafe`: connecting never auto-sends!).
    fmd.mark_record("R1", &records).expect("mark R1");
    fmd.mark_record("R4", &records).expect("mark R4");
    fmd.stage_unsaved_note("N1", "Unsaved draft note", vec!["R2".to_owned()]);
    assert_eq!(fmd.notes()["N1"].sync_state, NoteSaveState::UnsavedLocal);
    let saved_n1 = fmd
        .save_note("N1", "Questions for visit", vec!["R2".to_owned()], 0)
        .expect("save N1");
    assert_eq!(saved_n1.revision, 1);
    // Stale remote save fails with optimistic concurrency conflict (`SaveHonesty`).
    assert!(matches!(
        fmd.save_note("N1", "Stale overwrite", vec![], 0),
        Err(ForMyDoctorError::NoteSaveConflict { .. })
    ));

    fmd.connect_practice(practice_1.clone());
    assert_eq!(receiver.total_received_count(), 0); // `NothingReachesWithoutConfirm` & `JoinAndLeaveSafe`!

    // Must acknowledge pre-send disclosure (`DisclosureBeforeSend`).
    assert!(SEND_IRREVERSIBLE_DISCLOSURE.contains("cannot be taken back"));
    assert_eq!(
        fmd.confirm_send(
            &practice_1,
            &["R1".to_owned()],
            &["N1".to_owned()],
            "send-key-1",
            false,
            Part2ConsentConfirmation::None,
            &records,
            &mut receiver,
            SendTransportOutcome::PersistedSent,
        ),
        Err(ForMyDoctorError::DisclosureNotAcknowledged)
    );

    // Uncertain timeout followed by idempotent retry (`SendReachesAtMostOnce`, `ConfirmedSendClearsCarriedOnly`).
    let uncertain = fmd
        .confirm_send(
            &practice_1,
            &["R1".to_owned()],
            &["N1".to_owned()],
            "send-key-1",
            true,
            Part2ConsentConfirmation::None,
            &records,
            &mut receiver,
            SendTransportOutcome::NetworkUncertain,
        )
        .expect("uncertain send");
    assert_eq!(uncertain.status, SendDeliveryStatus::Uncertain);
    assert!(uncertain.try_again_available);
    // Unconfirmed send clears NOTHING from To send!
    assert!(fmd.marked_record_ids().contains("R1"));
    assert!(fmd.notes().contains_key("N1"));

    let confirmed = fmd
        .retry_uncertain_send(&uncertain.snapshot.send_id, &mut receiver)
        .expect("retry succeeds");
    assert_eq!(confirmed.status, SendDeliveryStatus::Sent);
    assert_eq!(receiver.delivery_count("send-key-1"), 1); // At most once!
    // Carried R1 & N1 cleared; unchosen R4 remains!
    assert!(!fmd.marked_record_ids().contains("R1"));
    assert!(!fmd.notes().contains_key("N1"));
    assert!(fmd.marked_record_ids().contains("R4"));
    // Both R1 and note-linked R2 are frozen in snapshot (`SendHoldsExactImmutableContent`).
    assert_eq!(confirmed.snapshot.records.len(), 2);

    // Mutate R1 upstream -> already-confirmed send snapshot stays immutable (`SourceCorrectionSafe`).
    records.apply_upstream_correction(&acct, "R1", "Mutated R1 v2");
    assert_eq!(
        fmd.sends()[0]
            .snapshot
            .records
            .iter()
            .find(|r| r.record_id == "R1")
            .expect("R1 in frozen send")
            .summary,
        "Original R1 v1"
    );

    // Practice pickup transitions `Sent` -> `ReceivedByCareTeam` (`SentAndReceivedTruth`).
    assert!(receiver.confirm_care_team_pickup("send-key-1"));
    fmd.sync_care_team_receipt(&receiver);
    assert_eq!(
        fmd.sends()[0].status,
        SendDeliveryStatus::ReceivedByCareTeam
    );

    // Remove R4 upstream -> `SourceRemovalClearsMark` clears R4 with a plain notice.
    assert!(records.mark_removed_upstream(&acct, "R4"));
    fmd.on_source_removed("R4");
    assert!(!fmd.marked_record_ids().contains("R4"));
    assert_eq!(fmd.notices().len(), 1);

    // 42 CFR Part 2 consent gate: R3 blocked without written consent, R5 requires separate counseling-note consent.
    fmd.mark_record("R3", &records).expect("mark R3");
    fmd.mark_record("R5", &records).expect("mark R5");
    assert_eq!(
        fmd.confirm_send(
            &practice_1,
            &["R3".to_owned()],
            &[],
            "send-key-part2",
            true,
            Part2ConsentConfirmation::None,
            &records,
            &mut receiver,
            SendTransportOutcome::PersistedSent,
        ),
        Err(ForMyDoctorError::Part2WrittenConsentRequired {
            record_id: "R3".to_owned()
        })
    );
    assert_eq!(
        fmd.confirm_send(
            &practice_1,
            &["R5".to_owned()],
            &[],
            "send-key-counseling",
            true,
            Part2ConsentConfirmation::SudRecordsOnly {
                consented_at: "2026-01-12T10:00:00Z".to_owned()
            },
            &records,
            &mut receiver,
            SendTransportOutcome::PersistedSent,
        ),
        Err(ForMyDoctorError::Part2CounselingNotesConsentRequired {
            record_id: "R5".to_owned()
        })
    );

    let part2_sent = fmd
        .confirm_send(
            &practice_1,
            &["R3".to_owned()],
            &[],
            "send-key-part2",
            true,
            Part2ConsentConfirmation::SudRecordsOnly {
                consented_at: "2026-01-12T10:00:00Z".to_owned(),
            },
            &records,
            &mut receiver,
            SendTransportOutcome::PersistedSent,
        )
        .expect("Part 2 send succeeds with written consent");
    assert_eq!(
        part2_sent.snapshot.part2_notice_attached.as_deref(),
        Some(PART2_REDISCLOSURE_NOTICE)
    );

    // Connection end stops any uncertain send & removes Try again while preserving Sent history (`ConnectionEndStopsUncertain`).
    let uncertain_r5 = fmd
        .confirm_send(
            &practice_1,
            &["R5".to_owned()],
            &[],
            "send-key-r5-uncertain",
            true,
            Part2ConsentConfirmation::SudAndCounselingNotes {
                consented_at: "2026-01-12T10:05:00Z".to_owned(),
            },
            &records,
            &mut receiver,
            SendTransportOutcome::NetworkUncertain,
        )
        .expect("uncertain R5");
    fmd.end_practice_connection(&practice_1);
    assert_eq!(fmd.practice_status(&practice_1), PracticeLinkStatus::Ended);
    assert_eq!(
        fmd.sends().last().expect("last").status,
        SendDeliveryStatus::StoppedConnectionEnded
    );
    assert!(!fmd.sends().last().expect("last").try_again_available);
    assert!(fmd.marked_record_ids().contains("R5"));
    assert_eq!(
        fmd.retry_uncertain_send(&uncertain_r5.snapshot.send_id, &mut receiver),
        Err(ForMyDoctorError::RetryNotAvailable {
            send_id: uncertain_r5.snapshot.send_id
        })
    );
    assert_eq!(
        fmd.sends()[0].status,
        SendDeliveryStatus::ReceivedByCareTeam
    );
    fmd.verify_invariants(&records, &receiver)
        .expect("all Shared.tla invariants hold");
}

#[test]
fn scn_m3_0011_portable_export_account_deletion_and_isolated_restore_replay_gate() {
    // Verify all 13 restriction ops serialize to their schema names.
    assert_eq!(RestrictionOperation::ALL.len(), 13);

    let mut auth = MobileAuthAuthority::new();
    let mut records = PatientRecordStore::new();
    let health = MobileHealthIngestStore::new();
    let push_tokens = PushTokenRegistry::new();
    let mut doctor_workspaces = BTreeMap::new();
    let mut bridge_staging = PracticeSendReceiver::new();
    let now = 1_760_000_000;

    let tok = auth
        .issue_sign_in_link("delete.me@example.invalid", now)
        .expect("link");
    let sess = auth
        .verify_sign_in("delete.me@example.invalid", &tok, now)
        .expect("verify");
    let acct = sess.account_id.clone();

    let _ = records.ingest_batch_with_savepoints(vec![IncomingBatchItem::Valid(PatientRecord {
        record_id: "rec-del-1".to_owned(),
        version: 1,
        account_id: acct.clone(),
        incarnation: sess.incarnation,
        source_kind: ImportSourceKind::CcdaR21Xml,
        portal_id: None,
        bucket: ClinicalRecordBucket::Documents,
        confidentiality: ConfidentialityTag::Standard,
        part2_notice: None,
        original_date: "2026-01-09".to_owned(),
        title: "Synthetic Summary".to_owned(),
        summary: "Sensitive narrative text that must NEVER enter privacy receipts".to_owned(),
        native_bytes: b"<doc/>".to_vec(),
        removed_upstream: false,
    })]);
    let practice = PracticeId::new("practice-1").expect("practice");
    let mut fmd = ForMyDoctorWorkspace::new(acct.clone());
    fmd.connect_practice(practice.clone());
    fmd.mark_record("rec-del-1", &records).expect("mark");
    let _ = fmd
        .confirm_send(
            &practice,
            &["rec-del-1".to_owned()],
            &[],
            "del-send-1",
            true,
            Part2ConsentConfirmation::None,
            &records,
            &mut bridge_staging,
            SendTransportOutcome::PersistedSent,
        )
        .expect("send");
    doctor_workspaces.insert(acct.clone(), fmd);

    // 1. Portable export + valid doctor-visit PDF prior to deletion.
    let export = PortablePatientExport {
        account_id: acct.clone(),
        exported_at: "2026-01-12T12:00:00Z".to_owned(),
        records: records
            .records_for_account(&acct)
            .into_iter()
            .cloned()
            .collect(),
        health_samples: Vec::new(),
        confirmed_sends: doctor_workspaces[&acct].sends().to_vec(),
    };
    let pdf = export.render_doctor_visit_pdf();
    assert!(pdf.starts_with(b"%PDF-1.4"));

    // 2. Take a pre-deletion PostgreSQL backup snapshot BEFORE account deletion!
    let mut restored_auth = auth.clone();
    let mut restored_records = records.clone();
    let mut restored_health = health.clone();
    let mut restored_push = push_tokens.clone();
    let mut restored_fmd = doctor_workspaces.clone();
    let mut restored_staging = bridge_staging.clone();

    // 3. Permanently delete account, writing immutable metadata-only receipt to independent journal.
    let mut journal = InMemoryReceiptJournal::new();
    let receipt = PrivacyReceipt {
        schema_version: 1,
        event_id: "evt-acct-del-001".to_owned(),
        account_id: acct.clone(),
        account_incarnation: sess.incarnation,
        operation: RestrictionOperation::AccountDelete,
        scope_id: String::new(),
        scope_version: 0,
        occurred_at: "2026-01-12T12:05:00Z".to_owned(),
    };
    assert_eq!(
        journal.put_if_absent(&receipt).expect("create-only write"),
        PutReceiptOutcome::Created
    );
    assert_eq!(
        journal
            .put_if_absent(&receipt)
            .expect("idempotent create-only"),
        PutReceiptOutcome::AlreadyPresent
    );

    // Verify receipt contains ZERO patient prose, PHI, or credentials.
    let raw_receipt = journal
        .read_exact("evt-acct-del-001")
        .expect("read exact")
        .expect("present");
    let raw_str = String::from_utf8(raw_receipt).expect("utf8");
    assert!(!raw_str.contains("Sensitive narrative text"));
    assert!(!raw_str.contains("delete.me@example.invalid"));

    // 4. Fail-closed check when independent journal is unavailable or budget is exhausted.
    journal.set_unavailable(Some("object store unreachable".to_owned()));
    {
        let mut target = MobilePrivacyTarget {
            auth: &mut restored_auth,
            records: &mut restored_records,
            health: &mut restored_health,
            push_tokens: &mut restored_push,
            doctor_workspaces: &mut restored_fmd,
            bridge_staging: &mut restored_staging,
        };
        assert!(matches!(
            run_restore_replay_gate(&journal, &mut target, 100),
            Err(RestrictionJobError::Journal(_))
        ));
    }
    journal.set_unavailable(None);
    {
        let mut target = MobilePrivacyTarget {
            auth: &mut restored_auth,
            records: &mut restored_records,
            health: &mut restored_health,
            push_tokens: &mut restored_push,
            doctor_workspaces: &mut restored_fmd,
            bridge_staging: &mut restored_staging,
        };
        assert!(matches!(
            run_restore_replay_gate(&journal, &mut target, 0),
            Err(RestrictionJobError::FailedClosed {
                budget_exhausted: true,
                ..
            })
        ));
    }

    // 5. Run mandatory restore replay gate against restored pre-deletion backup BEFORE serving!
    let report = {
        let mut target = MobilePrivacyTarget {
            auth: &mut restored_auth,
            records: &mut restored_records,
            health: &mut restored_health,
            push_tokens: &mut restored_push,
            doctor_workspaces: &mut restored_fmd,
            bridge_staging: &mut restored_staging,
        };
        run_restore_replay_gate(&journal, &mut target, 100).expect("restore replay gate passes")
    };
    assert_eq!(report.applied, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.pending, 0);
    assert_eq!(report.scrubbed_staging_records, 1);

    // Deleted account CANNOT sign in or resurrect; records purged; connected-care link terminated; staging scrubbed!
    assert!(!restored_auth.is_session_live(&sess.session_id, now + 30));
    assert!(
        restored_auth
            .issue_sign_in_link("delete.me@example.invalid", now + 30)
            .is_err()
    );
    assert!(restored_records.records_for_account(&acct).is_empty());
    assert_eq!(
        restored_fmd[&acct].practice_status(&practice),
        PracticeLinkStatus::Ended
    );
    assert_eq!(restored_staging.total_received_count(), 0);
}
