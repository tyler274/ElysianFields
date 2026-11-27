//! M1 Bevy 0.20.1 accessibility feasibility prototype.
//!
//! Models four required clinical workspace surfaces (navigation list, streaming
//! transcript with interactive citations, clinical assessment form, and modal
//! dialog with focus trap and restoration) behind a single-owner AccessKit tree,
//! keyboard focus manager, WCAG 2.1 AA contrast verifier, 200% zoom reflow
//! checker, reduced-motion policy, throttled streaming live-announcement queue,
//! and narrow browser ARIA semantic bridge.

pub mod bridge;
pub mod contrast;
pub mod domain;
pub mod focus;
pub mod live_region;
pub mod plugin;
pub mod preferences;
pub mod tree;

pub use bridge::{
    AriaRole, BridgeBudgetReport, BridgeElement, BridgeModalScope, BridgePatch, BridgeProjection,
    BridgeProjectionError, BrowserA11yBridge, ELYSIAN_SUBPATH_PREFIX, NarrowSemanticBridge,
    verify_bijective_projection,
};
pub use contrast::{
    ContrastAuditEntry, ContrastRequirement, ContrastViolation, PalettePair, PalettePairId,
    PaletteTheme, SrgbColor, contrast_ratio, elysian_palette, relative_luminance,
    verify_palette_wcag_aa,
};
pub use domain::{
    ActiveCitation, CheckboxState, CitationId, ControlAvailability, DisabledReason,
    DisclosureState, FieldValidation, FocusTarget, FormValidationError, ModalActionId, ModalState,
    NavItemId, ScreenState, ScreenSurface, SemanticControlId, SignOffStatus, SubmissionState,
    SubmittedClinicalNote,
};
pub use focus::{
    FocusManager, FocusRingState, FocusRingStyle, FocusTransitionOutcome, KeyboardCommand,
    MODAL_FOCUS_ORDER, ModalDismissReason, NON_MODAL_FOCUS_ORDER, ordered_focus_targets,
};
pub use live_region::{
    AnnouncementPriority, CoalescingPolicy, CoalescingPolicyError, LiveAnnouncement,
    LiveAnnouncementQueue,
};
pub use plugin::{
    AccessibilityPrototypePlugin, BridgeSyncStatus, KeyboardCommandQueue, SemanticEntityRegistry,
    build_accesskit_node,
};
pub use preferences::{
    AccessibilityPreferences, AnimationMode, ControlLayoutRect, MotionPolicy, ReducedMotion,
    TextScale, TextScaleError, ViewportSize, ZoomLayoutError, ZoomLayoutReport,
    compute_scaled_layout, verify_zoom_layout,
};
pub use tree::{
    AccessibilityInvariantError, AccessibilityNodeRecord, AccessibilityTreeSnapshot,
    ExpansionState, ModalRole, NodeAvailability, NodeInteractivity, NodeVisibility, SelectionState,
    SemanticControlMarker, TreeAuditSummary, node_id_for_entity,
};

#[cfg(test)]
mod tests {
    use bevy::input_focus::{InputFocus, InputFocusVisible};
    use bevy::prelude::{App, MinimalPlugins};

    use super::*;

