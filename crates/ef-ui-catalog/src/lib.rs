//! Shared Bevy `0.20.1` UI catalog serving both Curie Clinic and Curie Mobile.
//!
//! Provides WCAG 2.1 AA Light/Dark theme tokens, `ReducedMotion` & `IdleRedrawGate`,
//! single-owner `ComposerSurface` + `PlatformTextAdapter`, single-owner
//! `A11yProjection` + differential ARIA bridge patch emitter + `PoliteLiveQueue`,
//! O(viewport) `VirtualizedListView` & `VirtualizedTableView`, scoped keyboard
//! `ShortcutRouter`, and compile-time exhaustive 9-locale i18n strings.

pub mod a11y;
pub mod composer;
pub mod i18n;
pub mod motion;
pub mod plugin;
pub mod prefs;
pub mod shortcuts;
pub mod theme;
pub mod virtualized;

pub use a11y::{
    A11yAuditSummary, A11yNodeId, A11yNodeMarker, A11yProjection, A11yProjectionError,
    A11ySemanticKey, AriaBridgeEmitter, AriaRole, BridgePatch, CoalescingWindow,
    CoalescingWindowError, ELYSIAN_SUBPATH_PREFIX, ExpansionState, LiveAnnouncement,
    LivePoliteness, ModalScopeKind, NodeAvailability, NodeInteractivity, NodeVisibility,
    PoliteLiveQueue, ProjectedA11yNode, SelectionState, UnsupportedAriaRole,
};
pub use composer::{
    ComposerAction, ComposerOutcome, ComposerSurface, ImeCompositionState, MemoryTextAdapter,
    PlatformTextAdapter, SelectionRange, SoftKeyboardInsets, TextAdapterError,
    next_cluster_boundary, prev_cluster_boundary,
};
pub use i18n::{
    ChromeProduct, HorizontalEdge, LOCALE_STORAGE_KEY, Locale, LocalizedChrome, TextDirection,
    UiKey, UiLocale, localized_chrome, translate,
};
pub use motion::{
    AnimationMode, FrameDecision, IdleRedrawGate, MotionPolicy, RedrawReason, ReducedMotion,
};
pub use plugin::{CatalogA11ySyncStatus, EfUiCatalogPlugin};
pub use prefs::{
    DesktopOsFamily, DesktopPreferences, DesktopPrefsError, DesktopPrefsStore, DesktopZoom,
    product_prefs_filename, resolve_desktop_prefs_path, resolve_prefs_dir_for_env,
};
pub use shortcuts::{
    ActiveOverlay, EditingContext, FocusId, ModifierChord, OverlayKind, OverlayModality,
    PrivacyCoverState, SearchQueryState, ShortcutAction, ShortcutContext, ShortcutInput,
    ShortcutKey, ShortcutRouter,
};
pub use theme::{
    ContrastAuditEntry, ContrastRequirement, ContrastViolation, SrgbColor, TextScale,
    TextScaleError, ThemeContrastPair, ThemeMode, ThemePairId, ThemeTokens, UiTheme,
    contrast_ratio, relative_luminance, verify_contrast_pairs, verify_theme_wcag_aa,
};
pub use virtualized::{
    ColumnAlignment, TableColumn, VirtualizedListView, VirtualizedTableView, VisibleRange,
    compute_visible_range,
};

#[cfg(test)]
mod tests {
    use bevy::a11y::AccessibilityNode;
    use bevy::input_focus::{InputFocus, InputFocusVisible};
    use bevy::prelude::{App, MinimalPlugins, Node};

    use super::*;

