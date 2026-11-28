//! End-to-end integration tests for the 4-Tab Curie Clinic Physician Workstation:
//! `Chat`, `Today`, `Messages`, and `Memory & Care`, plus `--view` CLI parsing,
//! `Ctrl/Cmd+1..4` keyboard shortcuts, RTL mirroring, and 0 duplicate AccessKit IDs.

use bevy::asset::AssetPlugin;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyCode, KeyboardInput};
use bevy::prelude::*;
use ef_clinic_client::{
    ClientBootConfig, CurieClinicClientPlugin, SharedBrowserSessionHub, SharedLocaleStorage,
};
use ef_clinic_workflows::today::{TodayChannelBanner, TodayUrgency, TodayViewState};
use ef_connected_care::ConnectedCareState;
use ef_core::TabId;
use ef_ui_catalog::{A11yProjection, Locale, ThemeMode, UiKey, translate};

use ef_clinic_desktop::{
    ClinicDesktopUiPlugin, ClinicWorkflowState, ClinicWorkspaceTab, DesktopClipboard,
    DesktopConfigError, DesktopInputFocus, DesktopLaunchConfig, DesktopStatusBanner,
    DesktopUiEvent, DesktopUiSlot, DesktopWorkspaceState, LaunchBackend, LivePromptWorker,
    LiveSubmissionState, ZoomLevel, bootstrap_clinic_session, create_transport_for_backend,
    run_headless_smoke,
};

fn build_workstation_app(initial_view: ClinicWorkspaceTab) -> App {
    let (transport, conn_mode, email, token, patients, citation_catalog) =
        create_transport_for_backend(&LaunchBackend::Demo);
    let initial_citations = patients
        .first()
        .map(|p| citation_catalog.cards_for(&p.workspace_id))
        .unwrap_or_default();
    let boot_config = ClientBootConfig::from_location("/elysianfields/?lang=en").expect("boot");
    let tab_id = TabId::new("workstation-e2e-tab").expect("tab id");

    let ws_state = DesktopWorkspaceState {
        connection_mode: conn_mode,
        patients,
        active_patient_index: 0,
        citations: initial_citations,
        selected_citation_index: 0,
        transcript: Vec::new(),
        streaming_draft: None,
        last_failed_prompt: None,
        status_message: String::new(),
        status_override: None,
        status_banner: DesktopStatusBanner::DefaultReady,
        input_focus: DesktopInputFocus::Composer,
        patient_search_query: String::new(),
        search_highlight_index: 0,
        live_submission: LiveSubmissionState::Idle,
        next_live_request_id: 1,
        shortcuts_modal_open: false,
        zoom_percent: 100,
        frames_rendered_ui: 0,
        prompts_submitted: 0,
    };

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .insert_resource(ButtonInput::<KeyCode>::default())
        .insert_resource(LivePromptWorker::disabled())
        .insert_resource(DesktopClipboard::in_memory())
        .insert_resource(transport)
        .insert_resource(citation_catalog)
        .insert_resource(ws_state)
        .insert_resource(ClinicWorkflowState::seeded_demo(initial_view))
        .add_plugins(CurieClinicClientPlugin::new(
            tab_id,
            boot_config,
            SharedBrowserSessionHub::new(None),
            SharedLocaleStorage::default(),
        ))
        .add_plugins(ClinicDesktopUiPlugin::new(
            ThemeMode::Dark,
            ZoomLevel::Pct100,
            None,
        ));
    bootstrap_clinic_session(&mut app, email, token);
    app
}

fn send_event(app: &mut App, evt: DesktopUiEvent) {
    app.world_mut().write_message(evt);
    app.update();
    app.update();
}

fn slot_display(app: &mut App, target: DesktopUiSlot) -> Display {
    let mut q = app.world_mut().query::<(&DesktopUiSlot, &Node)>();
    q.iter(app.world())
        .find(|(slot, _)| **slot == target)
        .map(|(_, node)| node.display)
        .unwrap_or_else(|| panic!("missing slot {target:?}"))
}

