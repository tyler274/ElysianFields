use bevy::prelude::*;
use m1_text_i18n::{
    ActiveTranscriptTurnViewNode, AdapterMechanism, AdapterReadback, AdapterReadbackState,
    ClipboardMutation, CommandOutcome, ComposerDocument, ComposerRuntimeMetrics, ComposerState,
    ComposerViewNode, CompositionPresence, CursorState, DictationSegment, DictationSpacing,
    FocusState, FontFamily, FontLicense, HeadlessSimulatedAdapter, ImeComposition, Locale,
    LocaleTier, MoveDirection, NoOpReason, PlatformInputAdapter, PlatformTarget, ScrollAdjustment,
    SelectionBehavior, SelectionPresence, SoftKeyboardState, StreamingTypingBenchmarkConfig,
    TextCommandMessage, TextDirection, TextI18nPrototypePlugin, TextInputCommand,
    TranscriptChunkMessage, TranscriptFeedState, ViewportMetrics,
    build_synthetic_80_turn_transcript, cluster_boundaries, run_streaming_typing_benchmark,
};

#[test]
fn locale_catalog_covers_all_shipped_and_shaping_probe_locales() {
    assert_eq!(Locale::SHIPPED_UI.len(), 12);
    assert_eq!(Locale::SHAPING_PROBES.len(), 5);
    assert_eq!(Locale::ALL.len(), 17);

    for locale in Locale::ALL {
        let meta = locale.metadata();
        assert_eq!(
            Locale::parse_bcp47(meta.bcp47).expect("parse canonical tag"),
            locale
        );
        assert!(!locale.synthetic_sample().is_empty());

        let font_spec = meta.primary_font.bundle_spec();
        assert_eq!(font_spec.license, FontLicense::SilOfl1_1);
        assert!(font_spec.compressed_kib > 0);
    }

    assert_eq!(Locale::Ar.direction(), TextDirection::Rtl);
    assert_eq!(Locale::He.direction(), TextDirection::Rtl);
    assert_eq!(Locale::Ur.direction(), TextDirection::Rtl);
    assert_eq!(Locale::En.direction(), TextDirection::Ltr);
    assert_eq!(Locale::Vi.direction(), TextDirection::Ltr);
    assert_eq!(Locale::Hi.direction(), TextDirection::Ltr);
    assert_eq!(Locale::parse_bcp47("iw"), Ok(Locale::He));
    assert_eq!(Locale::parse_bcp47("he-IL"), Ok(Locale::He));
    assert_eq!(Locale::parse_bcp47("vi-VN"), Ok(Locale::Vi));
    assert_eq!(Locale::He.primary_font(), FontFamily::NotoSansHebrew);
    assert_eq!(Locale::Vi.primary_font(), FontFamily::NotoSans);
    assert_eq!(Locale::Ur.metadata().tier, LocaleTier::ShapingProbe);
    assert_eq!(Locale::En.metadata().tier, LocaleTier::ShippedUi);
    assert_eq!(Locale::He.metadata().tier, LocaleTier::ShippedUi);
    assert_eq!(Locale::Vi.metadata().tier, LocaleTier::ShippedUi);
    assert_eq!(Locale::Hi.metadata().tier, LocaleTier::ShippedUi);

    let err = Locale::parse_bcp47("xx-INVALID").expect_err("unknown locale must fail");
    assert_eq!(err.invalid_tag(), "xx-INVALID");

    assert!(FontFamily::core_eager_bundle_kib() < 512);
    assert!(FontFamily::total_all_scripts_kib() > FontFamily::core_eager_bundle_kib());
}

