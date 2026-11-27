use bevy::prelude::Resource;
use bevy::ui::FlexDirection;
use serde::{Deserialize, Serialize};

/// Persisted user preference storage key mandated by the M2 i18n specification.
pub const LOCALE_STORAGE_KEY: &str = "curie.locale.v1";

/// Writing direction for layout mirroring and accessibility metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextDirection {
    Ltr,
    Rtl,
}

/// Horizontal edge resolved according to active [`TextDirection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HorizontalEdge {
    Left,
    Right,
}

/// All 12 shipped UI locales for Curie Clinic and Curie Mobile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Locale {
    #[default]
    En,
    Es,
    Fr,
    Pt,
    De,
    ZhHans,
    Ja,
    Ko,
    Ar,
    He,
    Vi,
    Hi,
}

impl Locale {
    pub const ALL: [Self; 12] = [
        Self::En,
        Self::Es,
        Self::Fr,
        Self::Pt,
        Self::De,
        Self::ZhHans,
        Self::Ja,
        Self::Ko,
        Self::Ar,
        Self::He,
        Self::Vi,
        Self::Hi,
    ];

    #[must_use]
    pub const fn bcp47_tag(self) -> &'static str {
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
            Self::He => "he",
            Self::Vi => "vi",
            Self::Hi => "hi",
        }
    }

    #[must_use]
    pub const fn native_autonym(self) -> &'static str {
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
            Self::He => "עברית",
            Self::Vi => "Tiếng Việt",
            Self::Hi => "हिन्दी",
        }
    }

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
            Self::He => "עברית",
            Self::Vi => "VI",
            Self::Hi => "हिन्दी",
        }
    }

    #[must_use]
    pub const fn direction(self) -> TextDirection {
        match self {
            Self::Ar | Self::He => TextDirection::Rtl,
            Self::En
            | Self::Es
            | Self::Fr
            | Self::Pt
            | Self::De
            | Self::ZhHans
            | Self::Ja
            | Self::Ko
            | Self::Vi
            | Self::Hi => TextDirection::Ltr,
        }
    }

    #[must_use]
    pub const fn is_rtl(self) -> bool {
        matches!(self.direction(), TextDirection::Rtl)
    }

    /// Parses a BCP-47 tag case-insensitively, accepting `-` or `_` separators
    /// and region/script subtags (e.g., `es-MX`, `zh_Hans`, `zh-CN`, `ar-EG`, `he-IL`, `iw`, `vi-VN`, `hi-IN`).
    #[must_use]
    pub fn parse_bcp47(raw: &str) -> Option<Self> {
        let normalized = raw.trim().replace('_', "-").to_ascii_lowercase();
        if normalized.is_empty() {
            return None;
        }
        let mut parts = normalized.split('-');
        let primary = parts.next()?;
        let subtag = parts.next();

        match primary {
            "en" => Some(Self::En),
            "es" => Some(Self::Es),
            "fr" => Some(Self::Fr),
            "pt" => Some(Self::Pt),
            "de" => Some(Self::De),
            "ja" => Some(Self::Ja),
            "ko" => Some(Self::Ko),
            "ar" => Some(Self::Ar),
            "he" | "iw" => Some(Self::He),
            "vi" => Some(Self::Vi),
            "hi" => Some(Self::Hi),
            "zh" => match subtag {
                None | Some("hans" | "cn" | "sg") => Some(Self::ZhHans),
                Some(_) => Some(Self::ZhHans),
            },
            _ => None,
        }
    }

    /// Single owner for startup locale resolution:
    /// 1. `?lang=<bcp47>` URL query parameter
    /// 2. Persisted user preference (`curie.locale.v1`)
    /// 3. OS / browser preferred language list
    /// 4. Default fallback `Locale::En`
    #[must_use]
    pub fn resolve(
        query_param: Option<&str>,
        persisted_pref: Option<&str>,
        os_languages: &[&str],
    ) -> Self {
        if let Some(tag) = query_param
            && let Some(locale) = Self::parse_bcp47(tag)
        {
            return locale;
        }
        if let Some(tag) = persisted_pref
            && let Some(locale) = Self::parse_bcp47(tag)
        {
            return locale;
        }
        for tag in os_languages {
            if let Some(locale) = Self::parse_bcp47(tag) {
                return locale;
            }
        }
        Self::En
    }

    /// Returns mirrored Bevy horizontal flex direction for RTL (`ar`, `he`) vs LTR.
    #[must_use]
    pub const fn row_flex_direction(self) -> FlexDirection {
        match self.direction() {
            TextDirection::Ltr => FlexDirection::Row,
            TextDirection::Rtl => FlexDirection::RowReverse,
        }
    }

    /// Sidebar navigation rail edge (`Left` in LTR, `Right` in RTL).
    #[must_use]
    pub const fn sidebar_edge(self) -> HorizontalEdge {
        match self.direction() {
            TextDirection::Ltr => HorizontalEdge::Left,
            TextDirection::Rtl => HorizontalEdge::Right,
        }
    }

    /// Cited-source reading panel edge (`Right` in LTR, `Left` in RTL).
    #[must_use]
    pub const fn citation_panel_edge(self) -> HorizontalEdge {
        match self.direction() {
            TextDirection::Ltr => HorizontalEdge::Right,
            TextDirection::Rtl => HorizontalEdge::Left,
        }
    }
}

/// Active UI locale stored as a Bevy resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Resource)]
pub struct UiLocale {
    pub current: Locale,
}

impl UiLocale {
    #[must_use]
    pub const fn new(locale: Locale) -> Self {
        Self { current: locale }
    }

    #[must_use]
    pub const fn t(self, key: UiKey) -> &'static str {
        translate(self.current, key)
    }
}

