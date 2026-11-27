//! Core domain engine for O(1) incremental delta streaming, non-modal citation
//! inspection, multiline composer input, and background/return session snapshots.
//!
//! This module owns all authoritative UI domain state independently of Bevy ECS
//! entities or rendering lifetimes.

use std::collections::VecDeque;

use crate::domain::{
    BlockId, CitationId, CitationSource, ClinicalLabRow, DeltaPayload, InlineSpan, MarkdownBlock,
    SnapshotInvalidReason, StreamChunk, StreamSeq, StreamingPerfError, TranscriptBlock, TurnAuthor,
    TurnId,
};
use crate::redraw::{IdleRedrawPolicy, RedrawReason};
use crate::virtualization::VirtualizedWindow;
use crate::workload::SyntheticClinicalWorkload;

/// Maximum characters accumulated into one streaming paragraph block before rolling
/// over to a fresh tail paragraph block, keeping per-block text sync strictly O(1).
pub const STREAMING_PARAGRAPH_ROLLOVER_CHARS: usize = 420;

/// Maximum transcript blocks retained in the bounded memory ring during multi-hour
/// streaming soaks (well above the 80-turn ~320-block transcript while guaranteeing
/// bounded RSS over 144,000+ tokens).
pub const DEFAULT_MAX_RETAINED_BLOCKS: usize = 1_024;

/// Fixed capacity for the diagnostic ring buffer recording recent stream deltas.
pub const RECENT_DELTA_RING_CAPACITY: usize = 256;

/// Lifecycle state of the active assistant response stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamPhase {
    Idle,
    Active {
        turn_id: TurnId,
        next_seq: StreamSeq,
    },
    Suspended {
        turn_id: TurnId,
        resume_seq: StreamSeq,
    },
}

/// Active citation shown inside the non-modal side panel.
#[derive(Debug, Clone, PartialEq)]
pub enum ActiveCitation {
    ProgressNote(CitationId),
    VirtualizedLabTable {
        citation: CitationId,
        window: VirtualizedWindow,
    },
}

/// Non-modal citation side panel state.
#[derive(Debug, Clone, PartialEq)]
pub enum CitationPanelState {
    Closed,
    Open(ActiveCitation),
}

/// Single authoritative owner for multiline physician composer text and cursor state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComposerState {
    buffer: String,
    cursor_chars: usize,
    keystrokes_applied: u64,
}

impl ComposerState {
    /// Creates an empty composer buffer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor_chars: 0,
            keystrokes_applied: 0,
        }
    }

    /// Inserts one character at the current cursor position.
    pub fn insert_char(&mut self, ch: char) {
        let byte_idx = self.cursor_byte_index();
        self.buffer.insert(byte_idx, ch);
        self.cursor_chars = self.cursor_chars.saturating_add(1);
        self.keystrokes_applied = self.keystrokes_applied.saturating_add(1);
    }

    /// Deletes one character immediately before the cursor, if present.
    pub fn backspace(&mut self) {
        if self.cursor_chars == 0 {
            return;
        }
        let target_char = self.cursor_chars - 1;
        if let Some((byte_idx, _)) = self.buffer.char_indices().nth(target_char) {
            self.buffer.remove(byte_idx);
            self.cursor_chars = target_char;
            self.keystrokes_applied = self.keystrokes_applied.saturating_add(1);
        }
    }

    /// Replaces the composer draft and cursor from a validated snapshot.
    pub fn restore(&mut self, buffer: String, cursor_chars: usize) {
        let max_chars = buffer.chars().count();
        self.buffer = buffer;
        self.cursor_chars = cursor_chars.min(max_chars);
    }

    /// Current multiline text draft.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.buffer
    }

    /// Current cursor offset in Unicode scalar values.
    #[must_use]
    pub const fn cursor_chars(&self) -> usize {
        self.cursor_chars
    }

    /// Total keystrokes applied during the session.
    #[must_use]
    pub const fn keystrokes_applied(&self) -> u64 {
        self.keystrokes_applied
    }

    fn cursor_byte_index(&self) -> usize {
        self.buffer
            .char_indices()
            .nth(self.cursor_chars)
            .map_or(self.buffer.len(), |(idx, _)| idx)
    }
}

/// Record stored in the fixed-capacity delta ring buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedDeltaRecord {
    pub seq: StreamSeq,
    pub block_id: BlockId,
    pub token_count: u32,
}

/// Complete background/return restoration snapshot.
///
/// Captures exact scroll positions, non-modal citation state, composer draft,
/// and the active streaming sequence number so resuming after backgrounding
/// never drops or duplicates tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshot {
    pub transcript_window: VirtualizedWindow,
    pub citation_panel: CitationPanelState,
    pub composer_text: String,
    pub composer_cursor_chars: usize,
    pub stream_phase: StreamPhase,
}

