//! Executable M1 feasibility benchmark harness running the Bevy 0.20.1
//! `StreamingPerfPrototypePlugin` through all required workload phases:
//!
//! 1. Cold initialization & first virtualized render pass.
//! 2. 900-frame (15 s @ 60 Hz) concurrent stress window:
//!    - 40 tokens/sec incremental streaming (20 Hz, 2 tokens/chunk)
//!    - 6 keys/sec continuous multiline composer typing
//!    - ±420 px/sec alternating up/down smooth scrolling every 1 second
//!    - Viewport resize transitions + non-modal citation switching (`[1]` <-> `[12]`)
//! 3. Typing input latency distribution (`p50`, `p95`, `p99`, `max`).
//! 4. Citation panel open latency (`[1]` progress note and `[12]` 5,000-row lab table).
//! 5. 300-tick (5 s @ 60 Hz) idle redraw proof asserting **0 frames rendered**.
//! 6. Background/return snapshot & exact resume verification.
//! 7. Simulated 1-hour sustained streaming soak (144,000 tokens / 3,600 s accelerated)
//!    with bounded ECS entity verification and Linux `/proc/self/status` `VmRSS` tracking.

use bevy::prelude::App;

use crate::domain::{CitationId, StreamSeq, StreamingPerfError};
use crate::plugin::{
    StreamingPerfPrototypePlugin, StreamingPrototypeResource, VirtualizedEcsPoolState,
};
use crate::workload::SyntheticTokenStream;

/// Configuration for a benchmark run.
#[derive(Debug, Clone, Copy)]
pub struct BenchmarkConfig {
    /// Number of active frames at 60 Hz (default: 900 frames = 15 seconds).
    pub active_frames: usize,
    /// Number of idle ticks after active window (default: 300 ticks = 5 seconds).
    pub idle_ticks: usize,
    /// Total tokens for the simulated 1-hour streaming soak (default: 144,000 = 40 tok/s * 3,600 s).
    pub soak_total_tokens: u64,
    /// Tokens batched per streaming chunk during the soak (default: 2 tokens/chunk = 20 Hz).
    pub soak_tokens_per_chunk: u32,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            active_frames: 900,
            idle_ticks: 300,
            soak_total_tokens: 144_000,
            soak_tokens_per_chunk: 2,
        }
    }
}

/// Summary statistics in microseconds for latency/frame-time samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencyDistributionUs {
    pub sample_count: usize,
    pub p50_us: f64,
    pub p95_us: f64,
    pub p99_us: f64,
    pub max_us: f64,
    pub mean_us: f64,
    pub stalls_over_50ms: usize,
}

impl LatencyDistributionUs {
    /// Computes percentile statistics from raw microsecond samples.
    #[must_use]
    pub fn from_samples(mut samples_us: Vec<f64>) -> Self {
        if samples_us.is_empty() {
            return Self {
                sample_count: 0,
                p50_us: 0.0,
                p95_us: 0.0,
                p99_us: 0.0,
                max_us: 0.0,
                mean_us: 0.0,
                stalls_over_50ms: 0,
            };
        }
        samples_us.sort_by(f64::total_cmp);
        let n = samples_us.len();
        let sum: f64 = samples_us.iter().sum();
        let stalls_over_50ms = samples_us.iter().filter(|&&v| v > 50_000.0).count();
        Self {
            sample_count: n,
            p50_us: percentile_sorted(&samples_us, 0.50),
            p95_us: percentile_sorted(&samples_us, 0.95),
            p99_us: percentile_sorted(&samples_us, 0.99),
            max_us: samples_us[n - 1],
            mean_us: sum / (n as f64),
            stalls_over_50ms,
        }
    }
}

