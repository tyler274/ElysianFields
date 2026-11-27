//! M1 feasibility prototype for multiline text input, OS IME bridging, and
//! international text shaping across Bevy 0.20.1 targets.

pub mod adapter;
pub mod benchmark;
pub mod composer;
pub mod grapheme;
pub mod locale;
pub mod plugin;

pub use adapter::{
    AdapterMechanism, AdapterReadback, CompositionPresence, HeadlessSimulatedAdapter,
    PlatformInputAdapter, PlatformTarget, SelectionPresence,
};
pub use benchmark::{
    StreamingTypingBenchmarkConfig, StreamingTypingBenchmarkReport,
    build_synthetic_80_turn_transcript, run_streaming_typing_benchmark,
};
pub use composer::{
    CaretRect, ClipboardMutation, CommandOutcome, ComposerDocument, ComposerLayoutSnapshot,
    CursorState, DEFAULT_MAX_UNDO_DEPTH, DictationSegment, DictationSpacing, DocumentRevision,
    FocusState, ImeComposition, MoveDirection, NoOpReason, ScrollAdjustment, SelectionBehavior,
    SoftKeyboardState, TextInputCommand, ViewportMetrics,
};
pub use grapheme::{
    ByteOffset, LineColumn, LogicalDirection, VisualDirection, cluster_boundaries, cluster_count,
    detect_paragraph_direction, line_column_at, line_end_offset, line_start_offset,
    next_cluster_boundary, offset_for_line_column, prev_cluster_boundary, snap_to_cluster_boundary,
    visual_to_logical,
};
pub use locale::{
    FontBundleSpec, FontDeliveryStrategy, FontFamily, FontLicense, InputComplexity, Locale,
    LocaleMetadata, LocaleParseError, LocaleTier, TextDirection, WordSpacingPolicy, WritingScript,
};
pub use plugin::{
    ActiveTranscriptTurnViewNode, AdapterReadbackState, ComposerOutcomeMessage,
    ComposerRuntimeMetrics, ComposerState, ComposerViewNode, TextCommandMessage,
    TextI18nPrototypePlugin, TranscriptChunkMessage, TranscriptFeedState, TranscriptRole,
    TranscriptTurn,
};
