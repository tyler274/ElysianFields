//! Integration tests validating all M1 clinical streaming feasibility invariants.

use bevy::prelude::App;
use m1_streaming_perf::{
    BenchmarkConfig, CITATION_SOURCE_COUNT, CitationId, CitationPanelState, DeltaPayload,
    InlineSpan, LAB_TABLE_ROW_COUNT, MarkdownBlock, StreamChunk, StreamSeq, StreamingPerfError,
    StreamingPerfPrototypePlugin, StreamingPrototypeResource, SyntheticClinicalWorkload,
    SyntheticTokenStream, VirtualizedEcsPoolState, VirtualizedWindow, WORKLOAD_TURN_COUNT,
    run_benchmark,
};

#[test]
fn synthetic_workload_matches_specification_contract() {
    let workload = SyntheticClinicalWorkload::generate();
    assert_eq!(workload.turns.len(), WORKLOAD_TURN_COUNT);
    assert_eq!(workload.lab_rows.len(), LAB_TABLE_ROW_COUNT);
    assert_eq!(workload.citation_sources.len(), CITATION_SOURCE_COUNT);
    assert!(
        workload.total_chars >= 36_000 && workload.total_chars <= 48_000,
        "expected ~40,000 synthetic chars, got {}",
        workload.total_chars
    );

    let mut seen_citations = [false; CITATION_SOURCE_COUNT];
    let mut has_heading = false;
    let mut has_paragraph = false;
    let mut has_list = false;
    let mut has_code = false;
    let mut has_table = false;
    let mut has_link = false;

    for turn in &workload.turns {
        for block in &turn.blocks {
            match &block.content {
                MarkdownBlock::Heading { .. } => has_heading = true,
                MarkdownBlock::Paragraph { spans } => {
                    has_paragraph = true;
                    for span in spans {
                        if let InlineSpan::Citation(id) = span {
                            seen_citations[usize::from(id.as_u8() - 1)] = true;
                        }
                    }
                }
                MarkdownBlock::List { .. } => has_list = true,
                MarkdownBlock::CodeBlock { .. } => has_code = true,
                MarkdownBlock::Table { .. } => has_table = true,
                MarkdownBlock::Link { .. } => has_link = true,
            }
        }
    }

    assert!(has_heading && has_paragraph && has_list && has_code && has_table && has_link);
    assert!(
        seen_citations.iter().all(|seen| *seen),
        "all 12 citation markers [1]..=[12] must appear in the transcript"
    );
}

#[test]
fn virtualized_window_materializes_only_visible_rows_for_5000_row_table()
-> Result<(), StreamingPerfError> {
    let mut window = VirtualizedWindow::new(0.0, 720.0, 28.0, 4)?;
    let initial = window.visible_range(LAB_TABLE_ROW_COUNT);
    assert!(
        initial.len() >= 25 && initial.len() <= 40,
        "expected ~30-40 visible rows, got {}",
        initial.len()
    );

    window.scroll_by(70_000.0, LAB_TABLE_ROW_COUNT);
    let mid = window.visible_range(LAB_TABLE_ROW_COUNT);
    assert!(mid.start > 2_000 && mid.end < 3_000);
    assert!(mid.len() <= 40);

    window.scroll_by(500_000.0, LAB_TABLE_ROW_COUNT);
    let tail = window.visible_range(LAB_TABLE_ROW_COUNT);
    assert_eq!(tail.end, LAB_TABLE_ROW_COUNT);
    assert!(tail.len() <= 40);
    Ok(())
}

