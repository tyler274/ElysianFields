use std::fmt;

use bevy::color::Color;
use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

/// 8-bit sRGB color triplet used across semantic UI tokens and WCAG 2.1 checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SrgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl SrgbColor {
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    #[must_use]
    pub fn to_bevy_color(self) -> Color {
        Color::srgb_u8(self.r, self.g, self.b)
    }
}

impl fmt::Display for SrgbColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

/// Active color scheme shared across Curie Clinic and Curie Mobile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    Light,
    Dark,
}

/// Validated UI text scale factor supporting WCAG 1.4.4 resize up to 200%.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextScale(f32);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextScaleError {
    NonFinite,
    OutOfRange { requested: f32, min: f32, max: f32 },
}

impl fmt::Display for TextScaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => write!(f, "text scale must be finite"),
            Self::OutOfRange {
                requested,
                min,
                max,
            } => write!(
                f,
                "text scale {requested} is outside supported range [{min}, {max}]"
            ),
        }
    }
}

impl std::error::Error for TextScaleError {}

impl TextScale {
    pub const MIN: f32 = 1.0;
    pub const MAX: f32 = 2.0;
    pub const STANDARD: Self = Self(1.0);
    pub const DOUBLE: Self = Self(2.0);

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
        Self::STANDARD
    }
}

/// WCAG 2.1 Level AA minimum contrast requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContrastRequirement {
    NormalText,
    LargeText,
    UiComponent,
}

impl ContrastRequirement {
    #[must_use]
    pub const fn min_ratio(self) -> f64 {
        match self {
            Self::NormalText => 4.5,
            Self::LargeText | Self::UiComponent => 3.0,
        }
    }
}

/// Complete semantic design token set for a light or dark theme.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ThemeTokens {
    pub canvas_bg: SrgbColor,
    pub surface_bg: SrgbColor,
    pub modal_bg: SrgbColor,
    pub privacy_veil_bg: SrgbColor,
    pub privacy_veil_fg: SrgbColor,
    pub text_primary: SrgbColor,
    pub text_secondary: SrgbColor,
    pub link_citation: SrgbColor,
    pub accent_fill: SrgbColor,
    pub accent_label: SrgbColor,
    pub focus_ring: SrgbColor,
    pub alert_error_bg: SrgbColor,
    pub alert_error_fg: SrgbColor,
    pub alert_info_bg: SrgbColor,
    pub alert_info_fg: SrgbColor,
    pub input_bg: SrgbColor,
    pub input_fg: SrgbColor,
    pub nav_selected_bg: SrgbColor,
    pub nav_selected_fg: SrgbColor,
    pub focus_ring_width_px: f32,
}

impl ThemeTokens {
    pub const LIGHT: Self = Self {
        canvas_bg: SrgbColor::rgb(0xF8, 0xFA, 0xFC),
        surface_bg: SrgbColor::rgb(0xFF, 0xFF, 0xFF),
        modal_bg: SrgbColor::rgb(0xFF, 0xFF, 0xFF),
        privacy_veil_bg: SrgbColor::rgb(0x0F, 0x17, 0x2A),
        privacy_veil_fg: SrgbColor::rgb(0xF8, 0xFA, 0xFC),
        text_primary: SrgbColor::rgb(0x16, 0x1B, 0x22),
        text_secondary: SrgbColor::rgb(0x47, 0x54, 0x67),
        link_citation: SrgbColor::rgb(0x0B, 0x53, 0x94),
        accent_fill: SrgbColor::rgb(0x0F, 0x5C, 0x78),
        accent_label: SrgbColor::rgb(0xFF, 0xFF, 0xFF),
        focus_ring: SrgbColor::rgb(0x0A, 0x58, 0xCA),
        alert_error_bg: SrgbColor::rgb(0xFD, 0xED, 0xED),
        alert_error_fg: SrgbColor::rgb(0x8E, 0x19, 0x19),
        alert_info_bg: SrgbColor::rgb(0xE0, 0xF2, 0xFE),
        alert_info_fg: SrgbColor::rgb(0x0B, 0x46, 0x60),
        input_bg: SrgbColor::rgb(0xFF, 0xFF, 0xFF),
        input_fg: SrgbColor::rgb(0x11, 0x18, 0x27),
        nav_selected_bg: SrgbColor::rgb(0xE0, 0xF2, 0xFE),
        nav_selected_fg: SrgbColor::rgb(0x0B, 0x46, 0x60),
        focus_ring_width_px: 3.0,
    };

