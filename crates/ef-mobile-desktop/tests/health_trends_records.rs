#![cfg(not(target_arch = "wasm32"))]
//! Integration tests for Curie Mobile Desktop Personal Health Trends & Records,
//! `--signed-in-demo` startup preview, `SyncDemoHealthSnapshot`, and `SignInHoldsNothing`.

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use ef_mobile_client::MobileClientState;
use ef_mobile_desktop::{
    ActionButton, ConfigError, DesktopAction, DesktopActionQueue, HEALTH_TRENDS_HEADING,
    HealthTrendsCardNode, LiveServerWorker, MobileDesktopConfig, PersonalHealthPanelView,
    SIGNED_IN_HEALTH_EMPTY_NOTICE, SIGNED_OUT_HEALTH_NOTICE, SYNC_DEMO_HEALTH_LABEL, TextSlot,
    build_headless_mobile_desktop_app, run_headless_smoke,
};
use ef_mobile_domain::{FileReceiptJournal, MobileClientScreen};
use ef_mobile_server::{MobileService, TcpMobileServer};
use ef_ui_catalog::{A11yProjection, CatalogA11ySyncStatus, Locale, UiKey, translate};

fn fresh_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "ef-mobile-health-e2e-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn dispatch_demo(app: &mut App, action: DesktopAction) {
    app.world_mut()
        .resource_mut::<DesktopActionQueue>()
        .push(action);
    app.update();
    app.update();
}

