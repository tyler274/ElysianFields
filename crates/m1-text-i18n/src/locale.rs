//! Locale catalog, script metadata, text direction, and bundled font requirements.

use std::fmt;

/// All supported locales for M1 multilingual text-input and shaping qualification.
///
/// Includes the 9 shipped UI locales plus complex-script shaping probe locales
/// (Urdu RTL and Indic scripts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locale {
    /// English (Latin, LTR) — shipped UI locale.
    En,
    /// Spanish (Latin, LTR) — shipped UI locale.
    Es,
    /// French (Latin, LTR) — shipped UI locale.
    Fr,
    /// Portuguese (Latin, LTR) — shipped UI locale.
    Pt,
    /// German (Latin, LTR) — shipped UI locale.
    De,
    /// Simplified Chinese (Han, LTR) — shipped UI locale.
    ZhHans,
    /// Japanese (Kanji/Kana, LTR) — shipped UI locale.
    Ja,
    /// Korean (Hangul, LTR) — shipped UI locale.
    Ko,
    /// Arabic (Arabic script, RTL) — shipped UI locale.
    Ar,
    /// Hebrew (Hebrew script, RTL) — shipped UI locale.
    He,
    /// Vietnamese (Latin script with stacked diacritics, LTR) — shipped UI locale.
    Vi,
    /// Urdu (Arabic Nastaliq/Naskh script, RTL) — complex-script shaping probe.
    Ur,
    /// Hindi (Devanagari script, LTR) — shipped UI locale & Indic shaping probe.
    Hi,
    /// Bengali (Bengali script, LTR) — complex-script shaping probe.
    Bn,
    /// Marathi (Devanagari script, LTR) — complex-script shaping probe.
    Mr,
    /// Tamil (Tamil script, LTR) — complex-script shaping probe.
    Ta,
    /// Telugu (Telugu script, LTR) — complex-script shaping probe.
    Te,
}

/// Qualification tier for a locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocaleTier {
    /// One of the 9 shipped UI languages.
    ShippedUi,
    /// Complex-script shaping probe exercised in M1 qualification.
    ShapingProbe,
}

/// Paragraph and layout base writing direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextDirection {
    /// Left-to-right text progression.
    Ltr,
    /// Right-to-left text progression.
    Rtl,
}

/// Primary Unicode writing system associated with a locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WritingScript {
    Latin,
    HanSimplified,
    JapaneseKanjiKana,
    Hangul,
    Arabic,
    Hebrew,
    Devanagari,
    Bengali,
    Tamil,
    Telugu,
}

/// Expected OS input and shaping complexity profile for a locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputComplexity {
    /// Direct key events plus dead-key accents.
    DirectKeyboard,
    /// Multi-step OS IME preedit and candidate selection window.
    ImeComposing,
    /// Bidirectional RTL layout, Arabic shaping, and contextual glyph joining.
    RtlBidirectional,
    /// Indic akshara conjuncts, virama ligatures, and reordering vowel signs.
    ComplexIndicShaping,
}

/// Permissively licensed open-source font families required for GPU clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontFamily {
    NotoSans,
    NotoSansMono,
    NotoSansArabic,
    NotoSansHebrew,
    NotoSansCjkSc,
    NotoSansCjkJp,
    NotoSansCjkKr,
    NotoSansDevanagari,
    NotoSansBengali,
    NotoSansTamil,
    NotoSansTelugu,
}

/// Bundled font open-source license classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontLicense {
    /// SIL Open Font License 1.1.
    SilOfl1_1,
}

/// Strategy used to keep Wasm download sizes within M1 budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontDeliveryStrategy {
    /// Bundled eagerly in the core application asset pack.
    CoreEagerBundle,
    /// Loaded on demand when a locale or script run enters the viewport.
    OnDemandScriptChunk,
}

/// Static specification for a bundled font family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontBundleSpec {
    pub family: FontFamily,
    pub family_name: &'static str,
    pub license: FontLicense,
    /// Approximate Brotli/WOFF2 regular+bold weight payload in KiB.
    pub compressed_kib: u32,
    pub delivery: FontDeliveryStrategy,
}

/// Word separation convention used by dictation auto-spacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WordSpacingPolicy {
    /// Separate dictated phrases with ASCII space when adjacent text lacks whitespace.
    SpaceDelimited,
    /// Do not inject spaces between adjacent phrases (e.g., Simplified Chinese, Japanese).
    ScriptContinuous,
}

