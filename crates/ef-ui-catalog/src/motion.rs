use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

/// User / OS reduced-motion preference (WCAG 2.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Resource)]
pub enum ReducedMotion {
    #[default]
    NoPreference,
    Reduce,
}

/// Whether decorative indicators animate continuously or stay static.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimationMode {
    Animated,
    Static,
}

/// Deterministic animation timing policy derived from [`ReducedMotion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MotionPolicy {
    pub overlay_transition_ms: u32,
    pub drawer_transition_ms: u32,
    pub scroll_easing_ms: u32,
    pub focus_ring_ms: u32,
    pub streaming_indicator: AnimationMode,
}

impl MotionPolicy {
    #[must_use]
    pub const fn for_preference(preference: ReducedMotion) -> Self {
        match preference {
            ReducedMotion::NoPreference => Self {
                overlay_transition_ms: 150,
                drawer_transition_ms: 180,
                scroll_easing_ms: 120,
                focus_ring_ms: 80,
                streaming_indicator: AnimationMode::Animated,
            },
            ReducedMotion::Reduce => Self {
                overlay_transition_ms: 0,
                drawer_transition_ms: 0,
                scroll_easing_ms: 0,
                focus_ring_ms: 0,
                streaming_indicator: AnimationMode::Static,
            },
        }
    }

    #[must_use]
    pub const fn allows_non_essential_motion(self) -> bool {
        self.overlay_transition_ms > 0
            || self.drawer_transition_ms > 0
            || self.scroll_easing_ms > 0
            || self.focus_ring_ms > 0
            || matches!(self.streaming_indicator, AnimationMode::Animated)
    }

    /// Evaluates normalized progress (`0.0..=1.0`). Under `ReducedMotion::Reduce`,
    /// durations are `0` and transitions settle instantaneously to `1.0`.
    #[must_use]
    pub fn progress(elapsed_ms: u32, duration_ms: u32) -> f32 {
        if duration_ms == 0 {
            return 1.0;
        }
        (elapsed_ms as f32 / duration_ms as f32).clamp(0.0, 1.0)
    }
}

/// Explicit cause justifying a reactive frame submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RedrawReason {
    UserInput,
    StreamDelta,
    ScrollOrResize,
    FocusTransition,
    OverlayTransition,
    LocaleOrThemeChange,
    DecorativeAnimationTick,
}

/// Decision returned by [`IdleRedrawGate::consume_frame`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameDecision {
    Render { reasons: Vec<RedrawReason> },
    SkipIdle,
}

/// Event-driven reactive redraw scheduler guaranteeing `0 frames/sec` when idle
/// per the M1/M2 idle budget contract.
#[derive(Debug, Clone, Default, Resource)]
pub struct IdleRedrawGate {
    pending: Vec<RedrawReason>,
    frames_rendered: u64,
    idle_ticks_skipped: u64,
}

impl IdleRedrawGate {
    /// Marks the frame dirty for a concrete interactive or streaming reason.
    pub fn request_redraw(&mut self, reason: RedrawReason) {
        if !self.pending.contains(&reason) {
            self.pending.push(reason);
        }
    }

    /// Requests a decorative animation tick only when `policy` permits motion.
    pub fn request_decorative_tick(&mut self, policy: MotionPolicy) {
        if policy.allows_non_essential_motion() {
            self.request_redraw(RedrawReason::DecorativeAnimationTick);
        }
    }

    /// Evaluates whether the renderer should submit a GPU frame on this tick.
    /// When no reason is queued, returns [`FrameDecision::SkipIdle`] and
    /// increments `idle_ticks_skipped` while keeping `frames_rendered` unchanged.
    pub fn consume_frame(&mut self) -> FrameDecision {
        if self.pending.is_empty() {
            self.idle_ticks_skipped += 1;
            return FrameDecision::SkipIdle;
        }
        let reasons = std::mem::take(&mut self.pending);
        self.frames_rendered += 1;
        FrameDecision::Render { reasons }
    }

    #[must_use]
    pub const fn is_idle(&self) -> bool {
        self.pending.is_empty()
    }

    #[must_use]
    pub const fn frames_rendered(&self) -> u64 {
        self.frames_rendered
    }

    #[must_use]
    pub const fn idle_ticks_skipped(&self) -> u64 {
        self.idle_ticks_skipped
    }
}
