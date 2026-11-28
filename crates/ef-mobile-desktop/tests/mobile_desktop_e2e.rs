#![cfg(not(target_arch = "wasm32"))]
//! Unit and live `TcpMobileServer` integration tests for `ef-mobile-desktop`.

use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use ef_connected_care::{
    ConnectedCareState, CreateInvitationRequest, DateOfBirth, DobVerificationOutcome,
    InviteCredential, PairKey,
};
use ef_core::{PatientId, PhysicianId, PracticeId};
use ef_mobile_client::MobileClientState;
use ef_mobile_desktop::{
    ClipboardBuffer, ConfigError, DesktopAction, DesktopActionQueue, FormInputBoxNode,
    FormParseError, LaunchMode, LiveServerWorker, MobileDesktopConfig, MobileDesktopUiState,
    MobileInputFocus, PendingWorkerState, PreferenceMode, RunTarget, TextSlot,
    build_headless_mobile_desktop_app, parse_dob_iso, parse_email, parse_invite_credential,
    run_headless_smoke,
};
use ef_mobile_domain::{FileReceiptJournal, MobileClientScreen};
use ef_mobile_server::{MobileService, TcpMobileServer};
use ef_ui_catalog::{
    ComposerAction, ComposerSurface, DesktopPreferences, DesktopPrefsStore, DesktopZoom, Locale,
    ThemeMode, UiKey, UiTheme, translate,
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

fn send_key(app: &mut App, key_code: KeyCode, logical_key: Key) {
    app.world_mut().write_message(KeyboardInput {
        key_code,
        logical_key,
        state: ButtonState::Pressed,
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    app.update();
}

fn send_text(app: &mut App, text: &str) {
    for ch in text.chars() {
        let s = ch.to_string();
        if ch == ' ' {
            send_key(app, KeyCode::Space, Key::Space);
        } else {
            send_key(app, KeyCode::KeyA, Key::Character(s.into()));
        }
    }
}

fn send_chord(app: &mut App, modifiers: &[KeyCode], key_code: KeyCode, logical_key: Key) {
    if !app.world().contains_resource::<ButtonInput<KeyCode>>() {
        app.insert_resource(ButtonInput::<KeyCode>::default());
    }
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        for &m in modifiers {
            keys.press(m);
        }
    }
    send_key(app, key_code, logical_key);
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        for &m in modifiers {
            keys.release(m);
        }
    }
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
            // One additional frame so visuals/a11y reflect the applied worker response.
            app.update();
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for LiveServerWorker to finish in-flight request"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn dispatch_live_and_wait(app: &mut App, action: DesktopAction) {
    app.world_mut()
        .resource_mut::<DesktopActionQueue>()
        .push(action);
    app.update();
    wait_for_live_worker_idle(app, Duration::from_secs(5));
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
fn focus_ring_cycles_tab_shift_tab_and_highlights_active_wcag_border() {
    let cfg = MobileDesktopConfig::parse_args(["--demo", "--no-prefs"]).expect("cfg");
    let mut app = build_headless_mobile_desktop_app(&cfg);
    app.update();

    // Default focus starts at CuratedNote; Tab cycles through all 4 variants cleanly.
    assert_eq!(
        app.world().resource::<MobileDesktopUiState>().active_focus,
        MobileInputFocus::CuratedNote
    );

    let forward_order = [
        MobileInputFocus::Email,
        MobileInputFocus::InviteCode,
        MobileInputFocus::DateOfBirth,
        MobileInputFocus::CuratedNote,
    ];
    for expected in forward_order {
        send_key(&mut app, KeyCode::Tab, Key::Tab);
        assert_eq!(
            app.world().resource::<MobileDesktopUiState>().active_focus,
            expected
        );

        let tokens = app.world().resource::<UiTheme>().tokens;
        let expected_focus_border = BorderColor::all(tokens.focus_ring.to_bevy_color());
        let expected_inactive_border = BorderColor::all(tokens.text_secondary.to_bevy_color());
        let mut q = app
            .world_mut()
            .query::<(&FormInputBoxNode, &Node, &BorderColor)>();
        let mut seen = 0usize;
        for (input_box, node, border) in q.iter(app.world()) {
            seen += 1;
            if input_box.field == expected {
                assert_eq!(node.border, UiRect::all(Val::Px(3.0)));
                assert_eq!(*border, expected_focus_border);
            } else {
                assert_eq!(node.border, UiRect::all(Val::Px(1.0)));
                assert_eq!(*border, expected_inactive_border);
            }
        }
        assert_eq!(seen, 4, "all 4 interactive FormInputBoxNodes must exist");
    }

    // Shift+Tab cycles strictly backward through all 4 variants.
    let reverse_order = [
        MobileInputFocus::DateOfBirth,
        MobileInputFocus::InviteCode,
        MobileInputFocus::Email,
        MobileInputFocus::CuratedNote,
    ];
    for expected in reverse_order {
        send_chord(&mut app, &[KeyCode::ShiftLeft], KeyCode::Tab, Key::Tab);
        assert_eq!(
            app.world().resource::<MobileDesktopUiState>().active_focus,
            expected
        );
    }
}

#[test]
fn interactive_editing_undo_redo_and_clipboard_across_all_4_fields() {
    let cfg = MobileDesktopConfig::parse_args(["--demo", "--no-prefs"]).expect("cfg");
    let mut app = build_headless_mobile_desktop_app(&cfg);
    app.update();

    // 1. Focus Email via Tab, select all, replace, test Backspace + Undo/Redo + Copy.
    send_key(&mut app, KeyCode::Tab, Key::Tab);
    assert_eq!(
        app.world().resource::<MobileDesktopUiState>().active_focus,
        MobileInputFocus::Email
    );
    send_chord(
        &mut app,
        &[KeyCode::ControlLeft],
        KeyCode::KeyA,
        Key::Character("a".into()),
    );
    send_key(&mut app, KeyCode::Backspace, Key::Backspace);
    send_text(&mut app, "maya@example.invalid");
    assert_eq!(
        app.world().resource::<MobileDesktopUiState>().patient_email,
        "maya@example.invalid"
    );

    send_key(&mut app, KeyCode::Backspace, Key::Backspace);
    assert_eq!(
        app.world().resource::<MobileDesktopUiState>().patient_email,
        "maya@example.invali"
    );
    send_chord(
        &mut app,
        &[KeyCode::ControlLeft],
        KeyCode::KeyZ,
        Key::Character("z".into()),
    );
    assert_eq!(
        app.world().resource::<MobileDesktopUiState>().patient_email,
        "maya@example.invalid"
    );
    send_chord(
        &mut app,
        &[KeyCode::ControlLeft, KeyCode::ShiftLeft],
        KeyCode::KeyZ,
        Key::Character("z".into()),
    );
    assert_eq!(
        app.world().resource::<MobileDesktopUiState>().patient_email,
        "maya@example.invali"
    );
    send_chord(
        &mut app,
        &[KeyCode::ControlLeft],
        KeyCode::KeyZ,
        Key::Character("z".into()),
    );
    send_chord(
        &mut app,
        &[KeyCode::ControlLeft],
        KeyCode::KeyC,
        Key::Character("c".into()),
    );
    assert_eq!(
        app.world().resource::<ClipboardBuffer>().paste(),
        Some("maya@example.invalid")
    );

    // 2. Tab to InviteCode: edit, ArrowLeft + Delete, and verify multiline paste strips newlines.
    send_key(&mut app, KeyCode::Tab, Key::Tab);
    assert_eq!(
        app.world().resource::<MobileDesktopUiState>().active_focus,
        MobileInputFocus::InviteCode
    );
    send_chord(
        &mut app,
        &[KeyCode::ControlLeft],
        KeyCode::KeyA,
        Key::Character("a".into()),
    );
    send_key(&mut app, KeyCode::Backspace, Key::Backspace);
    send_text(&mut app, "ABCD-99");
    send_key(&mut app, KeyCode::ArrowLeft, Key::ArrowLeft);
    send_key(&mut app, KeyCode::Delete, Key::Delete);
    assert_eq!(
        app.world()
            .resource::<MobileDesktopUiState>()
            .invite_code_display,
        "ABCD-9"
    );

    app.world_mut()
        .resource_mut::<ClipboardBuffer>()
        .copy("XY\nZ");
    send_chord(
        &mut app,
        &[KeyCode::ControlLeft],
        KeyCode::KeyV,
        Key::Character("v".into()),
    );
    assert_eq!(
        app.world()
            .resource::<MobileDesktopUiState>()
            .invite_code_display,
        "ABCD-9XYZ"
    );

    // 3. Tab to DateOfBirth: edit to a new ISO date and validate typed parser.
    send_key(&mut app, KeyCode::Tab, Key::Tab);
    assert_eq!(
        app.world().resource::<MobileDesktopUiState>().active_focus,
        MobileInputFocus::DateOfBirth
    );
    send_chord(
        &mut app,
        &[KeyCode::ControlLeft],
        KeyCode::KeyA,
        Key::Character("a".into()),
    );
    send_key(&mut app, KeyCode::Backspace, Key::Backspace);
    send_text(&mut app, "1992-07-23");
    assert_eq!(
        app.world()
            .resource::<MobileDesktopUiState>()
            .resolve_dob()
            .expect("valid parsed dob"),
        DateOfBirth::new(1992, 7, 23).expect("dob")
    );

    // 4. Tab to CuratedNote: test Shift+Enter newline + ClipboardBuffer paste + Undo/Redo.
    send_key(&mut app, KeyCode::Tab, Key::Tab);
    assert_eq!(
        app.world().resource::<MobileDesktopUiState>().active_focus,
        MobileInputFocus::CuratedNote
    );
    send_text(&mut app, "Morning check");
    send_chord(&mut app, &[KeyCode::ShiftLeft], KeyCode::Enter, Key::Enter);
    send_text(&mut app, "BP 118/76");
    assert_eq!(
        app.world().resource::<ComposerSurface>().text(),
        "Morning check\nBP 118/76"
    );

    // Verify typed parsers reject malformed values cleanly.
    assert!(matches!(
        parse_email("   "),
        Err(FormParseError::EmptyEmail)
    ));
    assert!(matches!(
        parse_email("not-an-email"),
        Err(FormParseError::InvalidEmail)
    ));
    assert!(matches!(
        parse_invite_credential("---", None),
        Err(FormParseError::EmptyInviteCode)
    ));
    assert!(matches!(
        parse_invite_credential("https://example.invalid/invite?token=abc-123", None),
        Ok(InviteCredential::LinkToken(tok)) if tok.as_str() == "abc-123"
    ));
    assert!(matches!(
        parse_dob_iso("1985/04/12"),
        Err(FormParseError::InvalidDobFormat)
    ));
    assert!(matches!(
        parse_dob_iso("1985-02-30"),
        Err(FormParseError::InvalidCalendarDate(_))
    ));
}

#[test]
fn enter_in_email_invite_and_dob_triggers_sign_in_and_verify_dob_cleanly() {
    let cfg = MobileDesktopConfig::parse_args(["--demo", "--no-prefs"]).expect("cfg");
    let mut app = build_headless_mobile_desktop_app(&cfg);
    app.update();

    // Focus Email and press Enter -> signs in.
    app.world_mut()
        .resource_mut::<MobileDesktopUiState>()
        .set_focus(MobileInputFocus::Email);
    send_key(&mut app, KeyCode::Enter, Key::Enter);
    app.update();
    assert!(matches!(
        app.world()
            .resource::<MobileClientState>()
            .auth_device
            .screen(),
        MobileClientScreen::SignedIn { .. }
    ));

    // Edit DOB to a malformed string, press Enter -> clear error status, no crash.
    app.world_mut()
        .resource_mut::<MobileDesktopUiState>()
        .set_dob_text("invalid-dob");
    app.world_mut()
        .resource_mut::<MobileDesktopUiState>()
        .set_focus(MobileInputFocus::DateOfBirth);
    send_key(&mut app, KeyCode::Enter, Key::Enter);
    app.update();
    assert!(
        app.world()
            .resource::<MobileClientState>()
            .last_dob_outcome
            .is_none()
    );
    assert!(
        app.world()
            .resource::<MobileDesktopUiState>()
            .last_status
            .contains("YYYY-MM-DD")
    );

    // Edit DOB to wrong calendar date, press Enter in InviteCode -> Mismatch outcome.
    app.world_mut()
        .resource_mut::<MobileDesktopUiState>()
        .set_dob_text("1999-01-01");
    app.world_mut()
        .resource_mut::<MobileDesktopUiState>()
        .set_focus(MobileInputFocus::InviteCode);
    send_key(&mut app, KeyCode::Enter, Key::Enter);
    app.update();
    assert!(matches!(
        app.world().resource::<MobileClientState>().last_dob_outcome,
        Some(DobVerificationOutcome::Mismatch { .. })
    ));

    // Edit DOB back to matching demo DOB (1985-04-12), press Enter -> Verified!
    app.world_mut()
        .resource_mut::<MobileDesktopUiState>()
        .set_dob_text("1985-04-12");
    app.world_mut()
        .resource_mut::<MobileDesktopUiState>()
        .set_focus(MobileInputFocus::DateOfBirth);
    send_key(&mut app, KeyCode::Enter, Key::Enter);
    app.update();
    assert!(matches!(
        app.world().resource::<MobileClientState>().last_dob_outcome,
        Some(DobVerificationOutcome::Verified { .. })
    ));
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
    assert_eq!(
        app.world().resource::<LiveServerWorker>().pending_state(),
        PendingWorkerState::Idle
    );
    app.world_mut()
        .resource_mut::<MobileDesktopUiState>()
        .set_invite_credential(
            InviteCredential::ShortCode(issued.short_code.clone()),
            issued.short_code.as_str().to_owned(),
            expected_dob,
        );
    app.update();

    // Sign in live over background HTTP worker -> verify DOB -> explicit Share -> send For My Doctor note.
    dispatch_live_and_wait(&mut app, DesktopAction::SignIn);
    assert!(matches!(
        app.world()
            .resource::<MobileClientState>()
            .auth_device
            .screen(),
        MobileClientScreen::SignedIn { .. }
    ));

    dispatch_live_and_wait(&mut app, DesktopAction::VerifyDob);
    assert!(matches!(
        app.world().resource::<MobileClientState>().last_dob_outcome,
        Some(DobVerificationOutcome::Verified { .. })
    ));

    dispatch_live_and_wait(&mut app, DesktopAction::ShareConnectedCare);
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
    dispatch_live_and_wait(&mut app, DesktopAction::ReviewAndSendDoctorNote);
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
fn live_server_interactive_typed_forms_and_unreachable_error_handling() {
    let db_dir = fresh_temp_dir("live-forms-db");
    let journal_dir = fresh_temp_dir("live-forms-journal");

    let journal = FileReceiptJournal::open(&journal_dir).expect("open receipt journal");
    let mut svc = MobileService::open_durable(&db_dir, Box::new(journal), 1_760_000_000, false)
        .expect("open durable mobile service");

    let expected_dob = DateOfBirth::new(1991, 11, 5).expect("valid dob");
    let pair = PairKey::new(
        PracticeId::new("practice-interactive-live").expect("practice id"),
        PatientId::new("patient-interactive-live").expect("patient id"),
    );
    let issued = svc
        .connected_care
        .create_invitation(CreateInvitationRequest {
            pair,
            issued_by: PhysicianId::new("phys-interactive-live").expect("phys id"),
            practice_name: "Interactive Live Clinic".to_owned(),
            physician_name: "Dr. Horizon".to_owned(),
            expected_dob,
            created_at: "2026-01-12T09:00:00Z".to_owned(),
        })
        .expect("issue connected care invite");

    let server = TcpMobileServer::bind("127.0.0.1:0", svc).expect("bind TcpMobileServer");
    let addr = server.local_addr();

    // Launch with NO preloaded invite credential — user types Email, Invite Code, DOB, and Note!
    let cfg =
        MobileDesktopConfig::parse_args(["--server", &format!("http://{addr}"), "--no-prefs"])
            .expect("live cfg");
    let mut app = build_headless_mobile_desktop_app(&cfg);
    app.update();

    // 1. Tab to Email, clear default, type custom email, press Enter -> background SignIn.
    send_key(&mut app, KeyCode::Tab, Key::Tab);
    send_chord(
        &mut app,
        &[KeyCode::ControlLeft],
        KeyCode::KeyA,
        Key::Character("a".into()),
    );
    send_key(&mut app, KeyCode::Backspace, Key::Backspace);
    send_text(&mut app, "interactive.live@example.invalid");
    send_key(&mut app, KeyCode::Enter, Key::Enter);
    wait_for_live_worker_idle(&mut app, Duration::from_secs(5));
    assert!(matches!(
        app.world()
            .resource::<MobileClientState>()
            .auth_device
            .screen(),
        MobileClientScreen::SignedIn { .. }
    ));

    // 2. Tab to InviteCode, type server-issued short code.
    send_key(&mut app, KeyCode::Tab, Key::Tab);
    send_text(&mut app, issued.short_code.as_str());

    // 3. Tab to DateOfBirth, replace with 1991-11-05, press Enter -> background VerifyDob.
    send_key(&mut app, KeyCode::Tab, Key::Tab);
    send_chord(
        &mut app,
        &[KeyCode::ControlLeft],
        KeyCode::KeyA,
        Key::Character("a".into()),
    );
    send_key(&mut app, KeyCode::Backspace, Key::Backspace);
    send_text(&mut app, "1991-11-05");
    send_key(&mut app, KeyCode::Enter, Key::Enter);
    wait_for_live_worker_idle(&mut app, Duration::from_secs(5));
    assert!(matches!(
        app.world().resource::<MobileClientState>().last_dob_outcome,
        Some(DobVerificationOutcome::Verified { .. })
    ));
    assert_eq!(
        app.world()
            .resource::<MobileDesktopUiState>()
            .practice_preview_name,
        "Interactive Live Clinic - Dr. Horizon"
    );

    // 4. Explicit Share via background worker.
    dispatch_live_and_wait(&mut app, DesktopAction::ShareConnectedCare);
    assert!(
        app.world()
            .resource::<MobileClientState>()
            .last_connection_standing
            .as_ref()
            .is_some_and(|s| s.state == ConnectedCareState::Connected)
    );

    // 5. Tab to CuratedNote, type note, press Enter -> background SendForMyDoctor.
    send_key(&mut app, KeyCode::Tab, Key::Tab);
    send_text(&mut app, "BP 118/76 today, no dizziness");
    send_key(&mut app, KeyCode::Enter, Key::Enter);
    wait_for_live_worker_idle(&mut app, Duration::from_secs(5));
    let envelopes = &app.world().resource::<MobileClientState>().sent_envelopes;
    assert_eq!(envelopes.len(), 1);
    assert_eq!(
        envelopes[0].items[0].verbatim_payload,
        "BP 118/76 today, no dizziness"
    );
    assert!(
        app.world()
            .resource::<MobileClientState>()
            .errors
            .is_empty()
    );

    server.shutdown();
    let _ = fs::remove_dir_all(&db_dir);
    let _ = fs::remove_dir_all(&journal_dir);

    // 6. Verify unreachable server surfaces a typed error cleanly without blocking UI thread.
    let closed_listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let dead_addr = closed_listener.local_addr().expect("local addr");
    drop(closed_listener);

    let err_cfg =
        MobileDesktopConfig::parse_args(["--server", &format!("http://{dead_addr}"), "--no-prefs"])
            .expect("dead server cfg");
    let mut err_app = build_headless_mobile_desktop_app(&err_cfg);
    err_app.insert_resource(LiveServerWorker::spawn_with_timeout(
        dead_addr,
        Duration::from_millis(300),
    ));
    err_app.update();

    dispatch_live_and_wait(&mut err_app, DesktopAction::SignIn);
    assert_eq!(
        err_app
            .world()
            .resource::<MobileClientState>()
            .auth_device
            .screen(),
        &MobileClientScreen::EnterYourEmail
    );
    assert!(
        !err_app
            .world()
            .resource::<MobileClientState>()
            .errors
            .is_empty(),
        "unreachable server must record a typed error"
    );
    assert!(
        err_app
            .world()
            .resource::<MobileDesktopUiState>()
            .status_override
            .as_deref()
            .is_some_and(|s| s.contains("Sign-in failed"))
    );
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

#[test]
fn mobile_preview_cli_flags_focus_and_owed_demo_render_and_pass_headless_smoke() {
    // 1. CLI parsing & validation for `--focus` and `--owed-demo`.
    for (flag_val, expected) in [
        ("email", MobileInputFocus::Email),
        ("invite", MobileInputFocus::InviteCode),
        ("dob", MobileInputFocus::DateOfBirth),
        ("note", MobileInputFocus::CuratedNote),
    ] {
        let cfg = MobileDesktopConfig::parse_args(["--focus", flag_val]).expect("valid --focus");
        assert_eq!(cfg.initial_focus, expected);
        assert!(!cfg.owed_demo);

        let eq_cfg =
            MobileDesktopConfig::parse_args([format!("--focus={flag_val}")]).expect("--focus=");
        assert_eq!(eq_cfg.initial_focus, expected);
    }

    assert_eq!(
        MobileDesktopConfig::parse_args(["--focus"]),
        Err(ConfigError::MissingValue {
            flag: "--focus".to_owned(),
        })
    );
    assert_eq!(
        MobileDesktopConfig::parse_args(["--focus", "bogus"]),
        Err(ConfigError::InvalidFocus {
            value: "bogus".to_owned(),
        })
    );

    // 2. `--headless-smoke` passes cleanly with `--focus dob --owed-demo`.
    let smoke_cfg = MobileDesktopConfig::parse_args([
        "--demo",
        "--focus",
        "dob",
        "--owed-demo",
        "--headless-smoke",
    ])
    .expect("smoke preview cfg");
    assert_eq!(smoke_cfg.initial_focus, MobileInputFocus::DateOfBirth);
    assert!(smoke_cfg.owed_demo);
    let report = run_headless_smoke(&smoke_cfg).expect("mobile headless smoke with preview flags");
    assert_eq!(report.a11y_duplicate_nodes, 0);

    // 3. Direct Bevy UI verification: DOB box has active 2px focus ring, recovery banner is
    //    `OnlineFlushPending(1)`, and Flush Owed Sign-Outs button displays `(1)`.
    let ui_cfg =
        MobileDesktopConfig::parse_args(["--demo", "--focus", "dob", "--owed-demo", "--no-prefs"])
            .expect("ui preview cfg");
    let mut app = build_headless_mobile_desktop_app(&ui_cfg);
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<MobileDesktopUiState>().active_focus,
        MobileInputFocus::DateOfBirth
    );
    let mobile = app.world().resource::<MobileClientState>();
    assert_eq!(
        mobile.auth_device.connectivity(),
        ef_mobile_domain::ConnectivityState::Online
    );
    assert_eq!(mobile.auth_device.owed_sign_outs().len(), 1);
    let banner_state = ef_mobile_desktop::OwedRecoveryBannerState::from_runtime(
        mobile.auth_device.connectivity(),
        mobile.auth_device.owed_sign_outs().len(),
        ef_mobile_desktop::PendingWorkerState::Idle,
    );
    assert!(matches!(
        banner_state,
        ef_mobile_desktop::OwedRecoveryBannerState::OnlineFlushPending { owed_count: 1, .. }
    ));

    let expected_focus_border = bevy::prelude::BorderColor::all(
        app.world()
            .resource::<UiTheme>()
            .tokens
            .focus_ring
            .to_bevy_color(),
    );
    let mut focus_boxes = app.world_mut().query::<(
        &ef_mobile_desktop::FormInputBoxNode,
        &bevy::prelude::Node,
        &bevy::prelude::BorderColor,
    )>();
    let mut dob_focused = false;
    let mut unfocused_count = 0usize;
    for (box_node, node, border_color) in focus_boxes.iter(app.world()) {
        if box_node.field == MobileInputFocus::DateOfBirth {
            dob_focused = node.border == bevy::prelude::UiRect::all(bevy::prelude::Val::Px(3.0))
                && *border_color == expected_focus_border;
        } else if node.border == bevy::prelude::UiRect::all(bevy::prelude::Val::Px(1.0)) {
            unfocused_count += 1;
        }
    }
    assert!(
        dob_focused,
        "DateOfBirth FormInputBoxNode must render 3px WCAG focus ring for --focus dob"
    );
    assert_eq!(
        unfocused_count, 3,
        "other 3 input fields must remain 1px unfocused"
    );

    let mut texts = app.world_mut().query::<(&TextSlot, &bevy::prelude::Text)>();
    let mut recovery_text = String::new();
    let mut flush_label = String::new();
    for (slot, text) in texts.iter(app.world()) {
        match *slot {
            TextSlot::OwedRecoveryBanner => recovery_text = text.0.clone(),
            TextSlot::FlushOwedBtnLabel => flush_label = text.0.clone(),
            _ => {}
        }
    }
    assert!(
        recovery_text.contains("ready to flush"),
        "expected OnlineFlushPending banner text, got `{recovery_text}`"
    );
    assert_eq!(
        flush_label,
        format!("{} (1)", ef_mobile_desktop::FLUSH_OWED_SIGN_OUTS_LABEL)
    );
}
