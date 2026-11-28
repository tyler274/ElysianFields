#![cfg(not(target_arch = "wasm32"))]
//! Unit and live `TcpMobileServer` integration tests for `ef-mobile-desktop`.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ef_connected_care::{
    ConnectedCareState, CreateInvitationRequest, DateOfBirth, DobVerificationOutcome,
    InviteCredential, PairKey,
};
use ef_core::{PatientId, PhysicianId, PracticeId};
use ef_mobile_client::MobileClientState;
use ef_mobile_desktop::{
    ConfigError, DesktopAction, DesktopActionQueue, LaunchMode, MobileDesktopConfig,
    MobileDesktopUiState, PreferenceMode, RunTarget, TextSlot, build_headless_mobile_desktop_app,
    run_headless_smoke,
};
use ef_mobile_domain::FileReceiptJournal;
use ef_mobile_server::{MobileService, TcpMobileServer};
use ef_ui_catalog::{
    ComposerAction, ComposerSurface, DesktopPreferences, DesktopPrefsStore, DesktopZoom, Locale,
    ThemeMode, UiKey, translate,
};

fn fresh_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "ef-mobile-desktop-test-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

#[test]
fn cli_config_parses_demo_server_locale_theme_smoke_and_rejects_invalid() {
    let default_cfg = MobileDesktopConfig::parse_args::<_, &str>([]).expect("default config");
    assert_eq!(default_cfg.mode, LaunchMode::Demo);
    assert_eq!(default_cfg.locale, Locale::En);
    assert_eq!(default_cfg.theme, ThemeMode::Dark);
    assert_eq!(
        default_cfg.target,
        RunTarget::Windowed {
            screenshot_path: None
        }
    );

    let smoke_cfg = MobileDesktopConfig::parse_args([
        "--demo",
        "--locale",
        "he",
        "--theme",
        "light",
        "--headless-smoke",
    ])
    .expect("smoke config");
    assert_eq!(smoke_cfg.mode, LaunchMode::Demo);
    assert_eq!(smoke_cfg.locale, Locale::He);
    assert_eq!(smoke_cfg.theme, ThemeMode::Light);
    assert_eq!(smoke_cfg.target, RunTarget::HeadlessSmoke);

    let live_cfg = MobileDesktopConfig::parse_args([
        "--server",
        "http://127.0.0.1:8089",
        "--email",
        "patient@example.invalid",
        "--screenshot",
        "/tmp/mobile.png",
    ])
    .expect("live config");
    assert!(matches!(live_cfg.mode, LaunchMode::LiveServer { .. }));
    assert_eq!(live_cfg.email, "patient@example.invalid");
    assert_eq!(
        live_cfg.target,
        RunTarget::Windowed {
            screenshot_path: Some(PathBuf::from("/tmp/mobile.png"))
        }
    );

    assert!(matches!(
        MobileDesktopConfig::parse_args(["--demo", "--server", "127.0.0.1:8089"]),
        Err(ConfigError::ConflictingLaunchModes)
    ));
    assert!(matches!(
        MobileDesktopConfig::parse_args(["--locale", "xx-ZZ"]),
        Err(ConfigError::InvalidLocale { .. })
    ));
    assert!(matches!(
        MobileDesktopConfig::parse_args(["--theme", "neon"]),
        Err(ConfigError::InvalidTheme { .. })
    ));
}

#[test]
fn headless_smoke_verifies_12_locales_rtl_themes_crisis_offline_signout_and_zero_a11y_dups() {
    let config = MobileDesktopConfig::parse_args(["--demo", "--headless-smoke"]).expect("config");
    let report = run_headless_smoke(&config).expect("headless smoke must pass");
    assert_eq!(report.app, "ef-mobile-desktop");
    assert_eq!(report.locales_verified.len(), 12);
    assert!(report.rtl_mirrored_locales.contains(&"he".to_owned()));
    assert!(report.rtl_mirrored_locales.contains(&"ar".to_owned()));
    assert_eq!(report.themes_verified, vec!["Dark", "Light"]);
    assert_eq!(report.crisis_banner_count, 1);
    assert_eq!(report.offline_owed_sign_outs_observed, 1);
    assert_eq!(report.owed_sign_outs_after_flush, 0);
    assert_eq!(report.connected_care_status, "Active");
    assert_eq!(report.sent_doctor_envelopes, 1);
    assert!(report.a11y_projected_nodes >= 10);
    assert_eq!(report.a11y_duplicate_nodes, 0);
}