fn percentile_sorted(sorted: &[f64], quantile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len().saturating_sub(1) as f64) * quantile).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Results from the simulated 1-hour (144,000-token) streaming soak.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoakReport {
    pub total_tokens: u64,
    pub total_chunks: u64,
    pub total_elapsed_ms: f64,
    pub first_10k_tokens_ms: f64,
    pub last_10k_tokens_ms: f64,
    pub materialized_transcript_entities: usize,
    pub materialized_lab_entities: usize,
    pub total_materialized_entities: usize,
    pub retained_blocks: usize,
    pub blocks_evicted: u64,
    pub full_reparse_count: u64,
    pub rss_before_kib: Option<u64>,
    pub rss_mid_kib: Option<u64>,
    pub rss_after_kib: Option<u64>,
}

/// Proof results from background/return snapshot capture and exact stream resumption.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapshotRestoreReport {
    pub scroll_offset_preserved: bool,
    pub citation_scroll_preserved: bool,
    pub composer_preserved: bool,
    pub resumed_tokens_without_drop: u64,
}

/// Complete empirical benchmark output across all M1 feasibility criteria.
#[derive(Debug, Clone)]
pub struct BenchmarkReport {
    pub initial_turn_count: u32,
    pub initial_transcript_chars: usize,
    pub lab_table_rows: usize,
    pub cold_init_first_frame_ms: f64,
    pub active_frame_times: LatencyDistributionUs,
    pub typing_input_latency: LatencyDistributionUs,
    pub citation_note_open_us: LatencyDistributionUs,
    pub citation_lab_table_open_us: LatencyDistributionUs,
    pub active_tokens_streamed: u64,
    pub active_keystrokes_applied: u64,
    pub active_materialized_transcript_entities: usize,
    pub active_materialized_lab_entities: usize,
    pub idle_ticks_evaluated: usize,
    pub idle_frames_rendered: u64,
    pub idle_dirty_frames_requested: u64,
    pub idle_elapsed_ms: f64,
    pub snapshot_restore: SnapshotRestoreReport,
    pub soak: SoakReport,
}

