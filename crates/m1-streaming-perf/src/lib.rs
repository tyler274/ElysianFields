//! M1 Bevy 0.20.1 feasibility prototype and empirical benchmark suite for:
//! - 80-turn (~40,000-char) synthetic clinical streaming transcripts
//! - O(1) incremental delta-append streaming with dirty-block invalidation
//! - 5,000-row virtualized clinical laboratory table (`O(viewport)` ECS entity pool)
//! - Non-modal citation side panel switching (`[1]` <-> `[12]`)
//! - Reactive zero-redraw idle policy (`0 frames/sec` when idle)
//! - Background/return state snapshot and exact sequence resumption
//! - Simulated 1-hour sustained streaming memory soak

pub mod benchmark;
pub mod domain;
pub mod engine;
pub mod plugin;
pub mod redraw;
pub mod virtualization;
pub mod workload;

pub use benchmark::{
    BenchmarkConfig, BenchmarkReport, LatencyDistributionUs, SnapshotRestoreReport, SoakReport,
    read_process_rss_kib, run_benchmark,
};
pub use domain::{
    BlockId, CitationId, CitationSource, ClinicalLabRow, CodeLanguage, ConversationTurn,
    DeltaPayload, HeadingLevel, InlineSpan, LabFlag, ListKind, MarkdownBlock, ProgressNoteCitation,
    ProgressNoteSection, SnapshotInvalidReason, StreamChunk, StreamSeq, StreamingPerfError,
    TranscriptBlock, TurnAuthor, TurnId,
};
pub use engine::{
    ActiveCitation, AppliedDeltaRecord, CitationPanelState, ComposerState,
    DEFAULT_MAX_RETAINED_BLOCKS, RECENT_DELTA_RING_CAPACITY, STREAMING_PARAGRAPH_ROLLOVER_CHARS,
    SessionSnapshot, StreamPhase, StreamingPrototypeStore,
};
pub use plugin::{
    CitationPanelSummarySlot, ComposerTextSlot, LabTableRowSlot, StreamingPerfPrototypePlugin,
    StreamingPrototypeResource, TranscriptSlot, VirtualizedEcsPoolState,
};
pub use redraw::{IdleRedrawPolicy, RedrawDecision, RedrawReason};
pub use virtualization::VirtualizedWindow;
pub use workload::{
    CITATION_SOURCE_COUNT, LAB_TABLE_ROW_COUNT, SyntheticClinicalWorkload, SyntheticTokenStream,
    WORKLOAD_TURN_COUNT,
};
