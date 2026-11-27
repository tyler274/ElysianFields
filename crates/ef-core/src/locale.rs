//! Nine-language UI locale model and deterministic startup resolution.

use serde::{Deserialize, Serialize};

/// Browser/local storage key persisting the user's selected locale.
pub const LOCALE_STORAGE_KEY: &str = "curie.locale.v1";

/// Layout and text progression direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextDirection {
    Ltr,
    Rtl,
}

/// All 9 shipped UI locales for Curie Clinic and Curie Mobile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Locale {
    #[serde(rename = "en")]
    En,
    #[serde(rename = "es")]
    Es,
    #[serde(rename = "fr")]
    Fr,
    #[serde(rename = "pt")]
    Pt,
    #[serde(rename = "de")]
    De,
    #[serde(rename = "zh-Hans")]
    ZhHans,
    #[serde(rename = "ja")]
    Ja,
    #[serde(rename = "ko")]
    Ko,
    #[serde(rename = "ar")]
    Ar,
}

impl Locale {
    /// Complete ordered list of all 9 shipped locales.
    pub const ALL: [Self; 9] = [
        Self::En,
        Self::Es,
        Self::Fr,
        Self::Pt,
        Self::De,
        Self::ZhHans,
        Self::Ja,
        Self::Ko,
        Self::Ar,
    ];

    /// Canonical BCP-47 language tag.
    #[must_use]
    pub const fn bcp47(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Es => "es",
            Self::Fr => "fr",
            Self::Pt => "pt",
            Self::De => "de",
            Self::ZhHans => "zh-Hans",
            Self::Ja => "ja",
            Self::Ko => "ko",
            Self::Ar => "ar",
        }
    }

    /// Native autonym shown in the language switcher.
    #[must_use]
    pub const fn autonym(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Es => "Español",
            Self::Fr => "Français",
            Self::Pt => "Português",
            Self::De => "Deutsch",
            Self::ZhHans => "简体中文",
            Self::Ja => "日本語",
            Self::Ko => "한국어",
            Self::Ar => "العربية",
        }
    }

    /// Short compact badge shown in the shell chrome.
    #[must_use]
    pub const fn short_badge(self) -> &'static str {
        match self {
            Self::En => "EN",
            Self::Es => "ES",
            Self::Fr => "FR",
            Self::Pt => "PT",
            Self::De => "DE",
            Self::ZhHans => "中文",
            Self::Ja => "日本語",
            Self::Ko => "한국어",
            Self::Ar => "العربية",
        }
    }

    /// Layout direction (`Rtl` for Arabic, `Ltr` for all other shipped locales).
    #[must_use]
    pub const fn direction(self) -> TextDirection {
        match self {
            Self::Ar => TextDirection::Rtl,
            Self::En
            | Self::Es
            | Self::Fr
            | Self::Pt
            | Self::De
            | Self::ZhHans
            | Self::Ja
            | Self::Ko => TextDirection::Ltr,
        }
    }

    /// Parses a BCP-47 language tag (case-insensitive, supports `_` and region/script subtags).
    #[must_use]
    pub fn parse_bcp47(raw: &str) -> Option<Self> {
        let normalized = raw.trim().replace('_', "-").to_ascii_lowercase();
        if normalized.is_empty() {
            return None;
        }
        let primary = normalized.split('-').next().unwrap_or("");
        match primary {
            "en" => Some(Self::En),
            "es" => Some(Self::Es),
            "fr" => Some(Self::Fr),
            "pt" => Some(Self::Pt),
            "de" => Some(Self::De),
            "zh" => Some(Self::ZhHans),
            "ja" => Some(Self::Ja),
            "ko" => Some(Self::Ko),
            "ar" => Some(Self::Ar),
            _ => None,
        }
    }

    /// Resolves startup locale in strict priority order:
    /// 1. `?lang=` query parameter
    /// 2. Persisted `curie.locale.v1`
    /// 3. OS/browser preferred language list
    /// 4. Default `en`
    #[must_use]
    pub fn resolve_startup(
        query_lang: Option<&str>,
        persisted: Option<&str>,
        os_preferred: &[&str],
    ) -> Self {
        if let Some(tag) = query_lang
            && let Some(locale) = Self::parse_bcp47(tag)
        {
            return locale;
        }
        if let Some(tag) = persisted
            && let Some(locale) = Self::parse_bcp47(tag)
        {
            return locale;
        }
        for tag in os_preferred {
            if let Some(locale) = Self::parse_bcp47(tag) {
                return locale;
            }
        }
        Self::En
    }
}