/// Authoritative domain store for the M1 streaming clinical transcript prototype.
#[derive(Debug, Clone)]
pub struct StreamingPrototypeStore {
    blocks: VecDeque<TranscriptBlock>,
    max_retained_blocks: usize,
    next_block_id: u32,
    turn_count: u32,
    total_chars_ingested: usize,
    total_tokens_streamed: u64,
    incremental_delta_count: u64,
    full_reparse_count: u64,
    blocks_evicted: u64,
    stream_phase: StreamPhase,
    dirty_blocks: Vec<BlockId>,
    recent_deltas: VecDeque<AppliedDeltaRecord>,
    citation_sources: Vec<CitationSource>,
    lab_rows: Vec<ClinicalLabRow>,
    transcript_window: VirtualizedWindow,
    citation_panel: CitationPanelState,
    composer: ComposerState,
    redraw_policy: IdleRedrawPolicy,
}

impl StreamingPrototypeStore {
    /// Builds a store initialized with the deterministic 80-turn clinical workload
    /// and 5,000-row laboratory table.
    pub fn from_workload(
        workload: SyntheticClinicalWorkload,
        transcript_window: VirtualizedWindow,
    ) -> Self {
        let mut blocks =
            VecDeque::with_capacity(workload.total_blocks.min(DEFAULT_MAX_RETAINED_BLOCKS));
        let mut next_block_id: u32 = 0;
        let turn_count = workload.turns.len() as u32;

        for turn in workload.turns {
            for block in turn.blocks {
                next_block_id = next_block_id.max(block.id.0.saturating_add(1));
                blocks.push_back(block);
            }
        }

        let mut store = Self {
            blocks,
            max_retained_blocks: DEFAULT_MAX_RETAINED_BLOCKS,
            next_block_id,
            turn_count,
            total_chars_ingested: workload.total_chars,
            total_tokens_streamed: 0,
            incremental_delta_count: 0,
            full_reparse_count: 0,
            blocks_evicted: 0,
            stream_phase: StreamPhase::Idle,
            dirty_blocks: Vec::with_capacity(8),
            recent_deltas: VecDeque::with_capacity(RECENT_DELTA_RING_CAPACITY),
            citation_sources: workload.citation_sources,
            lab_rows: workload.lab_rows,
            transcript_window,
            citation_panel: CitationPanelState::Closed,
            composer: ComposerState::new(),
            redraw_policy: IdleRedrawPolicy::new(),
        };

        // Mark initial layout dirty so the first frame materializes the viewport.
        store
            .redraw_policy
            .request_redraw(RedrawReason::StateRestored);
        store
    }

    /// Configures the bounded block ring capacity used for long streaming soaks.
    pub fn set_max_retained_blocks(&mut self, max_retained_blocks: usize) {
        self.max_retained_blocks = max_retained_blocks.max(64);
        self.enforce_block_retention();
    }

    /// Begins a new streaming assistant turn starting at `initial_seq`.
    pub fn begin_assistant_stream(&mut self, initial_seq: StreamSeq) -> TurnId {
        let turn_id = TurnId(self.turn_count);
        self.turn_count = self.turn_count.saturating_add(1);
        let block_id = self.push_new_block(
            turn_id,
            TurnAuthor::Assistant,
            MarkdownBlock::Paragraph {
                spans: vec![InlineSpan::Text(String::with_capacity(
                    STREAMING_PARAGRAPH_ROLLOVER_CHARS,
                ))],
            },
        );
        self.stream_phase = StreamPhase::Active {
            turn_id,
            next_seq: initial_seq,
        };
        self.mark_block_dirty(block_id);
        self.redraw_policy.request_redraw(RedrawReason::StreamDelta);
        turn_id
    }