/// Metadata describing a locale's writing system, direction, input mode, and fonts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocaleMetadata {
    pub locale: Locale,
    pub bcp47: &'static str,
    pub english_name: &'static str,
    pub tier: LocaleTier,
    pub direction: TextDirection,
    pub script: WritingScript,
    pub input_complexity: InputComplexity,
    pub primary_font: FontFamily,
    pub word_spacing: WordSpacingPolicy,
}

/// Error returned when parsing an unsupported BCP-47 locale identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleParseError {
    invalid_tag: String,
}

impl LocaleParseError {
    #[must_use]
    pub fn invalid_tag(&self) -> &str {
        &self.invalid_tag
    }
}

impl fmt::Display for LocaleParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported locale tag '{}'", self.invalid_tag)
    }
}

impl std::error::Error for LocaleParseError {}

impl FontFamily {
    /// All bundled font families covering shipped locales, monospace, and Indic/Urdu probes.
    pub const ALL: [FontFamily; 11] = [
        FontFamily::NotoSans,
        FontFamily::NotoSansMono,
        FontFamily::NotoSansArabic,
        FontFamily::NotoSansHebrew,
        FontFamily::NotoSansCjkSc,
        FontFamily::NotoSansCjkJp,
        FontFamily::NotoSansCjkKr,
        FontFamily::NotoSansDevanagari,
        FontFamily::NotoSansBengali,
        FontFamily::NotoSansTamil,
        FontFamily::NotoSansTelugu,
    ];

    /// Static bundle specification for this font family.
    #[must_use]
    pub const fn bundle_spec(self) -> FontBundleSpec {
        match self {
            FontFamily::NotoSans => FontBundleSpec {
                family: self,
                family_name: "Noto Sans",
                license: FontLicense::SilOfl1_1,
                compressed_kib: 145,
                delivery: FontDeliveryStrategy::CoreEagerBundle,
            },
            FontFamily::NotoSansMono => FontBundleSpec {
                family: self,
                family_name: "Noto Sans Mono",
                license: FontLicense::SilOfl1_1,
                compressed_kib: 95,
                delivery: FontDeliveryStrategy::CoreEagerBundle,
            },
            FontFamily::NotoSansArabic => FontBundleSpec {
                family: self,
                family_name: "Noto Sans Arabic",
                license: FontLicense::SilOfl1_1,
                compressed_kib: 120,
                delivery: FontDeliveryStrategy::OnDemandScriptChunk,
            },
            FontFamily::NotoSansHebrew => FontBundleSpec {
                family: self,
                family_name: "Noto Sans Hebrew",
                license: FontLicense::SilOfl1_1,
                compressed_kib: 58,
                delivery: FontDeliveryStrategy::OnDemandScriptChunk,
            },
            FontFamily::NotoSansCjkSc => FontBundleSpec {
                family: self,
                family_name: "Noto Sans CJK SC (subset chunks)",
                license: FontLicense::SilOfl1_1,
                compressed_kib: 1_850,
                delivery: FontDeliveryStrategy::OnDemandScriptChunk,
            },
            FontFamily::NotoSansCjkJp => FontBundleSpec {
                family: self,
                family_name: "Noto Sans CJK JP (subset chunks)",
                license: FontLicense::SilOfl1_1,
                compressed_kib: 1_700,
                delivery: FontDeliveryStrategy::OnDemandScriptChunk,
            },
            FontFamily::NotoSansCjkKr => FontBundleSpec {
                family: self,
                family_name: "Noto Sans CJK KR (subset chunks)",
                license: FontLicense::SilOfl1_1,
                compressed_kib: 1_450,
                delivery: FontDeliveryStrategy::OnDemandScriptChunk,
            },
            FontFamily::NotoSansDevanagari => FontBundleSpec {
                family: self,
                family_name: "Noto Sans Devanagari",
                license: FontLicense::SilOfl1_1,
                compressed_kib: 110,
                delivery: FontDeliveryStrategy::OnDemandScriptChunk,
            },
            FontFamily::NotoSansBengali => FontBundleSpec {
                family: self,
                family_name: "Noto Sans Bengali",
                license: FontLicense::SilOfl1_1,
                compressed_kib: 125,
                delivery: FontDeliveryStrategy::OnDemandScriptChunk,
            },
            FontFamily::NotoSansTamil => FontBundleSpec {
                family: self,
                family_name: "Noto Sans Tamil",
                license: FontLicense::SilOfl1_1,
                compressed_kib: 65,
                delivery: FontDeliveryStrategy::OnDemandScriptChunk,
            },
            FontFamily::NotoSansTelugu => FontBundleSpec {
                family: self,
                family_name: "Noto Sans Telugu",
                license: FontLicense::SilOfl1_1,
                compressed_kib: 105,
                delivery: FontDeliveryStrategy::OnDemandScriptChunk,
            },
        }
    }