    fn build_headless_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AccessibilityPrototypePlugin));
        // Run startup + first update so AccessKit nodes, snapshot, and bridge sync.
        app.update();
        app
    }

    fn send_command(
        app: &mut App,
        now_ms: u64,
        command: KeyboardCommand,
    ) -> FocusTransitionOutcome {
        app.world_mut()
            .resource_mut::<KeyboardCommandQueue>()
            .push(now_ms, command);
        app.update();
        app.world()
            .resource::<KeyboardCommandQueue>()
            .last_outcomes()
            .last()
            .cloned()
            .expect("single keyboard command must produce one outcome")
    }

    fn expected_interactive_controls(modal: &ModalState) -> Vec<SemanticControlId> {
        ordered_focus_targets(modal)
            .iter()
            .map(|target| target.semantic_id())
            .collect()
    }

    #[test]
    fn single_owner_accesskit_tree_and_bridge_have_zero_duplicate_nodes() {
        let app = build_headless_app();
        let screen = app.world().resource::<ScreenState>();
        let snapshot = app.world().resource::<AccessibilityTreeSnapshot>();
        let bridge = app.world().resource::<NarrowSemanticBridge>();
        let sync_status = app.world().resource::<BridgeSyncStatus>();

        assert!(sync_status.last_error.is_none());

        let expected = expected_interactive_controls(&screen.modal);
        let summary = snapshot
            .verify_zero_duplicate_nodes(&expected, &screen.modal)
            .expect("initial AccessKit tree must pass zero-duplicate-node audit");
        assert_eq!(
            summary.total_nodes,
            SemanticControlId::ALL_IN_READING_ORDER.len()
        );
        assert_eq!(
            summary.visible_interactive_nodes,
            NON_MODAL_FOCUS_ORDER.len()
        );
        assert_eq!(summary.live_regions, 2);

        let projection = bridge
            .last_projection()
            .expect("narrow browser bridge must project initial snapshot");
        let budget = verify_bijective_projection(snapshot, projection)
            .expect("browser bridge must project a 1:1 bijection with AccessKit nodes");
        assert_eq!(budget.duplicated_nodes, 0);
        assert_eq!(budget.mirrored_css_rules, 0);
        assert_eq!(budget.interactive_elements, NON_MODAL_FOCUS_ORDER.len());
    }

    #[test]
    fn zero_duplicate_checker_catches_injected_duplicate_node() {
        let app = build_headless_app();
        let screen = app.world().resource::<ScreenState>();
        let snapshot = app.world().resource::<AccessibilityTreeSnapshot>();

        let mut duplicated_records = snapshot.nodes().to_vec();
        let clone_submit = snapshot
            .find_by_control(SemanticControlId::SubmitNoteButton)
            .expect("submit button must exist")
            .clone();
        duplicated_records.push(clone_submit);

        let corrupted =
            AccessibilityTreeSnapshot::from_records(duplicated_records, snapshot.focused_node_id());
        let expected = expected_interactive_controls(&screen.modal);
        let errs = corrupted
            .verify_zero_duplicate_nodes(&expected, &screen.modal)
            .expect_err("duplicate node must fail invariant check");

        assert!(
            errs.iter()
                .any(|err| matches!(err, AccessibilityInvariantError::DuplicateNodeId { .. }))
        );
        assert!(errs.iter().any(|err| matches!(
            err,
            AccessibilityInvariantError::DuplicateSemanticControl {
                control: SemanticControlId::SubmitNoteButton,
                count: 2
            }
        )));
    }

    #[test]
    fn full_keyboard_only_workflow_form_submission_modal_trap_and_focus_restore() {
        let mut app = build_headless_app();

        // Start on NavItem::Transcript; exercise arrow-key list navigation & Home/End.
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::NavItem(NavItemId::Transcript)
        );
        let _ = send_command(&mut app, 100, KeyboardCommand::ArrowDown);
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::NavItem(NavItemId::ClinicalForm)
        );
        let _ = send_command(&mut app, 150, KeyboardCommand::Home);
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::NavItem(NavItemId::Inbox)
        );
        let _ = send_command(&mut app, 200, KeyboardCommand::End);
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::NavItem(NavItemId::LabRecords)
        );

        // Tab to Citation 1 and activate with Enter.
        let _ = send_command(&mut app, 250, KeyboardCommand::TabForward);
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::CitationLink(CitationId::ProgressNoteSummary)
        );
        let cite_outcome = send_command(&mut app, 300, KeyboardCommand::ActivateEnter);
        assert_eq!(
            cite_outcome,
            FocusTransitionOutcome::CitationOpened(CitationId::ProgressNoteSummary)
        );
        assert_eq!(
            app.world().resource::<ScreenState>().active_citation,
            ActiveCitation::Inspecting(CitationId::ProgressNoteSummary)
        );

        // Tab past Citation 2 -> AssessmentNoteInput -> Checkbox -> Disclosure -> Submit.
        let _ = send_command(&mut app, 350, KeyboardCommand::TabForward); // Citation 2
        let _ = send_command(&mut app, 400, KeyboardCommand::TabForward); // AssessmentNoteInput
        let _ = send_command(&mut app, 450, KeyboardCommand::TabForward); // Checkbox
        let toggle_outcome = send_command(&mut app, 500, KeyboardCommand::ActivateSpace);
        assert_eq!(
            toggle_outcome,
            FocusTransitionOutcome::CheckboxToggled(CheckboxState::Unchecked)
        );
        let _ = send_command(&mut app, 550, KeyboardCommand::ActivateSpace); // back to Checked

        let _ = send_command(&mut app, 600, KeyboardCommand::TabForward); // Disclosure
        let disc_outcome = send_command(&mut app, 650, KeyboardCommand::ActivateEnter);
        assert_eq!(
            disc_outcome,
            FocusTransitionOutcome::DisclosureToggled(DisclosureState::Expanded)
        );

        // Verify expanded state propagated directly into the Bevy AccessKit snapshot.
        let disc_record = app
            .world()
            .resource::<AccessibilityTreeSnapshot>()
            .find_by_control(SemanticControlId::GuidanceDisclosure)
            .expect("guidance disclosure must exist")
            .clone();
        assert_eq!(disc_record.expansion, ExpansionState::Expanded);

        // Tab to Submit while empty -> assertive validation alert fires.
        let _ = send_command(&mut app, 700, KeyboardCommand::TabForward);
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::SubmitNoteButton
        );
        let invalid_outcome = send_command(&mut app, 750, KeyboardCommand::ActivateEnter);
        assert_eq!(
            invalid_outcome,
            FocusTransitionOutcome::FormValidationFailed(FormValidationError::EmptyNote)
        );
        assert!(
            app.world()
                .resource::<LiveAnnouncementQueue>()
                .latest_text(AnnouncementPriority::Assertive)
                .is_some_and(|msg| msg.contains("required"))
        );

        // Shift+Tab back to AssessmentNoteInput and type a valid synthetic clinical note.
        let _ = send_command(&mut app, 800, KeyboardCommand::TabBackward); // Disclosure
        let _ = send_command(&mut app, 820, KeyboardCommand::TabBackward); // Checkbox
        let _ = send_command(&mut app, 840, KeyboardCommand::TabBackward); // AssessmentNoteInput
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::AssessmentNoteInput
        );
        for (idx, ch) in "Synthetic vitals stable.".chars().enumerate() {
            let _ = send_command(
                &mut app,
                900 + idx as u64 * 10,
                KeyboardCommand::TypeChar(ch),
            );
        }

        // Tab forward to SubmitNoteButton and submit cleanly.
        let _ = send_command(&mut app, 1300, KeyboardCommand::TabForward); // Checkbox
        let _ = send_command(&mut app, 1320, KeyboardCommand::TabForward); // Disclosure
        let _ = send_command(&mut app, 1340, KeyboardCommand::TabForward); // SubmitNoteButton
        let submit_outcome = send_command(&mut app, 1360, KeyboardCommand::ActivateEnter);
        assert!(matches!(
            submit_outcome,
            FocusTransitionOutcome::FormSubmitted(_)
        ));

        // Tab to OpenSignOffModalButton (now enabled) and open the modal.
        let _ = send_command(&mut app, 1400, KeyboardCommand::TabForward);
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::OpenSignOffModalButton
        );
        let open_outcome = send_command(&mut app, 1450, KeyboardCommand::ActivateEnter);
        assert_eq!(
            open_outcome,
            FocusTransitionOutcome::ModalOpened {
                restore_focus: FocusTarget::OpenSignOffModalButton
            }
        );

        // Verify focus moved inside modal and Bevy InputFocus + visible focus ring agree.
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::ModalConfirmButton
        );
        assert!(app.world().resource::<InputFocusVisible>().0);
        let expected_entity = app
            .world()
            .resource::<SemanticEntityRegistry>()
            .entity_for(SemanticControlId::ModalConfirmButton);
        assert_eq!(app.world().resource::<InputFocus>().get(), expected_entity);

        // Verify AccessKit tree + browser bridge strictly scope visible controls to the modal.
        {
            let screen = app.world().resource::<ScreenState>();
            let snapshot = app.world().resource::<AccessibilityTreeSnapshot>();
            let bridge = app.world().resource::<NarrowSemanticBridge>();
            let expected_modal = expected_interactive_controls(&screen.modal);
            let audit = snapshot
                .verify_zero_duplicate_nodes(&expected_modal, &screen.modal)
                .expect("modal-open tree must pass strict focus-trap & zero-duplicate audit");
            assert_eq!(audit.visible_interactive_nodes, MODAL_FOCUS_ORDER.len());

            let projection = bridge.last_projection().expect("modal projection exists");
            assert!(matches!(
                projection.modal_state,
                BridgeModalScope::ModalDialogActive { .. }
            ));
            let budget = verify_bijective_projection(snapshot, projection)
                .expect("modal bridge projection must stay 1:1");
            assert_eq!(budget.interactive_elements, MODAL_FOCUS_ORDER.len());
        }

        // Verify Tab and Shift+Tab wrap strictly inside the modal (focus trap!).
        let _ = send_command(&mut app, 1500, KeyboardCommand::TabForward);
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::ModalCancelButton
        );
        let _ = send_command(&mut app, 1520, KeyboardCommand::TabForward);
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::ModalConfirmButton
        );
        let _ = send_command(&mut app, 1540, KeyboardCommand::TabBackward);
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::ModalCancelButton
        );

        // Press Escape -> modal closes and exact prior FocusTarget is restored!
        let dismiss_outcome = send_command(&mut app, 1600, KeyboardCommand::Escape);
        assert_eq!(
            dismiss_outcome,
            FocusTransitionOutcome::ModalDismissed {
                restored_focus: FocusTarget::OpenSignOffModalButton,
                reason: ModalDismissReason::Escaped,
            }
        );
        assert_eq!(
            app.world().resource::<FocusManager>().current(),
            FocusTarget::OpenSignOffModalButton
        );
        assert!(!app.world().resource::<ScreenState>().modal.is_open());

        // Visible focus ring must remain active with >= 3.0:1 contrast after restore.
        match app.world().resource::<FocusManager>().ring() {
            FocusRingState::Visible(style) => {
                assert_eq!(style.target, FocusTarget::OpenSignOffModalButton);
                assert!(style.contrast_ratio >= ContrastRequirement::UiComponent.min_ratio());
            }
            FocusRingState::Hidden => panic!("keyboard focus ring must remain visible"),
        }
    }

    #[test]
    fn zoom_200_percent_keeps_every_control_unclipped_and_non_overlapping() {
        let expected_non_modal: Vec<SemanticControlId> = NON_MODAL_FOCUS_ORDER
            .iter()
            .map(|t| t.semantic_id())
            .collect();

        for viewport in [
            ViewportSize::DESKTOP_1280X720,
            ViewportSize::MOBILE_390X844,
            ViewportSize::POPUP_360X520,
        ] {
            let rects = compute_scaled_layout(TextScale::DOUBLE_200, viewport, &ModalState::Closed);
            let report =
                verify_zoom_layout(TextScale::DOUBLE_200, viewport, &rects, &expected_non_modal)
                    .expect("200% zoom layout must have zero clipped or overlapping controls");
            assert_eq!(report.verified_controls, expected_non_modal.len());
            assert!(report.total_scroll_height_px > 0.0);
        }

        // Negative test: verify clipped and overlapping controls are caught.
        let mut broken = compute_scaled_layout(
            TextScale::DOUBLE_200,
            ViewportSize::MOBILE_390X844,
            &ModalState::Closed,
        );
        broken[0].width = 500.0; // horizontal overflow on 390px viewport
        broken[1].y = broken[0].y; // overlaps control 0
        let errs = verify_zoom_layout(
            TextScale::DOUBLE_200,
            ViewportSize::MOBILE_390X844,
            &broken,
            &expected_non_modal,
        )
        .expect_err("overflowing and overlapping layout must fail zoom check");
        assert!(
            errs.iter()
                .any(|e| matches!(e, ZoomLayoutError::HorizontalOverflow { .. }))
        );
        assert!(
            errs.iter()
                .any(|e| matches!(e, ZoomLayoutError::OverlappingControls { .. }))
        );
    }

    #[test]
    fn reduced_motion_disables_all_non_essential_animations() {
        let standard = MotionPolicy::for_preference(ReducedMotion::NoPreference);
        assert!(standard.allows_non_essential_motion());
        assert_eq!(standard.streaming_cursor, AnimationMode::Animated);
        assert!(standard.modal_progress(80) < 1.0);

        let reduced = MotionPolicy::for_preference(ReducedMotion::Reduce);
        assert!(!reduced.allows_non_essential_motion());
        assert_eq!(reduced.modal_transition_ms, 0);
        assert_eq!(reduced.scroll_easing_ms, 0);
        assert_eq!(reduced.focus_ring_transition_ms, 0);
        assert_eq!(reduced.streaming_cursor, AnimationMode::Static);
        assert!((reduced.modal_progress(0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn wcag_21_aa_contrast_passes_for_both_elysian_themes_and_rejects_low_contrast() {
        for theme in [PaletteTheme::Light, PaletteTheme::Dark] {
            let entries = verify_palette_wcag_aa(elysian_palette(theme))
                .expect("Elysian Fields palette must pass WCAG 2.1 AA");
            assert_eq!(entries.len(), elysian_palette(theme).len());
            for entry in entries {
                assert!(entry.ratio >= entry.required);
            }
        }

        // Known WCAG sanity anchors: black on white is 21:1.
        let bw = contrast_ratio(SrgbColor::rgb(0, 0, 0), SrgbColor::rgb(255, 255, 255));
        assert!((bw - 21.0).abs() < 0.01);

        // Negative test: light grey text on white must fail NormalText (4.5:1).
        let failing_pair = [PalettePair {
            id: PalettePairId::SecondaryTextOnSurface,
            foreground: SrgbColor::rgb(0x99, 0x99, 0x99),
            background: SrgbColor::rgb(0xFF, 0xFF, 0xFF),
            requirement: ContrastRequirement::NormalText,
        }];
        let violations = verify_palette_wcag_aa(&failing_pair)
            .expect_err("low-contrast grey on white must fail WCAG AA");
        assert_eq!(violations.len(), 1);
        assert!(violations[0].actual_ratio < 4.5);
    }

    #[test]
    fn throttled_live_queue_caps_40_tok_per_sec_streaming_announcements() {
        let policy = CoalescingPolicy::new(750, 1500).expect("750..=1500ms is valid");
        let mut queue = LiveAnnouncementQueue::new(policy);

        // Simulate M1 15-second active window at 40 tokens/sec = 600 tokens (every 25ms).
        let mut now_ms = 0_u64;
        for token_idx in 0..600 {
            let token = if token_idx % 12 == 11 {
                "complete. "
            } else {
                "token "
            };
            let _ = queue.push_stream_token(token, now_ms);
            now_ms += 25;
        }
        let _ = queue.finish_stream(now_ms);

        let polite_count = queue.count_in_window(0, 15_000, AnnouncementPriority::Polite);
        // 15 seconds with >= 750ms coalescing floor must emit <= 20 polite announcements
        // (and >= 10 because max window is 1500ms), never 600 token-level floods!
        assert!(
            (10..=20).contains(&polite_count),
            "expected 10..=20 coalesced polite announcements over 15s at 40 tok/s, got {polite_count}"
        );

        // Verify assertive alert delivers immediately without losing pending polite tokens.
        let mut mixed_queue = LiveAnnouncementQueue::default();
        assert!(mixed_queue.push_stream_token("Synthetic ", 100).is_none());
        let alert = mixed_queue.announce_assertive("Validation error", 200);
        assert_eq!(alert.priority, AnnouncementPriority::Assertive);
        let flushed = mixed_queue
            .finish_stream(300)
            .expect("pending polite stream text must survive assertive alert");
        assert_eq!(flushed.text, "Synthetic");
    }

    #[test]
    fn bridge_diff_emits_zero_dom_patches_while_stream_tokens_coalesce() {
        let mut app = build_headless_app();
        // Second clean frame with no changes emits zero bridge patches.
        app.update();
        assert!(
            app.world()
                .resource::<NarrowSemanticBridge>()
                .last_patches()
                .is_empty(),
            "idle frame must emit zero DOM bridge mutations"
        );

        // Pushing coalesced tokens below the 750ms threshold emits zero DOM mutations.
        {
            let mut live = app.world_mut().resource_mut::<LiveAnnouncementQueue>();
            assert!(live.push_stream_token("Baseline ", 100).is_none());
            assert!(live.push_stream_token("vitals ", 125).is_none());
        }
        app.update();
        assert!(
            app.world()
                .resource::<NarrowSemanticBridge>()
                .last_patches()
                .is_empty(),
            "unflushed stream tokens must emit zero DOM mutations"
        );

        // Completing a sentence after >= 750ms emits only live-region patches.
        {
            let mut live = app.world_mut().resource_mut::<LiveAnnouncementQueue>();
            assert!(live.push_stream_token("stable.", 900).is_some());
        }
        app.update();
        let patches = app
            .world()
            .resource::<NarrowSemanticBridge>()
            .last_patches();
        assert!(!patches.is_empty());
        assert!(patches.iter().any(|patch| matches!(
            patch,
            BridgePatch::SetPoliteLive {
                text: Some(text)
            } if text == "Baseline vitals stable."
        )));
    }
}