    /// Single owner for incremental O(1) delta-append streaming.
    ///
    /// Never re-parses or re-diffs prior turns or blocks; mutates only the active
    /// tail block (or appends one fresh tail block on rollover) and invalidates
    /// only that single block ID.
    pub fn apply_stream_chunk(
        &mut self,
        chunk: StreamChunk,
    ) -> Result<BlockId, StreamingPerfError> {
        let (turn_id, expected_seq) = match self.stream_phase {
            StreamPhase::Active { turn_id, next_seq } => (turn_id, next_seq),
            StreamPhase::Idle | StreamPhase::Suspended { .. } => {
                return Err(StreamingPerfError::NoActiveStreamingTurn);
            }
        };

        if chunk.seq != expected_seq {
            return Err(StreamingPerfError::OutOfOrderStreamChunk {
                expected: expected_seq,
                actual: chunk.seq,
            });
        }

        let affected_block_id = match chunk.payload {
            DeltaPayload::AppendText(delta) => {
                self.total_chars_ingested = self
                    .total_chars_ingested
                    .saturating_add(delta.chars().count());
                self.append_text_to_tail(turn_id, &delta)
            }
            DeltaPayload::AppendCitation(citation_id) => {
                self.total_chars_ingested = self.total_chars_ingested.saturating_add(4);
                self.append_citation_to_tail(turn_id, citation_id)
            }
            DeltaPayload::StartBlock(block) => {
                self.total_chars_ingested =
                    self.total_chars_ingested.saturating_add(block.char_count());
                self.push_new_block(turn_id, TurnAuthor::Assistant, block)
            }
        };

        self.stream_phase = StreamPhase::Active {
            turn_id,
            next_seq: expected_seq.next(),
        };
        self.total_tokens_streamed = self
            .total_tokens_streamed
            .saturating_add(u64::from(chunk.token_count));
        self.incremental_delta_count = self.incremental_delta_count.saturating_add(1);

        if self.recent_deltas.len() >= RECENT_DELTA_RING_CAPACITY {
            let _ = self.recent_deltas.pop_front();
        }
        self.recent_deltas.push_back(AppliedDeltaRecord {
            seq: chunk.seq,
            block_id: affected_block_id,
            token_count: chunk.token_count,
        });

        self.mark_block_dirty(affected_block_id);
        self.redraw_policy.request_redraw(RedrawReason::StreamDelta);
        Ok(affected_block_id)
    }

    /// Ends the active assistant stream and returns to idle streaming phase.
    pub fn finish_assistant_stream(&mut self) {
        self.stream_phase = StreamPhase::Idle;
    }

    /// Applies one keystroke to the multiline composer and requests a reactive redraw.
    pub fn type_composer_char(&mut self, ch: char) {
        self.composer.insert_char(ch);
        self.redraw_policy
            .request_redraw(RedrawReason::ComposerInput);
    }

    /// Applies a backspace in the composer and requests a reactive redraw.
    pub fn composer_backspace(&mut self) {
        self.composer.backspace();
        self.redraw_policy
            .request_redraw(RedrawReason::ComposerInput);
    }

    /// Scrolls the main transcript viewport by `delta_px` and requests redraw only if offset changed.
    pub fn scroll_transcript_by(&mut self, delta_px: f32) {
        let before = self.transcript_window.scroll_offset_px;
        self.transcript_window
            .scroll_by(delta_px, self.blocks.len());
        if (self.transcript_window.scroll_offset_px - before).abs() > f32::EPSILON {
            self.redraw_policy
                .request_redraw(RedrawReason::TranscriptScroll);
        }
    }

    /// Resizes the main viewport height (e.g., desktop resize or mobile soft-keyboard change).
    pub fn resize_viewport(&mut self, new_height_px: f32) -> Result<(), StreamingPerfError> {
        self.transcript_window
            .set_viewport_height(new_height_px, self.blocks.len())?;
        if let CitationPanelState::Open(ActiveCitation::VirtualizedLabTable { window, .. }) =
            &mut self.citation_panel
        {
            window.set_viewport_height(new_height_px, self.lab_rows.len())?;
        }
        self.redraw_policy
            .request_redraw(RedrawReason::ViewportResize);
        Ok(())
    }

    /// Opens or switches the non-modal citation side panel to the specified citation `[1]`..=`[12]`.
    pub fn open_citation(&mut self, citation: CitationId) -> Result<(), StreamingPerfError> {
        let source = self
            .citation_source(citation)
            .ok_or(StreamingPerfError::MissingCitationSource { citation })?;

        let active = match source {
            CitationSource::ProgressNote(_) => ActiveCitation::ProgressNote(citation),
            CitationSource::VirtualizedLabTable { .. } => ActiveCitation::VirtualizedLabTable {
                citation,
                window: VirtualizedWindow::new(
                    0.0,
                    self.transcript_window.viewport_height_px,
                    28.0,
                    4,
                )?,
            },
        };

        self.citation_panel = CitationPanelState::Open(active);
        self.redraw_policy
            .request_redraw(RedrawReason::CitationPanelChange);
        Ok(())
    }

    /// Closes the non-modal citation side panel.
    pub fn close_citation_panel(&mut self) {
        if !matches!(self.citation_panel, CitationPanelState::Closed) {
            self.citation_panel = CitationPanelState::Closed;
            self.redraw_policy
                .request_redraw(RedrawReason::CitationPanelChange);
        }
    }