    /// Total compressed KiB for core eagerly loaded fonts.
    #[must_use]
    pub fn core_eager_bundle_kib() -> u32 {
        Self::ALL
            .iter()
            .map(|family| family.bundle_spec())
            .filter(|spec| spec.delivery == FontDeliveryStrategy::CoreEagerBundle)
            .map(|spec| spec.compressed_kib)
            .sum()
    }

    /// Total compressed KiB if every script pack is bundled offline (native desktop/mobile).
    #[must_use]
    pub fn total_all_scripts_kib() -> u32 {
        Self::ALL
            .iter()
            .map(|family| family.bundle_spec().compressed_kib)
            .sum()
    }
}

impl Locale {
    /// The 12 shipped UI locales covering Latin, RTL (Arabic & Hebrew), CJK, Vietnamese, and Hindi.
    pub const SHIPPED_UI: [Locale; 12] = [
        Locale::En,
        Locale::Es,
        Locale::Fr,
        Locale::Pt,
        Locale::De,
        Locale::ZhHans,
        Locale::Ja,
        Locale::Ko,
        Locale::Ar,
        Locale::He,
        Locale::Vi,
        Locale::Hi,
    ];

    /// Complex-script shaping probe locales (Urdu RTL and additional Indic scripts).
    pub const SHAPING_PROBES: [Locale; 5] =
        [Locale::Ur, Locale::Bn, Locale::Mr, Locale::Ta, Locale::Te];

    /// Every locale covered by multilingual qualification.
    pub const ALL: [Locale; 17] = [
        Locale::En,
        Locale::Es,
        Locale::Fr,
        Locale::Pt,
        Locale::De,
        Locale::ZhHans,
        Locale::Ja,
        Locale::Ko,
        Locale::Ar,
        Locale::He,
        Locale::Vi,
        Locale::Hi,
        Locale::Ur,
        Locale::Bn,
        Locale::Mr,
        Locale::Ta,
        Locale::Te,
    ];

