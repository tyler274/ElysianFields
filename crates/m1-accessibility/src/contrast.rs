use std::fmt;

/// 8-bit sRGB color triplet used for WCAG 2.1 contrast evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

impl fmt::Display for SrgbColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

/// WCAG 2.1 Level AA minimum contrast thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContrastRequirement {
    /// Normal body and label text (WCAG 1.4.3 AA): >= 4.5:1.
    NormalText,
    /// Large text (>= 18pt or >= 14pt bold, WCAG 1.4.3 AA): >= 3.0:1.
    LargeText,
    /// Interactive UI components and focus indicators (WCAG 1.4.11 AA): >= 3.0:1.
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

/// Semantic color pairs across the Elysian Fields light and dark themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PalettePairId {
    BodyTextOnSurface,
    SecondaryTextOnSurface,
    CitationLinkOnSurface,
    PrimaryButtonLabelOnFill,
    FocusRingOnSurface,
    FocusRingOnModal,
    ErrorAlertTextOnBanner,
    InputTextOnField,
    ModalBodyTextOnDialog,
    NavSelectedTextOnHighlight,
    LargeHeadingOnSurface,
}

/// Active visual theme for contrast verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaletteTheme {
    #[default]
    Light,
    Dark,
}

/// A foreground/background palette pair with its required WCAG 2.1 AA threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PalettePair {
    pub id: PalettePairId,
    pub foreground: SrgbColor,
    pub background: SrgbColor,
    pub requirement: ContrastRequirement,
}

/// Verified contrast measurement for a passing palette pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContrastAuditEntry {
    pub id: PalettePairId,
    pub ratio: f64,
    pub required: f64,
}

/// Error reported when a foreground/background pair falls below WCAG 2.1 AA.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContrastViolation {
    pub id: PalettePairId,
    pub foreground: SrgbColor,
    pub background: SrgbColor,
    pub actual_ratio: f64,
    pub required_ratio: f64,
}

impl fmt::Display for ContrastViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "palette pair {:?} ({} on {}) has contrast {:.2}:1, below WCAG AA {:.1}:1",
            self.id, self.foreground, self.background, self.actual_ratio, self.required_ratio
        )
    }
}

impl std::error::Error for ContrastViolation {}

/// Linearizes a single 8-bit sRGB channel per WCAG 2.1 relative luminance definition.
#[must_use]
fn linearize_channel(channel: u8) -> f64 {
    let normalized = f64::from(channel) / 255.0;
    if normalized <= 0.04045 {
        normalized / 12.92
    } else {
        ((normalized + 0.055) / 1.055).powf(2.4)
    }
}