    pub const DARK: Self = Self {
        canvas_bg: SrgbColor::rgb(0x0B, 0x12, 0x20),
        surface_bg: SrgbColor::rgb(0x0F, 0x17, 0x2A),
        modal_bg: SrgbColor::rgb(0x1E, 0x29, 0x3B),
        privacy_veil_bg: SrgbColor::rgb(0x02, 0x06, 0x17),
        privacy_veil_fg: SrgbColor::rgb(0xF8, 0xFA, 0xFC),
        text_primary: SrgbColor::rgb(0xF1, 0xF5, 0xF9),
        text_secondary: SrgbColor::rgb(0xCB, 0xD5, 0xE1),
        link_citation: SrgbColor::rgb(0x7D, 0xD3, 0xFC),
        accent_fill: SrgbColor::rgb(0x38, 0xBD, 0xF8),
        accent_label: SrgbColor::rgb(0x0B, 0x13, 0x24),
        focus_ring: SrgbColor::rgb(0x7D, 0xD3, 0xFC),
        alert_error_bg: SrgbColor::rgb(0x5B, 0x12, 0x12),
        alert_error_fg: SrgbColor::rgb(0xFE, 0xCA, 0xCA),
        alert_info_bg: SrgbColor::rgb(0x15, 0x3E, 0x5C),
        alert_info_fg: SrgbColor::rgb(0xE0, 0xF2, 0xFE),
        input_bg: SrgbColor::rgb(0x1E, 0x29, 0x3B),
        input_fg: SrgbColor::rgb(0xF8, 0xFA, 0xFC),
        nav_selected_bg: SrgbColor::rgb(0x15, 0x3E, 0x5C),
        nav_selected_fg: SrgbColor::rgb(0xE0, 0xF2, 0xFE),
        focus_ring_width_px: 3.0,
    };