#[test]
fn hebrew_nikud_vietnamese_diacritics_and_hindi_conjuncts_preserve_clusters_and_bidi() {
    use m1_text_i18n::{detect_paragraph_direction, next_cluster_boundary, prev_cluster_boundary};

    // Hebrew Nikud vowel-pointed words (`שָׁלוֹם`, `מְטוּפָּל`) preserve orthographic clusters and detect RTL.
    let shalom = "שָׁלוֹם";
    assert_eq!(
        detect_paragraph_direction(shalom, TextDirection::Ltr),
        TextDirection::Rtl
    );
    let shalom_bounds = cluster_boundaries(shalom);
    assert_eq!(
        shalom_bounds.len(),
        5,
        "שָׁ + ל + וֹ + ם = 4 clusters (5 boundaries)"
    );
    assert_eq!(&shalom[shalom_bounds[0]..shalom_bounds[1]], "שָׁ");

    let mut he_doc = ComposerDocument::new(Locale::He);
    he_doc.apply(TextInputCommand::Focus);
    he_doc.apply(TextInputCommand::InsertText("מְטוּפָּל HbA1c 6.8%".into()));
    assert_eq!(he_doc.effective_direction(), TextDirection::Rtl);
    // Backspacing through `מְטוּפָּל` after selecting just the word deletes whole nikud clusters.
    let mut word_doc = ComposerDocument::new(Locale::He);
    word_doc.apply(TextInputCommand::Focus);
    word_doc.apply(TextInputCommand::InsertText("מְטוּפָּל".into()));
    word_doc.apply(TextInputCommand::Backspace);
    assert_eq!(word_doc.text(), "מְטוּפָּ");
    word_doc.apply(TextInputCommand::Backspace);
    assert_eq!(word_doc.text(), "מְטוּ");

    // Vietnamese stacked diacritics detect LTR even when starting with Latin Extended Additional.
    let vi = "Ứng dụng — Nguyễn Thị Hương";
    assert_eq!(
        detect_paragraph_direction(vi, TextDirection::Rtl),
        TextDirection::Ltr
    );
    let vi_nfd = "Tie\u{0302}\u{0301}ng";
    let t_end = next_cluster_boundary(vi_nfd, m1_text_i18n::ByteOffset(0));
    let i_end = next_cluster_boundary(vi_nfd, t_end);
    let e_end = next_cluster_boundary(vi_nfd, i_end);
    assert_eq!(
        &vi_nfd[i_end.as_usize()..e_end.as_usize()],
        "e\u{0302}\u{0301}"
    );
    assert_eq!(prev_cluster_boundary(vi_nfd, e_end), i_end);

    // Modern Hindi Devanagari virama conjuncts (`चिकित्सा मूल्यांकन`, `रक्तचाप`).
    let hi = "चिकित्सा मूल्यांकन · रक्तचाप";
    assert_eq!(
        detect_paragraph_direction(hi, TextDirection::Rtl),
        TextDirection::Ltr
    );
    let rakta_bounds = cluster_boundaries("रक्तचाप");
    assert_eq!(&"रक्तचाप"[rakta_bounds[1]..rakta_bounds[2]], "क्त");
}

#[test]
fn multiline_typing_selection_clipboard_and_bounded_undo_redo() {
    let mut doc = ComposerDocument::with_config(Locale::En, ViewportMetrics::default(), 4);

    // Commands while unfocused are rejected cleanly.
    assert_eq!(
        doc.apply(TextInputCommand::InsertText("ignored".into())),
        CommandOutcome::NoOp(NoOpReason::UnfocusedIgnored)
    );

    assert!(matches!(
        doc.apply(TextInputCommand::Focus),
        CommandOutcome::FocusChanged {
            state: FocusState::Focused,
            ..
        }
    ));

    doc.apply(TextInputCommand::InsertText("Line one".into()));
    doc.apply(TextInputCommand::InsertNewline);
    doc.apply(TextInputCommand::InsertText("Line two".into()));
    assert_eq!(doc.text(), "Line one\nLine two");
    assert_eq!(doc.layout_snapshot().total_lines, 2);

    // Move to line start, select "Line", copy, then cut.
    doc.apply(TextInputCommand::MoveCaret {
        direction: MoveDirection::LineStart,
        selection: SelectionBehavior::Collapse,
    });
    for _ in 0..4 {
        doc.apply(TextInputCommand::MoveCaret {
            direction: MoveDirection::VisualRight,
            selection: SelectionBehavior::Extend,
        });
    }

    let copy_outcome = doc.apply(TextInputCommand::Copy);
    assert_eq!(
        copy_outcome,
        CommandOutcome::ClipboardProduced {
            text: "Line".into(),
            mutation: ClipboardMutation::CopiedOnly,
        }
    );
    assert_eq!(doc.text(), "Line one\nLine two");

    let cut_outcome = doc.apply(TextInputCommand::Cut);
    assert!(matches!(
        cut_outcome,
        CommandOutcome::ClipboardProduced {
            ref text,
            mutation: ClipboardMutation::CutApplied { .. }
        } if text == "Line"
    ));
    assert_eq!(doc.text(), "Line one\n two");

    // Paste with CRLF normalization.
    doc.apply(TextInputCommand::Paste("Entry\r\nRow".into()));
    assert_eq!(doc.text(), "Line one\nEntry\nRow two");
    assert_eq!(doc.layout_snapshot().total_lines, 3);

    // Undo and redo round-trip (bounded at max depth 4).
    assert!(doc.undo_depth() <= 4);
    assert!(matches!(
        doc.apply(TextInputCommand::Undo),
        CommandOutcome::DocumentChanged { .. }
    ));
    assert_eq!(doc.text(), "Line one\n two");

    assert!(matches!(
        doc.apply(TextInputCommand::Redo),
        CommandOutcome::DocumentChanged { .. }
    ));
    assert_eq!(doc.text(), "Line one\nEntry\nRow two");

    // Select all and replace.
    doc.apply(TextInputCommand::SelectAll);
    doc.apply(TextInputCommand::InsertText("Cleared".into()));
    assert_eq!(doc.text(), "Cleared");
    assert_eq!(doc.redo_depth(), 0);
}