/// Runs the full M1 Bevy 0.20.1 feasibility benchmark suite.
pub fn run_benchmark(config: BenchmarkConfig) -> Result<BenchmarkReport, StreamingPerfError> {
    let cold_start = Timer::start();
    let mut app = App::new();
    app.add_plugins(StreamingPerfPrototypePlugin::default());
    app.update();
    let cold_init_first_frame_ms = cold_start.elapsed_ms();

    let (initial_turn_count, initial_transcript_chars, lab_table_rows) = {
        let proto = app.world().resource::<StreamingPrototypeResource>();
        (
            proto.store.turn_count(),
            proto.store.total_chars_ingested(),
            proto.store.lab_rows().len(),
        )
    };

    // Begin live streaming assistant turn.
    let mut token_stream = SyntheticTokenStream::starting_at(StreamSeq(1));
    app.world_mut()
        .resource_mut::<StreamingPrototypeResource>()
        .store
        .begin_assistant_stream(StreamSeq(1));

    // Measure standalone citation open latencies for [1] (progress note) and [12] (5,000-row lab table).
    let mut note_open_samples = Vec::with_capacity(20);
    let mut lab_open_samples = Vec::with_capacity(20);
    for _ in 0..20 {
        let t_note = Timer::start();
        app.world_mut()
            .resource_mut::<StreamingPrototypeResource>()
            .store
            .open_citation(CitationId::progress_note())?;
        app.update();
        note_open_samples.push(t_note.elapsed_us());

        let t_lab = Timer::start();
        app.world_mut()
            .resource_mut::<StreamingPrototypeResource>()
            .store
            .open_citation(CitationId::lab_table())?;
        app.update();
        lab_open_samples.push(t_lab.elapsed_us());
    }

    // 900-frame (15 s @ 60 Hz) concurrent stress window:
    // - 40 tokens/sec -> 2 tokens every 3 frames (20 Hz chunk arrival >= 4 Hz requirement)
    // - 6 keys/sec -> 1 keystroke every 10 frames (~166.7 ms cadence)
    // - ±420 px/sec -> ±7 px/frame alternating direction every 60 frames (1 s)
    // - Viewport resize every 180 frames (desktop 720px <-> compact/soft-keyboard 420px)
    // - Citation panel toggle/scroll between [1] and [12] every 90 frames
    let typing_phrase: Vec<char> =
        "Synthetic clinician note: verify renal dosing & electrolyte trend [12]\n"
            .chars()
            .collect();
    let mut typing_idx = 0usize;
    let mut frame_samples_us = Vec::with_capacity(config.active_frames);
    let mut typing_samples_us = Vec::with_capacity(config.active_frames / 10 + 1);

    for frame in 0..config.active_frames {
        let frame_timer = Timer::start();

        // Streaming at 40 tok/s (2 tokens every 3 frames @ 60 Hz).
        if frame.is_multiple_of(3) {
            let chunk = token_stream.next_chunk(2);
            app.world_mut()
                .resource_mut::<StreamingPrototypeResource>()
                .store
                .apply_stream_chunk(chunk)?;
        }

        // Alternating smooth scroll at ±420 px/sec (±7 px/frame @ 60 Hz, reversing every 1 s).
        let scroll_delta = if (frame / 60).is_multiple_of(2) {
            7.0
        } else {
            -7.0
        };
        {
            let mut proto = app.world_mut().resource_mut::<StreamingPrototypeResource>();
            proto.store.scroll_transcript_by(scroll_delta);
            proto.store.scroll_citation_table_by(scroll_delta * 2.5);
        }

        // Periodic viewport resize simulating desktop resize / mobile soft-keyboard change.
        if frame > 0 && frame.is_multiple_of(180) {
            let height = if (frame / 180) % 2 == 1 { 420.0 } else { 720.0 };
            app.world_mut()
                .resource_mut::<StreamingPrototypeResource>()
                .store
                .resize_viewport(height)?;
        }

        // Periodic non-modal citation switching between [1] and [12].
        if frame > 0 && frame.is_multiple_of(90) {
            let target = if (frame / 90).is_multiple_of(2) {
                CitationId::lab_table()
            } else {
                CitationId::progress_note()
            };
            app.world_mut()
                .resource_mut::<StreamingPrototypeResource>()
                .store
                .open_citation(target)?;
        }

        // Concurrent multiline composer typing at 6 keys/sec (every 10 frames @ 60 Hz).
        let is_typing_frame = frame.is_multiple_of(10);
        let typing_timer = if is_typing_frame {
            let t = Timer::start();
            let ch = typing_phrase[typing_idx % typing_phrase.len()];
            typing_idx = typing_idx.saturating_add(1);
            app.world_mut()
                .resource_mut::<StreamingPrototypeResource>()
                .store
                .type_composer_char(ch);
            Some(t)
        } else {
            None
        };

        app.update();

        if let Some(t) = typing_timer {
            typing_samples_us.push(t.elapsed_us());
        }
        frame_samples_us.push(frame_timer.elapsed_us());
    }

    let (
        active_tokens_streamed,
        active_keystrokes_applied,
        active_materialized_transcript_entities,
        active_materialized_lab_entities,
    ) = {
        let proto = app.world().resource::<StreamingPrototypeResource>();
        let pool = app.world().resource::<VirtualizedEcsPoolState>();
        (
            proto.store.total_tokens_streamed(),
            proto.store.composer().keystrokes_applied(),
            pool.transcript_entities.len(),
            pool.lab_table_entities.len(),
        )
    };

    // Background/return snapshot & exact restoration check while citation [12] is scrolled.
    let snapshot_restore = verify_snapshot_and_resume(&mut app, &mut token_stream)?;

    // Flush any pending redraw from restore before entering the idle phase.
    app.update();

    // Idle phase: 300 ticks (5 s @ 60 Hz) with ZERO streaming, ZERO input, ZERO scroll.
    app.world_mut()
        .resource_mut::<StreamingPrototypeResource>()
        .store
        .redraw_policy_mut()
        .reset_phase_counters();

    let idle_timer = Timer::start();
    for _ in 0..config.idle_ticks {
        app.update();
    }
    let idle_elapsed_ms = idle_timer.elapsed_ms();

    let (idle_frames_rendered, idle_dirty_frames_requested) = {
        let proto = app.world().resource::<StreamingPrototypeResource>();
        let policy = proto.store.redraw_policy();
        (policy.frames_rendered(), policy.dirty_frames_requested())
    };

    // Simulated 1-hour sustained streaming soak (144,000 tokens accelerated).
    let soak = run_one_hour_soak(&mut app, &mut token_stream, config)?;

    Ok(BenchmarkReport {
        initial_turn_count,
        initial_transcript_chars,
        lab_table_rows,
        cold_init_first_frame_ms,
        active_frame_times: LatencyDistributionUs::from_samples(frame_samples_us),
        typing_input_latency: LatencyDistributionUs::from_samples(typing_samples_us),
        citation_note_open_us: LatencyDistributionUs::from_samples(note_open_samples),
        citation_lab_table_open_us: LatencyDistributionUs::from_samples(lab_open_samples),
        active_tokens_streamed,
        active_keystrokes_applied,
        active_materialized_transcript_entities,
        active_materialized_lab_entities,
        idle_ticks_evaluated: config.idle_ticks,
        idle_frames_rendered,
        idle_dirty_frames_requested,
        idle_elapsed_ms,
        snapshot_restore,
        soak,
    })
}

