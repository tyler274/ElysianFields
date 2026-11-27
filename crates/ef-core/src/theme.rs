//! WCAG 2.1 AA theme tokens and luminance contrast verification.

use serde::{Deserialize, Serialize};

/// Supported UI theme modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
}

/// 8-bit sRGB color value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    /// Constructs a new sRGB color.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Computes WCAG 2.1 relative luminance in `0.0..=1.0`.
    #[must_use]
    pub fn relative_luminance(self) -> f64 {
        fn linearize(channel: u8) -> f64 {
            let s = f64::from(channel) / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * linearize(self.r) + 0.7152 * linearize(self.g) + 0.0722 * linearize(self.b)
    }

    /// Computes the WCAG 2.1 contrast ratio (`1.0..=21.0`) between two colors.
    #[must_use]
    pub fn contrast_ratio(self, other: Self) -> f64 {
        let l1 = self.relative_luminance();
        let l2 = other.relative_luminance();
        let (lighter, darker) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (lighter + 0.05) / (darker + 0.05)
    }
}

/// Shared WCAG 2.1 AA semantic color palette for Elysian Fields surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemePalette {
    pub mode: ThemeMode,
    pub surface_canvas: RgbColor,
    pub surface_elevated: RgbColor,
    pub text_primary: RgbColor,
    pub text_secondary: RgbColor,
    pub accent_primary: RgbColor,
    pub accent_on_primary: RgbColor,
    pub focus_ring: RgbColor,
    pub status_danger: RgbColor,
    pub status_warning: RgbColor,
    pub status_success: RgbColor,
    pub privacy_veil: RgbColor,
}

impl ThemePalette {
    /// Returns the canonical WCAG 2.1 AA palette for the requested theme mode.
    #[must_use]
    pub const fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light => Self {
                mode,
                surface_canvas: RgbColor::new(249, 250, 251),
                surface_elevated: RgbColor::new(255, 255, 255),
                text_primary: RgbColor::new(17, 24, 39),
                text_secondary: RgbColor::new(55, 65, 81),
                accent_primary: RgbColor::new(29, 78, 216),
                accent_on_primary: RgbColor::new(255, 255, 255),
                focus_ring: RgbColor::new(30, 64, 175),
                status_danger: RgbColor::new(185, 28, 28),
                status_warning: RgbColor::new(146, 64, 14),
                status_success: RgbColor::new(21, 128, 61),
                privacy_veil: RgbColor::new(15, 23, 42),
            },
            ThemeMode::Dark => Self {
                mode,
                surface_canvas: RgbColor::new(15, 23, 42),
                surface_elevated: RgbColor::new(30, 41, 59),
                text_primary: RgbColor::new(248, 250, 252),
                text_secondary: RgbColor::new(203, 213, 225),
                accent_primary: RgbColor::new(96, 165, 250),
                accent_on_primary: RgbColor::new(15, 23, 42),
                focus_ring: RgbColor::new(147, 197, 253),
                status_danger: RgbColor::new(248, 113, 113),
                status_warning: RgbColor::new(251, 191, 36),
                status_success: RgbColor::new(74, 222, 128),
                privacy_veil: RgbColor::new(2, 6, 23),
            },
        }
    }

    /// Verifies that every foreground/background pair satisfies WCAG 2.1 AA
    /// (`>= 4.5:1` for text and `>= 3.0:1` for UI focus rings).
    #[must_use]
    pub fn meets_wcag_aa(&self) -> bool {
        let text_pairs = [
            (self.text_primary, self.surface_canvas),
            (self.text_primary, self.surface_elevated),
            (self.text_secondary, self.surface_canvas),
            (self.text_secondary, self.surface_elevated),
            (self.accent_on_primary, self.accent_primary),
            (self.status_danger, self.surface_canvas),
            (self.status_warning, self.surface_canvas),
            (self.status_success, self.surface_canvas),
        ];
        let ui_pairs = [
            (self.focus_ring, self.surface_canvas),
            (self.focus_ring, self.surface_elevated),
        ];

        text_pairs
            .iter()
            .all(|(fg, bg)| fg.contrast_ratio(*bg) >= 4.5)
            && ui_pairs
                .iter()
                .all(|(fg, bg)| fg.contrast_ratio(*bg) >= 3.0)
    }
}
