//! Surface context-loss and restoration state machine without page reload.

use std::fmt;

use bevy::prelude::Resource;

use crate::graphics::{BrowserGraphicsCapabilities, FallbackDiagnostic, GraphicsBackend};

/// Cause of a browser graphics surface interruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceLossCause {
    /// DOM `webglcontextlost` event fired on the active canvas.
    WebGlContextLost,
    /// WebGPU `GPUDevice.lost` promise resolved unexpectedly.
    WebGpuDeviceLost,
}

impl fmt::Display for SurfaceLossCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WebGlContextLost => f.write_str("webglcontextlost"),
            Self::WebGpuDeviceLost => f.write_str("webgpu-device-lost"),
        }
    }
}

/// Whether the frame loop is permitted to submit GPU work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderExecutionState {
    Active,
    PausedForRecovery,
}

/// Client UI state preserved across graphics surface loss and restoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservedUiSnapshot {
    pub active_route: String,
    pub draft_composer_text: String,
    pub scroll_offset_px: u32,
    pub focused_control: Option<String>,
}

/// Recovery strategy guarantee: surface restoration never reloads the browser page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadPolicy {
    NoPageReload,
}

/// State machine phase for graphics context loss and surface restoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextRecoveryPhase {
    /// Surface is active and rendering frames on the given backend and generation.
    Healthy {
        backend: GraphicsBackend,
        surface_generation: u32,
        total_recoveries: u32,
    },
    /// Surface was lost; rendering is paused and UI state is preserved until restoration.
    Suspended {
        previous_backend: GraphicsBackend,
        cause: SurfaceLossCause,
        preserved_ui: PreservedUiSnapshot,
        surface_generation: u32,
        total_recoveries: u32,
    },
    /// Browser signaled restoration (`webglcontextrestored` or fresh adapter); rebuilding GPU pipelines.
    Restoring {
        target_backend: GraphicsBackend,
        preserved_ui: PreservedUiSnapshot,
        next_generation: u32,
        total_recoveries: u32,
    },
    /// Restoration probe found no usable backend; rendering stays paused while UI draft remains preserved.
    Unrecoverable {
        cause: SurfaceLossCause,
        diagnostic: FallbackDiagnostic,
        preserved_ui: PreservedUiSnapshot,
        surface_generation: u32,
        total_recoveries: u32,
    },
}

/// Errors returned for invalid state transitions in [`ContextLossRecovery`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextRecoveryError {
    /// Initial backend is not renderable; cannot start a live surface recovery tracker.
    UnsupportedInitialBackend,
    /// Received context-lost while the surface is not currently healthy.
    NotHealthyForContextLoss,
    /// Received context-restored while not paused in a suspended or unrecoverable phase.
    NotAwaitingRestoration,
    /// Attempted to complete surface rebuild when no restoration is in progress.
    NotRestoring,
}

impl fmt::Display for ContextRecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedInitialBackend => {
                f.write_str("cannot initialize context recovery with an unsupported backend")
            }
            Self::NotHealthyForContextLoss => {
                f.write_str("context loss event is only valid from a healthy surface state")
            }
            Self::NotAwaitingRestoration => {
                f.write_str("context restored event is only valid while suspended or unrecoverable")
            }
            Self::NotRestoring => {
                f.write_str("cannot finalize surface rebuild outside the restoring phase")
            }
        }
    }
}

impl std::error::Error for ContextRecoveryError {}

/// Owns the browser GPU context-loss and restoration lifecycle without reloading the page.
#[derive(Debug, Clone, PartialEq, Eq, Resource)]
pub struct ContextLossRecovery {
    phase: ContextRecoveryPhase,
    reload_policy: ReloadPolicy,
}

impl ContextLossRecovery {
    /// Creates a healthy recovery state machine for a renderable initial backend.
    pub fn new(initial_backend: GraphicsBackend) -> Result<Self, ContextRecoveryError> {
        if !initial_backend.is_renderable() {
            return Err(ContextRecoveryError::UnsupportedInitialBackend);
        }

        Ok(Self {
            phase: ContextRecoveryPhase::Healthy {
                backend: initial_backend,
                surface_generation: 1,
                total_recoveries: 0,
            },
            reload_policy: ReloadPolicy::NoPageReload,
        })
    }

    /// Handles `webglcontextlost` or WebGPU device loss by pausing rendering and capturing UI state.
    pub fn on_context_lost(
        &mut self,
        cause: SurfaceLossCause,
        ui_snapshot: PreservedUiSnapshot,
    ) -> Result<(), ContextRecoveryError> {
        let (previous_backend, surface_generation, total_recoveries) = match &self.phase {
            ContextRecoveryPhase::Healthy {
                backend,
                surface_generation,
                total_recoveries,
            } => (backend.clone(), *surface_generation, *total_recoveries),
            ContextRecoveryPhase::Suspended { .. }
            | ContextRecoveryPhase::Restoring { .. }
            | ContextRecoveryPhase::Unrecoverable { .. } => {
                return Err(ContextRecoveryError::NotHealthyForContextLoss);
            }
        };

        self.phase = ContextRecoveryPhase::Suspended {
            previous_backend,
            cause,
            preserved_ui: ui_snapshot,
            surface_generation,
            total_recoveries,
        };
        Ok(())
    }

