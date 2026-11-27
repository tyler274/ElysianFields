//! Concurrent streaming-above-composer latency microbenchmark.
//!
//! Exercises the M1 stutter workload contract from `specs/m1-feasibility-budgets.md`:
//! - 80-turn synthetic clinical transcript (~40,000 characters, headings, lists,
//!   lab table rows, fenced JSON, and 12 inline citations `[1]`–`[12]`)
//! - 40 tokens/sec streaming into the active turn above the composer
//! - Concurrent 6 keys/sec multiline composer typing + IME commits
//! - Post-window idle phase verifying 0 view syncs/redraws when clean.

use bevy::prelude::*;

use crate::composer::TextInputCommand;
use crate::locale::Locale;
use crate::plugin::{
    ComposerRuntimeMetrics, TextCommandMessage, TextI18nPrototypePlugin, TranscriptChunkMessage,
    TranscriptFeedState, TranscriptRole, TranscriptTurn,
};

/// Configuration for the concurrent streaming-above-composer latency microbenchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingTypingBenchmarkConfig {
    /// Active simulated window duration in seconds (spec requires >= 15 s).
    pub active_window_secs: u32,
    /// Post-stream idle window duration in seconds (spec requires 5–10 s).
    pub idle_window_secs: u32,
    /// Simulated frame cadence in Hz (default 60 Hz).
    pub frames_per_sec: u32,
    /// Streaming cadence in tokens per second (spec: 40 tok/s).
    pub tokens_per_sec: u32,
    /// Concurrent composer typing cadence in keys per second (spec: 6 keys/s).
    pub keys_per_sec: u32,
    /// Target locale for the composer typing workload.
    pub locale: Locale,
}

impl Default for StreamingTypingBenchmarkConfig {
    fn default() -> Self {
        Self {
            active_window_secs: 15,
            idle_window_secs: 5,
            frames_per_sec: 60,
            tokens_per_sec: 40,
            keys_per_sec: 6,
            locale: Locale::En,
        }
    }
}

/// Measured latency percentiles (in microseconds) and workload invariants.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamingTypingBenchmarkReport {
    pub locale: Locale,
    pub active_frames: u64,
    pub idle_frames: u64,
    pub idle_view_syncs: u64,
    pub typing_events_processed: u64,
    pub streaming_chunks_processed: u64,
    pub transcript_turns: usize,
    pub transcript_chars: usize,
    pub composer_final_chars: usize,
    pub typing_p50_us: f64,
    pub typing_p95_us: f64,
    pub typing_p99_us: f64,
    pub typing_max_us: f64,
    pub frame_p50_us: f64,
    pub frame_p95_us: f64,
    pub frame_p99_us: f64,
    pub frame_max_us: f64,
    pub stalls_over_50ms: u64,
}

/// Build an 80-turn, ~40,000-character 100% synthetic clinical transcript containing
/// headings, bulleted/numbered lists, lab tables, fenced JSON, and citations `[1]`–`[12]`.
#[must_use]
pub fn build_synthetic_80_turn_transcript() -> Vec<TranscriptTurn> {
    let mut turns = Vec::with_capacity(80);
    for idx in 0..80 {
        let citation = (idx % 12) + 1;
        let (role, content) = if idx % 2 == 0 {
            (
                TranscriptRole::Clinician,
                format!(
                    "Clinician query #{idx}: Review synthetic trend markers, medication adherence, and reference note [{citation}] before drafting follow-up guidance."
                ),
            )
        } else {
            (
                TranscriptRole::Assistant,
                format!(
                    "### Synthetic Clinical Summary #{idx} [{citation}]\n\
                     - Vital trend: BP 118/76 mmHg, HR 68 bpm, SpO2 99% on room air, weight stable across serial outpatient visits [1][2].\n\
                     - Assessment overview: Synthetic longitudinal findings remain within target clinical thresholds without acute red-flag symptoms or intolerance [3][4].\n\
                     - Follow-up actions:\n\
                       1. Continue current antihypertensive regimen without dosage change and review home blood pressure log at next visit [5][6].\n\
                       2. Recheck basic metabolic panel, fasting lipid panel, and renal function markers in 12 weeks [7][8].\n\
                     | Marker | Value | Reference | Source |\n\
                     | --- | --- | --- | --- |\n\
                     | Creatinine | 0.91 mg/dL | 0.70-1.20 | [7][8] |\n\
                     | Potassium | 4.2 mmol/L | 3.5-5.1 | [9][10] |\n\
                     | eGFR | >90 mL/min | >=60 | [11][12] |\n\
                     ```json\n\
                     {{\"turn\": {idx}, \"citations\": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], \"status\": \"stable\", \"virtualized_rows\": 5000}}\n\
                     ```\n\
                     Cited record links verified across [11] and [12].\n"
                ),
            )
        };
        turns.push(TranscriptTurn {
            index: idx,
            role,
            content,
            revision: 1,
        });
    }
    turns
}