fn slot_flex_direction(app: &mut App, target: DesktopUiSlot) -> FlexDirection {
    let mut q = app.world_mut().query::<(&DesktopUiSlot, &Node)>();
    q.iter(app.world())
        .find(|(slot, _)| **slot == target)
        .map(|(_, node)| node.flex_direction)
        .unwrap_or_else(|| panic!("missing slot {target:?}"))
}

fn slot_text(app: &mut App, target: DesktopUiSlot) -> String {
    let mut q = app.world_mut().query::<(&DesktopUiSlot, &Text)>();
    q.iter(app.world())
        .find(|(slot, _)| **slot == target)
        .map(|(_, txt)| txt.0.clone())
        .unwrap_or_else(|| panic!("missing text slot {target:?}"))
}

#[test]
fn cli_view_flag_parses_all_tabs_and_rejects_invalid_values() {
    let default_cfg = DesktopLaunchConfig::parse_args(Vec::<String>::new()).expect("default");
    assert_eq!(default_cfg.initial_view, ClinicWorkspaceTab::Chat);

    for (raw, expected) in [
        ("chat", ClinicWorkspaceTab::Chat),
        ("today", ClinicWorkspaceTab::Today),
        ("messages", ClinicWorkspaceTab::Messages),
        ("memory", ClinicWorkspaceTab::MemoryCare),
    ] {
        let cfg = DesktopLaunchConfig::parse_args(["--view", raw]).expect("parse --view");
        assert_eq!(cfg.initial_view, expected);

        let eq_cfg =
            DesktopLaunchConfig::parse_args([format!("--view={raw}")]).expect("parse --view=");
        assert_eq!(eq_cfg.initial_view, expected);
    }

    assert_eq!(
        DesktopLaunchConfig::parse_args(["--view", "billing"]),
        Err(DesktopConfigError::InvalidView {
            value: "billing".to_owned(),
        })
    );
    assert_eq!(
        DesktopLaunchConfig::parse_args(["--view"]),
        Err(DesktopConfigError::MissingValue {
            flag: "--view".to_owned(),
        })
    );

    // Headless smoke succeeds cleanly for every `--view` tab!
    for raw in ["chat", "today", "messages", "memory"] {
        let smoke_cfg =
            DesktopLaunchConfig::parse_args(["--headless-smoke", "--view", raw]).expect("smoke");
        let report = run_headless_smoke(&smoke_cfg).expect("headless smoke");
        assert!(report.passed);
        assert_eq!(report.duplicate_a11y_nodes, 0);
    }
}

#[test]
fn keyboard_ctrl_1_through_4_switch_all_four_tabs_and_mirror_rtl() {
    let mut app = build_workstation_app(ClinicWorkspaceTab::Chat);
    assert_eq!(
        slot_display(&mut app, DesktopUiSlot::WorkspaceColumns),
        Display::Flex
    );
    assert_eq!(
        slot_display(&mut app, DesktopUiSlot::TodayPanel),
        Display::None
    );

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ControlLeft);

    for (key_code, expected_tab, expected_panel) in [
        (
            KeyCode::Digit2,
            ClinicWorkspaceTab::Today,
            DesktopUiSlot::TodayPanel,
        ),
        (
            KeyCode::Digit3,
            ClinicWorkspaceTab::Messages,
            DesktopUiSlot::MessagesPanel,
        ),
        (
            KeyCode::Digit4,
            ClinicWorkspaceTab::MemoryCare,
            DesktopUiSlot::MemoryCarePanel,
        ),
        (
            KeyCode::Digit1,
            ClinicWorkspaceTab::Chat,
            DesktopUiSlot::WorkspaceColumns,
        ),
    ] {
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        app.update();
        app.update();

        assert_eq!(
            app.world().resource::<ClinicWorkflowState>().active_tab,
            expected_tab
        );
        assert_eq!(slot_display(&mut app, expected_panel), Display::Flex);
    }

    // Verify Hebrew RTL mirroring across tab bar, Today panel, and Messages action row.
    send_event(&mut app, DesktopUiEvent::SetLocale(Locale::He));
    assert_eq!(
        slot_flex_direction(&mut app, DesktopUiSlot::WorkspaceTabBar),
        FlexDirection::RowReverse
    );
    assert_eq!(
        slot_flex_direction(&mut app, DesktopUiSlot::TodayPanel),
        FlexDirection::RowReverse
    );
    assert_eq!(
        slot_flex_direction(&mut app, DesktopUiSlot::MessagesActionRow),
        FlexDirection::RowReverse
    );

    let audit = app
        .world()
        .resource::<A11yProjection>()
        .verify_zero_duplicate_nodes()
        .expect("zero duplicate AccessKit nodes");
    assert_eq!(audit.duplicated_nodes, 0);
}