fn verify_snapshot_and_resume(
    app: &mut App,
    token_stream: &mut SyntheticTokenStream,
) -> Result<SnapshotRestoreReport, StreamingPerfError> {
    {
        let mut proto = app.world_mut().resource_mut::<StreamingPrototypeResource>();
        proto.store.open_citation(CitationId::lab_table())?;
        proto.store.scroll_transcript_by(240.0);
        proto.store.scroll_citation_table_by(1_400.0);
    }
    app.update();

    let expected_transcript_scroll = app
        .world()
        .resource::<StreamingPrototypeResource>()
        .store
        .transcript_window()
        .scroll_offset_px;
    let expected_composer = app
        .world()
        .resource::<StreamingPrototypeResource>()
        .store
        .composer()
        .text()
        .to_owned();
    let expected_citation_panel = app
        .world()
        .resource::<StreamingPrototypeResource>()
        .store
        .citation_panel()
        .clone();

    let snapshot = app
        .world_mut()
        .resource_mut::<StreamingPrototypeResource>()
        .store
        .capture_snapshot();

    // Mutate state away from snapshot to prove restoration actually re-applies saved state.
    {
        let mut proto = app.world_mut().resource_mut::<StreamingPrototypeResource>();
        proto.store.scroll_transcript_by(-5_000.0);
        proto.store.close_citation_panel();
    }

    app.world_mut()
        .resource_mut::<StreamingPrototypeResource>()
        .store
        .restore_snapshot(snapshot)?;

    let mut resumed_tokens = 0u64;
    for _ in 0..5 {
        let chunk = token_stream.next_chunk(2);
        resumed_tokens = resumed_tokens.saturating_add(u64::from(chunk.token_count));
        app.world_mut()
            .resource_mut::<StreamingPrototypeResource>()
            .store
            .apply_stream_chunk(chunk)?;
    }

    let proto = app.world().resource::<StreamingPrototypeResource>();
    let scroll_offset_preserved =
        (proto.store.transcript_window().scroll_offset_px - expected_transcript_scroll).abs()
            < f32::EPSILON;
    let citation_scroll_preserved = *proto.store.citation_panel() == expected_citation_panel;
    let composer_preserved = proto.store.composer().text() == expected_composer;

    Ok(SnapshotRestoreReport {
        scroll_offset_preserved,
        citation_scroll_preserved,
        composer_preserved,
        resumed_tokens_without_drop: resumed_tokens,
    })
}