    fn build_catalog_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, EfUiCatalogPlugin));
        app
    }

    #[test]
    fn wcag_21_aa_themes_pass_light_and_dark_and_reject_bad_contrast() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let entries = verify_theme_wcag_aa(mode)
                .expect("every semantic pair in Light and Dark themes must meet WCAG 2.1 AA");
            assert_eq!(entries.len(), 13);
            let theme = UiTheme::new(mode, TextScale::DOUBLE);
            assert!(theme.focus_ring_meets_spec());
        }

        let bad_pair = [ThemeContrastPair {
            id: ThemePairId::SecondaryTextOnSurface,
            foreground: SrgbColor::rgb(0xA0, 0xA0, 0xA0),
            background: SrgbColor::rgb(0xFF, 0xFF, 0xFF),
            requirement: ContrastRequirement::NormalText,
        }];
        let violations = verify_contrast_pairs(&bad_pair).expect_err("low contrast must fail");
        assert_eq!(violations.len(), 1);
        assert!(violations[0].actual_ratio < 4.5);
    }

    #[test]
    fn reduced_motion_and_idle_redraw_gate_render_zero_frames_when_idle() {
        let reduced = MotionPolicy::for_preference(ReducedMotion::Reduce);
        assert!(!reduced.allows_non_essential_motion());
        assert_eq!(reduced.overlay_transition_ms, 0);
        assert_eq!(reduced.streaming_indicator, AnimationMode::Static);
        assert!(
            (MotionPolicy::progress(0, reduced.overlay_transition_ms) - 1.0).abs() < f32::EPSILON
        );

        let mut gate = IdleRedrawGate::default();
        // Decorative ticks under ReducedMotion::Reduce must never wake the renderer.
        for _ in 0..300 {
            gate.request_decorative_tick(reduced);
            assert_eq!(gate.consume_frame(), FrameDecision::SkipIdle);
        }
        assert_eq!(gate.frames_rendered(), 0);
        assert_eq!(gate.idle_ticks_skipped(), 300);

        // Concrete input wakes exactly one frame, then immediately returns to idle.
        gate.request_redraw(RedrawReason::UserInput);
        assert_eq!(
            gate.consume_frame(),
            FrameDecision::Render {
                reasons: vec![RedrawReason::UserInput]
            }
        );
        assert_eq!(gate.frames_rendered(), 1);
        assert_eq!(gate.consume_frame(), FrameDecision::SkipIdle);
    }

    #[test]
    fn nine_locale_catalog_is_exhaustive_and_resolves_priority_with_rtl_mirroring() {
        // Priority: ?lang > persisted > OS > fallback en.
        assert_eq!(
            Locale::resolve(Some("ar-EG"), Some("fr"), &["de-DE"]),
            Locale::Ar
        );
        assert_eq!(Locale::parse_bcp47("he-IL"), Some(Locale::He));
        assert_eq!(Locale::parse_bcp47("iw"), Some(Locale::He));
        assert_eq!(Locale::parse_bcp47("vi-VN"), Some(Locale::Vi));
        assert_eq!(Locale::parse_bcp47("hi-IN"), Some(Locale::Hi));
        assert_eq!(
            Locale::resolve(Some("unknown"), Some("zh_Hans"), &["ja-JP"]),
            Locale::ZhHans
        );
        assert_eq!(
            Locale::resolve(None, None, &["xx-YY", "es-MX", "en-US"]),
            Locale::Es
        );
        assert_eq!(Locale::resolve(None, None, &["xx-YY"]), Locale::En);

        // RTL mirroring for BOTH Arabic and Hebrew vs LTR for all other 10 locales.
        for rtl_locale in [Locale::Ar, Locale::He] {
            assert!(rtl_locale.is_rtl());
            assert_eq!(rtl_locale.direction(), TextDirection::Rtl);
            assert_eq!(
                rtl_locale.row_flex_direction(),
                bevy::ui::FlexDirection::RowReverse
            );
            assert_eq!(rtl_locale.sidebar_edge(), HorizontalEdge::Right);
            assert_eq!(rtl_locale.citation_panel_edge(), HorizontalEdge::Left);
        }
        for ltr_locale in [
            Locale::En,
            Locale::ZhHans,
            Locale::Ja,
            Locale::Vi,
            Locale::Hi,
        ] {
            assert!(!ltr_locale.is_rtl());
            assert_eq!(ltr_locale.sidebar_edge(), HorizontalEdge::Left);
            assert_eq!(ltr_locale.citation_panel_edge(), HorizontalEdge::Right);
        }

        // Compile-time exhaustive check: all 12 locales x 107 keys (1,284 entries) are non-empty.
        assert_eq!(Locale::ALL.len(), 12);
        assert_eq!(UiKey::ALL.len(), 107);
        for locale in Locale::ALL {
            for key in UiKey::ALL {
                let text = translate(locale, key);
                assert!(
                    !text.trim().is_empty(),
                    "empty translation for {locale:?} / {key:?}"
                );
            }
            for streaming_key in [
                UiKey::Cancel,
                UiKey::Retry,
                UiKey::Streaming,
                UiKey::Cancelled,
            ] {
                let text = translate(locale, streaming_key);
                assert!(!text.trim().is_empty());
                assert!(!text.contains("TODO"));
                assert!(!text.contains("TBD"));
            }
            assert!(translate(locale, UiKey::MobileCrisisLine988).contains("988"));
            assert!(translate(locale, UiKey::MobileCrisisLine911).contains("911"));
            assert!(translate(locale, UiKey::MobileCrisisLineSamhsa).contains("1-800-662-4357"));
            let clinic_chrome = localized_chrome(locale, ChromeProduct::Clinic);
            let mobile_chrome = localized_chrome(locale, ChromeProduct::Mobile);
            assert!(!clinic_chrome.app_brand.is_empty());
            assert!(!mobile_chrome.app_brand.is_empty());
            assert!(mobile_chrome.crisis_heading.contains("988"));
            assert!(mobile_chrome.crisis_heading.contains("911"));
            assert!(mobile_chrome.crisis_heading.contains("SAMHSA"));
        }
    }

    #[test]
    fn composer_supports_multiline_ime_rtl_indic_clusters_undo_redo_and_clipboard() {
        let mut composer = ComposerSurface::new(TextDirection::Ltr);
        let mut adapter = MemoryTextAdapter::default();

        // Multiline Shift+Enter vs Enter submit.
        assert_eq!(
            composer.apply(ComposerAction::InsertText("Line one".to_owned())),
            ComposerOutcome::Edited
        );
        assert_eq!(
            composer.apply(ComposerAction::ShiftEnterNewline),
            ComposerOutcome::Edited
        );
        assert_eq!(
            composer.apply(ComposerAction::InsertText("Line two".to_owned())),
            ComposerOutcome::Edited
        );
        assert_eq!(composer.text(), "Line one\nLine two");

        // Clipboard copy / cut / paste via PlatformTextAdapter.
        let _ = composer.apply(ComposerAction::SelectAll);
        assert!(composer.copy_to_adapter(&mut adapter).expect("copy ok"));
        assert_eq!(adapter.clipboard, "Line one\nLine two");
        assert!(composer.cut_to_adapter(&mut adapter).expect("cut ok"));
        assert_eq!(composer.text(), "");
        assert_eq!(
            composer.paste_from_adapter(&mut adapter).expect("paste ok"),
            ComposerOutcome::Edited
        );
        assert_eq!(composer.text(), "Line one\nLine two");

        // Undo & Redo.
        assert_eq!(
            composer.apply(ComposerAction::Undo),
            ComposerOutcome::Edited
        );
        assert_eq!(composer.text(), "");
        assert_eq!(
            composer.apply(ComposerAction::Redo),
            ComposerOutcome::Edited
        );
        assert_eq!(composer.text(), "Line one\nLine two");

        // Enter submits and clears buffer.
        assert_eq!(
            composer.apply(ComposerAction::EnterSubmit),
            ComposerOutcome::Submitted("Line one\nLine two".to_owned())
        );
        assert_eq!(composer.text(), "");

        // OS IME composition (CJK) pauses Enter submission until committed.
        let _ = composer.apply(ComposerAction::ImePreedit {
            preedit: "かんじ".to_owned(),
            cursor_offset: 3,
        });
        assert_eq!(composer.display_text(), "かんじ");
        assert_eq!(
            composer.apply(ComposerAction::EnterSubmit),
            ComposerOutcome::NoOp
        );
        let _ = composer.apply(ComposerAction::ImeCommit("漢字".to_owned()));
        assert_eq!(composer.text(), "漢字");
        assert_eq!(composer.ime_state(), &ImeCompositionState::Inactive);

        // Indic akshara cluster navigation (Devanagari "क्ष" = क + ् + ष as one step).
        let mut indic = ComposerSurface::new(TextDirection::Ltr);
        let _ = indic.apply(ComposerAction::InsertText("क्षेत्र".to_owned()));
        let len = indic.text().len();
        let prev_after_tra = prev_cluster_boundary(indic.text(), len);
        assert_eq!(&indic.text()[prev_after_tra..], "त्र");
        let prev_after_kshe = prev_cluster_boundary(indic.text(), prev_after_tra);
        assert_eq!(prev_after_kshe, 0);
        assert_eq!(&indic.text()[..prev_after_tra], "क्षे");

        // Arabic RTL + BiDi lab units + soft-keyboard viewport inset.
        let mut arabic = ComposerSurface::new(TextDirection::Rtl);
        let _ = arabic.apply(ComposerAction::InsertText(
            "الكرياتينين 54 mL/min/1.73m²".to_owned(),
        ));
        arabic.set_soft_keyboard_insets(SoftKeyboardInsets {
            bottom_inset_px: 310.0,
        });
        assert!((arabic.visible_height_above_keyboard(844.0) - 534.0).abs() < 0.01);

        // Hebrew RTL with Nikud vowel points (`שָׁלוֹם`, `מְטוּפָּל`): never split mid-diacritic.
        let mut hebrew = ComposerSurface::new(TextDirection::Rtl);
        let _ = hebrew.apply(ComposerAction::InsertText("שָׁלוֹם".to_owned()));
        let first_end = next_cluster_boundary(hebrew.text(), 0);
        assert_eq!(&hebrew.text()[..first_end], "שָׁ");
        assert_eq!(
            hebrew.apply(ComposerAction::Backspace),
            ComposerOutcome::Edited
        );
        assert_eq!(hebrew.text(), "שָׁלוֹ");

        let mut hebrew_patient = ComposerSurface::new(TextDirection::Rtl);
        let _ = hebrew_patient.apply(ComposerAction::InsertText("מְטוּפָּל".to_owned()));
        let m_end = next_cluster_boundary(hebrew_patient.text(), 0);
        assert_eq!(&hebrew_patient.text()[..m_end], "מְ");
        let t_end = next_cluster_boundary(hebrew_patient.text(), m_end);
        assert_eq!(&hebrew_patient.text()[m_end..t_end], "ט");
        let vav_end = next_cluster_boundary(hebrew_patient.text(), t_end);
        assert_eq!(&hebrew_patient.text()[t_end..vav_end], "וּ");
        let p_end = next_cluster_boundary(hebrew_patient.text(), vav_end);
        assert_eq!(&hebrew_patient.text()[vav_end..p_end], "פָּ");

        // Vietnamese stacked diacritics (precomposed & decomposed) stay intact.
        let mut viet = ComposerSurface::new(TextDirection::Ltr);
        let _ = viet.apply(ComposerAction::InsertText("Nguyễn Thị Hương".to_owned()));
        assert_eq!(viet.text(), "Nguyễn Thị Hương");
        let decomposed_vi = "e\u{0302}\u{0303}";
        assert_eq!(next_cluster_boundary(decomposed_vi, 0), decomposed_vi.len());

        // Modern Hindi Devanagari virama conjuncts (`चिकित्सा मूल्यांकन`, `रक्तचाप`).
        let mut hindi = ComposerSurface::new(TextDirection::Ltr);
        let _ = hindi.apply(ComposerAction::InsertText("रक्तचाप".to_owned()));
        let ra_end = next_cluster_boundary(hindi.text(), 0);
        let kta_end = next_cluster_boundary(hindi.text(), ra_end);
        assert_eq!(&hindi.text()[..ra_end], "र");
        assert_eq!(&hindi.text()[ra_end..kta_end], "क्त");
    }

    #[test]
    fn virtualized_list_and_5000_row_table_stay_o_viewport() {
        let mut list = VirtualizedListView::new(5_000, 600.0, 40.0, 4);
        let initial = list.visible_range();
        // 600 / 40 = 15 visible + 1 + 4 overscan = 20 rows (never 5,000!).
        assert!(initial.materialized_count() <= 25);
        assert_eq!(initial.start_row, 0);

        list.scroll_to(100_000.0);
        let middle = list.visible_range();
        assert!(middle.materialized_count() <= 25);
        assert!(middle.start_row > 2_000);
        assert!(middle.top_spacer_px > 0.0);
        assert!(middle.bottom_spacer_px > 0.0);

        let columns = vec![
            TableColumn {
                id: "analyte".to_owned(),
                header_key: UiKey::ClinicLabRecordsHeading,
                width_px: 220.0,
                alignment: ColumnAlignment::Leading,
            },
            TableColumn {
                id: "value".to_owned(),
                header_key: UiKey::ClinicCitedSourcesHeading,
                width_px: 120.0,
                alignment: ColumnAlignment::Trailing,
            },
        ];
        let rows: Vec<u32> = (0..5_000).collect();
        let mut table = VirtualizedTableView::new(columns, rows.len(), 480.0, 32.0, 3);
        table.scroll_vertical_to(64_000.0);
        let (range, slice) = table.visible_rows_slice(&rows);
        assert_eq!(slice.len(), range.materialized_count());
        assert!(slice.len() <= 24);
        assert_eq!(slice[0], range.start_row as u32);
    }

    #[test]
    fn shortcut_router_enforces_global_privacy_cover_scopes_and_escape_restoration() {
        let initial_focus = FocusId(A11ySemanticKey::CitationChip(2));
        let mut router = ShortcutRouter::new(initial_focus.clone());

        // Single-key shortcuts pause while editing text.
        let editing = ShortcutContext {
            editing: EditingContext::ActiveTextField,
            search_query: SearchQueryState::Empty,
        };
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::Char('/'), ModifierChord::None),
                editing
            ),
            ShortcutAction::Unhandled
        );
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::Char('j'), ModifierChord::None),
                editing
            ),
            ShortcutAction::Unhandled
        );

        // Cmd/Ctrl+Shift+L is GLOBAL: fires even while editing text, then restores exact focus!
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::Char('l'), ModifierChord::PrimaryShift),
                editing
            ),
            ShortcutAction::PrivacyCoverActivated
        );
        assert!(router.is_privacy_cover_active());
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::Char('L'), ModifierChord::PrimaryShift),
                editing
            ),
            ShortcutAction::PrivacyCoverDismissed {
                restored_focus: initial_focus.clone()
            }
        );
        assert_eq!(router.current_focus(), &initial_focus);

        // Workspace navigation when not editing.
        let workspace = ShortcutContext::default();
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::Char('j'), ModifierChord::None),
                workspace
            ),
            ShortcutAction::ListMoveNext
        );
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::ArrowUp, ModifierChord::None),
                workspace
            ),
            ShortcutAction::ListMovePrevious
        );
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::Enter, ModifierChord::None),
                workspace
            ),
            ShortcutAction::ActivateFocused {
                target: initial_focus.clone()
            }
        );

        // `/` opens PatientFinder modal; `Cmd+K` and `?` are paused inside modals.
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::Char('/'), ModifierChord::None),
                workspace
            ),
            ShortcutAction::OpenPatientFinder {
                restore_focus: initial_focus.clone()
            }
        );
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::Char('k'), ModifierChord::Primary),
                workspace
            ),
            ShortcutAction::Unhandled
        );
        // First Escape clears non-empty search query; second Escape dismisses overlay & restores focus.
        let with_query = ShortcutContext {
            editing: EditingContext::ActiveTextField,
            search_query: SearchQueryState::NonEmpty,
        };
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::Escape, ModifierChord::None),
                with_query
            ),
            ShortcutAction::ClearSearchInput
        );
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::Escape, ModifierChord::None),
                workspace
            ),
            ShortcutAction::DismissOverlay {
                dismissed: OverlayKind::PatientFinder,
                restored_focus: initial_focus.clone()
            }
        );
        assert_eq!(router.current_focus(), &initial_focus);

        // `?` opens ShortcutsHelp and `Cmd+Shift+L` STILL works inside that modal.
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::Char('?'), ModifierChord::Shift),
                workspace
            ),
            ShortcutAction::OpenShortcutsHelp {
                restore_focus: initial_focus
            }
        );
        assert_eq!(
            router.route(
                ShortcutInput::new(ShortcutKey::Char('l'), ModifierChord::PrimaryShift),
                workspace
            ),
            ShortcutAction::PrivacyCoverActivated
        );
    }

    #[test]
    fn headless_bevy_plugin_projects_single_owner_a11y_bridge_and_throttles_live_stream() {
        let mut app = build_catalog_app();

        // Spawn interactive Bevy UI nodes with AccessibilityNode components.
        let mut row0 = accesskit::Node::new(accesskit::Role::Button);
        row0.set_label("Synthetic Patient Alpha");
        let mut send_btn = accesskit::Node::new(accesskit::Role::Button);
        send_btn.set_label("Send");
        let mut composer_node = accesskit::Node::new(accesskit::Role::TextInput);
        composer_node.set_label("Ask a clinical question");

        let row0_entity = app
            .world_mut()
            .spawn((
                Node::default(),
                A11yNodeMarker {
                    key: A11ySemanticKey::ListRow(0),
                    reading_order: 0,
                    interactivity: NodeInteractivity::Interactive,
                },
                AccessibilityNode(row0),
            ))
            .id();
        let _ = app.world_mut().spawn((
            Node::default(),
            A11yNodeMarker {
                key: A11ySemanticKey::ComposerInput,
                reading_order: 1,
                interactivity: NodeInteractivity::Interactive,
            },
            AccessibilityNode(composer_node),
        ));
        let _ = app.world_mut().spawn((
            Node::default(),
            A11yNodeMarker {
                key: A11ySemanticKey::SendButton,
                reading_order: 2,
                interactivity: NodeInteractivity::Interactive,
            },
            AccessibilityNode(send_btn),
        ));

        // Switch locale to Arabic RTL and run a frame.
        app.world_mut().resource_mut::<UiLocale>().current = Locale::Ar;
        app.update();

        assert!(
            app.world()
                .resource::<CatalogA11ySyncStatus>()
                .errors
                .is_empty()
        );
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(row0_entity)
        );
        assert!(app.world().resource::<InputFocusVisible>().0);
        assert_eq!(
            app.world().resource::<ComposerSurface>().direction(),
            TextDirection::Rtl
        );

        let audit = app
            .world()
            .resource::<A11yProjection>()
            .verify_zero_duplicate_nodes()
            .expect("projected AccessKit tree must have zero duplicates");
        assert_eq!(audit.duplicated_nodes, 0);
        assert_eq!(audit.mirrored_css_rules, 0);
        assert_eq!(audit.interactive_nodes, 3);

        // Initial frame emits JSON-serializable ARIA patches including Arabic RTL locale.
        let json = app
            .world()
            .resource::<AriaBridgeEmitter>()
            .serialize_last_patches()
            .expect("patches must serialize cleanly");
        assert!(json.contains("\"SetDocumentLocale\""));
        assert!(json.contains("\"ar\""));

        // Idle second frame emits ZERO DOM bridge patches.
        app.update();
        assert!(
            app.world()
                .resource::<AriaBridgeEmitter>()
                .last_patches()
                .is_empty()
        );

        // Throttled 40 tok/s stream (600 chunks / 15s) stays capped to 10..=20 polite announcements.
        {
            let mut live = app.world_mut().resource_mut::<PoliteLiveQueue>();
            for idx in 0..600_u64 {
                let token = if idx % 12 == 11 { "done. " } else { "word " };
                let _ = live.push_stream_chunk(token, idx * 25);
            }
            let _ = live.finish_stream(15_000);
        }
        let polite_count = app.world().resource::<PoliteLiveQueue>().count_in_window(
            0,
            15_000,
            LivePoliteness::Polite,
        );
        assert!(
            (10..=20).contains(&polite_count),
            "expected 10..=20 throttled announcements over 15s, got {polite_count}"
        );

        // Negative test: duplicate node injection must be rejected by the invariant verifier.
        let mut duplicate_nodes = app.world().resource::<A11yProjection>().nodes.clone();
        duplicate_nodes.push(duplicate_nodes[0].clone());
        let bad_projection = A11yProjection::from_nodes(duplicate_nodes, None, Locale::En);
        let errs = bad_projection
            .verify_zero_duplicate_nodes()
            .expect_err("duplicate accessibility node must fail verification");
        assert!(
            errs.iter()
                .any(|e| matches!(e, A11yProjectionError::DuplicateNodeId { .. }))
        );
        assert!(errs.iter().any(|e| matches!(
            e,
            A11yProjectionError::DuplicateSemanticKey { count: 2, .. }
        )));
    }
}