#[test]
fn live_tcp_mobile_server_integration_end_to_end() {
    let db_dir = fresh_temp_dir("db");
    let journal_dir = fresh_temp_dir("journal");

    let journal = FileReceiptJournal::open(&journal_dir).expect("open receipt journal");
    let mut svc = MobileService::open_durable(&db_dir, Box::new(journal), 1_760_000_000, false)
        .expect("open durable mobile service");

    let expected_dob = DateOfBirth::new(1988, 6, 14).expect("valid dob");
    let pair = PairKey::new(
        PracticeId::new("practice-desktop-live-1").expect("practice id"),
        PatientId::new("patient-desktop-live-1").expect("patient id"),
    );
    let issued = svc
        .connected_care
        .create_invitation(CreateInvitationRequest {
            pair,
            issued_by: PhysicianId::new("phys-desktop-live-1").expect("phys id"),
            practice_name: "Live Desktop Care Clinic".to_owned(),
            physician_name: "Dr. Live Desktop".to_owned(),
            expected_dob,
            created_at: "2026-01-12T09:00:00Z".to_owned(),
        })
        .expect("issue connected care invite");

    let server = TcpMobileServer::bind("127.0.0.1:0", svc).expect("bind TcpMobileServer");
    let addr = server.local_addr();

    let cfg = MobileDesktopConfig::parse_args([
        "--server",
        &format!("http://{addr}"),
        "--email",
        "desktop.live.patient@example.invalid",
        "--locale",
        "ar",
        "--theme",
        "light",
    ])
    .expect("live server config");

    let mut app = build_headless_mobile_desktop_app(&cfg);
    app.world_mut()
        .resource_mut::<MobileDesktopUiState>()
        .set_invite_credential(
            InviteCredential::ShortCode(issued.short_code.clone()),
            issued.short_code.as_str().to_owned(),
            expected_dob,
        );
    app.update();

    fn dispatch(app: &mut bevy::prelude::App, action: DesktopAction) {
        app.world_mut()
            .resource_mut::<DesktopActionQueue>()
            .push(action);
        app.update();
        app.update();
    }

    // Sign in live over TCP HTTP/1.1 -> verify DOB -> explicit Share -> send For My Doctor note.
    dispatch(&mut app, DesktopAction::SignIn);
    dispatch(&mut app, DesktopAction::VerifyDob);
    assert!(matches!(
        app.world().resource::<MobileClientState>().last_dob_outcome,
        Some(DobVerificationOutcome::Verified { .. })
    ));
    dispatch(&mut app, DesktopAction::ShareConnectedCare);
    assert!(
        app.world()
            .resource::<MobileClientState>()
            .last_connection_standing
            .as_ref()
            .is_some_and(|s| {
                s.state == ConnectedCareState::Connected && s.active_connection_id.is_some()
            })
    );

    let _ = app
        .world_mut()
        .resource_mut::<ComposerSurface>()
        .apply(ComposerAction::InsertText(
            "فحصُ السكّري HbA1c 6.8% — ضغط الدم BP 122/78".to_owned(),
        ));
    dispatch(&mut app, DesktopAction::ReviewAndSendDoctorNote);
    assert_eq!(
        app.world()
            .resource::<MobileClientState>()
            .sent_envelopes
            .len(),
        1
    );
    assert!(
        app.world()
            .resource::<MobileClientState>()
            .errors
            .is_empty(),
        "unexpected mobile client errors: {:?}",
        app.world().resource::<MobileClientState>().errors
    );

    server.shutdown();
    let _ = fs::remove_dir_all(&db_dir);
    let _ = fs::remove_dir_all(&journal_dir);
}