#[test]
fn incremental_delta_streaming_enforces_sequence_and_never_reparses_history()
-> Result<(), StreamingPerfError> {
    let mut app = App::new();
    app.add_plugins(StreamingPerfPrototypePlugin::default());
    app.update();

    {
        let mut proto = app.world_mut().resource_mut::<StreamingPrototypeResource>();
        proto.store.begin_assistant_stream(StreamSeq(1));

        let err = proto.store.apply_stream_chunk(StreamChunk {
            seq: StreamSeq(2),
            token_count: 1,
            payload: DeltaPayload::AppendText("out-of-order ".to_owned()),
        });
        assert_eq!(
            err,
            Err(StreamingPerfError::OutOfOrderStreamChunk {
                expected: StreamSeq(1),
                actual: StreamSeq(2),
            })
        );

        let mut stream = SyntheticTokenStream::starting_at(StreamSeq(1));
        for _ in 0..40 {
            let chunk = stream.next_chunk(2);
            let _ = proto.store.apply_stream_chunk(chunk)?;
        }

        assert_eq!(proto.store.full_reparse_count(), 0);
        assert_eq!(proto.store.incremental_delta_count(), 40);
        assert_eq!(proto.store.total_tokens_streamed(), 80);
    }

    app.update();

    let pool = app.world().resource::<VirtualizedEcsPoolState>();
    assert!(
        pool.transcript_entities.len() <= 40,
        "transcript ECS entities must remain O(viewport), got {}",
        pool.transcript_entities.len()
    );
    Ok(())
}

#[test]
fn citation_panel_switches_cleanly_and_idle_policy_renders_zero_frames()
-> Result<(), StreamingPerfError> {
    let mut app = App::new();
    app.add_plugins(StreamingPerfPrototypePlugin::default());
    app.update();

    app.world_mut()
        .resource_mut::<StreamingPrototypeResource>()
        .store
        .open_citation(CitationId::progress_note())?;
    app.update();

    app.world_mut()
        .resource_mut::<StreamingPrototypeResource>()
        .store
        .open_citation(CitationId::lab_table())?;
    app.update();

    {
        let pool = app.world().resource::<VirtualizedEcsPoolState>();
        assert!(
            pool.lab_table_entities.len() >= 25 && pool.lab_table_entities.len() <= 40,
            "expected ~30-40 virtualized lab entities, got {}",
            pool.lab_table_entities.len()
        );
    }

    // Reset counters and run 300 completely idle ticks -> MUST render 0 frames!
    app.world_mut()
        .resource_mut::<StreamingPrototypeResource>()
        .store
        .redraw_policy_mut()
        .reset_phase_counters();

    for _ in 0..300 {
        app.update();
    }

    let proto = app.world().resource::<StreamingPrototypeResource>();
    assert_eq!(proto.store.redraw_policy().dirty_frames_requested(), 0);
    assert_eq!(proto.store.redraw_policy().frames_rendered(), 0);
    assert_eq!(proto.store.redraw_policy().idle_ticks_skipped(), 300);
    assert!(matches!(
        proto.store.citation_panel(),
        CitationPanelState::Open(_)
    ));
    Ok(())
}

#[test]
fn fast_benchmark_smoke_verifies_all_phases_and_invariants() -> Result<(), StreamingPerfError> {
    let report = run_benchmark(BenchmarkConfig {
        active_frames: 180,
        idle_ticks: 300,
        soak_total_tokens: 4_000,
        soak_tokens_per_chunk: 2,
    })?;

    assert_eq!(report.initial_turn_count, 80);
    assert_eq!(report.lab_table_rows, 5_000);
    assert_eq!(report.idle_frames_rendered, 0);
    assert_eq!(report.idle_dirty_frames_requested, 0);
    assert!(report.snapshot_restore.scroll_offset_preserved);
    assert!(report.snapshot_restore.citation_scroll_preserved);
    assert!(report.snapshot_restore.composer_preserved);
    assert_eq!(report.snapshot_restore.resumed_tokens_without_drop, 10);
    assert_eq!(report.soak.full_reparse_count, 0);
    assert!(report.soak.materialized_lab_entities <= 40);
    assert!(report.soak.materialized_transcript_entities <= 40);
    assert_eq!(report.active_frame_times.stalls_over_50ms, 0);
    Ok(())
}