/// Compile-time exhaustive message keys covering shared controls, Curie Clinic,
/// and Curie Mobile (56 keys across all 12 locales = 672 statically checked strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UiKey {
    // Shared core actions & chrome (20)
    AppBrandClinic,
    AppBrandMobile,
    ActionSignIn,
    ActionSignOut,
    ActionCancel,
    ActionConfirm,
    ActionClose,
    ActionRetry,
    ActionSave,
    ActionSearch,
    ActionCopy,
    ActionPaste,
    ActionUndo,
    ActionRedo,
    LabelLanguage,
    LabelThemeLight,
    LabelThemeDark,
    LabelLoading,
    LabelSettings,
    LabelHelp,
    // Privacy & keyboard shortcuts (6)
    PrivacyCoverTitle,
    PrivacyCoverHint,
    PrivacyCoverDismiss,
    ShortcutsDialogTitle,
    ShortcutsHintOpenFinder,
    ShortcutsHintPrivacyCover,
    // Curie Clinic workspace & chat (22)
    ClinicPatientsHeading,
    ClinicPatientFinderPlaceholder,
    ClinicPatientRosterEmpty,
    ClinicPatientRosterError,
    ClinicTranscriptHeading,
    ClinicComposerPlaceholder,
    ClinicSendMessage,
    ClinicStopWork,
    ClinicResumeWork,
    ClinicStatusWorking,
    ClinicStatusStopped,
    ClinicStatusCouldntFinish,
    ClinicCitedSourcesHeading,
    ClinicCitationLoading,
    ClinicCitationUnavailable,
    ClinicOriginalLanguageNote,
    ClinicDraftSaved,
    ClinicDraftConflict,
    ClinicDraftNotSaved,
    ClinicSessionEnded,
    ClinicLabRecordsHeading,
    ClinicMobileMenuButton,
    // Curie Mobile workspace (8)
    MobileTabHome,
    MobileTabCarePlan,
    MobileTabMessages,
    MobileTabHealthRecords,
    MobileRecordsSynced,
    MobileHealthPermissionsNeeded,
    MobileNotificationsHeading,
    MobileOfflineBanner,
}

impl UiKey {
    pub const ALL: [Self; 56] = [
        Self::AppBrandClinic,
        Self::AppBrandMobile,
        Self::ActionSignIn,
        Self::ActionSignOut,
        Self::ActionCancel,
        Self::ActionConfirm,
        Self::ActionClose,
        Self::ActionRetry,
        Self::ActionSave,
        Self::ActionSearch,
        Self::ActionCopy,
        Self::ActionPaste,
        Self::ActionUndo,
        Self::ActionRedo,
        Self::LabelLanguage,
        Self::LabelThemeLight,
        Self::LabelThemeDark,
        Self::LabelLoading,
        Self::LabelSettings,
        Self::LabelHelp,
        Self::PrivacyCoverTitle,
        Self::PrivacyCoverHint,
        Self::PrivacyCoverDismiss,
        Self::ShortcutsDialogTitle,
        Self::ShortcutsHintOpenFinder,
        Self::ShortcutsHintPrivacyCover,
        Self::ClinicPatientsHeading,
        Self::ClinicPatientFinderPlaceholder,
        Self::ClinicPatientRosterEmpty,
        Self::ClinicPatientRosterError,
        Self::ClinicTranscriptHeading,
        Self::ClinicComposerPlaceholder,
        Self::ClinicSendMessage,
        Self::ClinicStopWork,
        Self::ClinicResumeWork,
        Self::ClinicStatusWorking,
        Self::ClinicStatusStopped,
        Self::ClinicStatusCouldntFinish,
        Self::ClinicCitedSourcesHeading,
        Self::ClinicCitationLoading,
        Self::ClinicCitationUnavailable,
        Self::ClinicOriginalLanguageNote,
        Self::ClinicDraftSaved,
        Self::ClinicDraftConflict,
        Self::ClinicDraftNotSaved,
        Self::ClinicSessionEnded,
        Self::ClinicLabRecordsHeading,
        Self::ClinicMobileMenuButton,
        Self::MobileTabHome,
        Self::MobileTabCarePlan,
        Self::MobileTabMessages,
        Self::MobileTabHealthRecords,
        Self::MobileRecordsSynced,
        Self::MobileHealthPermissionsNeeded,
        Self::MobileNotificationsHeading,
        Self::MobileOfflineBanner,
    ];
}

#[derive(Debug, Clone, Copy)]
struct LocalizedRow {
    en: &'static str,
    es: &'static str,
    fr: &'static str,
    pt: &'static str,
    de: &'static str,
    zh_hans: &'static str,
    ja: &'static str,
    ko: &'static str,
    ar: &'static str,
    he: &'static str,
    vi: &'static str,
    hi: &'static str,
}

impl LocalizedRow {
    const fn pick(self, locale: Locale) -> &'static str {
        match locale {
            Locale::En => self.en,
            Locale::Es => self.es,
            Locale::Fr => self.fr,
            Locale::Pt => self.pt,
            Locale::De => self.de,
            Locale::ZhHans => self.zh_hans,
            Locale::Ja => self.ja,
            Locale::Ko => self.ko,
            Locale::Ar => self.ar,
            Locale::He => self.he,
            Locale::Vi => self.vi,
            Locale::Hi => self.hi,
        }
    }
}