#[test]
fn cjk_ime_preedit_candidate_anchor_commit_and_cancel() {
    let mut doc = ComposerDocument::new(Locale::Ja);
    doc.apply(TextInputCommand::Focus);
    doc.apply(TextInputCommand::InsertText("診察：".into()));
    let base_caret = doc.caret_rect();
    let rev_before = doc.revision();

    // Start IME preedit and update candidate anchor.
    assert!(matches!(
        doc.apply(TextInputCommand::ImeStart),
        CommandOutcome::PreeditUpdated { .. }
    ));

    let preedit_outcome = doc.apply(TextInputCommand::ImeUpdatePreedit {
        text: "けつあつ".into(),
        cursor_offset: Some("けつ".len()),
    });
    let anchor = match preedit_outcome {
        CommandOutcome::PreeditUpdated { anchor } => anchor,
        other => panic!("expected PreeditUpdated, got {other:?}"),
    };
    assert!(
        anchor.x > base_caret.x,
        "candidate anchor should advance right in LTR CJK preedit"
    );
    assert_eq!(
        doc.text(),
        "診察：",
        "preedit must not mutate committed buffer"
    );
    assert_eq!(doc.display_text(), "診察：けつあつ");
    assert_eq!(doc.revision(), rev_before);

    let readback = AdapterReadback::from_document(&doc);
    assert_eq!(readback.composition, CompositionPresence::Active);
    assert_eq!(readback.preedit_anchor, Some(anchor));

    // Commit kanji candidate.
    assert!(matches!(
        doc.apply(TextInputCommand::ImeCommit("血圧".into())),
        CommandOutcome::DocumentChanged { .. }
    ));
    assert_eq!(doc.text(), "診察：血圧");
    assert_eq!(doc.ime(), &ImeComposition::Idle);

    // Start and cancel a second Chinese/Korean preedit without changing committed buffer.
    doc.apply(TextInputCommand::SetLocale(Locale::ZhHans));
    let rev_after_commit = doc.revision();
    doc.apply(TextInputCommand::ImeStart);
    doc.apply(TextInputCommand::ImeUpdatePreedit {
        text: "xueya".into(),
        cursor_offset: None,
    });
    assert_eq!(doc.display_text(), "診察：血圧xueya");

    assert!(matches!(
        doc.apply(TextInputCommand::ImeCancel),
        CommandOutcome::PreeditUpdated { .. }
    ));
    assert_eq!(doc.ime(), &ImeComposition::Idle);
    assert_eq!(doc.text(), "診察：血圧");
    assert_eq!(doc.revision(), rev_after_commit);
}

