#![cfg(not(target_arch = "wasm32"))]
//! Live TCP integration tests exercising `LiveServerWorker::spawn(...)` against a real
//! `ef-mobile-server` TCP listener end-to-end:
//! - Sign-in (`request_sign_in_link` + `verify_sign_in`)
//! - Connected Care DOB verification + explicit Share consent
//! - Idempotent For My Doctor curated note delivery (proving duplicate suppression)
//! - Offline-safe sign-out (`SignInHoldsNothing`) + `FlushOwedSignOuts` recovery UI & server revocation

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use ef_connected_care::{
    ConfidentialityTag, ConnectedCareState, CreateInvitationRequest, DateOfBirth,
    DobVerificationOutcome, EnvelopeItem, InviteCredential, PairKey,
};
use ef_core::{IdempotencyKey, PatientId, PhysicianId, PracticeId};
use ef_mobile_client::{
    LiveHttpMobileTransport, MobileClientState, MobileClientTransport, MobileTransportError,
};
use ef_mobile_desktop::{
    ActionButton, DesktopAction, DesktopActionQueue, FLUSH_OWED_SIGN_OUTS_LABEL, LiveServerWorker,
    LiveWorkerRequest, LiveWorkerResponse, MobileDesktopConfig, MobileDesktopUiState,
    OwedRecoveryBannerNode, OwedRecoveryBannerState, PendingWorkerState, TextSlot,
    build_headless_mobile_desktop_app,
};
use ef_mobile_domain::{
    ConnectivityState, FileReceiptJournal, MobileAuthError, MobileClientScreen,
    PrivacyReceiptJournal,
};
use ef_mobile_server::{MobileService, TcpMobileServer};
use ef_ui_catalog::{
    A11yProjection, CatalogA11ySyncStatus, ComposerAction, ComposerSurface, Locale, UiKey, UiTheme,
    translate,
};