    /// Handles `webglcontextrestored` (or a replacement WebGPU/WebGL2 adapter) without reloading the page.
    pub fn on_context_restored(
        &mut self,
        capabilities: BrowserGraphicsCapabilities,
    ) -> Result<(), ContextRecoveryError> {
        let (cause, preserved_ui, surface_generation, total_recoveries) = match &self.phase {
            ContextRecoveryPhase::Suspended {
                cause,
                preserved_ui,
                surface_generation,
                total_recoveries,
                ..
            }
            | ContextRecoveryPhase::Unrecoverable {
                cause,
                preserved_ui,
                surface_generation,
                total_recoveries,
                ..
            } => (
                *cause,
                preserved_ui.clone(),
                *surface_generation,
                *total_recoveries,
            ),
            ContextRecoveryPhase::Healthy { .. } | ContextRecoveryPhase::Restoring { .. } => {
                return Err(ContextRecoveryError::NotAwaitingRestoration);
            }
        };

        let detected = GraphicsBackend::detect(capabilities);
        if detected.is_renderable() {
            self.phase = ContextRecoveryPhase::Restoring {
                target_backend: detected,
                preserved_ui,
                next_generation: surface_generation.saturating_add(1),
                total_recoveries,
            };
        } else {
            self.phase = ContextRecoveryPhase::Unrecoverable {
                cause,
                diagnostic: FallbackDiagnostic::context_permanently_lost(),
                preserved_ui,
                surface_generation,
                total_recoveries,
            };
        }

        Ok(())
    }

    /// Marks GPU surface and pipeline reconstruction complete, resuming rendering and returning
    /// the preserved UI snapshot so the caller restores state without a page reload.
    pub fn complete_surface_rebuild(
        &mut self,
    ) -> Result<PreservedUiSnapshot, ContextRecoveryError> {
        let (target_backend, preserved_ui, next_generation, total_recoveries) = match &self.phase {
            ContextRecoveryPhase::Restoring {
                target_backend,
                preserved_ui,
                next_generation,
                total_recoveries,
            } => (
                target_backend.clone(),
                preserved_ui.clone(),
                *next_generation,
                *total_recoveries,
            ),
            ContextRecoveryPhase::Healthy { .. }
            | ContextRecoveryPhase::Suspended { .. }
            | ContextRecoveryPhase::Unrecoverable { .. } => {
                return Err(ContextRecoveryError::NotRestoring);
            }
        };

        self.phase = ContextRecoveryPhase::Healthy {
            backend: target_backend,
            surface_generation: next_generation,
            total_recoveries: total_recoveries.saturating_add(1),
        };
        Ok(preserved_ui)
    }

    /// Returns whether rendering is active or paused for context recovery.
    #[must_use]
    pub const fn render_state(&self) -> RenderExecutionState {
        match &self.phase {
            ContextRecoveryPhase::Healthy { .. } => RenderExecutionState::Active,
            ContextRecoveryPhase::Suspended { .. }
            | ContextRecoveryPhase::Restoring { .. }
            | ContextRecoveryPhase::Unrecoverable { .. } => RenderExecutionState::PausedForRecovery,
        }
    }

    /// Returns the reload policy; surface recovery is strictly in-place without page reloads.
    #[must_use]
    pub const fn reload_policy(&self) -> ReloadPolicy {
        self.reload_policy
    }

    /// Returns the current recovery phase.
    #[must_use]
    pub const fn phase(&self) -> &ContextRecoveryPhase {
        &self.phase
    }

    /// Returns the active or last healthy surface generation counter.
    #[must_use]
    pub const fn surface_generation(&self) -> u32 {
        match &self.phase {
            ContextRecoveryPhase::Healthy {
                surface_generation, ..
            }
            | ContextRecoveryPhase::Suspended {
                surface_generation, ..
            }
            | ContextRecoveryPhase::Unrecoverable {
                surface_generation, ..
            } => *surface_generation,
            ContextRecoveryPhase::Restoring {
                next_generation, ..
            } => *next_generation,
        }
    }

    /// Returns the currently preserved UI snapshot whenever recovery is in progress or blocked.
    #[must_use]
    pub const fn preserved_ui(&self) -> Option<&PreservedUiSnapshot> {
        match &self.phase {
            ContextRecoveryPhase::Healthy { .. } => None,
            ContextRecoveryPhase::Suspended { preserved_ui, .. }
            | ContextRecoveryPhase::Restoring { preserved_ui, .. }
            | ContextRecoveryPhase::Unrecoverable { preserved_ui, .. } => Some(preserved_ui),
        }
    }
}