#[test]
fn today_tab_enforces_today_says_so_urgency_sorting_mark_seen_done_and_visit_open() {
    let mut app = build_workstation_app(ClinicWorkspaceTab::Today);

    // 1. Channel health banner obeys `TodaySaysSo` (TempError never asks to reconnect).
    let view = app
        .world()
        .resource::<ClinicWorkflowState>()
        .build_today_view();
    match view {
        TodayViewState::Actionable {
            ref items,
            ref channel_banners,
            ..
        } => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0].item.urgency, TodayUrgency::Now);
            assert_eq!(items[1].item.urgency, TodayUrgency::Now);
            assert_eq!(items[2].item.urgency, TodayUrgency::Today);
            assert!(channel_banners.iter().any(|b| matches!(
                b,
                TodayChannelBanner::TemporaryTrouble {
                    asks_to_reconnect: false,
                    ..
                }
            )));
        }
        TodayViewState::Quiet { .. } => panic!("expected actionable Today queue"),
    }
    let banner_text = slot_text(&mut app, DesktopUiSlot::TodayChannelHealthBanner);
    assert!(banner_text.contains("spruce-sms"));
    assert!(!banner_text.to_ascii_lowercase().contains("reconnect"));

    // 2. Mark Seen updates standing at current version_id while keeping item visible.
    send_event(&mut app, DesktopUiEvent::TodayMarkSeen(0));
    let items_after_seen = app
        .world()
        .resource::<ClinicWorkflowState>()
        .visible_today_items();
    assert_eq!(items_after_seen.len(), 3);
    assert!(items_after_seen[0].seen);

    // 3. Mark Done removes the item from the active Today queue.
    send_event(&mut app, DesktopUiEvent::TodayMarkDone(0));
    let items_after_done = app
        .world()
        .resource::<ClinicWorkflowState>()
        .visible_today_items();
    assert_eq!(items_after_done.len(), 2);

    // 4. Clicking Scheduled Visit opens that patient in Chat tab.
    send_event(&mut app, DesktopUiEvent::OpenScheduledVisit(1));
    assert_eq!(
        app.world().resource::<ClinicWorkflowState>().active_tab,
        ClinicWorkspaceTab::Chat
    );
    assert_eq!(
        app.world()
            .resource::<DesktopWorkspaceState>()
            .active_patient_index,
        1
    );
}