/// Compile-time exhaustive translation lookup for all 12 shipped locales.
#[must_use]
pub const fn translate(locale: Locale, key: UiKey) -> &'static str {
    let row = match key {
        UiKey::AppBrandClinic => LocalizedRow {
            en: "Curie Clinic",
            es: "Curie Clínica",
            fr: "Curie Clinique",
            pt: "Curie Clínica",
            de: "Curie Klinik",
            zh_hans: "Curie 临床",
            ja: "Curie クリニック",
            ko: "Curie 클리닉",
            ar: "عيادة كوري",
            he: "מרפאת קירי",
            vi: "Phòng khám Curie",
            hi: "क्यूरी क्लिनिक",
        },
        UiKey::AppBrandMobile => LocalizedRow {
            en: "Curie Mobile",
            es: "Curie Móvil",
            fr: "Curie Mobile",
            pt: "Curie Móvel",
            de: "Curie Mobil",
            zh_hans: "Curie 移动版",
            ja: "Curie モバイル",
            ko: "Curie 모바일",
            ar: "كوري للجوال",
            he: "קירי מובייל",
            vi: "Curie Di động",
            hi: "क्यूरी मोबाइल",
        },
        UiKey::ActionSignIn => LocalizedRow {
            en: "Sign in",
            es: "Iniciar sesión",
            fr: "Se connecter",
            pt: "Entrar",
            de: "Anmelden",
            zh_hans: "登录",
            ja: "サインイン",
            ko: "로그인",
            ar: "تسجيل الدخول",
            he: "התחברות",
            vi: "Đăng nhập",
            hi: "साइन इन करें",
        },
        UiKey::ActionSignOut => LocalizedRow {
            en: "Sign out",
            es: "Cerrar sesión",
            fr: "Se déconnecter",
            pt: "Sair",
            de: "Abmelden",
            zh_hans: "退出登录",
            ja: "サインアウト",
            ko: "로그아웃",
            ar: "تسجيل الخروج",
            he: "התנתקות",
            vi: "Đăng xuất",
            hi: "साइन आउट करें",
        },
        UiKey::ActionCancel => LocalizedRow {
            en: "Cancel",
            es: "Cancelar",
            fr: "Annuler",
            pt: "Cancelar",
            de: "Abbrechen",
            zh_hans: "取消",
            ja: "キャンセル",
            ko: "취소",
            ar: "إلغاء",
            he: "ביטול",
            vi: "Hủy",
            hi: "रद्द करें",
        },
        UiKey::ActionConfirm => LocalizedRow {
            en: "Confirm",
            es: "Confirmar",
            fr: "Confirmer",
            pt: "Confirmar",
            de: "Bestätigen",
            zh_hans: "确认",
            ja: "確認",
            ko: "확인",
            ar: "تأكيد",
            he: "אישור",
            vi: "Xác nhận",
            hi: "पुष्टि करें",
        },
        UiKey::ActionClose => LocalizedRow {
            en: "Close",
            es: "Cerrar",
            fr: "Fermer",
            pt: "Fechar",
            de: "Schließen",
            zh_hans: "关闭",
            ja: "閉じる",
            ko: "닫기",
            ar: "إغلاق",
            he: "סגירה",
            vi: "Đóng",
            hi: "बंद करें",
        },
        UiKey::ActionRetry => LocalizedRow {
            en: "Try again",
            es: "Intentar de nuevo",
            fr: "Réessayer",
            pt: "Tentar novamente",
            de: "Erneut versuchen",
            zh_hans: "重试",
            ja: "再試行",
            ko: "다시 시도",
            ar: "إعادة المحاولة",
            he: "נסה שוב",
            vi: "Thử lại",
            hi: "पुनः प्रयास करें",
        },
        UiKey::ActionSave => LocalizedRow {
            en: "Save",
            es: "Guardar",
            fr: "Enregistrer",
            pt: "Salvar",
            de: "Speichern",
            zh_hans: "保存",
            ja: "保存",
            ko: "저장",
            ar: "حفظ",
            he: "שמירה",
            vi: "Lưu",
            hi: "सहेजें",
        },
        UiKey::ActionSearch => LocalizedRow {
            en: "Search",
            es: "Buscar",
            fr: "Rechercher",
            pt: "Pesquisar",
            de: "Suchen",
            zh_hans: "搜索",
            ja: "検索",
            ko: "검색",
            ar: "بحث",
            he: "חיפוש",
            vi: "Tìm kiếm",
            hi: "खोजें",
        },
        UiKey::ActionCopy => LocalizedRow {
            en: "Copy",
            es: "Copiar",
            fr: "Copier",
            pt: "Copiar",
            de: "Kopieren",
            zh_hans: "复制",
            ja: "コピー",
            ko: "복사",
            ar: "نسخ",
            he: "העתקה",
            vi: "Sao chép",
            hi: "कॉपी करें",
        },
        UiKey::ActionPaste => LocalizedRow {
            en: "Paste",
            es: "Pegar",
            fr: "Coller",
            pt: "Colar",
            de: "Einfügen",
            zh_hans: "粘贴",
            ja: "貼り付け",
            ko: "붙여넣기",
            ar: "لصق",
            he: "הדבקה",
            vi: "Dán",
            hi: "पेस्ट करें",
        },
        UiKey::ActionUndo => LocalizedRow {
            en: "Undo",
            es: "Deshacer",
            fr: "Annuler",
            pt: "Desfazer",
            de: "Rückgängig",
            zh_hans: "撤销",
            ja: "元に戻す",
            ko: "실행 취소",
            ar: "تراجع",
            he: "ביטול פעולה",
            vi: "Hoàn tác",
            hi: "पूर्ववत करें",
        },
        UiKey::ActionRedo => LocalizedRow {
            en: "Redo",
            es: "Rehacer",
            fr: "Rétablir",
            pt: "Refazer",
            de: "Wiederholen",
            zh_hans: "重做",
            ja: "やり直す",
            ko: "다시 실행",
            ar: "إعادة",
            he: "ביצוע מחדש",
            vi: "Làm lại",
            hi: "फिर से करें",
        },
        UiKey::LabelLanguage => LocalizedRow {
            en: "Language",
            es: "Idioma",
            fr: "Langue",
            pt: "Idioma",
            de: "Sprache",
            zh_hans: "语言",
            ja: "言語",
            ko: "언어",
            ar: "اللغة",
            he: "שפה",
            vi: "Ngôn ngữ",
            hi: "भाषा",
        },
        UiKey::LabelThemeLight => LocalizedRow {
            en: "Light theme",
            es: "Tema claro",
            fr: "Thème clair",
            pt: "Tema claro",
            de: "Helles Design",
            zh_hans: "浅色主题",
            ja: "ライトテーマ",
            ko: "라이트 테마",
            ar: "المظهر الفاتح",
            he: "ערכת נושא בהירה",
            vi: "Giao diện sáng",
            hi: "हल्की थीम",
        },
        UiKey::LabelThemeDark => LocalizedRow {
            en: "Dark theme",
            es: "Tema oscuro",
            fr: "Thème sombre",
            pt: "Tema escuro",
            de: "Dunkles Design",
            zh_hans: "深色主题",
            ja: "ダークテーマ",
            ko: "다크 테마",
            ar: "المظهر الداكن",
            he: "ערכת נושא כהה",
            vi: "Giao diện tối",
            hi: "गहरी थीम",
        },
        UiKey::LabelLoading => LocalizedRow {
            en: "Loading",
            es: "Cargando",
            fr: "Chargement",
            pt: "Carregando",
            de: "Wird geladen",
            zh_hans: "正在加载",
            ja: "読み込み中",
            ko: "불러오는 중",
            ar: "جاري التحميل",
            he: "טוען",
            vi: "Đang tải",
            hi: "लोड हो रहा है",
        },
        UiKey::LabelSettings => LocalizedRow {
            en: "Settings",
            es: "Configuración",
            fr: "Paramètres",
            pt: "Configurações",
            de: "Einstellungen",
            zh_hans: "设置",
            ja: "設定",
            ko: "설정",
            ar: "الإعدادات",
            he: "הגדרות",
            vi: "Cài đặt",
            hi: "सेटिंग्स",
        },
        UiKey::LabelHelp => LocalizedRow {
            en: "Help",
            es: "Ayuda",
            fr: "Aide",
            pt: "Ajuda",
            de: "Hilfe",
            zh_hans: "帮助",
            ja: "ヘルプ",
            ko: "도움말",
            ar: "مساعدة",
            he: "עזרה",
            vi: "Trợ giúp",
            hi: "सहायता",
        },
        UiKey::PrivacyCoverTitle => LocalizedRow {
            en: "Privacy screen active",
            es: "Pantalla de privacidad activa",
            fr: "Écran de confidentialité actif",
            pt: "Tela de privacidade ativa",
            de: "Sichtschutz aktiv",
            zh_hans: "隐私遮罩已开启",
            ja: "プライバシーカバー有効",
            ko: "개인정보 보호 화면 활성화됨",
            ar: "شاشة الخصوصية نشطة",
            he: "מסך פרטיות פעיל",
            vi: "Màn hình bảo mật đang bật",
            hi: "गोपनीयता स्क्रीन सक्रिय है",
        },
        UiKey::PrivacyCoverHint => LocalizedRow {
            en: "Clinical details are hidden. Press Cmd/Ctrl+Shift+L to return.",
            es: "Los datos clínicos están ocultos. Pulsa Cmd/Ctrl+Mayús+L para volver.",
            fr: "Les détails cliniques sont masqués. Appuyez sur Cmd/Ctrl+Maj+L pour revenir.",
            pt: "Os detalhes clínicos estão ocultos. Pressione Cmd/Ctrl+Shift+L para voltar.",
            de: "Klinische Angaben sind verborgen. Drücken Sie Cmd/Strg+Umschalt+L zur Rückkehr.",
            zh_hans: "临床内容已隐藏。按 Cmd/Ctrl+Shift+L 返回。",
            ja: "臨床情報は非表示です。Cmd/Ctrl+Shift+L で戻ります。",
            ko: "임상 정보가 숨겨졌습니다. 돌아가려면 Cmd/Ctrl+Shift+L을 누르세요.",
            ar: "التفاصيل السريرية مخفية. اضغط Cmd/Ctrl+Shift+L للعودة.",
            he: "פרטים קליניים מוסתרים. לחץ על Cmd/Ctrl+Shift+L כדי לחזור.",
            vi: "Thông tin lâm sàng đang được ẩn. Nhấn Cmd/Ctrl+Shift+L để quay lại.",
            hi: "नैदानिक विवरण छिपे हैं। वापस आने के लिए Cmd/Ctrl+Shift+L दबाएँ।",
        },
        UiKey::PrivacyCoverDismiss => LocalizedRow {
            en: "Resume session",
            es: "Reanudar sesión",
            fr: "Reprendre la session",
            pt: "Retomar sessão",
            de: "Sitzung fortsetzen",
            zh_hans: "恢复会话",
            ja: "セッションを再開",
            ko: "세션 재개",
            ar: "استئناف الجلسة",
            he: "המשך הפעלה",
            vi: "Tiếp tục phiên làm việc",
            hi: "सत्र फिर से शुरू करें",
        },
        UiKey::ShortcutsDialogTitle => LocalizedRow {
            en: "Keyboard shortcuts",
            es: "Atajos de teclado",
            fr: "Raccourcis clavier",
            pt: "Atalhos de teclado",
            de: "Tastenkürzel",
            zh_hans: "键盘快捷键",
            ja: "キーボードショートカット",
            ko: "키보드 단축키",
            ar: "اختصارات لوحة المفاتيح",
            he: "קיצורי מקלדת",
            vi: "Phím tắt bàn phím",
            hi: "कीबोर्ड शॉर्टकट",
        },
        UiKey::ShortcutsHintOpenFinder => LocalizedRow {
            en: "Open patient finder",
            es: "Abrir buscador de pacientes",
            fr: "Ouvrir la recherche patient",
            pt: "Abrir busca de pacientes",
            de: "Patientensuche öffnen",
            zh_hans: "打开患者查找",
            ja: "患者検索を開く",
            ko: "환자 검색 열기",
            ar: "فتح البحث عن المرضى",
            he: "פתיחת חיפוש מטופלים",
            vi: "Mở tìm kiếm bệnh nhân",
            hi: "मरीज़ खोज खोलें",
        },
        UiKey::ShortcutsHintPrivacyCover => LocalizedRow {
            en: "Toggle privacy screen cover",
            es: "Alternar pantalla de privacidad",
            fr: "Activer l'écran de confidentialité",
            pt: "Alternar tela de privacidade",
            de: "Sichtschutz umschalten",
            zh_hans: "切换隐私遮罩",
            ja: "プライバシーカバーを切り替え",
            ko: "개인정보 보호 화면 전환",
            ar: "تبديل شاشة الخصوصية",
            he: "הפעלה או כיבוי של מסך פרטיות",
            vi: "Bật/tắt màn hình bảo mật",
            hi: "गोपनीयता आवरण टॉगल करें",
        },
        UiKey::ClinicPatientsHeading => LocalizedRow {
            en: "Patients",
            es: "Pacientes",
            fr: "Patients",
            pt: "Pacientes",
            de: "Patienten",
            zh_hans: "患者列表",
            ja: "患者一覧",
            ko: "환자 목록",
            ar: "المرضى",
            he: "מטופלים",
            vi: "Danh sách bệnh nhân",
            hi: "मरीज़",
        },
        UiKey::ClinicPatientFinderPlaceholder => LocalizedRow {
            en: "Find a patient...",
            es: "Buscar un paciente...",
            fr: "Rechercher un patient...",
            pt: "Encontrar um paciente...",
            de: "Patient suchen...",
            zh_hans: "查找患者…",
            ja: "患者を検索…",
            ko: "환자 검색…",
            ar: "ابحث عن مريض...",
            he: "חיפוש מטופל...",
            vi: "Tìm bệnh nhân...",
            hi: "मरीज़ खोजें...",
        },
        UiKey::ClinicPatientRosterEmpty => LocalizedRow {
            en: "No patients in this workspace yet.",
            es: "Aún no hay pacientes en este espacio de trabajo.",
            fr: "Aucun patient dans cet espace de travail pour le moment.",
            pt: "Ainda não há pacientes neste espaço de trabalho.",
            de: "Noch keine Patienten in diesem Arbeitsbereich.",
            zh_hans: "此工作区暂无患者。",
            ja: "このワークスペースにはまだ患者がいません。",
            ko: "이 워크스페이스에 아직 환자가 없습니다.",
            ar: "لا يوجد مرضى في مساحة العمل هذه بعد.",
            he: "אין עדיין מטופלים בסביבת עבודה זו.",
            vi: "Chưa có bệnh nhân nào trong không gian làm việc này.",
            hi: "इस कार्यक्षेत्र में अभी कोई मरीज़ नहीं है।",
        },
        UiKey::ClinicPatientRosterError => LocalizedRow {
            en: "Could not load the patient list.",
            es: "No se pudo cargar la lista de pacientes.",
            fr: "Impossible de charger la liste des patients.",
            pt: "Não foi possível carregar a lista de pacientes.",
            de: "Patientenliste konnte nicht geladen werden.",
            zh_hans: "无法加载患者列表。",
            ja: "患者一覧を読み込めませんでした。",
            ko: "환자 목록을 불러올 수 없습니다.",
            ar: "تعذر تحميل قائمة المرضى.",
            he: "לא ניתן היה לטעון את רשימת המטופלים.",
            vi: "Không thể tải danh sách bệnh nhân.",
            hi: "मरीज़ों की सूची लोड नहीं हो सकी।",
        },
        UiKey::ClinicTranscriptHeading => LocalizedRow {
            en: "Clinical conversation",
            es: "Conversación clínica",
            fr: "Conversation clinique",
            pt: "Conversa clínica",
            de: "Klinisches Gespräch",
            zh_hans: "临床对话",
            ja: "臨床会話",
            ko: "임상 대화",
            ar: "المحادثة السريرية",
            he: "שיחה קלינית",
            vi: "Trao đổi lâm sàng",
            hi: "नैदानिक संवाद",
        },
        UiKey::ClinicComposerPlaceholder => LocalizedRow {
            en: "Ask a clinical question or review records...",
            es: "Haz una pregunta clínica o revisa registros...",
            fr: "Posez une question clinique ou consultez le dossier...",
            pt: "Faça uma pergunta clínica ou revise registros...",
            de: "Klinische Frage stellen oder Akte prüfen...",
            zh_hans: "输入临床问题或查阅病历…",
            ja: "臨床上の質問を入力または記録を確認…",
            ko: "임상 질문을 입력하거나 기록을 검토하세요…",
            ar: "اطرح سؤالاً سريرياً أو راجع السجلات...",
            he: "שאל שאלה קלינית או עיין ברשומות...",
            vi: "Đặt câu hỏi lâm sàng hoặc xem lại hồ sơ...",
            hi: "नैदानिक प्रश्न पूछें या मेडिकल रिकॉर्ड देखें...",
        },
        UiKey::ClinicSendMessage => LocalizedRow {
            en: "Send",
            es: "Enviar",
            fr: "Envoyer",
            pt: "Enviar",
            de: "Senden",
            zh_hans: "发送",
            ja: "送信",
            ko: "보내기",
            ar: "إرسال",
            he: "שליחה",
            vi: "Gửi",
            hi: "भेजें",
        },
        UiKey::ClinicStopWork => LocalizedRow {
            en: "Stop",
            es: "Detener",
            fr: "Arrêter",
            pt: "Parar",
            de: "Stoppen",
            zh_hans: "停止",
            ja: "停止",
            ko: "중지",
            ar: "إيقاف",
            he: "עצירה",
            vi: "Dừng",
            hi: "रोकें",
        },
        UiKey::ClinicResumeWork => LocalizedRow {
            en: "Resume",
            es: "Reanudar",
            fr: "Reprendre",
            pt: "Retomar",
            de: "Fortsetzen",
            zh_hans: "继续",
            ja: "再開",
            ko: "재개",
            ar: "استئناف",
            he: "המשך",
            vi: "Tiếp tục",
            hi: "जारी रखें",
        },
        UiKey::ClinicStatusWorking => LocalizedRow {
            en: "Working on clinical review...",
            es: "Procesando revisión clínica...",
            fr: "Analyse clinique en cours...",
            pt: "Processando revisão clínica...",
            de: "Klinische Prüfung läuft...",
            zh_hans: "正在进行临床分析…",
            ja: "臨床レビューを作成中…",
            ko: "임상 검토 진행 중…",
            ar: "جاري إعداد المراجعة السريرية...",
            he: "מעבד סקירה קלינית...",
            vi: "Đang xử lý đánh giá lâm sàng...",
            hi: "नैदानिक समीक्षा पर कार्य जारी है...",
        },
        UiKey::ClinicStatusStopped => LocalizedRow {
            en: "Stopped.",
            es: "Detenido.",
            fr: "Arrêté.",
            pt: "Interrompido.",
            de: "Gestoppt.",
            zh_hans: "已停止。",
            ja: "停止しました。",
            ko: "중지됨.",
            ar: "تم الإيقاف.",
            he: "נעצר.",
            vi: "Đã dừng.",
            hi: "रोका गया।",
        },
        UiKey::ClinicStatusCouldntFinish => LocalizedRow {
            en: "Could not finish this response.",
            es: "No se pudo completar esta respuesta.",
            fr: "Impossible de terminer cette réponse.",
            pt: "Não foi possível concluir esta resposta.",
            de: "Antwort konnte nicht abgeschlossen werden.",
            zh_hans: "无法完成此回复。",
            ja: "この応答を完了できませんでした。",
            ko: "이 응답을 완료할 수 없습니다.",
            ar: "تعذر إكمال هذه الاستجابة.",
            he: "לא ניתן היה להשלים תשובה זו.",
            vi: "Không thể hoàn tất phản hồi này.",
            hi: "यह उत्तर पूरा नहीं किया जा सका।",
        },
        UiKey::ClinicCitedSourcesHeading => LocalizedRow {
            en: "Cited sources",
            es: "Fuentes citadas",
            fr: "Sources citées",
            pt: "Fontes citadas",
            de: "Zitierte Quellen",
            zh_hans: "引用来源",
            ja: "引用元",
            ko: "인용 출처",
            ar: "المصادر المستشهد بها",
            he: "מקורות מצוטטים",
            vi: "Nguồn trích dẫn",
            hi: "उद्धृत स्रोत",
        },
        UiKey::ClinicCitationLoading => LocalizedRow {
            en: "Loading cited record...",
            es: "Cargando registro citado...",
            fr: "Chargement du document cité...",
            pt: "Carregando registro citado...",
            de: "Zitierte Quelle wird geladen...",
            zh_hans: "正在加载引用病历…",
            ja: "引用記録を読み込み中…",
            ko: "인용 기록을 불러오는 중…",
            ar: "جاري تحميل السجل المستشهد به...",
            he: "טוען רשומה מצוטטת...",
            vi: "Đang tải hồ sơ được trích dẫn...",
            hi: "उद्धृत रिकॉर्ड लोड हो रहा है...",
        },
        UiKey::ClinicCitationUnavailable => LocalizedRow {
            en: "This cited record is no longer available.",
            es: "Este registro citado ya no está disponible.",
            fr: "Ce document cité n'est plus disponible.",
            pt: "Este registro citado não está mais disponível.",
            de: "Diese zitierte Quelle ist nicht mehr verfügbar.",
            zh_hans: "该引用病历已不可用。",
            ja: "この引用記録は利用できません。",
            ko: "이 인용 기록은 더 이상 사용할 수 없습니다.",
            ar: "هذا السجل المستشهد به لم يعد متاحاً.",
            he: "רשומה מצוטטת זו אינה זמינה עוד.",
            vi: "Hồ sơ được trích dẫn này không còn khả dụng.",
            hi: "यह उद्धृत रिकॉर्ड अब उपलब्ध नहीं है।",
        },
        UiKey::ClinicOriginalLanguageNote => LocalizedRow {
            en: "Shown verbatim in its original source language.",
            es: "Mostrado textualmente en su idioma original.",
            fr: "Affiché mot pour mot dans sa langue d'origine.",
            pt: "Exibido literalmente no idioma original.",
            de: "Wortgetreu in der Originalsprache angezeigt.",
            zh_hans: "按原始语言逐字显示。",
            ja: "元の言語のまま原文で表示しています。",
            ko: "원본 언어로 그대로 표시됩니다.",
            ar: "معروض حرفياً بلغته الأصلية.",
            he: "מוצג מילה במילה בשפת המקור.",
            vi: "Hiển thị nguyên văn bằng ngôn ngữ gốc của tài liệu.",
            hi: "मूल स्रोत भाषा में शब्दशः प्रदर्शित।",
        },
        UiKey::ClinicDraftSaved => LocalizedRow {
            en: "Draft saved",
            es: "Borrador guardado",
            fr: "Brouillon enregistré",
            pt: "Rascunho salvo",
            de: "Entwurf gespeichert",
            zh_hans: "草稿已保存",
            ja: "下書きを保存しました",
            ko: "초안 저장됨",
            ar: "تم حفظ المسودة",
            he: "הטיוטה נשמרה",
            vi: "Đã lưu bản nháp",
            hi: "ड्राफ्ट सहेजा गया",
        },
        UiKey::ClinicDraftConflict => LocalizedRow {
            en: "Draft updated in another tab.",
            es: "Borrador actualizado en otra pestaña.",
            fr: "Brouillon modifié dans un autre onglet.",
            pt: "Rascunho atualizado em outra aba.",
            de: "Entwurf in einem anderen Tab geändert.",
            zh_hans: "草稿已在另一个标签页中更新。",
            ja: "下書きが別のタブで更新されました。",
            ko: "다른 탭에서 초안이 업데이트되었습니다.",
            ar: "تم تحديث المسودة في علامة تبويب أخرى.",
            he: "הטיוטה עודכנה בלשונית אחרת.",
            vi: "Bản nháp đã được cập nhật ở thẻ khác.",
            hi: "ड्राफ्ट दूसरे टैब में अपडेट हो गया है।",
        },
        UiKey::ClinicDraftNotSaved => LocalizedRow {
            en: "Draft could not be saved.",
            es: "No se pudo guardar el borrador.",
            fr: "Le brouillon n'a pas pu être enregistré.",
            pt: "Não foi possível salvar o rascunho.",
            de: "Entwurf konnte nicht gespeichert werden.",
            zh_hans: "无法保存草稿。",
            ja: "下書きを保存できませんでした。",
            ko: "초안을 저장할 수 없습니다.",
            ar: "تعذر حفظ المسودة.",
            he: "לא ניתן היה לשמור את הטיוטה.",
            vi: "Không thể lưu bản nháp.",
            hi: "ड्राफ्ट सहेजा नहीं जा सका।",
        },
        UiKey::ClinicSessionEnded => LocalizedRow {
            en: "Your session has ended. Sign in again to continue.",
            es: "Tu sesión ha finalizado. Inicia sesión de nuevo para continuar.",
            fr: "Votre session a expiré. Reconnectez-vous pour continuer.",
            pt: "Sua sessão terminou. Entre novamente para continuar.",
            de: "Ihre Sitzung ist abgelaufen. Melden Sie sich erneut an.",
            zh_hans: "会话已结束，请重新登录以继续。",
            ja: "セッションが終了しました。続行するには再度サインインしてください。",
            ko: "세션이 종료되었습니다. 계속하려면 다시 로그인하세요.",
            ar: "انتهت جلستك. يرجى تسجيل الدخول مرة أخرى للمتابعة.",
            he: "ההפעלה שלך הסתיימה. התחבר שוב כדי להמשיך.",
            vi: "Phiên của bạn đã kết thúc. Vui lòng đăng nhập lại để tiếp tục.",
            hi: "आपका सत्र समाप्त हो गया है। जारी रखने के लिए पुनः साइन इन करें।",
        },
        UiKey::ClinicLabRecordsHeading => LocalizedRow {
            en: "Laboratory results",
            es: "Resultados de laboratorio",
            fr: "Résultats de laboratoire",
            pt: "Resultados laboratoriais",
            de: "Laborergebnisse",
            zh_hans: "检验结果",
            ja: "検査結果",
            ko: "검사 결과",
            ar: "نتائج المختبر",
            he: "תוצאות מעבדה",
            vi: "Kết quả xét nghiệm",
            hi: "प्रयोगशाला परिणाम",
        },
        UiKey::ClinicMobileMenuButton => LocalizedRow {
            en: "Open navigation menu",
            es: "Abrir menú de navegación",
            fr: "Ouvrir le menu de navigation",
            pt: "Abrir menu de navegação",
            de: "Navigationsmenü öffnen",
            zh_hans: "打开导航菜单",
            ja: "ナビゲーションメニューを開く",
            ko: "탐색 메뉴 열기",
            ar: "فتح قائمة التنقل",
            he: "פתיחת תפריט ניווט",
            vi: "Mở trình đơn điều hướng",
            hi: "नेविगेशन मेनू खोलें",
        },
        UiKey::MobileTabHome => LocalizedRow {
            en: "Home",
            es: "Inicio",
            fr: "Accueil",
            pt: "Início",
            de: "Start",
            zh_hans: "首页",
            ja: "ホーム",
            ko: "홈",
            ar: "الرئيسية",
            he: "בית",
            vi: "Trang chủ",
            hi: "होम",
        },
        UiKey::MobileTabCarePlan => LocalizedRow {
            en: "Care plan",
            es: "Plan de cuidado",
            fr: "Plan de soins",
            pt: "Plano de cuidados",
            de: "Behandlungsplan",
            zh_hans: "照护计划",
            ja: "ケアプラン",
            ko: "케어 플랜",
            ar: "خطة الرعاية",
            he: "תוכנית טיפול",
            vi: "Kế hoạch chăm sóc",
            hi: "देखभाल योजना",
        },
        UiKey::MobileTabMessages => LocalizedRow {
            en: "Messages",
            es: "Mensajes",
            fr: "Messages",
            pt: "Mensagens",
            de: "Nachrichten",
            zh_hans: "消息",
            ja: "メッセージ",
            ko: "메시지",
            ar: "الرسائل",
            he: "הודעות",
            vi: "Tin nhắn",
            hi: "संदेश",
        },
        UiKey::MobileTabHealthRecords => LocalizedRow {
            en: "Health records",
            es: "Registros de salud",
            fr: "Dossiers de santé",
            pt: "Registros de saúde",
            de: "Gesundheitsdaten",
            zh_hans: "健康记录",
            ja: "健康記録",
            ko: "건강 기록",
            ar: "السجلات الصحية",
            he: "רשומות בריאות",
            vi: "Hồ sơ sức khỏe",
            hi: "स्वास्थ्य रिकॉर्ड",
        },
        UiKey::MobileRecordsSynced => LocalizedRow {
            en: "Records synced",
            es: "Registros sincronizados",
            fr: "Dossiers synchronisés",
            pt: "Registros sincronizados",
            de: "Daten synchronisiert",
            zh_hans: "记录已同步",
            ja: "記録を同期しました",
            ko: "기록 동기화됨",
            ar: "تمت مزامنة السجلات",
            he: "הרשומות סונכרנו",
            vi: "Hồ sơ đã đồng bộ",
            hi: "रिकॉर्ड सिंक हो गए",
        },
        UiKey::MobileHealthPermissionsNeeded => LocalizedRow {
            en: "Health data permission required",
            es: "Se requiere permiso de datos de salud",
            fr: "Autorisation d'accès aux données de santé requise",
            pt: "Permissão de dados de saúde necessária",
            de: "Freigabe für Gesundheitsdaten erforderlich",
            zh_hans: "需要健康数据访问权限",
            ja: "ヘルスケアデータの許可が必要です",
            ko: "건강 데이터 권한이 필요합니다",
            ar: "مطلوب إذن الوصول إلى البيانات الصحية",
            he: "נדרשת הרשאה לנתוני בריאות",
            vi: "Cần quyền truy cập dữ liệu sức khỏe",
            hi: "स्वास्थ्य डेटा की अनुमति आवश्यक है",
        },
        UiKey::MobileNotificationsHeading => LocalizedRow {
            en: "Notifications",
            es: "Notificaciones",
            fr: "Notifications",
            pt: "Notificações",
            de: "Benachrichtigungen",
            zh_hans: "通知",
            ja: "通知",
            ko: "알림",
            ar: "الإشعارات",
            he: "התראות",
            vi: "Thông báo",
            hi: "सूचनाएँ",
        },
        UiKey::MobileOfflineBanner => LocalizedRow {
            en: "Offline mode — changes will sync when reconnected.",
            es: "Modo sin conexión: los cambios se sincronizarán al reconectar.",
            fr: "Mode hors ligne — synchronisation dès la reconnexion.",
            pt: "Modo offline — as alterações serão sincronizadas ao reconectar.",
            de: "Offline-Modus — Änderungen werden nach Verbindung synchronisiert.",
            zh_hans: "离线模式——重新连接后将同步更改。",
            ja: "オフラインモード — 再接続時に変更が同期されます。",
            ko: "오프라인 모드 — 다시 연결되면 변경 사항이 동기화됩니다.",
            ar: "وضع عدم الاتصال — ستتم مزامنة التغييرات عند إعادة الاتصال.",
            he: "מצב לא מקוון — השינויים יסונכרנו עם החיבור מחדש.",
            vi: "Chế độ ngoại tuyến — thay đổi sẽ đồng bộ khi kết nối lại.",
            hi: "ऑफ़लाइन मोड — पुनः कनेक्ट होने पर परिवर्तन सिंक हो जाएँगे।",
        },
    };
    row.pick(locale)
}

