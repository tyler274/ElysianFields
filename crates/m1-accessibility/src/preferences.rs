use std::fmt;

use bevy::prelude::Resource;

use crate::contrast::PaletteTheme;
use crate::domain::{FocusTarget, ModalState, NavItemId, SemanticControlId};

/// Validated text/UI scale factor supporting up to 200% WCAG 1.4.4 resize.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextScale(f32);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextScaleError {
    OutOfRange { requested: f32, min: f32, max: f32 },
    NonFinite,
}

impl fmt::Display for TextScaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange {
                requested,
                min,
                max,
            } => write!(
                f,
                "text scale {requested} is outside supported range [{min}, {max}]"
            ),
            Self::NonFinite => write!(f, "text scale must be finite"),
        }
    }
}

impl std::error::Error for TextScaleError {}

impl TextScale {
    pub const MIN: f32 = 1.0;
    pub const MAX: f32 = 2.0;

    pub const STANDARD_100: Self = Self(1.0);
    pub const LARGE_150: Self = Self(1.5);
    pub const DOUBLE_200: Self = Self(2.0);

    pub fn new(factor: f32) -> Result<Self, TextScaleError> {
        if !factor.is_finite() {
            return Err(TextScaleError::NonFinite);
        }
        if !(Self::MIN..=Self::MAX).contains(&factor) {
            return Err(TextScaleError::OutOfRange {
                requested: factor,
                min: Self::MIN,
                max: Self::MAX,
            });
        }
        Ok(Self(factor))
    }

    #[must_use]
    pub const fn factor(self) -> f32 {
        self.0
    }
}

impl Default for TextScale {
    fn default() -> Self {
        Self::STANDARD_100
    }
}

/// OS / user reduced-motion preference (WCAG 2.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReducedMotion {
    #[default]
    NoPreference,
    Reduce,
}

/// Whether decorative animations such as streaming cursor pulsing run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationMode {
    Animated,
    Static,
}

/// Concrete motion policy derived from [`ReducedMotion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotionPolicy {
    pub modal_transition_ms: u32,
    pub scroll_easing_ms: u32,
    pub focus_ring_transition_ms: u32,
    pub streaming_cursor: AnimationMode,
}

impl MotionPolicy {
    /// Single owner mapping [`ReducedMotion`] into deterministic animation behavior.
    #[must_use]
    pub const fn for_preference(preference: ReducedMotion) -> Self {
        match preference {
            ReducedMotion::NoPreference => Self {
                modal_transition_ms: 160,
                scroll_easing_ms: 140,
                focus_ring_transition_ms: 80,
                streaming_cursor: AnimationMode::Animated,
            },
            ReducedMotion::Reduce => Self {
                modal_transition_ms: 0,
                scroll_easing_ms: 0,
                focus_ring_transition_ms: 0,
                streaming_cursor: AnimationMode::Static,
            },
        }
    }

    #[must_use]
    pub const fn allows_non_essential_motion(self) -> bool {
        self.modal_transition_ms > 0
            || self.scroll_easing_ms > 0
            || self.focus_ring_transition_ms > 0
            || matches!(self.streaming_cursor, AnimationMode::Animated)
    }

    /// Evaluates normalized animation progress (`0.0..=1.0`) for a given elapsed time.
    /// Under `ReducedMotion::Reduce`, transitions settle immediately at `1.0`.
    #[must_use]
    pub fn modal_progress(self, elapsed_ms: u32) -> f32 {
        if self.modal_transition_ms == 0 {
            return 1.0;
        }
        (elapsed_ms as f32 / self.modal_transition_ms as f32).clamp(0.0, 1.0)
    }
}

/// Logical viewport dimensions in CSS / logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportSize {
    pub width_px: f32,
    pub height_px: f32,
}

impl ViewportSize {
    pub const DESKTOP_1280X720: Self = Self {
        width_px: 1280.0,
        height_px: 720.0,
    };
    pub const MOBILE_390X844: Self = Self {
        width_px: 390.0,
        height_px: 844.0,
    };
    pub const POPUP_360X520: Self = Self {
        width_px: 360.0,
        height_px: 520.0,
    };
}

/// Computed layout rectangle for an interactive control in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlLayoutRect {
    pub control: SemanticControlId,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ControlLayoutRect {
    #[must_use]
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    #[must_use]
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }

    #[must_use]
    pub fn to_accesskit_rect(self) -> accesskit::Rect {
        accesskit::Rect {
            x0: f64::from(self.x),
            y0: f64::from(self.y),
            x1: f64::from(self.right()),
            y1: f64::from(self.bottom()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ZoomLayoutError {
    HorizontalOverflow {
        control: SemanticControlId,
        right_edge_px: f32,
        viewport_width_px: f32,
    },
    NegativeOrigin {
        control: SemanticControlId,
        x: f32,
        y: f32,
    },
    UndersizedTarget {
        control: SemanticControlId,
        width: f32,
        height: f32,
        min_height_px: f32,
    },
    OverlappingControls {
        first: SemanticControlId,
        second: SemanticControlId,
    },
    MissingInteractiveControl {
        control: SemanticControlId,
    },
}

impl fmt::Display for ZoomLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HorizontalOverflow {
                control,
                right_edge_px,
                viewport_width_px,
            } => write!(
                f,
                "control {control:?} overflows horizontally ({right_edge_px:.1}px > {viewport_width_px:.1}px)"
            ),
            Self::NegativeOrigin { control, x, y } => {
                write!(
                    f,
                    "control {control:?} clipped at negative origin ({x:.1}, {y:.1})"
                )
            }
            Self::UndersizedTarget {
                control,
                width,
                height,
                min_height_px,
            } => write!(
                f,
                "control {control:?} collapsed to {width:.1}x{height:.1}px (minimum height {min_height_px:.1}px)"
            ),
            Self::OverlappingControls { first, second } => {
                write!(
                    f,
                    "controls {first:?} and {second:?} overlap and occlude interaction"
                )
            }
            Self::MissingInteractiveControl { control } => {
                write!(
                    f,
                    "interactive control {control:?} missing from scaled layout"
                )
            }
        }
    }
}

