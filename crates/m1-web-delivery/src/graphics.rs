//! Graphics backend detection and human-readable fallback diagnostics for browser delivery.

use std::fmt;

use bevy::prelude::Resource;

/// Whether the browser exposes a working WebGPU adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebGpuAvailability {
    Available,
    Unavailable,
}

/// Whether the browser can create a WebGL2 rendering context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebGl2Availability {
    Available,
    Unavailable,
}

/// Typed graphics capabilities probed from the browser host environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserGraphicsCapabilities {
    pub webgpu: WebGpuAvailability,
    pub webgl2: WebGl2Availability,
}

impl BrowserGraphicsCapabilities {
    /// Capabilities representing a browser with both WebGPU and WebGL2 available.
    #[must_use]
    pub const fn webgpu_and_webgl2() -> Self {
        Self {
            webgpu: WebGpuAvailability::Available,
            webgl2: WebGl2Availability::Available,
        }
    }

    /// Capabilities representing the mandatory WebGL2 baseline without WebGPU.
    #[must_use]
    pub const fn webgl2_only() -> Self {
        Self {
            webgpu: WebGpuAvailability::Unavailable,
            webgl2: WebGl2Availability::Available,
        }
    }

    /// Capabilities representing an environment where neither WebGPU nor WebGL2 is usable.
    #[must_use]
    pub const fn unsupported() -> Self {
        Self {
            webgpu: WebGpuAvailability::Unavailable,
            webgl2: WebGl2Availability::Unavailable,
        }
    }
}

/// Structured reason why hardware-accelerated browser rendering cannot start or resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedGraphicsReason {
    MissingWebGpuAndWebGl2,
    ContextPermanentlyLost,
}

impl fmt::Display for UnsupportedGraphicsReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingWebGpuAndWebGl2 => {
                f.write_str("neither WebGPU nor WebGL2 is available in this browser")
            }
            Self::ContextPermanentlyLost => {
                f.write_str("graphics context could not be restored after device loss")
            }
        }
    }
}

/// Actionable, readable diagnostic shown when browser graphics initialization fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackDiagnostic {
    pub reason: UnsupportedGraphicsReason,
    pub headline: &'static str,
    pub detail: &'static str,
    pub remediation_steps: &'static [&'static str],
}

impl FallbackDiagnostic {
    /// Diagnostic presented when neither WebGPU nor WebGL2 is available at startup.
    #[must_use]
    pub const fn missing_webgpu_and_webgl2() -> Self {
        Self {
            reason: UnsupportedGraphicsReason::MissingWebGpuAndWebGl2,
            headline: "Hardware-accelerated browser graphics unavailable",
            detail: "Elysian Fields requires WebGL2 (or WebGPU) to render interactive workspaces. This browser session reported neither backend as available.",
            remediation_steps: &[
                "Enable hardware acceleration in your browser settings and reopen this tab.",
                "Update to a supported browser version (Chromium 120+, Firefox 121+, or Safari 17.4+).",
                "Check that enterprise policies or browser extensions are not blocking WebGL2 contexts.",
            ],
        }
    }

    /// Diagnostic presented when the GPU context is lost and cannot be restored.
    #[must_use]
    pub const fn context_permanently_lost() -> Self {
        Self {
            reason: UnsupportedGraphicsReason::ContextPermanentlyLost,
            headline: "Graphics surface could not be restored",
            detail: "The browser graphics device was reset and no replacement WebGPU or WebGL2 context is currently available. Your draft state remains preserved.",
            remediation_steps: &[
                "Close unused GPU-heavy tabs to free graphics memory.",
                "Verify your graphics driver or browser GPU process is healthy, then retry surface restoration.",
            ],
        }
    }

    /// Formats a plain-text diagnostic suitable for fallback UI or logs.
    #[must_use]
    pub fn render_message(&self) -> String {
        let mut output = String::new();
        output.push_str(self.headline);
        output.push_str("\n\n");
        output.push_str(self.detail);
        output.push_str("\n\nRecommended actions:");
        for step in self.remediation_steps {
            output.push_str("\n- ");
            output.push_str(step);
        }
        output
    }
}

/// Active browser graphics backend selected by the single detection rule.
#[derive(Debug, Clone, PartialEq, Eq, Resource)]
pub enum GraphicsBackend {
    /// Opportunistic WebGPU path when supported by the browser.
    WebGpu,
    /// Mandatory cross-browser WebGL2 baseline across Chromium, Firefox, and Safari.
    WebGl2,
    /// Neither WebGPU nor WebGL2 is available; carries a readable fallback diagnostic.
    Unsupported(FallbackDiagnostic),
}

impl GraphicsBackend {
    /// Selects the active browser graphics backend from probed capabilities.
    ///
    /// WebGPU is chosen opportunistically when available; WebGL2 is the mandatory
    /// fallback baseline; otherwise an `Unsupported` variant with a readable diagnostic
    /// is returned.
    #[must_use]
    pub const fn detect(capabilities: BrowserGraphicsCapabilities) -> Self {
        match (capabilities.webgpu, capabilities.webgl2) {
            (WebGpuAvailability::Available, _) => Self::WebGpu,
            (WebGpuAvailability::Unavailable, WebGl2Availability::Available) => Self::WebGl2,
            (WebGpuAvailability::Unavailable, WebGl2Availability::Unavailable) => {
                Self::Unsupported(FallbackDiagnostic::missing_webgpu_and_webgl2())
            }
        }
    }

    /// Returns true when the selected backend can render frames.
    #[must_use]
    pub const fn is_renderable(&self) -> bool {
        match self {
            Self::WebGpu | Self::WebGl2 => true,
            Self::Unsupported(_) => false,
        }
    }

    /// Returns a stable identifier for telemetry and diagnostics.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::WebGpu => "webgpu",
            Self::WebGl2 => "webgl2",
            Self::Unsupported(_) => "unsupported",
        }
    }

    /// Returns the fallback diagnostic when the backend is unsupported.
    #[must_use]
    pub const fn fallback_diagnostic(&self) -> Option<&FallbackDiagnostic> {
        match self {
            Self::WebGpu | Self::WebGl2 => None,
            Self::Unsupported(diagnostic) => Some(diagnostic),
        }
    }
}

impl fmt::Display for GraphicsBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}
