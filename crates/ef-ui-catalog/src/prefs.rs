//! OS-native persisted desktop preferences (`theme`, `locale`, `zoom`) shared by
//! Curie Clinic Desktop and Curie Mobile Desktop while keeping per-product storage files separate.

use std::fmt;
use std::path::{Path, PathBuf};

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::i18n::{ChromeProduct, Locale};
use crate::theme::{TextScale, ThemeMode};

/// Supported UI text zoom levels persisted across desktop launches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopZoom {
    #[default]
    Pct100,
    Pct150,
    Pct200,
}

impl DesktopZoom {
    /// Parses `100`, `150`, or `200` (with optional `%` suffix).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().trim_end_matches('%') {
            "100" => Some(Self::Pct100),
            "150" => Some(Self::Pct150),
            "200" => Some(Self::Pct200),
            _ => None,
        }
    }

    #[must_use]
    pub const fn from_percent(percent: u16) -> Option<Self> {
        match percent {
            100 => Some(Self::Pct100),
            150 => Some(Self::Pct150),
            200 => Some(Self::Pct200),
            _ => None,
        }
    }

    #[must_use]
    pub const fn percent(self) -> u16 {
        match self {
            Self::Pct100 => 100,
            Self::Pct150 => 150,
            Self::Pct200 => 200,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pct100 => "100%",
            Self::Pct150 => "150%",
            Self::Pct200 => "200%",
        }
    }

    #[must_use]
    pub const fn cycle_next(self) -> Self {
        match self {
            Self::Pct100 => Self::Pct150,
            Self::Pct150 => Self::Pct200,
            Self::Pct200 => Self::Pct100,
        }
    }

    #[must_use]
    pub fn text_scale(self) -> TextScale {
        match self {
            Self::Pct100 => TextScale::STANDARD,
            Self::Pct150 => TextScale::new(1.5).unwrap_or(TextScale::STANDARD),
            Self::Pct200 => TextScale::DOUBLE,
        }
    }
}

/// Typed user preferences persisted in OS-native config storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopPreferences {
    pub theme: ThemeMode,
    pub locale: Locale,
    pub zoom: DesktopZoom,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Dark,
            locale: Locale::En,
            zoom: DesktopZoom::Pct100,
        }
    }
}

/// Target desktop OS family used for deterministic path resolution and testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopOsFamily {
    Linux,
    MacOs,
    Windows,
}

impl DesktopOsFamily {
    #[must_use]
    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::MacOs
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Self::Linux
        }
    }
}

/// Typed failure emitted when reading or writing desktop preferences fails.
#[derive(Debug)]
pub enum DesktopPrefsError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    Serialize {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl fmt::Display for DesktopPrefsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(
                    f,
                    "failed to access desktop preferences '{}': {source}",
                    path.display()
                )
            }
            Self::Parse { path, source } => {
                write!(
                    f,
                    "failed to parse desktop preferences '{}': {source}",
                    path.display()
                )
            }
            Self::Serialize { path, source } => {
                write!(
                    f,
                    "failed to serialize desktop preferences '{}': {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for DesktopPrefsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } | Self::Serialize { source, .. } => Some(source),
        }
    }
}

/// Returns the isolated per-product preference filename.
#[must_use]
pub const fn product_prefs_filename(product: ChromeProduct) -> &'static str {
    match product {
        ChromeProduct::Clinic => "curie-clinic-prefs.json",
        ChromeProduct::Mobile => "curie-mobile-prefs.json",
    }
}

/// Resolves the OS-native preferences directory from explicit environment variables.
///
/// - Linux/Unix: `$XDG_CONFIG_HOME/elysian-fields`, else `$HOME/.config/elysian-fields`
/// - macOS: `$HOME/Library/Application Support/ElysianFields`
/// - Windows: `%APPDATA%\ElysianFields`
#[must_use]
pub fn resolve_prefs_dir_for_env(
    os: DesktopOsFamily,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
    appdata: Option<&str>,
) -> Option<PathBuf> {
    fn non_empty(opt: Option<&str>) -> Option<&str> {
        opt.map(str::trim).filter(|val| !val.is_empty())
    }
    match os {
        DesktopOsFamily::Linux => {
            if let Some(xdg) = non_empty(xdg_config_home) {
                Some(PathBuf::from(xdg).join("elysian-fields"))
            } else {
                non_empty(home).map(|h| PathBuf::from(h).join(".config").join("elysian-fields"))
            }
        }
        DesktopOsFamily::MacOs => non_empty(home).map(|h| {
            PathBuf::from(h)
                .join("Library")
                .join("Application Support")
                .join("ElysianFields")
        }),
        DesktopOsFamily::Windows => {
            non_empty(appdata).map(|a| PathBuf::from(a).join("ElysianFields"))
        }
    }
}

/// Resolves the active host OS preferences file path for `product`.
#[must_use]
pub fn resolve_desktop_prefs_path(product: ChromeProduct) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let xdg = std::env::var("XDG_CONFIG_HOME").ok();
        let home = std::env::var("HOME").ok();
        let appdata = std::env::var("APPDATA").ok();
        resolve_prefs_dir_for_env(
            DesktopOsFamily::current(),
            xdg.as_deref(),
            home.as_deref(),
            appdata.as_deref(),
        )
        .map(|dir| dir.join(product_prefs_filename(product)))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = product;
        None
    }
}