#[test]
fn arabic_and_urdu_rtl_directionality_and_visual_caret_motion() {
    for locale in [Locale::Ar, Locale::Ur] {
        let mut doc = ComposerDocument::new(locale);
        doc.apply(TextInputCommand::Focus);
        let sample = locale.synthetic_sample();
        doc.apply(TextInputCommand::InsertText(sample.into()));
        assert_eq!(doc.effective_direction(), TextDirection::Rtl);

        // Move to logical start of line (rightmost visual position in RTL).
        doc.apply(TextInputCommand::MoveCaret {
            direction: MoveDirection::LineStart,
            selection: SelectionBehavior::Collapse,
        });
        let start_caret = doc.caret_rect();
        assert_eq!(doc.cursor().focus().as_usize(), 0);

        // VisualLeft in RTL moves logically FORWARD (offset increases, visual x decreases).
        doc.apply(TextInputCommand::MoveCaret {
            direction: MoveDirection::VisualLeft,
            selection: SelectionBehavior::Collapse,
        });
        let after_left = doc.caret_rect();
        assert!(
            doc.cursor().focus().as_usize() > 0,
            "VisualLeft in RTL must advance logical offset for {locale:?}"
        );
        assert!(
            after_left.x < start_caret.x,
            "VisualLeft in RTL must move caret leftward on screen for {locale:?}"
        );

        // VisualRight in RTL moves logically BACKWARD back to offset 0.
        doc.apply(TextInputCommand::MoveCaret {
            direction: MoveDirection::VisualRight,
            selection: SelectionBehavior::Collapse,
        });
        assert_eq!(doc.cursor().focus().as_usize(), 0);
    }

    // Arabic combining harakat stay bound to their base consonant as one cluster.
    let vocalized = "كِتَابٌ";
    let mut doc = ComposerDocument::new(Locale::Ar);
    doc.apply(TextInputCommand::Focus);
    doc.apply(TextInputCommand::InsertText(vocalized.into()));
    doc.apply(TextInputCommand::Backspace);
    assert_eq!(
        doc.text(),
        "كِتَا",
        "Backspace must delete base+tanwin as one cluster"
    );
}

#[test]
fn indic_scripts_preserve_conjuncts_and_combining_marks() {
    let indic_cases = [
        (Locale::Hi, "क्षा"), // Devanagari ka + virama + ssa + aa-matra = 1 cluster
        (Locale::Mr, "क्ति"), // Devanagari ka + virama + ta + i-matra = 1 cluster
        (Locale::Bn, "ক্ষি"), // Bengali ka + virama + ssa + i-matra = 1 cluster
        (Locale::Ta, "க்ஷி"), // Tamil ka + pulli + ssa + i-matra = 1 cluster
        (Locale::Te, "క్షి"),  // Telugu ka + virama + ssa + i-matra = 1 cluster
    ];

    for (locale, akshara) in indic_cases {
        let boundaries = cluster_boundaries(akshara);
        assert_eq!(
            boundaries,
            vec![0, akshara.len()],
            "{locale:?} conjunct '{akshara}' must remain a single orthographic cluster"
        );

        let mut doc = ComposerDocument::new(locale);
        doc.apply(TextInputCommand::Focus);
        doc.apply(TextInputCommand::InsertText(format!("A{akshara}B")));

        // Move left from end over 'B', then over the entire Indic akshara in one step.
        doc.apply(TextInputCommand::MoveCaret {
            direction: MoveDirection::VisualLeft,
            selection: SelectionBehavior::Collapse,
        });
        assert_eq!(doc.cursor().focus().as_usize(), 1 + akshara.len());

        doc.apply(TextInputCommand::MoveCaret {
            direction: MoveDirection::VisualLeft,
            selection: SelectionBehavior::Collapse,
        });
        assert_eq!(
            doc.cursor().focus().as_usize(),
            1,
            "Caret must step cleanly across whole {locale:?} akshara without splitting"
        );

        // Delete forward removes the full akshara cleanly without orphaned virama/matra.
        doc.apply(TextInputCommand::DeleteForward);
        assert_eq!(doc.text(), "AB");
    }
}