fn wait_for_live_worker_idle(app: &mut App, timeout: Duration) {
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
            "timed out waiting for LiveServerWorker"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn dispatch_live(app: &mut App, action: DesktopAction) {
    app.world_mut()
        .resource_mut::<DesktopActionQueue>()
        .push(action);
    app.update();
    wait_for_live_worker_idle(app, Duration::from_secs(5));
}

fn text_slot(app: &mut App, target: TextSlot) -> String {
    let mut q = app.world_mut().query::<(&TextSlot, &Text)>();
    for (slot, text) in q.iter(app.world()) {
        if *slot == target {
            return text.0.clone();
        }
    }
    panic!("missing TextSlot::{target:?}");
}

#[test]
fn signed_in_demo_cli_flag_parses_validates_and_passes_headless_smoke() {
    let default_cfg = MobileDesktopConfig::parse_args::<_, &str>([]).expect("default cfg");
    assert!(!default_cfg.signed_in_demo);

    let signed_in_cfg =
        MobileDesktopConfig::parse_args(["--demo", "--signed-in-demo", "--headless-smoke"])
            .expect("signed-in-demo cfg");
    assert!(signed_in_cfg.signed_in_demo);

    assert!(matches!(
        MobileDesktopConfig::parse_args(["--owed-demo", "--signed-in-demo"]),
        Err(ConfigError::ConflictingDemoPreviews)
    ));

    let report = run_headless_smoke(&signed_in_cfg).expect("--signed-in-demo smoke must pass");
    assert_eq!(report.app, "ef-mobile-desktop");
    assert_eq!(report.a11y_duplicate_nodes, 0);
    assert!(report.a11y_projected_nodes >= 16);
}

#[test]
fn health_trends_records_render_update_and_wipe_on_sign_out_online_and_offline() {
    let cfg = MobileDesktopConfig::parse_args(["--demo", "--no-prefs"]).expect("demo cfg");
    let mut app = build_headless_mobile_desktop_app(&cfg);
    app.update();

    // Verify Health Trends card and Sync Demo Health Snapshot button exist in UI tree.
    let card_count = app
        .world_mut()
        .query_filtered::<Entity, With<HealthTrendsCardNode>>()
        .iter(app.world())
        .count();
    assert_eq!(card_count, 1);

    let sync_btn_count = app
        .world_mut()
        .query::<&ActionButton>()
        .iter(app.world())
        .filter(|b| b.action == DesktopAction::SyncDemoHealthSnapshot)
        .count();
    assert_eq!(sync_btn_count, 1);
    assert_eq!(
        text_slot(&mut app, TextSlot::HealthTrendsHeading),
        HEALTH_TRENDS_HEADING
    );
    assert_eq!(
        text_slot(&mut app, TextSlot::SyncHealthBtnLabel),
        SYNC_DEMO_HEALTH_LABEL
    );

    // 1. Signed out: SignInHoldsNothing shows privacy notice and empty records; sync is ignored.
    assert_eq!(
        PersonalHealthPanelView::from_client_state(app.world().resource::<MobileClientState>()),
        PersonalHealthPanelView::SignedOut
    );
    assert_eq!(
        text_slot(&mut app, TextSlot::HealthTrendsBody),
        SIGNED_OUT_HEALTH_NOTICE
    );
    assert!(text_slot(&mut app, TextSlot::HealthRecordsBody).is_empty());

    dispatch_demo(&mut app, DesktopAction::SyncDemoHealthSnapshot);
    assert!(
        app.world()
            .resource::<MobileClientState>()
            .visible_health_trends
            .is_empty()
    );
    assert!(
        app.world()
            .resource::<MobileClientState>()
            .visible_records
            .is_empty()
    );

    // 2. Sign in -> shows signed-in empty prompt prior to first sync.
    dispatch_demo(&mut app, DesktopAction::SignIn);
    assert_eq!(
        text_slot(&mut app, TextSlot::HealthTrendsBody),
        SIGNED_IN_HEALTH_EMPTY_NOTICE
    );

    // 3. First SyncDemoHealthSnapshot -> populates all 4 metrics + 2 personal records.
    dispatch_demo(&mut app, DesktopAction::SyncDemoHealthSnapshot);
    {
        let mobile = app.world().resource::<MobileClientState>();
        assert_eq!(mobile.visible_health_trends.len(), 4);
        assert_eq!(mobile.visible_records.len(), 2);
        assert!(mobile.errors.is_empty());
    }
    let trends_v1 = text_slot(&mut app, TextSlot::HealthTrendsBody);
    assert!(trends_v1.contains("Daily Steps: 8240 steps"));
    assert!(trends_v1.contains("Resting Heart Rate: 62 bpm"));
    assert!(trends_v1.contains("Blood Pressure: 118 mmHg systolic"));
    assert!(trends_v1.contains("Sleep Duration: 7.4 hours"));

    let records_v1 = text_slot(&mut app, TextSlot::HealthRecordsBody);
    assert!(records_v1.contains("Personal Records (2):"));
    assert!(records_v1.contains("HbA1c Lab Panel"));
    assert!(records_v1.contains("Lisinopril 10 mg Daily"));

    // 4. Second SyncDemoHealthSnapshot -> visibly updates metrics and adds 3rd record.
    dispatch_demo(&mut app, DesktopAction::SyncDemoHealthSnapshot);
    let trends_v2 = text_slot(&mut app, TextSlot::HealthTrendsBody);
    assert!(trends_v2.contains("Daily Steps: 8590 steps"));
    assert!(trends_v2.contains("Resting Heart Rate: 61 bpm"));
    assert!(trends_v2.contains("Blood Pressure: 119 mmHg systolic"));
    assert!(trends_v2.contains("Sleep Duration: 7.5 hours"));
    assert_ne!(trends_v1, trends_v2);

    let records_v2 = text_slot(&mut app, TextSlot::HealthRecordsBody);
    assert!(records_v2.contains("Personal Records (3):"));
    assert!(records_v2.contains("Primary Care Follow-Up"));

    // 5. Online Sign-Out -> SignInHoldsNothing immediately wipes state AND rendered Text!
    dispatch_demo(&mut app, DesktopAction::OfflineSignOut);
    {
        let mobile = app.world().resource::<MobileClientState>();
        assert_eq!(
            mobile.auth_device.screen(),
            &MobileClientScreen::EnterYourEmail
        );
        assert!(mobile.visible_health_trends.is_empty());
        assert!(mobile.visible_records.is_empty());
    }
    assert_eq!(
        text_slot(&mut app, TextSlot::HealthTrendsBody),
        SIGNED_OUT_HEALTH_NOTICE
    );
    assert!(text_slot(&mut app, TextSlot::HealthRecordsBody).is_empty());

    // 6. Sign back in, sync, go OFFLINE, and sign out -> SignInHoldsNothing still wipes everything!
    dispatch_demo(&mut app, DesktopAction::SignIn);
    dispatch_demo(&mut app, DesktopAction::SyncDemoHealthSnapshot);
    assert!(
        !app.world()
            .resource::<MobileClientState>()
            .visible_health_trends
            .is_empty()
    );
    dispatch_demo(&mut app, DesktopAction::ToggleConnectivity);
    dispatch_demo(&mut app, DesktopAction::OfflineSignOut);
    {
        let mobile = app.world().resource::<MobileClientState>();
        assert!(mobile.visible_health_trends.is_empty());
        assert!(mobile.visible_records.is_empty());
        assert_eq!(mobile.auth_device.owed_sign_outs().len(), 1);
    }
    assert_eq!(
        text_slot(&mut app, TextSlot::HealthTrendsBody),
        SIGNED_OUT_HEALTH_NOTICE
    );
    assert!(text_slot(&mut app, TextSlot::HealthRecordsBody).is_empty());

    // 7. Zero AccessKit sync errors or duplicate IDs.
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
}

#[test]
fn signed_in_demo_launch_prepopulates_health_trends_and_records_deterministically() {
    let cfg =
        MobileDesktopConfig::parse_args(["--demo", "--signed-in-demo", "--no-prefs"]).expect("cfg");
    let mut app = build_headless_mobile_desktop_app(&cfg);
    app.update();
    app.update();

    let mobile = app.world().resource::<MobileClientState>();
    assert!(matches!(
        mobile.auth_device.screen(),
        MobileClientScreen::SignedIn { .. }
    ));
    assert_eq!(mobile.visible_health_trends.len(), 4);
    assert_eq!(mobile.visible_records.len(), 2);

    let trends = text_slot(&mut app, TextSlot::HealthTrendsBody);
    assert!(trends.contains("Daily Steps: 8240 steps"));
    assert!(trends.contains("Resting Heart Rate: 62 bpm"));
    assert!(trends.contains("Blood Pressure: 118 mmHg systolic"));
    assert!(trends.contains("Sleep Duration: 7.4 hours"));

    let records = text_slot(&mut app, TextSlot::HealthRecordsBody);
    assert!(records.contains("HbA1c Lab Panel"));
    assert!(records.contains("Lisinopril 10 mg Daily"));

    let audit = app
        .world()
        .resource::<A11yProjection>()
        .verify_zero_duplicate_nodes()
        .expect("zero duplicate AccessKit nodes");
    assert_eq!(audit.duplicated_nodes, 0);
}

#[test]
fn live_server_sync_health_snapshot_and_sign_out_wipe() {
    let db_dir = fresh_temp_dir("live-health-db");
    let journal_dir = fresh_temp_dir("live-health-journal");

    let journal = FileReceiptJournal::open(&journal_dir).expect("open receipt journal");
    let svc = MobileService::open_durable(&db_dir, Box::new(journal), 1_760_000_000, false)
        .expect("open durable mobile service");
    let server = TcpMobileServer::bind("127.0.0.1:0", svc).expect("bind TcpMobileServer");
    let addr = server.local_addr();

    let cfg = MobileDesktopConfig::parse_args([
        "--server",
        &format!("http://{addr}"),
        "--email",
        "w18.health.live@example.invalid",
        "--no-prefs",
    ])
    .expect("live cfg");
    let mut app = build_headless_mobile_desktop_app(&cfg);
    app.update();

    dispatch_live(&mut app, DesktopAction::SignIn);
    dispatch_live(&mut app, DesktopAction::SyncDemoHealthSnapshot);
    {
        let mobile = app.world().resource::<MobileClientState>();
        assert_eq!(mobile.visible_health_trends.len(), 4);
        assert_eq!(mobile.visible_records.len(), 2);
        assert!(mobile.errors.is_empty(), "errors: {:?}", mobile.errors);
    }
    assert!(text_slot(&mut app, TextSlot::HealthTrendsBody).contains("Daily Steps: 8240 steps"));
    assert!(text_slot(&mut app, TextSlot::HealthRecordsBody).contains("HbA1c Lab Panel"));

    // Sign out live -> wipes all visible health trends & records immediately.
    dispatch_live(&mut app, DesktopAction::OfflineSignOut);
    {
        let mobile = app.world().resource::<MobileClientState>();
        assert!(mobile.visible_health_trends.is_empty());
        assert!(mobile.visible_records.is_empty());
        assert!(mobile.errors.is_empty(), "errors: {:?}", mobile.errors);
    }
    assert_eq!(
        text_slot(&mut app, TextSlot::HealthTrendsBody),
        SIGNED_OUT_HEALTH_NOTICE
    );
    assert!(text_slot(&mut app, TextSlot::HealthRecordsBody).is_empty());

    server.shutdown();
    let _ = fs::remove_dir_all(&db_dir);
    let _ = fs::remove_dir_all(&journal_dir);
}

#[test]
fn mobile_recovery_and_health_chrome_fully_localized_for_he_ar_and_hi() {
    let en_flush = translate(Locale::En, UiKey::MobileFlushOwedSignOuts);
    let en_ready = translate(Locale::En, UiKey::MobileReadyToFlush);
    let en_reconnect = translate(Locale::En, UiKey::MobileReconnectToFlush);
    let en_heading = translate(Locale::En, UiKey::MobileSectionHealthTrends);
    let en_sync = translate(Locale::En, UiKey::MobileActionSyncHealth);
    let en_notice = translate(Locale::En, UiKey::MobileHealthSignedOutNotice);

    for (tag, locale) in [("he", Locale::He), ("ar", Locale::Ar), ("hi", Locale::Hi)] {
        let cfg = MobileDesktopConfig::parse_args(["--demo", "--locale", tag, "--no-prefs"])
            .expect("localized cfg");
        let mut app = build_headless_mobile_desktop_app(&cfg);
        app.update();

        // 1. Verify all 3 Health Trends card keys + Flush button label render in target locale (zero English).
        let heading = text_slot(&mut app, TextSlot::HealthTrendsHeading);
        assert_eq!(heading, translate(locale, UiKey::MobileSectionHealthTrends));
        assert_ne!(heading, en_heading);

        let sync_label = text_slot(&mut app, TextSlot::SyncHealthBtnLabel);
        assert_eq!(sync_label, translate(locale, UiKey::MobileActionSyncHealth));
        assert_ne!(sync_label, en_sync);

        let notice = text_slot(&mut app, TextSlot::HealthTrendsBody);
        assert_eq!(
            notice,
            translate(locale, UiKey::MobileHealthSignedOutNotice)
        );
        assert_ne!(notice, en_notice);

        let flush_idle = text_slot(&mut app, TextSlot::FlushOwedBtnLabel);
        assert_eq!(
            flush_idle,
            translate(locale, UiKey::MobileFlushOwedSignOuts)
        );
        assert_ne!(flush_idle, en_flush);

        // 2. Sign in -> Go offline -> Sign out -> verify localized MobileReconnectToFlush banner (zero English).
        dispatch_demo(&mut app, DesktopAction::SignIn);
        dispatch_demo(&mut app, DesktopAction::ToggleConnectivity);
        dispatch_demo(&mut app, DesktopAction::OfflineSignOut);

        let offline_banner = text_slot(&mut app, TextSlot::OwedRecoveryBanner);
        let expected_reconnect = translate(locale, UiKey::MobileReconnectToFlush);
        assert!(
            offline_banner.contains(expected_reconnect),
            "locale {tag} offline banner missing MobileReconnectToFlush: {offline_banner}"
        );
        assert!(
            !offline_banner.contains(en_reconnect)
                && !offline_banner.contains("reconnect to flush"),
            "locale {tag} leaked English reconnect text: {offline_banner}"
        );

        // 3. Reconnect online -> verify localized MobileReadyToFlush banner & MobileFlushOwedSignOuts (1) button.
        dispatch_demo(&mut app, DesktopAction::ToggleConnectivity);

        let ready_banner = text_slot(&mut app, TextSlot::OwedRecoveryBanner);
        let expected_ready = translate(locale, UiKey::MobileReadyToFlush);
        assert!(
            ready_banner.contains(expected_ready),
            "locale {tag} ready banner missing MobileReadyToFlush: {ready_banner}"
        );
        assert!(
            !ready_banner.contains(en_ready) && !ready_banner.contains("ready to flush"),
            "locale {tag} leaked English ready text: {ready_banner}"
        );

        let flush_pending = text_slot(&mut app, TextSlot::FlushOwedBtnLabel);
        assert_eq!(
            flush_pending,
            format!("{} (1)", translate(locale, UiKey::MobileFlushOwedSignOuts))
        );
        assert!(!flush_pending.contains(en_flush));

        // 4. Flush -> returns cleanly to unbadged localized button label with 0 duplicate AccessKit nodes.
        dispatch_demo(&mut app, DesktopAction::FlushOwedSignOuts);
        assert_eq!(
            text_slot(&mut app, TextSlot::FlushOwedBtnLabel),
            translate(locale, UiKey::MobileFlushOwedSignOuts)
        );
        let audit = app
            .world()
            .resource::<A11yProjection>()
            .verify_zero_duplicate_nodes()
            .expect("zero duplicate AccessKit nodes");
        assert_eq!(audit.duplicated_nodes, 0);
    }
}