/// Computes WCAG 2.1 relative luminance in `[0.0, 1.0]`.
#[must_use]
pub fn relative_luminance(color: SrgbColor) -> f64 {
    let r = linearize_channel(color.r);
    let g = linearize_channel(color.g);
    let b = linearize_channel(color.b);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Computes WCAG 2.1 contrast ratio in `[1.0, 21.0]`.
#[must_use]
pub fn contrast_ratio(foreground: SrgbColor, background: SrgbColor) -> f64 {
    let l1 = relative_luminance(foreground);
    let l2 = relative_luminance(background);
    let (lighter, darker) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Elysian Fields light theme semantic palette pairs.
pub const LIGHT_PALETTE: [PalettePair; 11] = [
    PalettePair {
        id: PalettePairId::BodyTextOnSurface,
        foreground: SrgbColor::rgb(0x16, 0x1B, 0x22),
        background: SrgbColor::rgb(0xFA, 0xFB, 0xFC),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::SecondaryTextOnSurface,
        foreground: SrgbColor::rgb(0x47, 0x54, 0x67),
        background: SrgbColor::rgb(0xFA, 0xFB, 0xFC),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::CitationLinkOnSurface,
        foreground: SrgbColor::rgb(0x0B, 0x53, 0x94),
        background: SrgbColor::rgb(0xFA, 0xFB, 0xFC),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::PrimaryButtonLabelOnFill,
        foreground: SrgbColor::rgb(0xFF, 0xFF, 0xFF),
        background: SrgbColor::rgb(0x0F, 0x5C, 0x78),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::FocusRingOnSurface,
        foreground: SrgbColor::rgb(0x0A, 0x58, 0xCA),
        background: SrgbColor::rgb(0xFA, 0xFB, 0xFC),
        requirement: ContrastRequirement::UiComponent,
    },
    PalettePair {
        id: PalettePairId::FocusRingOnModal,
        foreground: SrgbColor::rgb(0x0A, 0x58, 0xCA),
        background: SrgbColor::rgb(0xFF, 0xFF, 0xFF),
        requirement: ContrastRequirement::UiComponent,
    },
    PalettePair {
        id: PalettePairId::ErrorAlertTextOnBanner,
        foreground: SrgbColor::rgb(0x8E, 0x19, 0x19),
        background: SrgbColor::rgb(0xFD, 0xED, 0xED),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::InputTextOnField,
        foreground: SrgbColor::rgb(0x11, 0x18, 0x27),
        background: SrgbColor::rgb(0xFF, 0xFF, 0xFF),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::ModalBodyTextOnDialog,
        foreground: SrgbColor::rgb(0x1F, 0x29, 0x37),
        background: SrgbColor::rgb(0xFF, 0xFF, 0xFF),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::NavSelectedTextOnHighlight,
        foreground: SrgbColor::rgb(0x0B, 0x46, 0x60),
        background: SrgbColor::rgb(0xE0, 0xF2, 0xFE),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::LargeHeadingOnSurface,
        foreground: SrgbColor::rgb(0x1D, 0x35, 0x57),
        background: SrgbColor::rgb(0xFA, 0xFB, 0xFC),
        requirement: ContrastRequirement::LargeText,
    },
];

/// Elysian Fields dark theme semantic palette pairs.
pub const DARK_PALETTE: [PalettePair; 11] = [
    PalettePair {
        id: PalettePairId::BodyTextOnSurface,
        foreground: SrgbColor::rgb(0xF1, 0xF5, 0xF9),
        background: SrgbColor::rgb(0x0F, 0x17, 0x2A),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::SecondaryTextOnSurface,
        foreground: SrgbColor::rgb(0xCB, 0xD5, 0xE1),
        background: SrgbColor::rgb(0x0F, 0x17, 0x2A),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::CitationLinkOnSurface,
        foreground: SrgbColor::rgb(0x7D, 0xD3, 0xFC),
        background: SrgbColor::rgb(0x0F, 0x17, 0x2A),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::PrimaryButtonLabelOnFill,
        foreground: SrgbColor::rgb(0x0B, 0x13, 0x24),
        background: SrgbColor::rgb(0x38, 0xBD, 0xF8),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::FocusRingOnSurface,
        foreground: SrgbColor::rgb(0x7D, 0xD3, 0xFC),
        background: SrgbColor::rgb(0x0F, 0x17, 0x2A),
        requirement: ContrastRequirement::UiComponent,
    },
    PalettePair {
        id: PalettePairId::FocusRingOnModal,
        foreground: SrgbColor::rgb(0x7D, 0xD3, 0xFC),
        background: SrgbColor::rgb(0x1E, 0x29, 0x3B),
        requirement: ContrastRequirement::UiComponent,
    },
    PalettePair {
        id: PalettePairId::ErrorAlertTextOnBanner,
        foreground: SrgbColor::rgb(0xFE, 0xCA, 0xCA),
        background: SrgbColor::rgb(0x5B, 0x12, 0x12),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::InputTextOnField,
        foreground: SrgbColor::rgb(0xF8, 0xFA, 0xFC),
        background: SrgbColor::rgb(0x1E, 0x29, 0x3B),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::ModalBodyTextOnDialog,
        foreground: SrgbColor::rgb(0xE2, 0xE8, 0xF0),
        background: SrgbColor::rgb(0x1E, 0x29, 0x3B),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::NavSelectedTextOnHighlight,
        foreground: SrgbColor::rgb(0xE0, 0xF2, 0xFE),
        background: SrgbColor::rgb(0x15, 0x3E, 0x5C),
        requirement: ContrastRequirement::NormalText,
    },
    PalettePair {
        id: PalettePairId::LargeHeadingOnSurface,
        foreground: SrgbColor::rgb(0xE2, 0xE8, 0xF0),
        background: SrgbColor::rgb(0x0F, 0x17, 0x2A),
        requirement: ContrastRequirement::LargeText,
    },
];

/// Returns the canonical Elysian Fields palette for the selected theme.
#[must_use]
pub const fn elysian_palette(theme: PaletteTheme) -> &'static [PalettePair] {
    match theme {
        PaletteTheme::Light => &LIGHT_PALETTE,
        PaletteTheme::Dark => &DARK_PALETTE,
    }
}

/// Audits every pair in `pairs` against WCAG 2.1 AA requirements.
pub fn verify_palette_wcag_aa(
    pairs: &[PalettePair],
) -> Result<Vec<ContrastAuditEntry>, Vec<ContrastViolation>> {
    let mut entries = Vec::with_capacity(pairs.len());
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
            entries.push(ContrastAuditEntry {
                id: pair.id,
                ratio,
                required,
            });
        }
    }

    if violations.is_empty() {
        Ok(entries)
    } else {
        Err(violations)
    }
}