#[test]
fn dictation_respects_locale_word_spacing_and_clears_preedit() {
    let mut en = ComposerDocument::new(Locale::En);
    en.apply(TextInputCommand::Focus);
    en.apply(TextInputCommand::InsertText("Blood pressure".into()));
    en.apply(TextInputCommand::ImeStart);
    en.apply(TextInputCommand::ImeUpdatePreedit {
        text: "stale".into(),
        cursor_offset: None,
    });
    en.apply(TextInputCommand::DictationInsert(DictationSegment {
        text: "is stable.".into(),
        spacing: DictationSpacing::AutoSpace,
    }));
    assert_eq!(en.text(), "Blood pressure is stable.");
    assert_eq!(en.ime(), &ImeComposition::Idle);

    // Simplified Chinese must not inject an ASCII space between dictated phrases.
    let mut zh = ComposerDocument::new(Locale::ZhHans);
    zh.apply(TextInputCommand::Focus);
    zh.apply(TextInputCommand::InsertText("血压稳定".into()));
    zh.apply(TextInputCommand::DictationInsert(DictationSegment {
        text: "继续观察。".into(),
        spacing: DictationSpacing::AutoSpace,
    }));
    assert_eq!(zh.text(), "血压稳定继续观察。");
}

#[test]
fn mobile_soft_keyboard_resize_clamps_caret_above_keyboard_inset() {
    let mobile_viewport = ViewportMetrics {
        width_px: 390.0,
        height_px: 667.0,
        line_height_px: 22.0,
        char_advance_px: 8.5,
        composer_min_height_px: 56.0,
        composer_max_height_px: 150.0,
        scroll_offset_px: 0.0,
    };
    let mut doc = ComposerDocument::with_config(Locale::En, mobile_viewport, 64);
    doc.apply(TextInputCommand::Focus);
    doc.apply(TextInputCommand::InsertText(
        "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6\nLine 7\nLine 8\nLine 9".into(),
    ));

    let outcome = doc.apply(TextInputCommand::UpdateSoftKeyboard(
        SoftKeyboardState::Visible {
            inset_bottom_px: 310.0,
        },
    ));
    let layout = match outcome {
        CommandOutcome::ViewportAdjusted { layout } => layout,
        other => panic!("expected ViewportAdjusted, got {other:?}"),
    };

    assert!((layout.visible_viewport_height_px - 357.0).abs() < 0.01);
    assert!(
        layout.composer_top_px + layout.composer_height_px
            <= layout.visible_viewport_height_px + 0.01
    );
    assert!(
        layout.caret_rect.y >= layout.composer_top_px
            && layout.caret_rect.y + layout.caret_rect.height
                <= layout.visible_viewport_height_px + 0.01,
        "caret must remain visible above the soft keyboard inset"
    );
    assert_eq!(layout.scroll_adjustment, ScrollAdjustment::Unchanged);
    assert!(layout.scroll_offset_px > 0.0);
}

#[test]
fn background_foreground_restores_focus_cursor_and_scroll_without_losing_text() {
    let mut doc = ComposerDocument::new(Locale::Ko);
    doc.apply(TextInputCommand::Focus);
    doc.apply(TextInputCommand::InsertText(
        "첫째 줄\n둘째 줄\n셋째 줄".into(),
    ));
    doc.apply(TextInputCommand::MoveCaret {
        direction: MoveDirection::VisualLeft,
        selection: SelectionBehavior::Extend,
    });
    let saved_cursor = doc.cursor();
    assert!(matches!(saved_cursor, CursorState::Selection { .. }));

    // Active IME preedit is cancelled cleanly on backgrounding while committed text remains.
    doc.apply(TextInputCommand::ImeStart);
    doc.apply(TextInputCommand::ImeUpdatePreedit {
        text: "ㅎ".into(),
        cursor_offset: None,
    });

    doc.apply(TextInputCommand::AppBackgrounded);
    assert!(matches!(
        doc.focus(),
        FocusState::SuspendedWaitingRestore { .. }
    ));
    assert_eq!(doc.ime(), &ImeComposition::Idle);
    assert_eq!(
        doc.apply(TextInputCommand::InsertText("blocked".into())),
        CommandOutcome::NoOp(NoOpReason::UnfocusedIgnored)
    );

    doc.apply(TextInputCommand::AppForegrounded);
    assert_eq!(doc.focus(), FocusState::Focused);
    assert_eq!(doc.cursor(), saved_cursor);
    assert_eq!(doc.text(), "첫째 줄\n둘째 줄\n셋째 줄");
}

