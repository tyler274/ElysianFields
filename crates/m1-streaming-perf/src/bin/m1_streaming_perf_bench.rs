//! Executable benchmark runner for the M1 Bevy 0.20.1 streaming performance prototype.

use std::process::ExitCode;

use m1_streaming_perf::{BenchmarkConfig, run_benchmark};

fn main() -> ExitCode {
    let report = match run_benchmark(BenchmarkConfig::default()) {
        Ok(report) => report,
        Err(err) => {
            eprintln!("benchmark failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    println!("=== M1 Bevy 0.20.1 Streaming Performance Benchmark Report ===");
    println!(
        "Workload: {} turns, {} chars, {} lab rows",
        report.initial_turn_count, report.initial_transcript_chars, report.lab_table_rows
    );
    println!(
        "Cold init + first virtualized frame: {:.3} ms",
        report.cold_init_first_frame_ms
    );
    println!(
        "Active 900-frame stress (40 tok/s + 6 keys/s + +-420 px/s scroll + resize + citations):"
    );
    println!(
        "  Frame time (n={}): p50={:.2} us ({:.4} ms) | p95={:.2} us ({:.4} ms) | p99={:.2} us ({:.4} ms) | max={:.2} us ({:.4} ms) | stalls>50ms={}",
        report.active_frame_times.sample_count,
        report.active_frame_times.p50_us,
        report.active_frame_times.p50_us / 1_000.0,
        report.active_frame_times.p95_us,
        report.active_frame_times.p95_us / 1_000.0,
        report.active_frame_times.p99_us,
        report.active_frame_times.p99_us / 1_000.0,
        report.active_frame_times.max_us,
        report.active_frame_times.max_us / 1_000.0,
        report.active_frame_times.stalls_over_50ms,
    );
    println!(
        "  Typing latency (n={}): p50={:.2} us ({:.4} ms) | p95={:.2} us ({:.4} ms) | p99={:.2} us ({:.4} ms) | max={:.2} us ({:.4} ms)",
        report.typing_input_latency.sample_count,
        report.typing_input_latency.p50_us,
        report.typing_input_latency.p50_us / 1_000.0,
        report.typing_input_latency.p95_us,
        report.typing_input_latency.p95_us / 1_000.0,
        report.typing_input_latency.p99_us,
        report.typing_input_latency.p99_us / 1_000.0,
        report.typing_input_latency.max_us,
        report.typing_input_latency.max_us / 1_000.0,
    );
    println!(
        "  Citation [1] note open (n={}): p50={:.2} us ({:.4} ms) | p95={:.2} us ({:.4} ms) | max={:.2} us ({:.4} ms)",
        report.citation_note_open_us.sample_count,
        report.citation_note_open_us.p50_us,
        report.citation_note_open_us.p50_us / 1_000.0,
        report.citation_note_open_us.p95_us,
        report.citation_note_open_us.p95_us / 1_000.0,
        report.citation_note_open_us.max_us,
        report.citation_note_open_us.max_us / 1_000.0,
    );
    println!(
        "  Citation [12] 5,000-row table open (n={}): p50={:.2} us ({:.4} ms) | p95={:.2} us ({:.4} ms) | max={:.2} us ({:.4} ms)",
        report.citation_lab_table_open_us.sample_count,
        report.citation_lab_table_open_us.p50_us,
        report.citation_lab_table_open_us.p50_us / 1_000.0,
        report.citation_lab_table_open_us.p95_us,
        report.citation_lab_table_open_us.p95_us / 1_000.0,
        report.citation_lab_table_open_us.max_us,
        report.citation_lab_table_open_us.max_us / 1_000.0,
    );
    println!(
        "  Tokens streamed: {} | Keystrokes: {} | Materialized ECS entities: transcript={}, lab_table={}",
        report.active_tokens_streamed,
        report.active_keystrokes_applied,
        report.active_materialized_transcript_entities,
        report.active_materialized_lab_entities,
    );
    println!(
        "Idle redraw proof ({} ticks): frames_rendered={} | dirty_frames_requested={} | elapsed={:.3} ms",
        report.idle_ticks_evaluated,
        report.idle_frames_rendered,
        report.idle_dirty_frames_requested,
        report.idle_elapsed_ms,
    );
    println!(
        "Snapshot/restore proof: scroll_preserved={} | citation_scroll_preserved={} | composer_preserved={} | resumed_tokens={}",
        report.snapshot_restore.scroll_offset_preserved,
        report.snapshot_restore.citation_scroll_preserved,
        report.snapshot_restore.composer_preserved,
        report.snapshot_restore.resumed_tokens_without_drop,
    );
    println!(
        "1-hour soak ({} tokens across {} chunks): total={:.2} ms | first_10k={:.2} ms | last_10k={:.2} ms | full_reparses={}",
        report.soak.total_tokens,
        report.soak.total_chunks,
        report.soak.total_elapsed_ms,
        report.soak.first_10k_tokens_ms,
        report.soak.last_10k_tokens_ms,
        report.soak.full_reparse_count,
    );
    println!(
        "  Soak ECS entities: transcript={} | lab_table={} | total={} | retained_blocks={} | blocks_evicted={}",
        report.soak.materialized_transcript_entities,
        report.soak.materialized_lab_entities,
        report.soak.total_materialized_entities,
        report.soak.retained_blocks,
        report.soak.blocks_evicted,
    );
    println!(
        "  Process VmRSS (KiB): before={:?} | mid={:?} | after={:?}",
        report.soak.rss_before_kib, report.soak.rss_mid_kib, report.soak.rss_after_kib,
    );

    ExitCode::SUCCESS
}