#[test]
fn mobile_cli_prefs_bypass_and_12_locale_chrome_and_persistence() {
    let default_cfg = MobileDesktopConfig::parse_args::<_, &str>([]).expect("default");
    assert!(default_cfg.should_use_prefs());
    assert_eq!(default_cfg.prefs_mode, PreferenceMode::PersistOsConfig);

    let no_prefs = MobileDesktopConfig::parse_args(["--no-prefs"]).expect("no-prefs");
    assert!(!no_prefs.should_use_prefs());

    let smoke = MobileDesktopConfig::parse_args(["--headless-smoke"]).expect("smoke");
    assert!(!smoke.should_use_prefs());

    let shot = MobileDesktopConfig::parse_args(["--screenshot", "/tmp/m.png"]).expect("shot");
    assert!(!shot.should_use_prefs());

    // CLI override precedence over saved preferences.
    let mut cfg = MobileDesktopConfig::parse_args(["--theme", "light"]).expect("theme override");
    cfg.apply_saved_preferences(DesktopPreferences {
        theme: ThemeMode::Dark,
        locale: Locale::Vi,
        zoom: DesktopZoom::Pct100,
    });
    assert_eq!(cfg.theme, ThemeMode::Light);
    assert_eq!(cfg.locale, Locale::Vi);

    // Temp-dir prefs persistence + 100% localized Hebrew & Hindi UI slots (zero English chrome).
    let prefs_dir = fresh_temp_dir("prefs");
    let store = DesktopPrefsStore::at_path(prefs_dir.join("curie-mobile-prefs.json"));
    let app_cfg =
        MobileDesktopConfig::parse_args(["--demo", "--locale", "he", "--no-prefs"]).expect("cfg");
    let mut app = build_headless_mobile_desktop_app(&app_cfg);
    app.insert_resource(store.clone());
    app.update();

    let mut q = app.world_mut().query::<(&TextSlot, &bevy::prelude::Text)>();
    let mut crisis_body = String::new();
    let mut conn_btn = String::new();
    let mut composer_label = String::new();
    let mut composer_placeholder = String::new();
    let mut sent_summary = String::new();
    let mut account_summary = String::new();
    let mut cc_summary = String::new();
    let mut status_banner = String::new();
    for (slot, text) in q.iter(app.world()) {
        match *slot {
            TextSlot::CrisisBody => crisis_body = text.0.clone(),
            TextSlot::ConnectivityBtnLabel => conn_btn = text.0.clone(),
            TextSlot::ComposerLabel => composer_label = text.0.clone(),
            TextSlot::ComposerContent => composer_placeholder = text.0.clone(),
            TextSlot::SentSummary => sent_summary = text.0.clone(),
            TextSlot::AccountSummary => account_summary = text.0.clone(),
            TextSlot::ConnectedCareSummary => cc_summary = text.0.clone(),
            TextSlot::StatusBanner => status_banner = text.0.clone(),
            _ => {}
        }
    }
    assert!(crisis_body.contains(translate(Locale::He, UiKey::MobileCrisisLine988)));
    assert!(crisis_body.contains("988"));
    assert!(crisis_body.contains("911"));
    assert!(crisis_body.contains("1-800-662-4357"));
    assert_eq!(
        conn_btn,
        translate(Locale::He, UiKey::MobileActionGoOffline)
    );
    assert_eq!(
        composer_label,
        translate(Locale::He, UiKey::MobileCuratedNoteLabel)
    );
    assert_eq!(
        composer_placeholder,
        translate(Locale::He, UiKey::MobileCuratedNotePlaceholder)
    );
    assert!(sent_summary.contains(translate(Locale::He, UiKey::MobileLabelSentUpdates)));
    assert!(account_summary.contains(translate(Locale::He, UiKey::MobileLabelOwedSignOuts)));
    assert!(cc_summary.contains(translate(Locale::He, UiKey::MobileDobNotVerified)));
    assert!(cc_summary.contains(translate(Locale::He, UiKey::MobileSharingNotShared)));
    assert!(status_banner.contains(translate(Locale::He, UiKey::MobileModeDemo)));
    assert!(status_banner.contains(translate(Locale::He, UiKey::MobileStatusReadyDemo)));

    // Switch to Hindi + Light and verify temp-dir DesktopPrefsStore saves both.
    app.world_mut()
        .resource_mut::<DesktopActionQueue>()
        .push(DesktopAction::SetLocale(Locale::Hi));
    app.world_mut()
        .resource_mut::<DesktopActionQueue>()
        .push(DesktopAction::SetTheme(ThemeMode::Light));
    app.update();
    app.update();

    let saved = store
        .load()
        .expect("load mobile prefs")
        .expect("saved mobile prefs");
    assert_eq!(saved.locale, Locale::Hi);
    assert_eq!(saved.theme, ThemeMode::Light);

    let _ = fs::remove_dir_all(&prefs_dir);
}
