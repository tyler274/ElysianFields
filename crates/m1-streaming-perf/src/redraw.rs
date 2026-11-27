//! Reactive zero-redraw idle scheduler.
//!
//! Frame rendering is strictly event-driven: when no streaming chunk, user input,
//! scroll, citation transition, viewport resize, or snapshot restoration occurs,
//! `IdleRedrawPolicy` suppresses frame submission so idle rendering drops to
//! `0 frames/sec`.

/// Typed reason why a redraw frame was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedrawReason {
    StreamDelta,
    ComposerInput,
    TranscriptScroll,
    CitationPanelChange,
    ViewportResize,
    StateRestored,
}

/// Decision returned by `IdleRedrawPolicy::evaluate_tick`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedrawDecision {
    /// Nothing changed; skip rendering completely (0 frames rendered).
    Idle,
    /// Visual state changed; render a single frame for the recorded primary reason.
    RenderNeeded(RedrawReason),
}

/// Single owner for reactive redraw scheduling and idle frame counters.
#[derive(Debug, Clone, Default)]
pub struct IdleRedrawPolicy {
    pending_reason: Option<RedrawReason>,
    dirty_frames_requested: u64,
    frames_rendered: u64,
    idle_ticks_skipped: u64,
}

impl IdleRedrawPolicy {
    /// Creates an idle redraw policy with zero pending work.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending_reason: None,
            dirty_frames_requested: 0,
            frames_rendered: 0,
            idle_ticks_skipped: 0,
        }
    }

    /// Marks the UI dirty for a concrete event reason.
    ///
    /// Multiple mutations within the same frame coalesce into a single frame request.
    pub fn request_redraw(&mut self, reason: RedrawReason) {
        if self.pending_reason.is_none() {
            self.dirty_frames_requested = self.dirty_frames_requested.saturating_add(1);
            self.pending_reason = Some(reason);
        }
    }

    /// Evaluates the current tick and consumes any pending redraw request.
    pub fn evaluate_tick(&mut self) -> RedrawDecision {
        match self.pending_reason.take() {
            Some(reason) => {
                self.frames_rendered = self.frames_rendered.saturating_add(1);
                RedrawDecision::RenderNeeded(reason)
            }
            None => {
                self.idle_ticks_skipped = self.idle_ticks_skipped.saturating_add(1);
                RedrawDecision::Idle
            }
        }
    }

    /// Returns true if a redraw is currently queued.
    #[must_use]
    pub const fn has_pending_redraw(&self) -> bool {
        self.pending_reason.is_some()
    }

    /// Total coalesced dirty frames requested since startup or reset.
    #[must_use]
    pub const fn dirty_frames_requested(&self) -> u64 {
        self.dirty_frames_requested
    }

    /// Total frames actually rendered since startup or reset.
    #[must_use]
    pub const fn frames_rendered(&self) -> u64 {
        self.frames_rendered
    }

    /// Total idle ticks where rendering was skipped (`0 frames/sec`).
    #[must_use]
    pub const fn idle_ticks_skipped(&self) -> u64 {
        self.idle_ticks_skipped
    }

    /// Resets window counters when entering a new measurement phase (e.g., active -> idle).
    pub fn reset_phase_counters(&mut self) {
        self.dirty_frames_requested = 0;
        self.frames_rendered = 0;
        self.idle_ticks_skipped = 0;
    }
}