#[test]
fn messages_tab_supports_copy_reply_close_reply_and_undo_close_invariants() {
    let mut app = build_workstation_app(ClinicWorkspaceTab::Messages);

    let initial_body = slot_text(&mut app, DesktopUiSlot::MessagesCardBody);
    assert!(initial_body.contains(translate(Locale::En, UiKey::MessagesSystemDraftBadge)));
    assert!(initial_body.contains("src-avery-note-1"));
    assert!(initial_body.contains("Flagged concern:"));
    assert_eq!(
        slot_display(&mut app, DesktopUiSlot::MessagesCopyReplyBtn),
        Display::Flex
    );
    assert_eq!(
        slot_display(&mut app, DesktopUiSlot::MessagesCloseReplyBtn),
        Display::Flex
    );
    assert_eq!(
        slot_display(&mut app, DesktopUiSlot::MessagesUndoCloseBtn),
        Display::None
    );

    // Copy Reply copies exact visible reply into DesktopClipboard (`CopiesWhatYouSee`).
    send_event(&mut app, DesktopUiEvent::MessagesCopyReply);
    let pasted = app
        .world_mut()
        .resource_mut::<DesktopClipboard>()
        .paste_text()
        .expect("clipboard text");
    assert!(pasted.contains("Metformin 1000 mg twice daily"));

    // Close Reply hides Copy/Close, preserves flagged concern (`ConcernStays`), shows Undo Close.
    send_event(&mut app, DesktopUiEvent::MessagesCloseReply);
    assert_eq!(
        slot_display(&mut app, DesktopUiSlot::MessagesCopyReplyBtn),
        Display::None
    );
    assert_eq!(
        slot_display(&mut app, DesktopUiSlot::MessagesUndoCloseBtn),
        Display::Flex
    );
    let closed_body = slot_text(&mut app, DesktopUiSlot::MessagesCardBody);
    assert!(closed_body.contains("Flagged concern:"));

    // Undo Close restores exact ready reply & open patient message (`UndoBringsBack`).
    send_event(&mut app, DesktopUiEvent::MessagesUndoClose);
    assert_eq!(
        slot_display(&mut app, DesktopUiSlot::MessagesCopyReplyBtn),
        Display::Flex
    );
    assert_eq!(
        slot_display(&mut app, DesktopUiSlot::MessagesUndoCloseBtn),
        Display::None
    );
    let restored_card = app
        .world()
        .resource::<ClinicWorkflowState>()
        .derive_active_reply_card()
        .expect("restored card");
    assert_eq!(restored_card.open_messages.len(), 1);
    assert!(restored_card.visible_reply_text.is_some());
}

#[test]
fn memory_and_care_tab_shows_disagreement_authority_badges_and_issues_ef_invite_code() {
    let mut app = build_workstation_app(ClinicWorkspaceTab::MemoryCare);

    let mem_text = slot_text(&mut app, DesktopUiSlot::MemoryEntriesBody);
    assert!(mem_text.contains(translate(Locale::En, UiKey::ChartRecordBadge)));
    assert!(mem_text.contains(translate(Locale::En, UiKey::PhysicianInstructionBadge)));
    assert!(mem_text.contains("[ACTIVE |"));
    assert!(mem_text.contains("[RETIRED "));

    let active_ws = app
        .world()
        .resource::<DesktopWorkspaceState>()
        .active_patient()
        .expect("active patient")
        .workspace_id
        .clone();
    assert_eq!(
        app.world()
            .resource::<ClinicWorkflowState>()
            .connected_care_state_for(&active_ws),
        ConnectedCareState::NotInvited
    );

    // Click Issue Connected Care Invite -> generates real `EF-XXXXXX` code via `ConnectedCareBridge`.
    send_event(&mut app, DesktopUiEvent::MemoryIssueInvite);
    assert_eq!(
        app.world()
            .resource::<ClinicWorkflowState>()
            .connected_care_state_for(&active_ws),
        ConnectedCareState::Invited
    );
    let invite_text = slot_text(&mut app, DesktopUiSlot::MemoryActiveInviteCodeText);
    assert!(
        invite_text.contains("EF-000001"),
        "expected EF-000001 invite code, got: {invite_text}"
    );
    let status_text = slot_text(&mut app, DesktopUiSlot::ConnectedCareStatusText);
    assert!(status_text.contains("Invited"));

    // Issuing again while already Invited returns the active open invite (`AtMostOneOpen`).
    send_event(&mut app, DesktopUiEvent::MemoryIssueInvite);
    let invite_text_again = slot_text(&mut app, DesktopUiSlot::MemoryActiveInviteCodeText);
    assert!(invite_text_again.contains("EF-000001"));
}