    /// Scrolls the 5,000-row virtualized clinical lab table when `[12]` is open.
    pub fn scroll_citation_table_by(&mut self, delta_px: f32) {
        if let CitationPanelState::Open(ActiveCitation::VirtualizedLabTable { window, .. }) =
            &mut self.citation_panel
        {
            let before = window.scroll_offset_px;
            window.scroll_by(delta_px, self.lab_rows.len());
            if (window.scroll_offset_px - before).abs() > f32::EPSILON {
                self.redraw_policy
                    .request_redraw(RedrawReason::CitationPanelChange);
            }
        }
    }

    /// Captures a background/suspend snapshot preserving scroll, citation, composer, and stream sequence.
    pub fn capture_snapshot(&mut self) -> SessionSnapshot {
        let suspended_phase = match self.stream_phase {
            StreamPhase::Active { turn_id, next_seq } => {
                let suspended = StreamPhase::Suspended {
                    turn_id,
                    resume_seq: next_seq,
                };
                self.stream_phase = suspended;
                suspended
            }
            other => other,
        };

        SessionSnapshot {
            transcript_window: self.transcript_window,
            citation_panel: self.citation_panel.clone(),
            composer_text: self.composer.text().to_owned(),
            composer_cursor_chars: self.composer.cursor_chars(),
            stream_phase: suspended_phase,
        }
    }

    /// Restores a background/return snapshot and resumes the stream at its exact sequence number.
    pub fn restore_snapshot(
        &mut self,
        snapshot: SessionSnapshot,
    ) -> Result<(), StreamingPerfError> {
        if let StreamPhase::Suspended { turn_id, .. } | StreamPhase::Active { turn_id, .. } =
            snapshot.stream_phase
            && turn_id.0 >= self.turn_count
        {
            return Err(StreamingPerfError::InvalidSnapshotState {
                reason: SnapshotInvalidReason::UnknownActiveTurn,
            });
        }

        if let CitationPanelState::Open(active) = &snapshot.citation_panel {
            let citation = match active {
                ActiveCitation::ProgressNote(id)
                | ActiveCitation::VirtualizedLabTable { citation: id, .. } => *id,
            };
            if self.citation_source(citation).is_none() {
                return Err(StreamingPerfError::InvalidSnapshotState {
                    reason: SnapshotInvalidReason::UnknownCitation,
                });
            }
        }

        self.transcript_window = snapshot.transcript_window;
        self.citation_panel = snapshot.citation_panel;
        self.composer
            .restore(snapshot.composer_text, snapshot.composer_cursor_chars);
        self.stream_phase = match snapshot.stream_phase {
            StreamPhase::Suspended {
                turn_id,
                resume_seq,
            } => StreamPhase::Active {
                turn_id,
                next_seq: resume_seq,
            },
            other => other,
        };
        self.redraw_policy
            .request_redraw(RedrawReason::StateRestored);
        Ok(())
    }

    /// Returns the citation backing document for `citation`, if present.
    #[must_use]
    pub fn citation_source(&self, citation: CitationId) -> Option<&CitationSource> {
        self.citation_sources.iter().find(|source| match source {
            CitationSource::ProgressNote(note) => note.id == citation,
            CitationSource::VirtualizedLabTable { id, .. } => *id == citation,
        })
    }

    /// Drains the dirty block IDs accumulated since the last UI synchronization pass.
    pub fn drain_dirty_blocks(&mut self) -> Vec<BlockId> {
        std::mem::take(&mut self.dirty_blocks)
    }

    /// Retained transcript blocks indexed in O(1).
    #[must_use]
    pub const fn blocks(&self) -> &VecDeque<TranscriptBlock> {
        &self.blocks
    }

    /// 5,000-row structured clinical lab table.
    #[must_use]
    pub fn lab_rows(&self) -> &[ClinicalLabRow] {
        &self.lab_rows
    }

    /// Active virtualized window for the main transcript.
    #[must_use]
    pub const fn transcript_window(&self) -> VirtualizedWindow {
        self.transcript_window
    }

    /// Current non-modal citation side panel state.
    #[must_use]
    pub const fn citation_panel(&self) -> &CitationPanelState {
        &self.citation_panel
    }

    /// Authoritative composer state.
    #[must_use]
    pub const fn composer(&self) -> &ComposerState {
        &self.composer
    }

    /// Reactive zero-redraw idle policy.
    #[must_use]
    pub const fn redraw_policy(&self) -> &IdleRedrawPolicy {
        &self.redraw_policy
    }