fn fresh_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "ef-mobile-worker-e2e-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn wait_for_single_response(
    worker: &mut LiveServerWorker,
    timeout: Duration,
) -> LiveWorkerResponse {
    let deadline = Instant::now() + timeout;
    loop {
        let mut drained = worker.drain_responses();
        if !drained.is_empty() {
            assert_eq!(drained.len(), 1, "expected exactly 1 worker response");
            return drained.remove(0);
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for LiveServerWorker response"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for_app_worker_idle(app: &mut App, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        app.update();
        let is_idle = app
            .world()
            .get_resource::<LiveServerWorker>()
            .is_none_or(LiveServerWorker::is_idle);
        if is_idle {
            app.update();
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for LiveServerWorker in Bevy App"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn dispatch_and_settle(app: &mut App, action: DesktopAction) {
    app.world_mut()
        .resource_mut::<DesktopActionQueue>()
        .push(action);
    app.update();
    wait_for_app_worker_idle(app, Duration::from_secs(15));
}

fn text_slot_value(app: &mut App, target: TextSlot) -> String {
    let mut q = app.world_mut().query::<(&TextSlot, &Text)>();
    for (slot, text) in q.iter(app.world()) {
        if *slot == target {
            return text.0.clone();
        }
    }
    panic!("missing TextSlot::{target:?} in UI entity tree");
}

#[test]
fn live_mobile_worker_round_trip_idempotent_delivery_and_owed_sign_out_recovery() {
    let db_dir = fresh_temp_dir("roundtrip-db");
    let journal_dir = fresh_temp_dir("roundtrip-journal");

    let journal = FileReceiptJournal::open(&journal_dir).expect("open receipt journal");
    let mut svc = MobileService::open_durable(&db_dir, Box::new(journal), 1_760_000_000, false)
        .expect("open durable mobile service");

    let expected_dob = DateOfBirth::new(1990, 3, 19).expect("valid dob");
    let pair = PairKey::new(
        PracticeId::new("practice-w15-live").expect("practice id"),
        PatientId::new("patient-w15-live").expect("patient id"),
    );
    let issued = svc
        .connected_care
        .create_invitation(CreateInvitationRequest {
            pair,
            issued_by: PhysicianId::new("phys-w15-live").expect("phys id"),
            practice_name: "Wave 15 Live Family Practice".to_owned(),
            physician_name: "Dr. Samira Patel".to_owned(),
            expected_dob,
            created_at: "2026-02-01T10:00:00Z".to_owned(),
        })
        .expect("issue connected care invite");
    let credential = InviteCredential::ShortCode(issued.short_code.clone());

    let server = TcpMobileServer::bind("127.0.0.1:0", svc).expect("bind TcpMobileServer");
    let addr = server.local_addr();
    let timeout = Duration::from_secs(5);

    // -------------------------------------------------------------------------
    // Phase A: Direct LiveServerWorker::spawn(...) round-trip + idempotency proof
    // -------------------------------------------------------------------------
    let mut worker = LiveServerWorker::spawn(addr);
    assert_eq!(worker.pending_state(), PendingWorkerState::Idle);

    // 1. Sign-in (`request_sign_in_link` + `verify_sign_in`)
    worker
        .submit(LiveWorkerRequest::SignIn {
            email: "w15.worker.patient@example.invalid".to_owned(),
            now_secs: 1_760_000_000,
        })
        .expect("submit sign-in");
    let session = match wait_for_single_response(&mut worker, timeout) {
        LiveWorkerResponse::SignIn { email, result } => {
            assert_eq!(email, "w15.worker.patient@example.invalid");
            result.expect("live worker sign-in succeeds")
        }
        other => panic!("unexpected response for SignIn: {other:?}"),
    };
    assert!(worker.is_idle());

    // 2. Connected Care DOB verification + explicit Share consent
    worker
        .submit(LiveWorkerRequest::VerifyDob {
            session_id: session.session_id.clone(),
            credential: credential.clone(),
            dob: expected_dob,
            now_secs: 1_760_000_010,
        })
        .expect("submit verify-dob");
    match wait_for_single_response(&mut worker, timeout) {
        LiveWorkerResponse::VerifyDob {
            result: Ok(DobVerificationOutcome::Verified { preview }),
        } => {
            assert_eq!(preview.practice_name, "Wave 15 Live Family Practice");
            assert_eq!(preview.physician_name, "Dr. Samira Patel");
        }
        other => panic!("expected verified DOB outcome, got: {other:?}"),
    }

    worker
        .submit(LiveWorkerRequest::ShareConnectedCare {
            session_id: session.session_id.clone(),
            credential: credential.clone(),
            now_secs: 1_760_000_020,
        })
        .expect("submit share");
    let connection_id = match wait_for_single_response(&mut worker, timeout) {
        LiveWorkerResponse::ShareConnectedCare {
            result: Ok(standing),
        } => {
            assert_eq!(standing.state, ConnectedCareState::Connected);
            standing
                .active_connection_id
                .expect("active connection id after explicit Share")
        }
        other => panic!("expected active connection standing, got: {other:?}"),
    };

    // 3. Idempotent For My Doctor curated note delivery (duplicate suppression proof)
    let shared_idem_key = IdempotencyKey::new("w15-idem-note-001").expect("idempotency key");
    let note_items = vec![EnvelopeItem {
        item_id: "w15-note-1".to_owned(),
        source_version: 1,
        confidentiality: ConfidentialityTag::Standard,
        payload_sha256: "w15-sha256-0001".to_owned(),
        verbatim_payload: "Morning glucose 104 mg/dL, steady energy".to_owned(),
        part2_notice: None,
    }];

    worker
        .submit(LiveWorkerRequest::SendForMyDoctor {
            session_id: session.session_id.clone(),
            idempotency_key: shared_idem_key.clone(),
            connection_id: connection_id.clone(),
            items: note_items.clone(),
            part2_consent: None,
            now_secs: 1_760_000_030,
        })
        .expect("submit first curated note");
    let first_envelope = match wait_for_single_response(&mut worker, timeout) {
        LiveWorkerResponse::SendForMyDoctor { result: Ok(env) } => env,
        other => panic!("expected first transfer envelope, got: {other:?}"),
    };

    // Re-submit with the exact same IdempotencyKey: server suppresses duplicates & returns same transfer_id.
    worker
        .submit(LiveWorkerRequest::SendForMyDoctor {
            session_id: session.session_id.clone(),
            idempotency_key: shared_idem_key,
            connection_id: connection_id.clone(),
            items: note_items,
            part2_consent: None,
            now_secs: 1_760_000_031,
        })
        .expect("submit duplicate curated note");
    let duplicate_envelope = match wait_for_single_response(&mut worker, timeout) {
        LiveWorkerResponse::SendForMyDoctor { result: Ok(env) } => env,
        other => panic!("expected idempotent replay envelope, got: {other:?}"),
    };
    assert_eq!(
        first_envelope.transfer_id, duplicate_envelope.transfer_id,
        "retrying with identical IdempotencyKey must suppress duplicates and return the same transfer_id"
    );
    assert_eq!(
        duplicate_envelope.items[0].verbatim_payload,
        "Morning glucose 104 mg/dL, steady energy"
    );

    // Distinct IdempotencyKey creates a distinct transfer_id.
    let second_idem_key = IdempotencyKey::new("w15-idem-note-002").expect("idempotency key 2");
    worker
        .submit(LiveWorkerRequest::SendForMyDoctor {
            session_id: session.session_id.clone(),
            idempotency_key: second_idem_key,
            connection_id,
            items: vec![EnvelopeItem {
                item_id: "w15-note-2".to_owned(),
                source_version: 2,
                confidentiality: ConfidentialityTag::Standard,
                payload_sha256: "w15-sha256-0002".to_owned(),
                verbatim_payload: "Evening walk 30 minutes, no symptoms".to_owned(),
                part2_notice: None,
            }],
            part2_consent: None,
            now_secs: 1_760_000_040,
        })
        .expect("submit second distinct note");
    let second_envelope = match wait_for_single_response(&mut worker, timeout) {
        LiveWorkerResponse::SendForMyDoctor { result: Ok(env) } => env,
        other => panic!("expected second transfer envelope, got: {other:?}"),
    };
    assert_ne!(
        first_envelope.transfer_id, second_envelope.transfer_id,
        "distinct IdempotencyKey must allocate a distinct transfer_id"
    );
    drop(worker);

    // -------------------------------------------------------------------------
    // Phase B: Offline-safe Sign-Out (`SignInHoldsNothing`) + Recovery UI + FlushOwedSignOuts
    // -------------------------------------------------------------------------
    let cfg = MobileDesktopConfig::parse_args([
        "--server",
        &format!("http://{addr}"),
        "--email",
        "w15.offline.recovery@example.invalid",
        "--no-prefs",
    ])
    .expect("live cfg");
    let mut app = build_headless_mobile_desktop_app(&cfg);
    app.update();

    // Sign in live, verify initial online recovery banner.
    dispatch_and_settle(&mut app, DesktopAction::SignIn);
    let active_session_id = match app
        .world()
        .resource::<MobileClientState>()
        .auth_device
        .screen()
    {
        MobileClientScreen::SignedIn { session_id, .. } => session_id.clone(),
        other => panic!("expected SignedIn screen, got {other:?}"),
    };

    // Add unsent draft text in composer to verify SignInHoldsNothing clears it on offline sign-out.
    let _ = app
        .world_mut()
        .resource_mut::<ComposerSurface>()
        .apply(ComposerAction::InsertText(
            "Sensitive unsent draft cleared on sign-out".to_owned(),
        ));
    app.update();
    assert!(!app.world().resource::<ComposerSurface>().text().is_empty());

    // Go offline and sign out -> SignInHoldsNothing clears local state immediately & queues 1 owed sign-out.
    dispatch_and_settle(&mut app, DesktopAction::ToggleConnectivity);
    assert_eq!(
        app.world()
            .resource::<MobileClientState>()
            .auth_device
            .connectivity(),
        ConnectivityState::Offline
    );

    dispatch_and_settle(&mut app, DesktopAction::OfflineSignOut);
    {
        let client_state = app.world().resource::<MobileClientState>();
        assert_eq!(
            client_state.auth_device.screen(),
            &MobileClientScreen::EnterYourEmail
        );
        assert_eq!(client_state.auth_device.owed_sign_outs().len(), 1);
        assert!(client_state.sent_envelopes.is_empty());
        assert!(client_state.last_dob_outcome.is_none());
        assert!(client_state.last_connection_standing.is_none());
        assert!(client_state.visible_records.is_empty());
        assert!(client_state.visible_health_trends.is_empty());
    }
    assert!(
        app.world().resource::<ComposerSurface>().text().is_empty(),
        "SignInHoldsNothing must clear composer text immediately on offline sign-out"
    );

    let offline_banner = text_slot_value(&mut app, TextSlot::OwedRecoveryBanner);
    assert!(offline_banner.contains(translate(Locale::En, UiKey::MobileConnectivityOffline)));
    assert!(offline_banner.contains("1"));
    assert!(offline_banner.contains(translate(Locale::En, UiKey::MobileReconnectToFlush)));

    // Confirm session is still alive on the server prior to reconnect flush.
    let probe_transport = LiveHttpMobileTransport::new(addr);
    assert!(
        probe_transport.is_session_live(&active_session_id, 1_760_000_050),
        "session must still be live on server before offline owed sign-out is flushed"
    );

    // Reconnect online -> banner prompts ready to flush & Flush button shows count badge (1).
    dispatch_and_settle(&mut app, DesktopAction::ToggleConnectivity);
    assert_eq!(
        app.world()
            .resource::<MobileClientState>()
            .auth_device
            .connectivity(),
        ConnectivityState::Online
    );
    let pending_banner = text_slot_value(&mut app, TextSlot::OwedRecoveryBanner);
    assert!(pending_banner.contains(translate(Locale::En, UiKey::MobileConnectivityOnline)));
    assert!(pending_banner.contains(translate(Locale::En, UiKey::MobileReadyToFlush)));
    assert_eq!(
        text_slot_value(&mut app, TextSlot::FlushOwedBtnLabel),
        format!("{FLUSH_OWED_SIGN_OUTS_LABEL} (1)")
    );

    // Trigger FlushOwedSignOuts over LiveServerWorker -> clears local queue AND revokes server session.
    dispatch_and_settle(&mut app, DesktopAction::FlushOwedSignOuts);
    {
        let client_state = app.world().resource::<MobileClientState>();
        assert_eq!(client_state.auth_device.owed_sign_outs().len(), 0);
        assert!(
            client_state.errors.is_empty(),
            "unexpected mobile client errors: {:?}",
            client_state.errors
        );
    }
    let synced_banner = text_slot_value(&mut app, TextSlot::OwedRecoveryBanner);
    assert!(synced_banner.contains(translate(Locale::En, UiKey::MobileConnectivityOnline)));
    assert!(synced_banner.contains("0"));
    assert_eq!(
        text_slot_value(&mut app, TextSlot::FlushOwedBtnLabel),
        FLUSH_OWED_SIGN_OUTS_LABEL
    );

    // Server-side revocation proof: old session_id is now rejected with HTTP 401 / SessionEnded.
    assert!(
        !probe_transport.is_session_live(&active_session_id, 1_760_000_060),
        "server session must be revoked after FlushOwedSignOuts completes"
    );
    let revoked_err = probe_transport
        .fetch_records(&active_session_id, 1_760_000_060)
        .expect_err("revoked session must fail fetch_records");
    assert!(
        matches!(
            revoked_err,
            MobileTransportError::Http { status: 401, .. }
                | MobileTransportError::Auth(MobileAuthError::SessionEnded)
        ),
        "expected HTTP 401 / SessionEnded for revoked session, got {revoked_err:?}"
    );

    // Verify 0 duplicate AccessKit IDs with the new Connection & Owed Sign-Out banner + Flush button.
    let a11y_errors = &app.world().resource::<CatalogA11ySyncStatus>().errors;
    assert!(
        a11y_errors.is_empty(),
        "unexpected AccessKit sync errors: {a11y_errors:?}"
    );
    let audit = app
        .world()
        .resource::<A11yProjection>()
        .verify_zero_duplicate_nodes()
        .expect("zero duplicate AccessKit nodes");
    assert_eq!(audit.duplicated_nodes, 0);
    assert!(audit.total_nodes >= 15);

    server.shutdown();
    let _ = fs::remove_dir_all(&db_dir);
    let _ = fs::remove_dir_all(&journal_dir);
}

#[test]
fn owed_recovery_banner_and_flush_button_states_and_a11y_integrity() {
    let cfg = MobileDesktopConfig::parse_args(["--demo", "--no-prefs"]).expect("demo cfg");
    let mut app = build_headless_mobile_desktop_app(&cfg);
    app.update();

    // Verify Flush Owed Sign-Outs button entity exists and is wired to DesktopAction::FlushOwedSignOuts.
    let flush_btn_count = app
        .world_mut()
        .query::<&ActionButton>()
        .iter(app.world())
        .filter(|b| b.action == DesktopAction::FlushOwedSignOuts)
        .count();
    assert_eq!(
        flush_btn_count, 1,
        "expected exactly 1 FlushOwedSignOuts action button"
    );

    let banner_count = app
        .world_mut()
        .query_filtered::<Entity, With<OwedRecoveryBannerNode>>()
        .iter(app.world())
        .count();
    assert_eq!(banner_count, 1, "expected exactly 1 OwedRecoveryBannerNode");

    // Sign in -> Offline -> Sign out -> Online -> verify visual prominence & state machine transitions.
    dispatch_and_settle(&mut app, DesktopAction::SignIn);
    dispatch_and_settle(&mut app, DesktopAction::ToggleConnectivity);
    dispatch_and_settle(&mut app, DesktopAction::OfflineSignOut);

    let state_offline = OwedRecoveryBannerState::from_runtime(
        ConnectivityState::Offline,
        1,
        PendingWorkerState::Idle,
    );
    assert!(!state_offline.flush_action_prominent());

    dispatch_and_settle(&mut app, DesktopAction::ToggleConnectivity);
    let state_pending = OwedRecoveryBannerState::from_runtime(
        ConnectivityState::Online,
        1,
        PendingWorkerState::Idle,
    );
    assert!(state_pending.flush_action_prominent());

    let focus_ring = BorderColor::all(
        app.world()
            .resource::<UiTheme>()
            .tokens
            .focus_ring
            .to_bevy_color(),
    );
    let mut banner_q = app
        .world_mut()
        .query_filtered::<(&Node, &BorderColor), With<OwedRecoveryBannerNode>>();
    let (banner_node, banner_border) = banner_q
        .iter(app.world())
        .next()
        .expect("OwedRecoveryBannerNode present");
    assert_eq!(banner_node.border, UiRect::all(Val::Px(2.0)));
    assert_eq!(*banner_border, focus_ring);

    // Flush owed sign-outs in demo mode -> returns cleanly to SyncedOnline (0 owed).
    dispatch_and_settle(&mut app, DesktopAction::FlushOwedSignOuts);
    assert_eq!(
        app.world()
            .resource::<MobileClientState>()
            .auth_device
            .owed_sign_outs()
            .len(),
        0
    );
    assert!(
        app.world()
            .resource::<CatalogA11ySyncStatus>()
            .errors
            .is_empty()
    );
    let audit = app
        .world()
        .resource::<A11yProjection>()
        .verify_zero_duplicate_nodes()
        .expect("zero duplicate AccessKit nodes in demo mode");
    assert_eq!(audit.duplicated_nodes, 0);
}

#[test]
fn live_mobile_privacy_controls_export_disconnect_and_delete_account_e2e() {
    let db_dir = fresh_temp_dir("privacy-db");
    let journal_dir = fresh_temp_dir("privacy-journal");

    let journal = FileReceiptJournal::open(&journal_dir).expect("open receipt journal");
    let mut svc = MobileService::open_durable(&db_dir, Box::new(journal), 1_760_000_000, false)
        .expect("open durable mobile service");

    let expected_dob = DateOfBirth::new(1989, 8, 22).expect("valid dob");
    let pair = PairKey::new(
        PracticeId::new("practice-w21-privacy").expect("practice id"),
        PatientId::new("patient-w21-privacy").expect("patient id"),
    );
    let issued = svc
        .connected_care
        .create_invitation(CreateInvitationRequest {
            pair,
            issued_by: PhysicianId::new("phys-w21-privacy").expect("phys id"),
            practice_name: "Wave 21 Privacy Clinic".to_owned(),
            physician_name: "Dr. Clara Lin".to_owned(),
            expected_dob,
            created_at: "2026-03-01T10:00:00Z".to_owned(),
        })
        .expect("issue connected care invite");

    let server = TcpMobileServer::bind("127.0.0.1:0", svc).expect("bind TcpMobileServer");
    let addr = server.local_addr();

    let cfg = MobileDesktopConfig::parse_args([
        "--server",
        &format!("http://{addr}"),
        "--email",
        "w21.privacy.patient@example.invalid",
        "--no-prefs",
    ])
    .expect("live cfg");
    let mut app = build_headless_mobile_desktop_app(&cfg);
    app.world_mut()
        .resource_mut::<MobileDesktopUiState>()
        .set_invite_credential(
            InviteCredential::ShortCode(issued.short_code.clone()),
            issued.short_code.as_str().to_owned(),
            expected_dob,
        );
    app.update();

    // Confirm all 3 clickable privacy buttons exist in the UI tree.
    for required_action in [
        DesktopAction::ExportData,
        DesktopAction::DeleteAccount,
        DesktopAction::DisconnectConnectedCare,
    ] {
        let count = app
            .world_mut()
            .query::<&ActionButton>()
            .iter(app.world())
            .filter(|b| b.action == required_action)
            .count();
        assert_eq!(count, 1, "expected 1 ActionButton for {required_action:?}");
    }

    // 1. Sign in live -> Sync Health Snapshot (4 samples + 2 personal records).
    dispatch_and_settle(&mut app, DesktopAction::SignIn);
    let active_session_id = match app
        .world()
        .resource::<MobileClientState>()
        .auth_device
        .screen()
    {
        MobileClientScreen::SignedIn { session_id, .. } => session_id.clone(),
        other => panic!("expected SignedIn screen, got {other:?}"),
    };

    dispatch_and_settle(&mut app, DesktopAction::SyncDemoHealthSnapshot);
    {
        let mobile = app.world().resource::<MobileClientState>();
        assert_eq!(mobile.visible_health_trends.len(), 4);
        assert_eq!(mobile.visible_records.len(), 2);
        assert!(mobile.errors.is_empty(), "errors: {:?}", mobile.errors);
    }

    // 2. ExportData -> returns PortablePatientExport containing all synced samples & records.
    dispatch_and_settle(&mut app, DesktopAction::ExportData);
    {
        let mobile = app.world().resource::<MobileClientState>();
        let export = mobile
            .last_export
            .as_ref()
            .expect("PortablePatientExport populated after ExportData");
        assert_eq!(export.health_samples.len(), 4);
        assert_eq!(export.records.len(), 2);
        assert!(mobile.errors.is_empty(), "errors: {:?}", mobile.errors);
    }
    assert!(
        text_slot_value(&mut app, TextSlot::PrivacySummary)
            .contains(translate(Locale::En, UiKey::MobileExportReady))
    );

    // 3. Verify DOB -> Share -> DisconnectConnectedCare transitions standing to Ended (`Disconnected`).
    dispatch_and_settle(&mut app, DesktopAction::VerifyDob);
    dispatch_and_settle(&mut app, DesktopAction::ShareConnectedCare);
    assert!(
        app.world()
            .resource::<MobileClientState>()
            .last_connection_standing
            .as_ref()
            .is_some_and(|s| s.state == ConnectedCareState::Connected)
    );

    dispatch_and_settle(&mut app, DesktopAction::DisconnectConnectedCare);
    assert!(
        app.world()
            .resource::<MobileClientState>()
            .last_connection_standing
            .as_ref()
            .is_some_and(|s| s.state == ConnectedCareState::Ended)
    );
    assert!(
        text_slot_value(&mut app, TextSlot::ConnectedCareSummary)
            .contains(translate(Locale::En, UiKey::MobileSharingDisconnected))
    );

    // 4. Add unsent composer text -> DeleteAccount -> durable PrivacyReceipt + strict SignInHoldsNothing + 401 session revocation.
    let _ = app
        .world_mut()
        .resource_mut::<ComposerSurface>()
        .apply(ComposerAction::InsertText(
            "Draft cleared on permanent account deletion".to_owned(),
        ));
    app.update();

    dispatch_and_settle(&mut app, DesktopAction::DeleteAccount);
    let receipt_event_id = {
        let mobile = app.world().resource::<MobileClientState>();
        let receipt = mobile
            .last_delete_receipt
            .as_ref()
            .expect("AccountDeleteReceipt recorded after DeleteAccount");
        assert!(receipt.deleted);
        assert!(!receipt.receipt_event_id.is_empty());
        assert_eq!(
            mobile.auth_device.screen(),
            &MobileClientScreen::EnterYourEmail
        );
        assert!(mobile.visible_health_trends.is_empty());
        assert!(mobile.visible_records.is_empty());
        assert!(mobile.last_export.is_none());
        assert!(mobile.last_dob_outcome.is_none());
        assert!(mobile.last_connection_standing.is_none());
        assert!(mobile.sent_envelopes.is_empty());
        assert!(mobile.errors.is_empty(), "errors: {:?}", mobile.errors);
        receipt.receipt_event_id.clone()
    };
    assert!(
        app.world().resource::<ComposerSurface>().text().is_empty(),
        "SignInHoldsNothing must clear composer text on DeleteAccount"
    );
    assert!(
        text_slot_value(&mut app, TextSlot::StatusBanner).contains(&receipt_event_id),
        "status banner must display recorded PrivacyReceipt event ID"
    );
    assert!(
        text_slot_value(&mut app, TextSlot::PrivacySummary)
            .contains(translate(Locale::En, UiKey::MobileAccountDeleted))
    );
    assert_eq!(
        text_slot_value(&mut app, TextSlot::HealthTrendsBody),
        translate(Locale::En, UiKey::MobileHealthSignedOutNotice)
    );
    assert!(text_slot_value(&mut app, TextSlot::HealthRecordsBody).is_empty());

    // Confirm durable PrivacyReceipt was written to ef-mobile-server's FileReceiptJournal.
    let reopened_journal =
        FileReceiptJournal::open(&journal_dir).expect("reopen FileReceiptJournal");
    let receipts = reopened_journal
        .inventory()
        .expect("list persisted privacy receipts");
    assert!(
        receipts.iter().any(|r| r.event_id == receipt_event_id),
        "expected durable PrivacyReceipt {receipt_event_id} in journal, got {receipts:?}"
    );

    // Confirm old session is revoked on live ef-mobile-server (rejected with HTTP 401).
    let probe = LiveHttpMobileTransport::new(addr);
    let export_err = probe
        .export_patient_data(&active_session_id, 1_760_000_099)
        .expect_err("deleted account session must be rejected on /export");
    assert!(
        matches!(
            export_err,
            MobileTransportError::Http { status: 401, .. }
                | MobileTransportError::Auth(MobileAuthError::SessionEnded)
        ),
        "expected HTTP 401 / SessionEnded after DeleteAccount, got {export_err:?}"
    );
    let records_err = probe
        .fetch_records(&active_session_id, 1_760_000_099)
        .expect_err("deleted account session must be rejected on /records");
    assert!(
        matches!(
            records_err,
            MobileTransportError::Http { status: 401, .. }
                | MobileTransportError::Auth(MobileAuthError::SessionEnded)
        ),
        "expected HTTP 401 / SessionEnded on /records, got {records_err:?}"
    );

    // Zero duplicate AccessKit IDs with the 3 new privacy controls.
    assert!(
        app.world()
            .resource::<CatalogA11ySyncStatus>()
            .errors
            .is_empty()
    );
    let audit = app
        .world()
        .resource::<A11yProjection>()
        .verify_zero_duplicate_nodes()
        .expect("zero duplicate AccessKit nodes");
    assert_eq!(audit.duplicated_nodes, 0);

    server.shutdown();
    let _ = fs::remove_dir_all(&db_dir);
    let _ = fs::remove_dir_all(&journal_dir);
}

#[test]
fn mobile_privacy_controls_demo_mode_and_12_locale_labels() {
    for (tag, locale) in [("he", Locale::He), ("ar", Locale::Ar), ("hi", Locale::Hi)] {
        let cfg = MobileDesktopConfig::parse_args(["--demo", "--locale", tag, "--no-prefs"])
            .expect("localized demo cfg");
        let mut app = build_headless_mobile_desktop_app(&cfg);
        app.update();

        assert_eq!(
            text_slot_value(&mut app, TextSlot::ExportDataBtnLabel),
            translate(locale, UiKey::MobileExportData)
        );
        assert_eq!(
            text_slot_value(&mut app, TextSlot::DeleteAccountBtnLabel),
            translate(locale, UiKey::MobileDeleteAccount)
        );
        assert_eq!(
            text_slot_value(&mut app, TextSlot::DisconnectCareBtnLabel),
            translate(locale, UiKey::MobileDisconnectCare)
        );

        // Exercise demo export + disconnect + delete flow in target locale.
        dispatch_and_settle(&mut app, DesktopAction::SignIn);
        dispatch_and_settle(&mut app, DesktopAction::SyncDemoHealthSnapshot);
        dispatch_and_settle(&mut app, DesktopAction::ExportData);
        assert!(
            text_slot_value(&mut app, TextSlot::PrivacySummary)
                .contains(translate(locale, UiKey::MobileExportReady))
        );

        dispatch_and_settle(&mut app, DesktopAction::VerifyDob);
        dispatch_and_settle(&mut app, DesktopAction::ShareConnectedCare);
        dispatch_and_settle(&mut app, DesktopAction::DisconnectConnectedCare);
        assert!(
            app.world()
                .resource::<MobileClientState>()
                .last_connection_standing
                .as_ref()
                .is_some_and(|s| s.state == ConnectedCareState::Ended)
        );

        dispatch_and_settle(&mut app, DesktopAction::DeleteAccount);
        assert_eq!(
            app.world()
                .resource::<MobileClientState>()
                .auth_device
                .screen(),
            &MobileClientScreen::EnterYourEmail
        );
        assert!(
            text_slot_value(&mut app, TextSlot::PrivacySummary)
                .contains(translate(locale, UiKey::MobileAccountDeleted))
        );
        let audit = app
            .world()
            .resource::<A11yProjection>()
            .verify_zero_duplicate_nodes()
            .expect("zero duplicate AccessKit nodes");
        assert_eq!(audit.duplicated_nodes, 0);
    }
}