#[test]
fn messages_tab_personal_draft_controls_and_badges_localize_without_english_fallback() {
    let mut app = build_workstation_app(ClinicWorkspaceTab::Messages);

    let en_save = translate(Locale::En, UiKey::MessagesSaveDraft);
    let en_restore = translate(Locale::En, UiKey::MessagesRestoreAiDraft);
    let en_placeholder = translate(Locale::En, UiKey::MessagesDraftPlaceholder);
    let en_personal_badge = translate(Locale::En, UiKey::MessagesPersonalDraftBadge);
    let en_ai_badge = translate(Locale::En, UiKey::MessagesSystemDraftBadge);

    // Verify Hebrew RTL, Arabic RTL, and Hindi Devanagari (plus all non-English locales)
    // render localized Messages buttons, draft placeholder, and origin badges with zero English fallback.
    for locale in [Locale::He, Locale::Ar, Locale::Hi] {
        send_event(&mut app, DesktopUiEvent::SetLocale(locale));

        // Ensure draft box is empty so placeholder text is active.
        send_event(&mut app, DesktopUiEvent::UpdateReplyDraft(String::new()));

        let save_label = slot_text(&mut app, DesktopUiSlot::MessagesSaveDraftLabel);
        let restore_label = slot_text(&mut app, DesktopUiSlot::MessagesRestoreSystemLabel);
        let draft_placeholder = slot_text(&mut app, DesktopUiSlot::MessagesDraftText);
        let ai_card_body = slot_text(&mut app, DesktopUiSlot::MessagesCardBody);

        assert_eq!(save_label, translate(locale, UiKey::MessagesSaveDraft));
        assert_eq!(
            restore_label,
            translate(locale, UiKey::MessagesRestoreAiDraft)
        );
        assert_eq!(
            draft_placeholder,
            translate(locale, UiKey::MessagesDraftPlaceholder)
        );
        assert!(
            ai_card_body.contains(translate(locale, UiKey::MessagesSystemDraftBadge)),
            "expected localized AI draft badge in {locale:?}, got: {ai_card_body}"
        );

        // Zero English fallback on Save Draft, Restore AI Draft, placeholder, and origin badge.
        assert_ne!(save_label, en_save);
        assert_ne!(restore_label, en_restore);
        assert_ne!(draft_placeholder, en_placeholder);
        assert!(!ai_card_body.contains(en_ai_badge));
        assert!(!ai_card_body.contains("SystemReply"));

        // Save a personal draft -> origin badge updates to localized PersonalDraft badge
        // and Restore AI Draft button becomes visible with localized label.
        send_event(
            &mut app,
            DesktopUiEvent::UpdateReplyDraft("Continue Metformin 1000 mg BID.".to_owned()),
        );
        send_event(&mut app, DesktopUiEvent::MessagesSaveDraft);
        assert_eq!(
            slot_display(&mut app, DesktopUiSlot::MessagesRestoreSystemBtn),
            Display::Flex
        );
        let personal_card_body = slot_text(&mut app, DesktopUiSlot::MessagesCardBody);
        assert!(
            personal_card_body.contains(translate(locale, UiKey::MessagesPersonalDraftBadge)),
            "expected localized Personal Draft badge in {locale:?}, got: {personal_card_body}"
        );
        assert!(!personal_card_body.contains(en_personal_badge));
        assert!(!personal_card_body.contains("PersonalDraft"));

        // Restore AI draft back so next locale starts cleanly on SystemReply.
        send_event(&mut app, DesktopUiEvent::MessagesRestoreSystem);
        assert_eq!(
            slot_display(&mut app, DesktopUiSlot::MessagesRestoreSystemBtn),
            Display::None
        );
    }

    // Also verify zero English fallback across all 11 non-English locales for all 5 keys.
    for locale in Locale::ALL
        .into_iter()
        .filter(|locale| *locale != Locale::En)
    {
        for key in [
            UiKey::MessagesSaveDraft,
            UiKey::MessagesRestoreAiDraft,
            UiKey::MessagesDraftPlaceholder,
            UiKey::MessagesPersonalDraftBadge,
            UiKey::MessagesSystemDraftBadge,
        ] {
            let localized = translate(locale, key);
            assert!(
                !localized.trim().is_empty(),
                "empty translation for {locale:?} / {key:?}"
            );
            assert_ne!(
                localized,
                translate(Locale::En, key),
                "unexpected English fallback for {locale:?} / {key:?}"
            );
        }
    }
}
