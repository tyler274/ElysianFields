//! Unit and live TCP server integration tests for `ef-clinic-desktop`.

use ef_clinic_desktop::{
    DesktopConfigError, DesktopLaunchConfig, ExecutionMode, LaunchBackend, SmokeCheckOutcome,
    ZoomLevel, parse_server_socket_addr, run_headless_smoke,
};
use ef_ui_catalog::{Locale, ThemeMode};

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
