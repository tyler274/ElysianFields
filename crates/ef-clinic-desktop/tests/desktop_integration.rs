//! Unit and live TCP server integration tests for `ef-clinic-desktop`.

use ef_clinic_desktop::{
    ClinicDesktopUiPlugin, DesktopConfigError, DesktopLaunchConfig, DesktopUiEvent, DesktopUiSlot,
    DesktopWorkspaceState, ExecutionMode, LaunchBackend, PreferenceMode, SmokeCheckOutcome,
    ZoomLevel, bootstrap_clinic_session, create_transport_for_backend, parse_server_socket_addr,
    run_headless_smoke,
};
use ef_ui_catalog::{
    DesktopPreferences, DesktopPrefsStore, DesktopZoom, Locale, ThemeMode, UiKey, translate,
};

#[test]
fn cli_defaults_to_demo_dark_en_100() {
    let cfg = DesktopLaunchConfig::parse_args(Vec::<String>::new()).expect("default args");
    assert_eq!(cfg.backend, LaunchBackend::Demo);
    assert_eq!(
        cfg.mode,
        ExecutionMode::Windowed {
            screenshot_path: None
        }
    );
    assert_eq!(cfg.locale, Locale::En);
    assert_eq!(cfg.theme, ThemeMode::Dark);
    assert_eq!(cfg.zoom, ZoomLevel::Pct100);
}

#[test]
fn cli_parses_all_supported_flags() {
    let cfg = DesktopLaunchConfig::parse_args([
        "--server",
        "http://127.0.0.1:8088",
        "--email",
        "clinician@example.invalid",
        "--token",
        "tok-123",
        "--locale",
        "he",
        "--theme",
        "light",
        "--zoom",
        "200",
        "--headless-smoke",
    ])
    .expect("valid full args");

    assert_eq!(cfg.locale, Locale::He);
    assert_eq!(cfg.theme, ThemeMode::Light);
    assert_eq!(cfg.zoom, ZoomLevel::Pct200);
    assert_eq!(cfg.mode, ExecutionMode::HeadlessSmoke);
    match cfg.backend {
        LaunchBackend::Server {
            base_url,
            socket_addr,
            email,
            token,
        } => {
            assert_eq!(base_url, "http://127.0.0.1:8088");
            assert_eq!(socket_addr.port(), 8088);
            assert_eq!(email.as_deref(), Some("clinician@example.invalid"));
            assert_eq!(token.as_deref(), Some("tok-123"));
        }
        LaunchBackend::Demo => panic!("expected server backend"),
    }
}

#[test]
fn cli_parses_screenshot_flag_and_rejects_conflicts() {
    let cfg = DesktopLaunchConfig::parse_args([
        "--demo",
        "--locale",
        "ar",
        "--theme",
        "dark",
        "--zoom",
        "150%",
        "--screenshot",
        "/tmp/clinic-shot.png",
    ])
    .expect("screenshot args");
    assert_eq!(cfg.locale, Locale::Ar);
    assert_eq!(cfg.zoom, ZoomLevel::Pct150);
    assert_eq!(
        cfg.mode,
        ExecutionMode::Windowed {
            screenshot_path: Some(std::path::PathBuf::from("/tmp/clinic-shot.png"))
        }
    );

    let conflict = DesktopLaunchConfig::parse_args(["--demo", "--server", "http://127.0.0.1:8088"]);
    assert_eq!(conflict, Err(DesktopConfigError::ConflictingBackendFlags));

    let bad_locale = DesktopLaunchConfig::parse_args(["--locale", "xx-Unknown"]);
    assert!(matches!(
        bad_locale,
        Err(DesktopConfigError::InvalidLocale { .. })
    ));

    let bad_theme = DesktopLaunchConfig::parse_args(["--theme", "neon"]);
    assert!(matches!(
        bad_theme,
        Err(DesktopConfigError::InvalidTheme { .. })
    ));

    let bad_zoom = DesktopLaunchConfig::parse_args(["--zoom", "175"]);
    assert!(matches!(
        bad_zoom,
        Err(DesktopConfigError::InvalidZoom { .. })
    ));

    let bad_flag = DesktopLaunchConfig::parse_args(["--unknown-flag"]);
    assert!(matches!(
        bad_flag,
        Err(DesktopConfigError::UnknownFlag { .. })
    ));

    let help = DesktopLaunchConfig::parse_args(["--help"]);
    assert!(matches!(help, Err(DesktopConfigError::HelpRequested(_))));

    let https_err = parse_server_socket_addr("https://127.0.0.1:8443");
    assert!(matches!(
        https_err,
        Err(DesktopConfigError::InvalidServerUrl { .. })
    ));
}