#[test]
fn single_owner_platform_adapter_roundtrip_and_target_mapping() {
    assert_eq!(PlatformTarget::ALL.len(), 10);
    assert_eq!(
        PlatformTarget::BrowserWasmDesktop.adapter_mechanism(),
        AdapterMechanism::HiddenTextareaImeBridge
    );
    assert_eq!(
        PlatformTarget::AndroidNative.adapter_mechanism(),
        AdapterMechanism::AndroidInputConnectionBridge
    );
    assert_eq!(
        PlatformTarget::IosNative.adapter_mechanism(),
        AdapterMechanism::IosUiTextInputBridge
    );
    assert_eq!(
        PlatformTarget::DesktopLinux.adapter_mechanism(),
        AdapterMechanism::WinitNativeIme
    );

    let mut adapter = HeadlessSimulatedAdapter::new(AdapterMechanism::HiddenTextareaImeBridge);
    let mut doc = ComposerDocument::new(Locale::Fr);

    adapter.enqueue(TextInputCommand::Focus);
    adapter.enqueue(TextInputCommand::InsertText("Bonjour".into()));
    adapter.enqueue(TextInputCommand::SelectAll);

    for cmd in adapter.drain_commands() {
        let _ = doc.apply(cmd);
    }
    adapter.sync_readback(AdapterReadback::from_document(&doc));

    let rb = adapter.last_readback().expect("readback synced");
    assert_eq!(rb.focus, FocusState::Focused);
    assert_eq!(rb.selection, SelectionPresence::Active);
    assert_eq!(rb.revision, doc.revision());
    assert_eq!(adapter.sync_count(), 1);
}

#[test]
fn headless_bevy_plugin_syncs_views_incrementally_and_idles_with_zero_syncs() {
    let mut app = App::new();
    app.add_plugins(TextI18nPrototypePlugin {
        initial_locale: Locale::Es,
    });
    app.world_mut()
        .resource_mut::<TranscriptFeedState>()
        .seed_turns(build_synthetic_80_turn_transcript(), 8);

    app.update();
    app.world_mut()
        .write_message(TextCommandMessage(TextInputCommand::Focus));
    app.world_mut()
        .write_message(TextCommandMessage(TextInputCommand::InsertText(
            "Nota clínica".into(),
        )));
    app.world_mut().write_message(TranscriptChunkMessage {
        turn_index: 79,
        token_delta: " +delta".into(),
    });
    app.update();

    let composer = app.world().resource::<ComposerState>();
    assert_eq!(composer.document().text(), "Nota clínica");

    let readback = app
        .world()
        .resource::<AdapterReadbackState>()
        .latest
        .expect("adapter readback populated");
    assert_eq!(readback.focus, FocusState::Focused);

    // Verify ECS view nodes mirror composer & active turn text.
    let mut composer_query = app.world_mut().query::<(&ComposerViewNode, &Text)>();
    let (_, composer_text) = composer_query
        .single(app.world())
        .expect("single composer view node");
    assert_eq!(composer_text.0, "Nota clínica");

    let mut turn_query = app
        .world_mut()
        .query::<(&ActiveTranscriptTurnViewNode, &Text)>();
    let (turn_view, turn_text) = turn_query
        .single(app.world())
        .expect("single active transcript view node");
    assert_eq!(turn_view.turn_index, 79);
    assert!(turn_text.0.ends_with(" +delta"));

    // Run 60 idle frames and assert 0 view syncs occur.
    let before = *app.world().resource::<ComposerRuntimeMetrics>();
    for _ in 0..60 {
        app.update();
    }
    let after = *app.world().resource::<ComposerRuntimeMetrics>();
    assert_eq!(after.composer_view_syncs, before.composer_view_syncs);
    assert_eq!(after.transcript_view_syncs, before.transcript_view_syncs);
    assert!(after.idle_frames >= 59);
}

