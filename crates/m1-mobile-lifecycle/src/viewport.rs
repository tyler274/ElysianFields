//! Mobile viewport safe-area layout calculator covering rotation, split-screen
//! multi-window resize, display cutouts, system bars, and soft-keyboard insets.

use std::fmt;

/// Minimum safe content width (logical px) required for two-column split layout.
const SPLIT_PANE_MIN_WIDTH_PX: u32 = 600;

/// Device screen orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Orientation {
    Portrait,
    LandscapeLeft,
    LandscapeRight,
    PortraitUpsideDown,
}

/// Window presentation mode on phones, foldables, and tablets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowPresentation {
    FullScreen,
    SplitScreen { pane_fraction_percent: u8 },
}

/// Display cutout geometry reported by Android `DisplayCutout` or iOS safe area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisplayCutout {
    None,
    TopNotch { height_px: u32 },
    DynamicIsland { top_inset_px: u32 },
    LandscapeSideCutout { left_px: u32, right_px: u32 },
}

/// Logical pixel edge insets (`top`, `right`, `bottom`, `left`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SafeAreaInsets {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

/// Logical pixel rectangle inside the mobile window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Responsive layout column mode derived from the safe content width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutColumns {
    SinglePane,
    SplitPane,
}

/// Axis that overflowed when validating viewport insets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutAxis {
    Horizontal,
    Vertical,
}

/// Input specification delivered by Android `WindowInsetsCompat` or iOS `UIWindow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportSpec {
    pub width_px: u32,
    pub height_px: u32,
    /// Display scale factor in thousandths (e.g., `3000` for 3.0x DPR).
    pub scale_factor_milli: u32,
    pub orientation: Orientation,
    pub presentation: WindowPresentation,
    pub system_bars: SafeAreaInsets,
    pub cutout: DisplayCutout,
    /// Soft-keyboard overlap from the bottom edge of the window (`0` when hidden).
    pub soft_keyboard_height_px: u32,
}

/// Validation failure when computing [`ViewportInsets`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportLayoutError {
    ZeroDimension {
        width_px: u32,
        height_px: u32,
    },
    InvalidScaleFactor {
        scale_factor_milli: u32,
    },
    InsetsExceedViewport {
        axis: LayoutAxis,
        viewport_px: u32,
        inset_sum_px: u32,
    },
}

impl fmt::Display for ViewportLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDimension {
                width_px,
                height_px,
            } => write!(
                f,
                "viewport dimensions must be positive (got {width_px}x{height_px})"
            ),
            Self::InvalidScaleFactor { scale_factor_milli } => {
                write!(
                    f,
                    "viewport scale factor must be positive (got {scale_factor_milli} milli)"
                )
            }
            Self::InsetsExceedViewport {
                axis,
                viewport_px,
                inset_sum_px,
            } => write!(
                f,
                "{axis:?} safe-area insets ({inset_sum_px}px) meet or exceed viewport ({viewport_px}px)"
            ),
        }
    }
}

impl std::error::Error for ViewportLayoutError {}

/// Computed safe-area insets and usable content rectangle for mobile UI layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportInsets {
    pub window_width_px: u32,
    pub window_height_px: u32,
    pub scale_factor_milli: u32,
    pub orientation: Orientation,
    pub presentation: WindowPresentation,
    pub effective_insets: SafeAreaInsets,
    pub safe_content_rect: LayoutRect,
    pub layout_columns: LayoutColumns,
}

impl Default for ViewportInsets {
    fn default() -> Self {
        Self {
            window_width_px: 390,
            window_height_px: 844,
            scale_factor_milli: 3000,
            orientation: Orientation::Portrait,
            presentation: WindowPresentation::FullScreen,
            effective_insets: SafeAreaInsets {
                top: 47,
                right: 0,
                bottom: 34,
                left: 0,
            },
            safe_content_rect: LayoutRect {
                x: 0,
                y: 47,
                width: 390,
                height: 763,
            },
            layout_columns: LayoutColumns::SinglePane,
        }
    }
}

impl ViewportInsets {
    /// Computes effective safe-area insets and the usable content rectangle from
    /// a platform [`ViewportSpec`].
    ///
    /// Rules enforced once here:
    /// - Cutout edges are combined via `max` with system bars so UI never clips
    ///   beneath status bars, notches, Dynamic Island, or landscape camera cutouts.
    /// - Soft-keyboard height overrides the bottom bar inset via `max` because
    ///   Android `WindowInsets.ime()` and iOS keyboard frames are measured from
    ///   the bottom window edge.
    /// - Horizontal and vertical inset sums must remain strictly smaller than the
    ///   viewport size.
    pub fn compute(spec: &ViewportSpec) -> Result<Self, ViewportLayoutError> {
        if spec.width_px == 0 || spec.height_px == 0 {
            return Err(ViewportLayoutError::ZeroDimension {
                width_px: spec.width_px,
                height_px: spec.height_px,
            });
        }
        if spec.scale_factor_milli == 0 {
            return Err(ViewportLayoutError::InvalidScaleFactor {
                scale_factor_milli: spec.scale_factor_milli,
            });
        }

        let (cutout_top, cutout_left, cutout_right) = match spec.cutout {
            DisplayCutout::None => (0, 0, 0),
            DisplayCutout::TopNotch { height_px } => (height_px, 0, 0),
            DisplayCutout::DynamicIsland { top_inset_px } => (top_inset_px, 0, 0),
            DisplayCutout::LandscapeSideCutout { left_px, right_px } => (0, left_px, right_px),
        };

        let effective_insets = SafeAreaInsets {
            top: spec.system_bars.top.max(cutout_top),
            right: spec.system_bars.right.max(cutout_right),
            bottom: spec.system_bars.bottom.max(spec.soft_keyboard_height_px),
            left: spec.system_bars.left.max(cutout_left),
        };

        let horizontal_insets = effective_insets.left.saturating_add(effective_insets.right);
        if horizontal_insets >= spec.width_px {
            return Err(ViewportLayoutError::InsetsExceedViewport {
                axis: LayoutAxis::Horizontal,
                viewport_px: spec.width_px,
                inset_sum_px: horizontal_insets,
            });
        }

        let vertical_insets = effective_insets.top.saturating_add(effective_insets.bottom);
        if vertical_insets >= spec.height_px {
            return Err(ViewportLayoutError::InsetsExceedViewport {
                axis: LayoutAxis::Vertical,
                viewport_px: spec.height_px,
                inset_sum_px: vertical_insets,
            });
        }

        let safe_width = spec.width_px - horizontal_insets;
        let safe_height = spec.height_px - vertical_insets;

        let layout_columns = if safe_width >= SPLIT_PANE_MIN_WIDTH_PX {
            LayoutColumns::SplitPane
        } else {
            LayoutColumns::SinglePane
        };

        Ok(Self {
            window_width_px: spec.width_px,
            window_height_px: spec.height_px,
            scale_factor_milli: spec.scale_factor_milli,
            orientation: spec.orientation,
            presentation: spec.presentation,
            effective_insets,
            safe_content_rect: LayoutRect {
                x: effective_insets.left,
                y: effective_insets.top,
                width: safe_width,
                height: safe_height,
            },
            layout_columns,
        })
    }
}