/// Product target selecting brand strings for [`LocalizedChrome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeProduct {
    Clinic,
    Mobile,
}

/// Single-owner localized UI chrome dictionary emitted by Rust Wasm snapshots
/// so browser shells render localized strings directly from [`translate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedChrome {
    pub app_brand: String,
    pub sign_in: String,
    pub sign_out: String,
    pub send: String,
    pub search: String,
    pub patient_roster: String,
    pub transcript_heading: String,
    pub evidence_heading: String,
    pub lab_records_heading: String,
    pub composer_label: String,
    pub composer_placeholder: String,
    pub language_label: String,
    pub privacy_cover_title: String,
    pub crisis_heading: String,
    pub nav_account: String,
    pub nav_connected_care: String,
    pub nav_for_my_doctor: String,
    pub review_and_send: String,
    pub verify_dob: String,
    pub share: String,
    pub status_ready: String,
    pub offline_banner: String,
}

/// Builds the complete Rust-owned [`LocalizedChrome`] dictionary for `locale` and `product`.
#[must_use]
pub fn localized_chrome(locale: Locale, product: ChromeProduct) -> LocalizedChrome {
    let app_brand_key = match product {
        ChromeProduct::Clinic => UiKey::AppBrandClinic,
        ChromeProduct::Mobile => UiKey::AppBrandMobile,
    };

    let crisis_heading = match locale {
        Locale::En => "Immediate Crisis & Recovery Support (988 / 911 / SAMHSA)",
        Locale::Es => "Apoyo inmediato en crisis y recuperación (988 / 911 / SAMHSA)",
        Locale::Fr => "Aide immédiate en cas de crise et rétablissement (988 / 911 / SAMHSA)",
        Locale::Pt => "Apoio imediato em crise e recuperação (988 / 911 / SAMHSA)",
        Locale::De => "Soforthilfe bei Krisen und Genesung (988 / 911 / SAMHSA)",
        Locale::ZhHans => "即时危机与康复支持（988 / 911 / SAMHSA）",
        Locale::Ja => "緊急危機・回復サポート（988 / 911 / SAMHSA）",
        Locale::Ko => "긴급 위기 및 회복 지원 (988 / 911 / SAMHSA)",
        Locale::Ar => "الدعم الفوري للأزمات والتعافي (988 / 911 / SAMHSA)",
        Locale::He => "תמיכה מיידית בשעת משבר והחלמה (988 / 911 / SAMHSA)",
        Locale::Vi => "Hỗ trợ khủng hoảng và phục hồi khẩn cấp (988 / 911 / SAMHSA)",
        Locale::Hi => "तत्काल संकट और स्वास्थ्य लाभ सहायता (988 / 911 / SAMHSA)",
    };

    let nav_account = match locale {
        Locale::En => "Account",
        Locale::Es => "Cuenta",
        Locale::Fr => "Compte",
        Locale::Pt => "Conta",
        Locale::De => "Konto",
        Locale::ZhHans => "账户",
        Locale::Ja => "アカウント",
        Locale::Ko => "계정",
        Locale::Ar => "الحساب",
        Locale::He => "חשבון",
        Locale::Vi => "Tài khoản",
        Locale::Hi => "खाता",
    };

    let nav_connected_care = match locale {
        Locale::En => "Connected Care",
        Locale::Es => "Atención conectada",
        Locale::Fr => "Soins connectés",
        Locale::Pt => "Cuidado conectado",
        Locale::De => "Vernetzte Versorgung",
        Locale::ZhHans => "互联照护",
        Locale::Ja => "連携ケア",
        Locale::Ko => "연계 진료",
        Locale::Ar => "الرعاية المتصلة",
        Locale::He => "טיפול מקושר",
        Locale::Vi => "Chăm sóc kết nối",
        Locale::Hi => "कनेक्टेड केयर",
    };

    let nav_for_my_doctor = match locale {
        Locale::En => "For my doctor",
        Locale::Es => "Para mi médico",
        Locale::Fr => "Pour mon médecin",
        Locale::Pt => "Para meu médico",
        Locale::De => "Für meine Ärztin / meinen Arzt",
        Locale::ZhHans => "致我的医生",
        Locale::Ja => "主治医へ",
        Locale::Ko => "주치의에게",
        Locale::Ar => "لطبيبي",
        Locale::He => "לרופא שלי",
        Locale::Vi => "Gửi bác sĩ của tôi",
        Locale::Hi => "मेरे डॉक्टर के लिए",
    };

    let review_and_send = match locale {
        Locale::En => "Review & Send",
        Locale::Es => "Revisar y enviar",
        Locale::Fr => "Vérifier et envoyer",
        Locale::Pt => "Revisar e enviar",
        Locale::De => "Prüfen & senden",
        Locale::ZhHans => "审核并发送",
        Locale::Ja => "確認して送信",
        Locale::Ko => "검토 후 보내기",
        Locale::Ar => "مراجعة وإرسال",
        Locale::He => "סקירה ושליחה",
        Locale::Vi => "Xem lại và gửi",
        Locale::Hi => "समीक्षा करें और भेजें",
    };

    let verify_dob = match locale {
        Locale::En => "Verify DOB",
        Locale::Es => "Verificar fecha de nacimiento",
        Locale::Fr => "Vérifier la date de naissance",
        Locale::Pt => "Verificar data de nascimento",
        Locale::De => "Geburtsdatum bestätigen",
        Locale::ZhHans => "验证出生日期",
        Locale::Ja => "生年月日を確認",
        Locale::Ko => "생년월일 확인",
        Locale::Ar => "تأكيد تاريخ الميلاد",
        Locale::He => "אימות תאריך לידה",
        Locale::Vi => "Xác minh ngày sinh",
        Locale::Hi => "जन्म तिथि सत्यापित करें",
    };

    let share = match locale {
        Locale::En => "Share",
        Locale::Es => "Compartir",
        Locale::Fr => "Partager",
        Locale::Pt => "Compartilhar",
        Locale::De => "Teilen",
        Locale::ZhHans => "共享",
        Locale::Ja => "共有",
        Locale::Ko => "공유",
        Locale::Ar => "مشاركة",
        Locale::He => "שיתוף",
        Locale::Vi => "Chia sẻ",
        Locale::Hi => "साझा करें",
    };

    let status_ready = match locale {
        Locale::En => "Ready",
        Locale::Es => "Listo",
        Locale::Fr => "Prêt",
        Locale::Pt => "Pronto",
        Locale::De => "Bereit",
        Locale::ZhHans => "已就绪",
        Locale::Ja => "準備完了",
        Locale::Ko => "준비됨",
        Locale::Ar => "جاهز",
        Locale::He => "מוכן",
        Locale::Vi => "Sẵn sàng",
        Locale::Hi => "तैयार",
    };

    LocalizedChrome {
        app_brand: translate(locale, app_brand_key).to_owned(),
        sign_in: translate(locale, UiKey::ActionSignIn).to_owned(),
        sign_out: translate(locale, UiKey::ActionSignOut).to_owned(),
        send: translate(locale, UiKey::ClinicSendMessage).to_owned(),
        search: translate(locale, UiKey::ActionSearch).to_owned(),
        patient_roster: translate(locale, UiKey::ClinicPatientsHeading).to_owned(),
        transcript_heading: translate(locale, UiKey::ClinicTranscriptHeading).to_owned(),
        evidence_heading: translate(locale, UiKey::ClinicCitedSourcesHeading).to_owned(),
        lab_records_heading: translate(locale, UiKey::ClinicLabRecordsHeading).to_owned(),
        composer_label: translate(locale, UiKey::ClinicComposerPlaceholder).to_owned(),
        composer_placeholder: translate(locale, UiKey::ClinicComposerPlaceholder).to_owned(),
        language_label: translate(locale, UiKey::LabelLanguage).to_owned(),
        privacy_cover_title: translate(locale, UiKey::PrivacyCoverTitle).to_owned(),
        crisis_heading: crisis_heading.to_owned(),
        nav_account: nav_account.to_owned(),
        nav_connected_care: nav_connected_care.to_owned(),
        nav_for_my_doctor: nav_for_my_doctor.to_owned(),
        review_and_send: review_and_send.to_owned(),
        verify_dob: verify_dob.to_owned(),
        share: share.to_owned(),
        status_ready: status_ready.to_owned(),
        offline_banner: translate(locale, UiKey::MobileOfflineBanner).to_owned(),
    }
}