    /// Parse a BCP-47 locale tag at the system boundary.
    pub fn parse_bcp47(tag: &str) -> Result<Self, LocaleParseError> {
        let normalized = tag.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "en" | "en-us" | "en-gb" => Ok(Self::En),
            "es" | "es-es" | "es-mx" | "es-419" => Ok(Self::Es),
            "fr" | "fr-fr" | "fr-ca" => Ok(Self::Fr),
            "pt" | "pt-br" | "pt-pt" => Ok(Self::Pt),
            "de" | "de-de" | "de-at" | "de-ch" => Ok(Self::De),
            "zh-hans" | "zh-cn" | "zh-sg" => Ok(Self::ZhHans),
            "ja" | "ja-jp" => Ok(Self::Ja),
            "ko" | "ko-kr" => Ok(Self::Ko),
            "ar" | "ar-sa" | "ar-eg" | "ar-ae" => Ok(Self::Ar),
            "he" | "he-il" | "iw" | "iw-il" => Ok(Self::He),
            "vi" | "vi-vn" => Ok(Self::Vi),
            "ur" | "ur-pk" | "ur-in" => Ok(Self::Ur),
            "hi" | "hi-in" => Ok(Self::Hi),
            "bn" | "bn-bd" | "bn-in" => Ok(Self::Bn),
            "mr" | "mr-in" => Ok(Self::Mr),
            "ta" | "ta-in" | "ta-sg" => Ok(Self::Ta),
            "te" | "te-in" => Ok(Self::Te),
            _ => Err(LocaleParseError {
                invalid_tag: tag.to_owned(),
            }),
        }
    }

    /// Return the single-source metadata record for this locale.
    #[must_use]
    pub const fn metadata(self) -> LocaleMetadata {
        match self {
            Self::En => LocaleMetadata {
                locale: self,
                bcp47: "en",
                english_name: "English",
                tier: LocaleTier::ShippedUi,
                direction: TextDirection::Ltr,
                script: WritingScript::Latin,
                input_complexity: InputComplexity::DirectKeyboard,
                primary_font: FontFamily::NotoSans,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::Es => LocaleMetadata {
                locale: self,
                bcp47: "es",
                english_name: "Spanish",
                tier: LocaleTier::ShippedUi,
                direction: TextDirection::Ltr,
                script: WritingScript::Latin,
                input_complexity: InputComplexity::DirectKeyboard,
                primary_font: FontFamily::NotoSans,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::Fr => LocaleMetadata {
                locale: self,
                bcp47: "fr",
                english_name: "French",
                tier: LocaleTier::ShippedUi,
                direction: TextDirection::Ltr,
                script: WritingScript::Latin,
                input_complexity: InputComplexity::DirectKeyboard,
                primary_font: FontFamily::NotoSans,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::Pt => LocaleMetadata {
                locale: self,
                bcp47: "pt",
                english_name: "Portuguese",
                tier: LocaleTier::ShippedUi,
                direction: TextDirection::Ltr,
                script: WritingScript::Latin,
                input_complexity: InputComplexity::DirectKeyboard,
                primary_font: FontFamily::NotoSans,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::De => LocaleMetadata {
                locale: self,
                bcp47: "de",
                english_name: "German",
                tier: LocaleTier::ShippedUi,
                direction: TextDirection::Ltr,
                script: WritingScript::Latin,
                input_complexity: InputComplexity::DirectKeyboard,
                primary_font: FontFamily::NotoSans,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::ZhHans => LocaleMetadata {
                locale: self,
                bcp47: "zh-Hans",
                english_name: "Simplified Chinese",
                tier: LocaleTier::ShippedUi,
                direction: TextDirection::Ltr,
                script: WritingScript::HanSimplified,
                input_complexity: InputComplexity::ImeComposing,
                primary_font: FontFamily::NotoSansCjkSc,
                word_spacing: WordSpacingPolicy::ScriptContinuous,
            },
            Self::Ja => LocaleMetadata {
                locale: self,
                bcp47: "ja",
                english_name: "Japanese",
                tier: LocaleTier::ShippedUi,
                direction: TextDirection::Ltr,
                script: WritingScript::JapaneseKanjiKana,
                input_complexity: InputComplexity::ImeComposing,
                primary_font: FontFamily::NotoSansCjkJp,
                word_spacing: WordSpacingPolicy::ScriptContinuous,
            },
            Self::Ko => LocaleMetadata {
                locale: self,
                bcp47: "ko",
                english_name: "Korean",
                tier: LocaleTier::ShippedUi,
                direction: TextDirection::Ltr,
                script: WritingScript::Hangul,
                input_complexity: InputComplexity::ImeComposing,
                primary_font: FontFamily::NotoSansCjkKr,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::Ar => LocaleMetadata {
                locale: self,
                bcp47: "ar",
                english_name: "Arabic",
                tier: LocaleTier::ShippedUi,
                direction: TextDirection::Rtl,
                script: WritingScript::Arabic,
                input_complexity: InputComplexity::RtlBidirectional,
                primary_font: FontFamily::NotoSansArabic,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::He => LocaleMetadata {
                locale: self,
                bcp47: "he",
                english_name: "Hebrew",
                tier: LocaleTier::ShippedUi,
                direction: TextDirection::Rtl,
                script: WritingScript::Hebrew,
                input_complexity: InputComplexity::RtlBidirectional,
                primary_font: FontFamily::NotoSansHebrew,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::Vi => LocaleMetadata {
                locale: self,
                bcp47: "vi",
                english_name: "Vietnamese",
                tier: LocaleTier::ShippedUi,
                direction: TextDirection::Ltr,
                script: WritingScript::Latin,
                input_complexity: InputComplexity::DirectKeyboard,
                primary_font: FontFamily::NotoSans,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::Ur => LocaleMetadata {
                locale: self,
                bcp47: "ur",
                english_name: "Urdu",
                tier: LocaleTier::ShapingProbe,
                direction: TextDirection::Rtl,
                script: WritingScript::Arabic,
                input_complexity: InputComplexity::RtlBidirectional,
                primary_font: FontFamily::NotoSansArabic,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::Hi => LocaleMetadata {
                locale: self,
                bcp47: "hi",
                english_name: "Hindi",
                tier: LocaleTier::ShippedUi,
                direction: TextDirection::Ltr,
                script: WritingScript::Devanagari,
                input_complexity: InputComplexity::ComplexIndicShaping,
                primary_font: FontFamily::NotoSansDevanagari,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::Bn => LocaleMetadata {
                locale: self,
                bcp47: "bn",
                english_name: "Bengali",
                tier: LocaleTier::ShapingProbe,
                direction: TextDirection::Ltr,
                script: WritingScript::Bengali,
                input_complexity: InputComplexity::ComplexIndicShaping,
                primary_font: FontFamily::NotoSansBengali,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::Mr => LocaleMetadata {
                locale: self,
                bcp47: "mr",
                english_name: "Marathi",
                tier: LocaleTier::ShapingProbe,
                direction: TextDirection::Ltr,
                script: WritingScript::Devanagari,
                input_complexity: InputComplexity::ComplexIndicShaping,
                primary_font: FontFamily::NotoSansDevanagari,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::Ta => LocaleMetadata {
                locale: self,
                bcp47: "ta",
                english_name: "Tamil",
                tier: LocaleTier::ShapingProbe,
                direction: TextDirection::Ltr,
                script: WritingScript::Tamil,
                input_complexity: InputComplexity::ComplexIndicShaping,
                primary_font: FontFamily::NotoSansTamil,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
            Self::Te => LocaleMetadata {
                locale: self,
                bcp47: "te",
                english_name: "Telugu",
                tier: LocaleTier::ShapingProbe,
                direction: TextDirection::Ltr,
                script: WritingScript::Telugu,
                input_complexity: InputComplexity::ComplexIndicShaping,
                primary_font: FontFamily::NotoSansTelugu,
                word_spacing: WordSpacingPolicy::SpaceDelimited,
            },
        }
    }

    /// Base text direction for this locale.
    #[must_use]
    pub const fn direction(self) -> TextDirection {
        self.metadata().direction
    }

    /// Primary bundled font family required for this locale.
    #[must_use]
    pub const fn primary_font(self) -> FontFamily {
        self.metadata().primary_font
    }

    /// 100% synthetic non-PHI clinical composer phrase exercising script-specific glyphs,
    /// diacritics, conjuncts, or RTL directionality.
    #[must_use]
    pub const fn synthetic_sample(self) -> &'static str {
        match self {
            Self::En => "Follow-up summary: blood pressure 118/76 mmHg [1].",
            Self::Es => "Resumen clínico: presión arterial estable en 118/76 mmHg [1].",
            Self::Fr => "Synthèse clinique : fréquence cardiaque régulière à 68 bpm [1].",
            Self::Pt => "Resumo clínico: medicação revisada sem reações adversas [1].",
            Self::De => "Klinische Übersicht: Laborwerte für Kreatinin überprüft [1].",
            Self::ZhHans => "临床随访摘要：血压稳定在118/76毫米汞柱，继续当前方案[1]。",
            Self::Ja => "臨床経過メモ：血圧は118/76 mmHgで安定しています[1]。",
            Self::Ko => "임상 경과 요약: 혈압 118/76 mmHg로 안정적입니다 [1].",
            Self::Ar => "ملخص المتابعة السريرية: ضغط الدم مستقر عند 118/76 ملم زئبق [1].",
            Self::He => "סיכום מעקב קליני: לחץ דם 118/76 מ״מ כספית, בדיקת HbA1c 6.8% [1].",
            Self::Vi => {
                "Tóm tắt theo dõi lâm sàng: huyết áp ổn định 118/76 mmHg, tiếp tục phác đồ [1]."
            }
            Self::Ur => "طبی فالو اپ خلاصہ: بلڈ پریشر 118/76 پر مستحکم ہے اور نسخہ برقرار ہے [1]۔",
            Self::Hi => "नैदानिक सारांश: रक्तचाप और परीक्षण परिणाम सामान्य हैं [1]।",
            Self::Bn => "ক্লিনিক্যাল সারাংশ: রক্তচাপ ও পরীক্ষার ফলাফল স্বাভাবিক রয়েছে [1]।",
            Self::Mr => "वैद्यकीय सारांश: रक्तदाब आणि चाचणी अहवाल स्थिर आहेत [1].",
            Self::Ta => "மருத்துவ சுருக்கம்: இரத்த அழுத்தம் மற்றும் பரிசோதனை முடிவுகள் சீராக உள்ளன [1].",
            Self::Te => "క్లినికల్ సారాంశం: రక్తపోటు మరియు పరీక్షా ఫలితాలు స్థిరంగా ఉన్నాయి [1].",
        }
    }
}