#[test]
fn headless_smoke_demo_verifies_all_wcag_rtl_privacy_a11y_invariants() {
    let cfg = DesktopLaunchConfig {
        backend: LaunchBackend::Demo,
        mode: ExecutionMode::HeadlessSmoke,
        locale: Locale::En,
        theme: ThemeMode::Dark,
        zoom: ZoomLevel::Pct100,
        ..DesktopLaunchConfig::default()
    };

    let report = run_headless_smoke(&cfg).expect("headless smoke must pass");
    assert!(report.passed);
    assert_eq!(report.app, "ef-clinic-desktop");
    assert!(report.dark_wcag_aa_pairs > 0);
    assert!(report.light_wcag_aa_pairs > 0);
    assert_eq!(report.theme_toggle, SmokeCheckOutcome::Passed);
    assert_eq!(report.zoom_200, SmokeCheckOutcome::Passed);
    assert_eq!(report.rtl_hebrew_row_reverse, SmokeCheckOutcome::Passed);
    assert_eq!(report.rtl_arabic_row_reverse, SmokeCheckOutcome::Passed);
    assert_eq!(report.locales_cycled, 7);
    assert_eq!(report.patients_count, 3);
    assert_eq!(report.prompts_submitted, 1);
    assert_eq!(report.privacy_cover, SmokeCheckOutcome::Passed);
    assert_eq!(report.shortcuts_overlay, SmokeCheckOutcome::Passed);
    assert!(report.a11y_nodes > 0);
    assert_eq!(report.duplicate_a11y_nodes, 0);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn headless_smoke_connects_to_live_tcp_clinic_server() {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use ef_clinic_domain::auth::{
        AccountStanding, EmailAddress, PhysicianAccount, PhysicianRole, SignInMode,
    };
    use ef_clinic_domain::chat::RetryBackoffMode;
    use ef_clinic_domain::workspace::{
        CitationOrigin, CitedSourcePayload, ReadablePart, classify_record_status,
    };
    use ef_clinic_server::{ClinicService, TcpClinicServer};
    use ef_core::{Locale as CoreLocale, PhysicianId, PracticeId, SourceKey, WorkspaceId};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let wal_dir = std::env::temp_dir().join(format!(
        "ef-clinic-desktop-live-tcp-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&wal_dir).expect("create temp wal dir");

    let now_secs = 1_760_000_000u64;
    let src_key = SourceKey::new("src-desktop-live-1").expect("src key");
    let mut service = ClinicService::open_durable(
        &wal_dir,
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        src_key.clone(),
        now_secs,
    )
    .expect("open durable clinic service");

    let practice = PracticeId::new("prac-desktop-live").expect("practice");
    let doc = PhysicianId::new("phys-desktop-live").expect("doc");
    let email = "desktop.clinician@example.invalid";
    service
        .seed_account(PhysicianAccount {
            physician_id: doc.clone(),
            practice_id: practice.clone(),
            email: EmailAddress::parse(email).expect("email"),
            display_name: "Dr. Live Desktop".to_owned(),
            role: PhysicianRole::Standard,
            standing: AccountStanding::Active,
            sessions_ended_before: None,
        })
        .expect("seed account");

    let ws1 = WorkspaceId::new("ws-live-avery").expect("ws1");
    let ws2 = WorkspaceId::new("ws-live-cohen").expect("ws2");
    service
        .seed_workspace(
            ws1.clone(),
            practice.clone(),
            "Avery, Jordan (Live)",
            vec![doc.clone()],
        )
        .expect("seed ws1");
    service
        .seed_workspace(ws2, practice, "Cohen, Avi (Live)", vec![doc])
        .expect("seed ws2");

    service
        .seed_source(CitedSourcePayload {
            source_key: src_key,
            workspace_id: ws1,
            number: 1,
            source_label: "Live Progress Note [1]".to_owned(),
            title: "Live Outpatient Follow-Up".to_owned(),
            record_type: "progress_note".to_owned(),
            recorded_at: "2026-10-18T12:00:00Z".to_owned(),
            record_status: "final".to_owned(),
            status_class: classify_record_status("final"),
            origin: CitationOrigin::SourceRecord,
            original_language: CoreLocale::En,
            quote: "HbA1c 6.8%, BP 122/78 mmHg over live loopback TCP.".to_owned(),
            body: "Synthetic live server chart record.".to_owned(),
            readable_parts: vec![ReadablePart {
                label: "Plan".to_owned(),
                text: "Continue Metformin and Lisinopril.".to_owned(),
                unreadable_notice: None,
            }],
            fields: Vec::new(),
            raw_content: None,
            external_url: None,
        })
        .expect("seed source");

    let server = TcpClinicServer::bind("127.0.0.1:0", service).expect("bind live TcpClinicServer");
    let addr = server.local_addr();
    let base_url = format!("http://{addr}");

    let cfg = DesktopLaunchConfig::parse_args([
        "--server",
        &base_url,
        "--email",
        email,
        "--locale",
        "he",
        "--theme",
        "dark",
        "--zoom",
        "100",
        "--headless-smoke",
    ])
    .expect("parse live server args");

    let report = run_headless_smoke(&cfg).expect("live TCP server headless smoke must pass");
    assert!(report.passed);
    assert!(report.connection_mode.contains(&base_url));
    assert_eq!(report.initial_locale, "he");
    assert_eq!(report.patients_count, 2);
    assert_eq!(report.prompts_submitted, 1);
    assert_eq!(report.rtl_hebrew_row_reverse, SmokeCheckOutcome::Passed);
    assert_eq!(report.rtl_arabic_row_reverse, SmokeCheckOutcome::Passed);
    assert_eq!(report.privacy_cover, SmokeCheckOutcome::Passed);
    assert_eq!(report.shortcuts_overlay, SmokeCheckOutcome::Passed);
    assert_eq!(report.duplicate_a11y_nodes, 0);

    drop(server);
    let _ = fs::remove_dir_all(&wal_dir);
}

#[test]
fn cli_prefs_auto_bypass_and_override_precedence() {
    let interactive = DesktopLaunchConfig::parse_args(Vec::<String>::new()).expect("default");
    assert!(interactive.should_use_prefs());
    assert_eq!(interactive.prefs_mode, PreferenceMode::PersistOsConfig);

    let no_prefs = DesktopLaunchConfig::parse_args(["--no-prefs"]).expect("no-prefs");
    assert!(!no_prefs.should_use_prefs());
    assert_eq!(no_prefs.prefs_mode, PreferenceMode::Disabled);

    let smoke = DesktopLaunchConfig::parse_args(["--headless-smoke"]).expect("smoke");
    assert!(!smoke.should_use_prefs());

    let shot = DesktopLaunchConfig::parse_args(["--screenshot", "/tmp/shot.png"]).expect("shot");
    assert!(!shot.should_use_prefs());

    // Saved prefs apply to non-overridden flags while explicit CLI flags win.
    let mut cfg = DesktopLaunchConfig::parse_args(["--locale", "ja"]).expect("locale override");
    cfg.apply_saved_preferences(DesktopPreferences {
        theme: ThemeMode::Light,
        locale: Locale::He,
        zoom: DesktopZoom::Pct200,
    });
    assert_eq!(cfg.locale, Locale::Ja);
    assert_eq!(cfg.theme, ThemeMode::Light);
    assert_eq!(cfg.zoom, ZoomLevel::Pct200);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn clinic_desktop_scrolling_localized_chrome_and_temp_dir_prefs_persistence() {
    use std::time::{SystemTime, UNIX_EPOCH};

    use bevy::asset::AssetPlugin;
    use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
    use bevy::prelude::*;
    use ef_clinic_client::{
        ClientBootConfig, CurieClinicClientPlugin, SharedBrowserSessionHub, SharedLocaleStorage,
    };
    use ef_core::TabId;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let temp_dir = std::env::temp_dir().join(format!(
        "ef-clinic-desktop-prefs-{}-{nanos}",
        std::process::id()
    ));
    let prefs_path = temp_dir.join("curie-clinic-prefs.json");
    let store = DesktopPrefsStore::at_path(prefs_path.clone());

    let (transport, conn_mode, email, token, patients, citation_catalog) =
        create_transport_for_backend(&LaunchBackend::Demo);
    let initial_citations = patients
        .first()
        .map(|p| citation_catalog.cards_for(&p.workspace_id))
        .unwrap_or_default();

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .insert_resource(store.clone())
        .insert_resource(transport)
        .insert_resource(citation_catalog)
        .insert_resource(DesktopWorkspaceState {
            connection_mode: conn_mode,
            patients,
            active_patient_index: 0,
            citations: initial_citations,
            selected_citation_index: 0,
            transcript: Vec::new(),
            status_message: String::new(),
            status_override: None,
            shortcuts_modal_open: false,
            zoom_percent: 100,
            frames_rendered_ui: 0,
            prompts_submitted: 0,
        })
        .add_plugins(CurieClinicClientPlugin::new(
            TabId::new("desktop-scroll-test").expect("tab id"),
            ClientBootConfig::from_location("/elysianfields/?lang=he").expect("boot"),
            SharedBrowserSessionHub::new(None),
            SharedLocaleStorage::default(),
        ))
        .add_plugins(ClinicDesktopUiPlugin::new(
            ThemeMode::Dark,
            ZoomLevel::Pct100,
            None,
        ));

    bootstrap_clinic_session(&mut app, email, token);

    // 1. Verify Hebrew chrome has zero hardcoded English chrome in zoom, badge, shortcuts, and status bar.
    let mut slot_texts = app.world_mut().query::<(&DesktopUiSlot, &Text)>();
    let mut zoom_text = String::new();
    let mut badge_text = String::new();
    let mut shortcuts_body = String::new();
    let mut status_text = String::new();
    for (slot, text) in slot_texts.iter(app.world()) {
        match *slot {
            DesktopUiSlot::ZoomButtonLabel => zoom_text = text.0.clone(),
            DesktopUiSlot::ConnectionBadge => badge_text = text.0.clone(),
            DesktopUiSlot::ShortcutsModalBody => shortcuts_body = text.0.clone(),
            DesktopUiSlot::StatusText => status_text = text.0.clone(),
            _ => {}
        }
    }
    assert!(zoom_text.contains(translate(Locale::He, UiKey::LabelZoom)));
    assert!(!zoom_text.contains("Zoom"));
    assert_eq!(badge_text, translate(Locale::He, UiKey::BadgeModeDemoLocal));
    assert_eq!(
        shortcuts_body,
        translate(Locale::He, UiKey::ClinicShortcutsBody)
    );
    assert!(status_text.contains(translate(Locale::He, UiKey::ClinicStatusReady)));

    // 2. Verify interactive mouse-wheel and event scrolling on Transcript and Evidence scroll areas.
    app.world_mut().write_message(MouseWheel {
        unit: MouseScrollUnit::Line,
        x: 0.0,
        y: -3.0,
        window: Entity::PLACEHOLDER,
        phase: bevy::input::touch::TouchPhase::Moved,
    });
    app.update();
    app.update();

    app.world_mut()
        .write_message(DesktopUiEvent::ScrollEvidence(120));
    app.update();
    app.update();

    let mut scroll_q = app.world_mut().query::<(&DesktopUiSlot, &ScrollPosition)>();
    let mut transcript_y = 0.0_f32;
    let mut evidence_y = 0.0_f32;
    for (slot, pos) in scroll_q.iter(app.world()) {
        match *slot {
            DesktopUiSlot::TranscriptScrollArea => transcript_y = pos.y,
            DesktopUiSlot::EvidenceScrollArea => evidence_y = pos.y,
            _ => {}
        }
    }
    assert!(
        transcript_y > 90.0,
        "expected mouse wheel scroll to advance transcript ScrollPosition.y, got {transcript_y}"
    );
    assert!(
        (evidence_y - 120.0).abs() < 0.01,
        "expected ScrollEvidence(120) to set evidence ScrollPosition.y to 120.0, got {evidence_y}"
    );

    // Clamping at 0.0 when scrolling upward past top.
    app.world_mut()
        .write_message(DesktopUiEvent::ScrollTranscript(-500));
    app.update();
    app.update();
    let mut scroll_q2 = app.world_mut().query::<(&DesktopUiSlot, &ScrollPosition)>();
    let clamped_y = scroll_q2
        .iter(app.world())
        .find(|(slot, _)| **slot == DesktopUiSlot::TranscriptScrollArea)
        .map_or(-1.0, |(_, pos)| pos.y);
    assert!(clamped_y.abs() < 0.01);

    // 3. Verify theme/locale/zoom changes persist to temp-dir DesktopPrefsStore.
    app.world_mut().write_message(DesktopUiEvent::ToggleTheme);
    app.world_mut()
        .write_message(DesktopUiEvent::SetLocale(Locale::Hi));
    app.world_mut()
        .write_message(DesktopUiEvent::SetZoom(ZoomLevel::Pct150));
    app.update();
    app.update();

    let saved = store
        .load()
        .expect("load persisted prefs")
        .expect("prefs written on interactive changes");
    assert_eq!(saved.theme, ThemeMode::Light);
    assert_eq!(saved.locale, Locale::Hi);
    assert_eq!(saved.zoom, DesktopZoom::Pct150);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