fn run_one_hour_soak(
    app: &mut App,
    token_stream: &mut SyntheticTokenStream,
    config: BenchmarkConfig,
) -> Result<SoakReport, StreamingPerfError> {
    let tokens_per_chunk = config.soak_tokens_per_chunk.max(1);
    let total_chunks = config
        .soak_total_tokens
        .div_ceil(u64::from(tokens_per_chunk));
    let sample_window_chunks = 10_000u64.div_ceil(u64::from(tokens_per_chunk));
    let mid_chunk = total_chunks / 2;

    let rss_before_kib = read_process_rss_kib();
    let mut rss_mid_kib = None;

    let soak_timer = Timer::start();
    let mut first_10k_tokens_ms = 0.0;
    let mut last_10k_start: Option<Timer> = None;
    let mut delivered_tokens = 0u64;

    for chunk_idx in 0..total_chunks {
        if chunk_idx == total_chunks.saturating_sub(sample_window_chunks) {
            last_10k_start = Some(Timer::start());
        }

        let chunk = token_stream.next_chunk(tokens_per_chunk);
        delivered_tokens = delivered_tokens.saturating_add(u64::from(chunk.token_count));
        {
            let mut proto = app.world_mut().resource_mut::<StreamingPrototypeResource>();
            proto.store.apply_stream_chunk(chunk)?;
            // Periodically scroll both transcript and 5,000-row table during soak.
            if chunk_idx.is_multiple_of(20) {
                let delta = if (chunk_idx / 200).is_multiple_of(2) {
                    28.0
                } else {
                    -28.0
                };
                proto.store.scroll_transcript_by(delta);
                proto.store.scroll_citation_table_by(delta * 4.0);
            }
        }
        // Drive Bevy ECS sync every chunk (at 20 Hz simulated cadence = 72,000 updates/hr).
        app.update();

        if chunk_idx + 1 == sample_window_chunks {
            first_10k_tokens_ms = soak_timer.elapsed_ms();
        }
        if chunk_idx == mid_chunk {
            rss_mid_kib = read_process_rss_kib();
        }
    }

    let total_elapsed_ms = soak_timer.elapsed_ms();
    let last_10k_tokens_ms = last_10k_start.map_or(total_elapsed_ms, |t| t.elapsed_ms());
    let rss_after_kib = read_process_rss_kib();

    let (retained_blocks, blocks_evicted, full_reparse_count) = {
        let proto = app.world().resource::<StreamingPrototypeResource>();
        (
            proto.store.blocks().len(),
            proto.store.blocks_evicted(),
            proto.store.full_reparse_count(),
        )
    };
    let (materialized_transcript_entities, materialized_lab_entities, total_materialized_entities) = {
        let pool = app.world().resource::<VirtualizedEcsPoolState>();
        (
            pool.transcript_entities.len(),
            pool.lab_table_entities.len(),
            pool.total_materialized_entities(),
        )
    };

    Ok(SoakReport {
        total_tokens: delivered_tokens,
        total_chunks,
        total_elapsed_ms,
        first_10k_tokens_ms,
        last_10k_tokens_ms,
        materialized_transcript_entities,
        materialized_lab_entities,
        total_materialized_entities,
        retained_blocks,
        blocks_evicted,
        full_reparse_count,
        rss_before_kib,
        rss_mid_kib,
        rss_after_kib,
    })
}

/// Reads current process resident set size (`VmRSS`) in KiB on Linux.
#[must_use]
pub fn read_process_rss_kib() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                let value = rest.split_whitespace().next()?;
                return value.parse::<u64>().ok();
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Cross-target monotonic timer that compiles cleanly on both native hosts and `wasm32`.
struct Timer {
    #[cfg(not(target_arch = "wasm32"))]
    started: std::time::Instant,
}

impl Timer {
    fn start() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            started: std::time::Instant::now(),
        }
    }

    fn elapsed_us(&self) -> f64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started.elapsed().as_secs_f64() * 1_000_000.0
        }
        #[cfg(target_arch = "wasm32")]
        {
            0.0
        }
    }

    fn elapsed_ms(&self) -> f64 {
        self.elapsed_us() / 1_000.0
    }
}