impl std::error::Error for ZoomLayoutError {}

/// Successful zoom reflow verification summary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomLayoutReport {
    pub scale: TextScale,
    pub viewport: ViewportSize,
    pub verified_controls: usize,
    pub total_scroll_height_px: f32,
}

/// Computes a responsive reflow layout for all active interactive controls at `scale`.
///
/// Stacks surfaces vertically when the viewport narrows or scale reaches 200%,
/// ensuring zero horizontal clipping and non-overlapping touch/click targets.
#[must_use]
pub fn compute_scaled_layout(
    scale: TextScale,
    viewport: ViewportSize,
    modal: &ModalState,
) -> Vec<ControlLayoutRect> {
    let margin = 12.0;
    let gap = 8.0 * scale.factor();
    let usable_width = (viewport.width_px - margin * 2.0).max(120.0);
    let base_control_height = 24.0 * scale.factor();
    let multiline_height = 56.0 * scale.factor();

    let targets: &[FocusTarget] = match modal {
        ModalState::Open { .. } => &[
            FocusTarget::ModalConfirmButton,
            FocusTarget::ModalCancelButton,
        ],
        ModalState::Closed => &[
            FocusTarget::NavItem(NavItemId::Inbox),
            FocusTarget::NavItem(NavItemId::Transcript),
            FocusTarget::NavItem(NavItemId::ClinicalForm),
            FocusTarget::NavItem(NavItemId::LabRecords),
            FocusTarget::CitationLink(crate::domain::CitationId::ProgressNoteSummary),
            FocusTarget::CitationLink(crate::domain::CitationId::ClinicalLabPanel),
            FocusTarget::AssessmentNoteInput,
            FocusTarget::IncludeCitationsCheckbox,
            FocusTarget::GuidanceDisclosure,
            FocusTarget::SubmitNoteButton,
            FocusTarget::OpenSignOffModalButton,
        ],
    };

    let mut rects = Vec::with_capacity(targets.len());
    let mut cursor_y = margin;

    for &target in targets {
        let height = match target {
            FocusTarget::AssessmentNoteInput => multiline_height,
            _ => base_control_height,
        };
        let rect = ControlLayoutRect {
            control: target.semantic_id(),
            x: margin,
            y: cursor_y,
            width: usable_width,
            height,
        };
        cursor_y = rect.bottom() + gap;
        rects.push(rect);
    }

    rects
}

/// Verifies that every interactive control in `expected_controls` is present,
/// unclipped horizontally, non-overlapping, and meets scaled target height at `scale`.
pub fn verify_zoom_layout(
    scale: TextScale,
    viewport: ViewportSize,
    rects: &[ControlLayoutRect],
    expected_controls: &[SemanticControlId],
) -> Result<ZoomLayoutReport, Vec<ZoomLayoutError>> {
    let mut errors = Vec::new();
    let min_height_px = 24.0 * scale.factor();
    let mut max_bottom = 0.0_f32;

    for &expected in expected_controls {
        if !rects.iter().any(|r| r.control == expected) {
            errors.push(ZoomLayoutError::MissingInteractiveControl { control: expected });
        }
    }

    for (idx, rect) in rects.iter().enumerate() {
        if rect.x < 0.0 || rect.y < 0.0 {
            errors.push(ZoomLayoutError::NegativeOrigin {
                control: rect.control,
                x: rect.x,
                y: rect.y,
            });
        }
        if rect.right() > viewport.width_px + 0.01 {
            errors.push(ZoomLayoutError::HorizontalOverflow {
                control: rect.control,
                right_edge_px: rect.right(),
                viewport_width_px: viewport.width_px,
            });
        }
        if rect.width <= 0.0 || rect.height + 0.01 < min_height_px {
            errors.push(ZoomLayoutError::UndersizedTarget {
                control: rect.control,
                width: rect.width,
                height: rect.height,
                min_height_px,
            });
        }
        max_bottom = max_bottom.max(rect.bottom());

        for other in &rects[(idx + 1)..] {
            if rect.overlaps(*other) {
                errors.push(ZoomLayoutError::OverlappingControls {
                    first: rect.control,
                    second: other.control,
                });
            }
        }
    }

    if errors.is_empty() {
        Ok(ZoomLayoutReport {
            scale,
            viewport,
            verified_controls: rects.len(),
            total_scroll_height_px: max_bottom,
        })
    } else {
        Err(errors)
    }
}

/// Combined accessibility user preferences stored as a Bevy resource.
#[derive(Debug, Clone, Copy, PartialEq, Resource)]
pub struct AccessibilityPreferences {
    pub text_scale: TextScale,
    pub reduced_motion: ReducedMotion,
    pub theme: PaletteTheme,
    pub viewport: ViewportSize,
}

impl Default for AccessibilityPreferences {
    fn default() -> Self {
        Self {
            text_scale: TextScale::STANDARD_100,
            reduced_motion: ReducedMotion::NoPreference,
            theme: PaletteTheme::Light,
            viewport: ViewportSize::DESKTOP_1280X720,
        }
    }
}

impl AccessibilityPreferences {
    #[must_use]
    pub const fn motion_policy(&self) -> MotionPolicy {
        MotionPolicy::for_preference(self.reduced_motion)
    }
}