/// Persistence handle for desktop preferences, inserted as a Bevy [`Resource`].
///
/// Automatically constructed as [`Self::disabled`] under `--headless-smoke`, `--screenshot`,
/// or `--no-prefs` so automated verification never reads or mutates host user settings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Resource)]
pub struct DesktopPrefsStore {
    path: Option<PathBuf>,
}

impl DesktopPrefsStore {
    /// Creates an enabled store targeting the active OS config location for `product`,
    /// or a disabled store if no user config directory is configured in the environment.
    #[must_use]
    pub fn for_product(product: ChromeProduct) -> Self {
        Self {
            path: resolve_desktop_prefs_path(product),
        }
    }

    /// Creates an enabled store targeting an explicit file path (e.g. temp directory in tests).
    #[must_use]
    pub const fn at_path(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    /// Creates a no-op store that neither reads nor writes disk state.
    #[must_use]
    pub const fn disabled() -> Self {
        Self { path: None }
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    /// Loads persisted preferences from disk. Returns `Ok(None)` when disabled or when the file
    /// does not exist yet; returns `Err` on unreadable or malformed preference files.
    pub fn load(&self) -> Result<Option<DesktopPreferences>, DesktopPrefsError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(path) = self.path.as_deref() else {
                return Ok(None);
            };
            match std::fs::read_to_string(path) {
                Ok(raw) => {
                    let prefs =
                        serde_json::from_str::<DesktopPreferences>(&raw).map_err(|source| {
                            DesktopPrefsError::Parse {
                                path: path.to_path_buf(),
                                source,
                            }
                        })?;
                    Ok(Some(prefs))
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(source) => Err(DesktopPrefsError::Io {
                    path: path.to_path_buf(),
                    source,
                }),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(None)
        }
    }

    /// Persists `prefs` as formatted JSON, creating parent directories if needed.
    /// Does nothing when the store is disabled.
    pub fn save(&self, prefs: &DesktopPreferences) -> Result<(), DesktopPrefsError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(path) = self.path.as_deref() else {
                return Ok(());
            };
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent).map_err(|source| DesktopPrefsError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            let payload = serde_json::to_string_pretty(prefs).map_err(|source| {
                DesktopPrefsError::Serialize {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
            std::fs::write(path, payload).map_err(|source| DesktopPrefsError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            Ok(())
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = prefs;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_os_native_paths_for_linux_macos_and_windows() {
        let xdg = resolve_prefs_dir_for_env(
            DesktopOsFamily::Linux,
            Some("/tmp/xdg-cfg"),
            Some("/home/user"),
            None,
        )
        .expect("linux xdg");
        assert_eq!(xdg, PathBuf::from("/tmp/xdg-cfg/elysian-fields"));

        let linux_home =
            resolve_prefs_dir_for_env(DesktopOsFamily::Linux, Some("  "), Some("/home/user"), None)
                .expect("linux home fallback");
        assert_eq!(
            linux_home,
            PathBuf::from("/home/user/.config/elysian-fields")
        );

        let mac =
            resolve_prefs_dir_for_env(DesktopOsFamily::MacOs, None, Some("/Users/clinician"), None)
                .expect("macos path");
        assert_eq!(
            mac,
            PathBuf::from("/Users/clinician/Library/Application Support/ElysianFields")
        );

        let win = resolve_prefs_dir_for_env(
            DesktopOsFamily::Windows,
            None,
            None,
            Some(r"C:\Users\clinician\AppData\Roaming"),
        )
        .expect("windows path");
        assert_eq!(
            win,
            PathBuf::from(r"C:\Users\clinician\AppData\Roaming").join("ElysianFields")
        );

        assert_ne!(
            product_prefs_filename(ChromeProduct::Clinic),
            product_prefs_filename(ChromeProduct::Mobile)
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn prefs_store_roundtrips_in_temp_dir_and_rejects_corrupt_json() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "ef-ui-catalog-prefs-{}-{nanos}",
            std::process::id()
        ));
        let path = dir.join(product_prefs_filename(ChromeProduct::Clinic));
        let store = DesktopPrefsStore::at_path(path.clone());

        assert_eq!(store.load().expect("missing file ok"), None);

        let saved = DesktopPreferences {
            theme: ThemeMode::Light,
            locale: Locale::He,
            zoom: DesktopZoom::Pct200,
        };
        store.save(&saved).expect("save prefs");
        assert_eq!(store.load().expect("load prefs"), Some(saved));

        // Corrupt JSON must surface a typed Parse error instead of silently returning empty.
        std::fs::write(&path, "{not valid json").expect("write corrupt");
        assert!(matches!(store.load(), Err(DesktopPrefsError::Parse { .. })));

        // Disabled store is a strict no-op.
        let disabled = DesktopPrefsStore::disabled();
        assert!(!disabled.is_enabled());
        assert_eq!(disabled.load().expect("disabled load"), None);
        disabled.save(&saved).expect("disabled save");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