    #[must_use]
    pub const fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light => Self::LIGHT,
            ThemeMode::Dark => Self::DARK,
        }
    }

    /// Returns all foreground/background pairs audited for WCAG 2.1 AA compliance.
    #[must_use]
    pub const fn contrast_pairs(&self) -> [ThemeContrastPair; 13] {
        [
            ThemeContrastPair {
                id: ThemePairId::PrimaryTextOnCanvas,
                foreground: self.text_primary,
                background: self.canvas_bg,
                requirement: ContrastRequirement::NormalText,
            },
            ThemeContrastPair {
                id: ThemePairId::PrimaryTextOnSurface,
                foreground: self.text_primary,
                background: self.surface_bg,
                requirement: ContrastRequirement::NormalText,
            },
            ThemeContrastPair {
                id: ThemePairId::SecondaryTextOnSurface,
                foreground: self.text_secondary,
                background: self.surface_bg,
                requirement: ContrastRequirement::NormalText,
            },
            ThemeContrastPair {
                id: ThemePairId::CitationLinkOnSurface,
                foreground: self.link_citation,
                background: self.surface_bg,
                requirement: ContrastRequirement::NormalText,
            },
            ThemeContrastPair {
                id: ThemePairId::AccentLabelOnFill,
                foreground: self.accent_label,
                background: self.accent_fill,
                requirement: ContrastRequirement::NormalText,
            },
            ThemeContrastPair {
                id: ThemePairId::FocusRingOnSurface,
                foreground: self.focus_ring,
                background: self.surface_bg,
                requirement: ContrastRequirement::UiComponent,
            },
            ThemeContrastPair {
                id: ThemePairId::FocusRingOnModal,
                foreground: self.focus_ring,
                background: self.modal_bg,
                requirement: ContrastRequirement::UiComponent,
            },
            ThemeContrastPair {
                id: ThemePairId::ErrorAlertTextOnBanner,
                foreground: self.alert_error_fg,
                background: self.alert_error_bg,
                requirement: ContrastRequirement::NormalText,
            },
            ThemeContrastPair {
                id: ThemePairId::InfoAlertTextOnBanner,
                foreground: self.alert_info_fg,
                background: self.alert_info_bg,
                requirement: ContrastRequirement::NormalText,
            },
            ThemeContrastPair {
                id: ThemePairId::InputTextOnField,
                foreground: self.input_fg,
                background: self.input_bg,
                requirement: ContrastRequirement::NormalText,
            },
            ThemeContrastPair {
                id: ThemePairId::ModalTextOnDialog,
                foreground: self.text_primary,
                background: self.modal_bg,
                requirement: ContrastRequirement::NormalText,
            },
            ThemeContrastPair {
                id: ThemePairId::NavSelectedTextOnHighlight,
                foreground: self.nav_selected_fg,
                background: self.nav_selected_bg,
                requirement: ContrastRequirement::NormalText,
            },
            ThemeContrastPair {
                id: ThemePairId::PrivacyVeilTextOnCover,
                foreground: self.privacy_veil_fg,
                background: self.privacy_veil_bg,
                requirement: ContrastRequirement::NormalText,
            },
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePairId {
    PrimaryTextOnCanvas,
    PrimaryTextOnSurface,
    SecondaryTextOnSurface,
    CitationLinkOnSurface,
    AccentLabelOnFill,
    FocusRingOnSurface,
    FocusRingOnModal,
    ErrorAlertTextOnBanner,
    InfoAlertTextOnBanner,
    InputTextOnField,
    ModalTextOnDialog,
    NavSelectedTextOnHighlight,
    PrivacyVeilTextOnCover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeContrastPair {
    pub id: ThemePairId,
    pub foreground: SrgbColor,
    pub background: SrgbColor,
    pub requirement: ContrastRequirement,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContrastAuditEntry {
    pub id: ThemePairId,
    pub ratio: f64,
    pub required: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContrastViolation {
    pub id: ThemePairId,
    pub foreground: SrgbColor,
    pub background: SrgbColor,
    pub actual_ratio: f64,
    pub required_ratio: f64,
}

impl fmt::Display for ContrastViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "theme pair {:?} ({} on {}) has contrast {:.2}:1, below WCAG AA {:.1}:1",
            self.id, self.foreground, self.background, self.actual_ratio, self.required_ratio
        )
    }
}

impl std::error::Error for ContrastViolation {}

/// Single-owner WCAG 2.1 sRGB relative luminance calculation.
#[must_use]
pub fn relative_luminance(color: SrgbColor) -> f64 {
    fn channel(c: u8) -> f64 {
        let s = f64::from(c) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
}

/// Single-owner WCAG 2.1 contrast ratio in `[1.0, 21.0]`.
#[must_use]
pub fn contrast_ratio(foreground: SrgbColor, background: SrgbColor) -> f64 {
    let l1 = relative_luminance(foreground);
    let l2 = relative_luminance(background);
    let (lighter, darker) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Verifies every pair in `pairs` meets WCAG 2.1 AA thresholds.
pub fn verify_contrast_pairs(
    pairs: &[ThemeContrastPair],
) -> Result<Vec<ContrastAuditEntry>, Vec<ContrastViolation>> {
    let mut passing = Vec::with_capacity(pairs.len());
    let mut violations = Vec::new();

    for pair in pairs {
        let ratio = contrast_ratio(pair.foreground, pair.background);
        let required = pair.requirement.min_ratio();
        if ratio < required {
            violations.push(ContrastViolation {
                id: pair.id,
                foreground: pair.foreground,
                background: pair.background,
                actual_ratio: ratio,
                required_ratio: required,
            });
        } else {
            passing.push(ContrastAuditEntry {
                id: pair.id,
                ratio,
                required,
            });
        }
    }

    if violations.is_empty() {
        Ok(passing)
    } else {
        Err(violations)
    }
}

/// Audits all semantic tokens for the selected [`ThemeMode`].
pub fn verify_theme_wcag_aa(
    mode: ThemeMode,
) -> Result<Vec<ContrastAuditEntry>, Vec<ContrastViolation>> {
    let tokens = ThemeTokens::for_mode(mode);
    verify_contrast_pairs(&tokens.contrast_pairs())
}

/// Active theme resource owned by the Bevy UI catalog plugin.
#[derive(Debug, Clone, Copy, PartialEq, Resource)]
pub struct UiTheme {
    pub mode: ThemeMode,
    pub tokens: ThemeTokens,
    pub text_scale: TextScale,
}

impl Default for UiTheme {
    fn default() -> Self {
        Self::new(ThemeMode::Light, TextScale::STANDARD)
    }
}

impl UiTheme {
    #[must_use]
    pub const fn new(mode: ThemeMode, text_scale: TextScale) -> Self {
        Self {
            mode,
            tokens: ThemeTokens::for_mode(mode),
            text_scale,
        }
    }

    pub fn set_mode(&mut self, mode: ThemeMode) {
        self.mode = mode;
        self.tokens = ThemeTokens::for_mode(mode);
    }

    #[must_use]
    pub const fn focus_ring_meets_spec(&self) -> bool {
        self.tokens.focus_ring_width_px >= 2.0
    }
}