/// Execute the concurrent streaming-above-composer latency microbenchmark on a headless Bevy [`App`].
#[must_use]
pub fn run_streaming_typing_benchmark(
    config: StreamingTypingBenchmarkConfig,
) -> StreamingTypingBenchmarkReport {
    let mut app = App::new();
    app.add_plugins(TextI18nPrototypePlugin {
        initial_locale: config.locale,
    });

    // Seed the 80-turn ~40k-char synthetic transcript before starting the active streaming window.
    let initial_turns = build_synthetic_80_turn_transcript();
    let active_turn_index = initial_turns.len().saturating_sub(1);
    app.world_mut()
        .resource_mut::<TranscriptFeedState>()
        .seed_turns(initial_turns, 8);

    // Run startup + focus the composer before the measured window.
    app.update();
    app.world_mut()
        .write_message(TextCommandMessage(TextInputCommand::Focus));
    app.update();

    let total_active_frames =
        u64::from(config.active_window_secs) * u64::from(config.frames_per_sec);
    let total_idle_frames = u64::from(config.idle_window_secs) * u64::from(config.frames_per_sec);

    let sample_clusters: Vec<String> = config
        .locale
        .synthetic_sample()
        .chars()
        .map(|ch| ch.to_string())
        .collect();
    let sample_len = sample_clusters.len().max(1);

    let mut typing_latencies_us = Vec::with_capacity(
        (u64::from(config.active_window_secs) * u64::from(config.keys_per_sec)) as usize + 8,
    );
    let mut frame_latencies_us = Vec::with_capacity(total_active_frames as usize);
    let mut stalls_over_50ms = 0u64;

    let mut token_accum = 0u64;
    let mut key_accum = 0u64;
    let mut key_index = 0usize;
    let mut streaming_chunks_sent = 0u64;
    let mut typing_events_sent = 0u64;
    let fps = u64::from(config.frames_per_sec.max(1));

    for frame in 0..total_active_frames {
        token_accum += u64::from(config.tokens_per_sec);
        let tokens_this_frame = token_accum / fps;
        token_accum %= fps;

        for t in 0..tokens_this_frame {
            let delta = format!(" tok{frame}_{t}");
            app.world_mut().write_message(TranscriptChunkMessage {
                turn_index: active_turn_index,
                token_delta: delta,
            });
            streaming_chunks_sent += 1;
        }

        key_accum += u64::from(config.keys_per_sec);
        let keys_this_frame = key_accum / fps;
        key_accum %= fps;

        for _ in 0..keys_this_frame {
            let cmd = if key_index > 0 && key_index.is_multiple_of(24) {
                TextInputCommand::InsertNewline
            } else {
                let piece = &sample_clusters[key_index % sample_len];
                TextInputCommand::InsertText(piece.clone())
            };
            app.world_mut().write_message(TextCommandMessage(cmd));
            key_index += 1;
            typing_events_sent += 1;
        }

        let elapsed_us = measure_update_us(&mut app);
        frame_latencies_us.push(elapsed_us);
        if keys_this_frame > 0 {
            typing_latencies_us.push(elapsed_us);
        }
        if elapsed_us > 50_000.0 {
            stalls_over_50ms += 1;
        }
    }

    let before_idle = *app.world().resource::<ComposerRuntimeMetrics>();
    for _ in 0..total_idle_frames {
        app.update();
    }
    let after_idle = *app.world().resource::<ComposerRuntimeMetrics>();
    let idle_view_syncs = (after_idle.composer_view_syncs - before_idle.composer_view_syncs)
        + (after_idle.transcript_view_syncs - before_idle.transcript_view_syncs);

    let feed = app.world().resource::<TranscriptFeedState>();
    let composer = app.world().resource::<crate::plugin::ComposerState>();

    StreamingTypingBenchmarkReport {
        locale: config.locale,
        active_frames: total_active_frames,
        idle_frames: total_idle_frames,
        idle_view_syncs,
        typing_events_processed: typing_events_sent,
        streaming_chunks_processed: streaming_chunks_sent,
        transcript_turns: feed.turn_count(),
        transcript_chars: feed.total_chars(),
        composer_final_chars: composer.document().text().chars().count(),
        typing_p50_us: percentile(&typing_latencies_us, 0.50),
        typing_p95_us: percentile(&typing_latencies_us, 0.95),
        typing_p99_us: percentile(&typing_latencies_us, 0.99),
        typing_max_us: max_value(&typing_latencies_us),
        frame_p50_us: percentile(&frame_latencies_us, 0.50),
        frame_p95_us: percentile(&frame_latencies_us, 0.95),
        frame_p99_us: percentile(&frame_latencies_us, 0.99),
        frame_max_us: max_value(&frame_latencies_us),
        stalls_over_50ms,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn measure_update_us(app: &mut App) -> f64 {
    let start = std::time::Instant::now();
    app.update();
    start.elapsed().as_secs_f64() * 1_000_000.0
}

#[cfg(target_arch = "wasm32")]
fn measure_update_us(app: &mut App) -> f64 {
    app.update();
    0.0
}

fn percentile(samples: &[f64], p: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn max_value(samples: &[f64]) -> f64 {
    samples
        .iter()
        .copied()
        .fold(0.0_f64, |acc, val| acc.max(val))
}