#[test]
fn all_15_locales_roundtrip_cluster_navigation_and_editing() {
    for locale in Locale::ALL {
        let mut doc = ComposerDocument::new(locale);
        doc.apply(TextInputCommand::Focus);
        let sample = locale.synthetic_sample();
        doc.apply(TextInputCommand::InsertText(sample.into()));
        assert_eq!(doc.text(), sample);
        assert_eq!(doc.effective_direction(), locale.direction());

        // Step across every orthographic cluster backward and forward without splitting UTF-8 or clusters.
        let expected_clusters = cluster_boundaries(sample).len() - 1;
        let step_toward_start = match locale.direction() {
            TextDirection::Ltr => MoveDirection::VisualLeft,
            TextDirection::Rtl => MoveDirection::VisualRight,
        };
        let step_toward_end = match locale.direction() {
            TextDirection::Ltr => MoveDirection::VisualRight,
            TextDirection::Rtl => MoveDirection::VisualLeft,
        };

        for _ in 0..expected_clusters {
            doc.apply(TextInputCommand::MoveCaret {
                direction: step_toward_start,
                selection: SelectionBehavior::Collapse,
            });
        }
        assert_eq!(
            doc.cursor().focus().as_usize(),
            0,
            "stepping back {expected_clusters} clusters must reach offset 0 for {locale:?}"
        );

        for _ in 0..expected_clusters {
            doc.apply(TextInputCommand::MoveCaret {
                direction: step_toward_end,
                selection: SelectionBehavior::Collapse,
            });
        }
        assert_eq!(
            doc.cursor().focus().as_usize(),
            sample.len(),
            "stepping forward {expected_clusters} clusters must reach end for {locale:?}"
        );

        // Select all, cut, undo restores original text identically.
        doc.apply(TextInputCommand::SelectAll);
        let cut = doc.apply(TextInputCommand::Cut);
        assert!(matches!(
            cut,
            CommandOutcome::ClipboardProduced {
                ref text,
                mutation: ClipboardMutation::CutApplied { .. }
            } if text == sample
        ));
        assert!(doc.text().is_empty());
        doc.apply(TextInputCommand::Undo);
        assert_eq!(doc.text(), sample);
    }
}

#[test]
fn concurrent_streaming_above_composer_benchmark_meets_m1_budgets() {
    for locale in [
        Locale::En,
        Locale::ZhHans,
        Locale::Ja,
        Locale::Ko,
        Locale::Ar,
        Locale::Ur,
        Locale::Hi,
        Locale::Bn,
        Locale::Ta,
        Locale::Te,
    ] {
        let report = run_streaming_typing_benchmark(StreamingTypingBenchmarkConfig {
            locale,
            ..StreamingTypingBenchmarkConfig::default()
        });

        assert_eq!(report.transcript_turns, 80);
        assert!(
            report.transcript_chars >= 40_000,
            "transcript volume must meet ~40k char contract (got {})",
            report.transcript_chars
        );
        assert_eq!(report.streaming_chunks_processed, 600);
        assert_eq!(report.typing_events_processed, 90);
        assert_eq!(report.idle_view_syncs, 0);
        assert_eq!(report.stalls_over_50ms, 0);

        // Assert well under the strictest M1 typing latency budget (16.7 ms = 16,700 us p95).
        assert!(
            report.typing_p95_us < 16_700.0,
            "typing p95 {} us exceeded 16.7 ms budget for {locale:?}",
            report.typing_p95_us
        );
        assert!(
            report.typing_p99_us < 25_000.0,
            "typing p99 {} us exceeded 25 ms budget for {locale:?}",
            report.typing_p99_us
        );

        println!(
            "BENCHMARK locale={} turns={} chars={} typing_events={} stream_chunks={} typing_p50_us={:.2} typing_p95_us={:.2} typing_p99_us={:.2} typing_max_us={:.2} frame_p95_us={:.2} frame_p99_us={:.2} stalls_gt_50ms={} idle_syncs={}",
            locale.metadata().bcp47,
            report.transcript_turns,
            report.transcript_chars,
            report.typing_events_processed,
            report.streaming_chunks_processed,
            report.typing_p50_us,
            report.typing_p95_us,
            report.typing_p99_us,
            report.typing_max_us,
            report.frame_p95_us,
            report.frame_p99_us,
            report.stalls_over_50ms,
            report.idle_view_syncs
        );
    }
}
