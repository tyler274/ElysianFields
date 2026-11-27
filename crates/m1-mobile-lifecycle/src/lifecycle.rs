//! Typed mobile application lifecycle state machine and render-loop suspension gate.
//!
//! This module owns the single invariant governing whether the client may submit
//! frames to the GPU: frames are rendered only while [`AppLifecycleState::Foreground`]
//! is active. Backgrounded and terminated states suspend rendering completely
//! (`0` frames rendered while hidden).

use std::fmt;

/// Canonical mobile application lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppLifecycleState {
    /// Application window is active and visible on screen.
    Foreground,
    /// Application is hidden in the background; rendering must be fully paused.
    BackgroundPaused,
    /// OS terminated the application process; cold launch is required to run again.
    ProcessTerminated,
}

/// Lifecycle transition inputs delivered by Android or iOS platform adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppLifecycleEvent {
    /// OS moved the application into the background (`onStop` / `sceneDidEnterBackground`).
    EnterBackground,
    /// OS returned the application to the foreground (`onResume` / `sceneWillEnterForeground`).
    ResumeForeground,
    /// OS reclaimed the application process while backgrounded or under memory pressure.
    OsProcessTerminate,
    /// OS launched a fresh process instance.
    ColdLaunch,
}

/// Result of evaluating a single render-loop tick against the lifecycle gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderTickOutcome {
    /// Frame was permitted and counted while in [`AppLifecycleState::Foreground`].
    Rendered { frame_index: u64 },
    /// Frame was suppressed because the app is not in the foreground.
    Suppressed { state: AppLifecycleState },
}

/// Error returned when an invalid lifecycle transition is requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifecycleTransitionError {
    pub current: AppLifecycleState,
    pub event: AppLifecycleEvent,
}

impl fmt::Display for LifecycleTransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid mobile lifecycle transition from {:?} via {:?}",
            self.current, self.event
        )
    }
}

impl std::error::Error for LifecycleTransitionError {}

/// Single authoritative owner of mobile lifecycle state and render suspension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleController {
    state: AppLifecycleState,
    frames_rendered_total: u64,
    frames_rendered_while_hidden: u64,
    suppressed_ticks: u64,
}

impl LifecycleController {
    /// Creates a lifecycle controller in [`AppLifecycleState::Foreground`].
    #[must_use]
    pub const fn new_foreground() -> Self {
        Self {
            state: AppLifecycleState::Foreground,
            frames_rendered_total: 0,
            frames_rendered_while_hidden: 0,
            suppressed_ticks: 0,
        }
    }

    /// Creates a lifecycle controller starting from a terminated process state,
    /// representing an app awaiting a cold launch.
    #[must_use]
    pub const fn new_terminated() -> Self {
        Self {
            state: AppLifecycleState::ProcessTerminated,
            frames_rendered_total: 0,
            frames_rendered_while_hidden: 0,
            suppressed_ticks: 0,
        }
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub const fn state(&self) -> AppLifecycleState {
        self.state
    }

    /// Single rule owner for the render-loop suspension gate.
    ///
    /// Returns `true` strictly in [`AppLifecycleState::Foreground`], ensuring
    /// zero frames are drawn while backgrounded or terminated.
    #[must_use]
    pub const fn should_draw(&self) -> bool {
        match self.state {
            AppLifecycleState::Foreground => true,
            AppLifecycleState::BackgroundPaused | AppLifecycleState::ProcessTerminated => false,
        }
    }

    /// Applies an OS lifecycle event, validating allowed state transitions.
    pub fn transition(
        &mut self,
        event: AppLifecycleEvent,
    ) -> Result<AppLifecycleState, LifecycleTransitionError> {
        let next = match (self.state, event) {
            (AppLifecycleState::Foreground, AppLifecycleEvent::EnterBackground) => {
                AppLifecycleState::BackgroundPaused
            }
            (AppLifecycleState::BackgroundPaused, AppLifecycleEvent::ResumeForeground) => {
                AppLifecycleState::Foreground
            }
            (
                AppLifecycleState::Foreground | AppLifecycleState::BackgroundPaused,
                AppLifecycleEvent::OsProcessTerminate,
            ) => AppLifecycleState::ProcessTerminated,
            (AppLifecycleState::ProcessTerminated, AppLifecycleEvent::ColdLaunch) => {
                AppLifecycleState::Foreground
            }
            // Idempotent same-state signals from platform callbacks.
            (AppLifecycleState::Foreground, AppLifecycleEvent::ResumeForeground) => {
                AppLifecycleState::Foreground
            }
            (AppLifecycleState::BackgroundPaused, AppLifecycleEvent::EnterBackground) => {
                AppLifecycleState::BackgroundPaused
            }
            (current, invalid_event) => {
                return Err(LifecycleTransitionError {
                    current,
                    event: invalid_event,
                });
            }
        };

        self.state = next;
        Ok(next)
    }

    /// Evaluates one frame-tick against [`Self::should_draw`].
    pub fn tick_render_frame(&mut self) -> RenderTickOutcome {
        if !self.should_draw() {
            self.suppressed_ticks = self.suppressed_ticks.saturating_add(1);
            return RenderTickOutcome::Suppressed { state: self.state };
        }

        self.frames_rendered_total = self.frames_rendered_total.saturating_add(1);
        RenderTickOutcome::Rendered {
            frame_index: self.frames_rendered_total,
        }
    }

    /// Total frames rendered while in foreground.
    #[must_use]
    pub const fn frames_rendered_total(&self) -> u64 {
        self.frames_rendered_total
    }

    /// Count of frames rendered while hidden (invariant: always `0`).
    #[must_use]
    pub const fn frames_rendered_while_hidden(&self) -> u64 {
        self.frames_rendered_while_hidden
    }

    /// Number of update ticks suppressed by the background/terminated gate.
    #[must_use]
    pub const fn suppressed_ticks(&self) -> u64 {
        self.suppressed_ticks
    }
}