    /// Mutable access to the reactive redraw policy for frame evaluation.
    pub fn redraw_policy_mut(&mut self) -> &mut IdleRedrawPolicy {
        &mut self.redraw_policy
    }

    /// Current streaming phase and next expected sequence number.
    #[must_use]
    pub const fn stream_phase(&self) -> StreamPhase {
        self.stream_phase
    }

    /// Total conversation turns created (initial 80 + any streaming turns).
    #[must_use]
    pub const fn turn_count(&self) -> u32 {
        self.turn_count
    }

    /// Total characters ingested across initial workload and streaming deltas.
    #[must_use]
    pub const fn total_chars_ingested(&self) -> usize {
        self.total_chars_ingested
    }

    /// Total tokens streamed via incremental delta-append.
    #[must_use]
    pub const fn total_tokens_streamed(&self) -> u64 {
        self.total_tokens_streamed
    }

    /// Number of incremental O(1) streaming deltas applied.
    #[must_use]
    pub const fn incremental_delta_count(&self) -> u64 {
        self.incremental_delta_count
    }

    /// Number of full-transcript reparses performed (invariant: always 0).
    #[must_use]
    pub const fn full_reparse_count(&self) -> u64 {
        self.full_reparse_count
    }

    /// Number of oldest off-screen blocks evicted by the bounded memory ring.
    #[must_use]
    pub const fn blocks_evicted(&self) -> u64 {
        self.blocks_evicted
    }

    /// Bounded diagnostic delta history ring.
    #[must_use]
    pub const fn recent_deltas(&self) -> &VecDeque<AppliedDeltaRecord> {
        &self.recent_deltas
    }

    fn append_text_to_tail(&mut self, turn_id: TurnId, delta: &str) -> BlockId {
        let needs_rollover = match self.blocks.back() {
            Some(last) if last.turn_id == turn_id => match &last.content {
                MarkdownBlock::Paragraph { spans } => {
                    let current_len: usize = spans.iter().map(InlineSpan::char_len).sum();
                    current_len.saturating_add(delta.chars().count())
                        > STREAMING_PARAGRAPH_ROLLOVER_CHARS
                }
                _ => true,
            },
            _ => true,
        };

        if needs_rollover {
            let mut text = String::with_capacity(STREAMING_PARAGRAPH_ROLLOVER_CHARS);
            text.push_str(delta);
            return self.push_new_block(
                turn_id,
                TurnAuthor::Assistant,
                MarkdownBlock::Paragraph {
                    spans: vec![InlineSpan::Text(text)],
                },
            );
        }

        if let Some(last) = self.blocks.back_mut()
            && let MarkdownBlock::Paragraph { spans } = &mut last.content
        {
            match spans.last_mut() {
                Some(InlineSpan::Text(existing)) => existing.push_str(delta),
                _ => spans.push(InlineSpan::Text(delta.to_owned())),
            }
            last.revision = last.revision.saturating_add(1);
            return last.id;
        }

        self.push_new_block(
            turn_id,
            TurnAuthor::Assistant,
            MarkdownBlock::Paragraph {
                spans: vec![InlineSpan::Text(delta.to_owned())],
            },
        )
    }

    fn append_citation_to_tail(&mut self, turn_id: TurnId, citation: CitationId) -> BlockId {
        if let Some(last) = self.blocks.back_mut()
            && last.turn_id == turn_id
            && let MarkdownBlock::Paragraph { spans } = &mut last.content
        {
            spans.push(InlineSpan::Citation(citation));
            last.revision = last.revision.saturating_add(1);
            return last.id;
        }

        self.push_new_block(
            turn_id,
            TurnAuthor::Assistant,
            MarkdownBlock::Paragraph {
                spans: vec![InlineSpan::Citation(citation)],
            },
        )
    }

    fn push_new_block(
        &mut self,
        turn_id: TurnId,
        author: TurnAuthor,
        content: MarkdownBlock,
    ) -> BlockId {
        let id = BlockId(self.next_block_id);
        self.next_block_id = self.next_block_id.saturating_add(1);
        self.blocks.push_back(TranscriptBlock {
            id,
            turn_id,
            author,
            revision: 1,
            content,
        });
        self.enforce_block_retention();
        id
    }

    fn enforce_block_retention(&mut self) {
        while self.blocks.len() > self.max_retained_blocks {
            if self.blocks.pop_front().is_some() {
                self.blocks_evicted = self.blocks_evicted.saturating_add(1);
            }
        }
    }

    fn mark_block_dirty(&mut self, block_id: BlockId) {
        if !self.dirty_blocks.contains(&block_id) {
            self.dirty_blocks.push(block_id);
        }
    }
}
