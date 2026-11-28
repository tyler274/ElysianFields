//! Typed CLI configuration for the native Curie Clinic desktop application.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;

use ef_ui_catalog::{DesktopPreferences, DesktopZoom, Locale, TextScale, ThemeMode};
use thiserror::Error;

use crate::workflows::ClinicWorkspaceTab;

/// Validated UI zoom factor (`100%`, `150%`, `200%`) mapped directly onto [`TextScale`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ZoomLevel {
    #[default]
    Pct100,
    Pct150,
    Pct200,
}

impl ZoomLevel {
    /// Parses `100`, `150`, or `200` (with optional `%` suffix).
    pub fn parse(raw: &str) -> Result<Self, DesktopConfigError> {
        let cleaned = raw.trim().trim_end_matches('%');
        match cleaned {
            "100" => Ok(Self::Pct100),
            "150" => Ok(Self::Pct150),
            "200" => Ok(Self::Pct200),
            other => Err(DesktopConfigError::InvalidZoom {
                value: other.to_owned(),
            }),
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
    pub fn text_scale(self) -> TextScale {
        match self {
            Self::Pct100 => TextScale::STANDARD,
            Self::Pct150 => TextScale::new(1.5).unwrap_or(TextScale::STANDARD),
            Self::Pct200 => TextScale::DOUBLE,
        }
    }

    #[must_use]
    pub const fn to_desktop_zoom(self) -> DesktopZoom {
        match self {
            Self::Pct100 => DesktopZoom::Pct100,
            Self::Pct150 => DesktopZoom::Pct150,
            Self::Pct200 => DesktopZoom::Pct200,
        }
    }

    #[must_use]
    pub const fn from_desktop_zoom(zoom: DesktopZoom) -> Self {
        match zoom {
            DesktopZoom::Pct100 => Self::Pct100,
            DesktopZoom::Pct150 => Self::Pct150,
            DesktopZoom::Pct200 => Self::Pct200,
        }
    }
}

/// Whether OS-native persisted preferences (`theme`, `locale`, `zoom`) are read and written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreferenceMode {
    #[default]
    PersistOsConfig,
    Disabled,
}

/// Explicit CLI overrides that take precedence over saved OS-native preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DesktopPrefOverrides {
    pub locale: Option<Locale>,
    pub theme: Option<ThemeMode>,
    pub zoom: Option<ZoomLevel>,
}

/// Authoritative backend transport selected at desktop startup.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LaunchBackend {
    /// Local synthetic vertical-slice demo backend (zero setup required).
    #[default]
    Demo,
    /// Live `ef-clinic-server` over HTTP/1.1 + SSE under `/elysianfields/api/...`.
    Server {
        base_url: String,
        socket_addr: SocketAddr,
        email: Option<String>,
        token: Option<String>,
    },
}

/// Execution mode for the desktop binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Interactive GPU window (`WinitSettings::desktop_app()`), with optional PNG capture.
    Windowed { screenshot_path: Option<PathBuf> },
    /// Deterministic headless smoke verification printing structured JSON and exiting `0`.
    HeadlessSmoke,
}

impl Default for ExecutionMode {
    fn default() -> Self {
        Self::Windowed {
            screenshot_path: None,
        }
    }
}

/// Validated launch configuration for Curie Clinic Desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopLaunchConfig {
    pub backend: LaunchBackend,
    pub mode: ExecutionMode,
    pub locale: Locale,
    pub theme: ThemeMode,
    pub zoom: ZoomLevel,
    pub prefs_mode: PreferenceMode,
    pub cli_overrides: DesktopPrefOverrides,
    pub search_query: Option<String>,
    pub streaming_demo: bool,
    pub initial_view: ClinicWorkspaceTab,
}

impl Default for DesktopLaunchConfig {
    fn default() -> Self {
        Self {
            backend: LaunchBackend::Demo,
            mode: ExecutionMode::default(),
            locale: Locale::En,
            theme: ThemeMode::Dark,
            zoom: ZoomLevel::Pct100,
            prefs_mode: PreferenceMode::PersistOsConfig,
            cli_overrides: DesktopPrefOverrides::default(),
            search_query: None,
            streaming_demo: false,
            initial_view: ClinicWorkspaceTab::Chat,
        }
    }
}

/// Typed CLI parsing error preserving exact causes and rejected values.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DesktopConfigError {
    #[error("{0}")]
    HelpRequested(String),
    #[error("unknown CLI flag '{flag}'")]
    UnknownFlag { flag: String },
    #[error("CLI flag '{flag}' requires a value")]
    MissingValue { flag: String },
    #[error("unsupported BCP-47 locale '{value}'")]
    InvalidLocale { value: String },
    #[error("invalid theme '{value}' (expected 'dark' or 'light')")]
    InvalidTheme { value: String },
    #[error("invalid zoom '{value}' (expected 100, 150, or 200)")]
    InvalidZoom { value: String },
    #[error("invalid view '{value}' (expected chat, today, messages, or memory)")]
    InvalidView { value: String },
    #[error("invalid search query (must not be empty)")]
    InvalidSearchQuery,
    #[error("invalid server URL '{url}': {reason}")]
    InvalidServerUrl { url: String, reason: String },
    #[error("cannot combine --demo with --server")]
    ConflictingBackendFlags,
}

impl DesktopLaunchConfig {
    /// Canonical CLI usage text returned for `--help`.
    #[must_use]
    pub fn usage_text() -> &'static str {
        "Elysian Fields — Curie Clinic Desktop (Bevy 0.20.1)\n\
         \n\
         USAGE:\n\
           ef-clinic-desktop [OPTIONS]\n\
         \n\
         OPTIONS:\n\
           --demo                      Boot with synthetic demo workspaces (default)\n\
           --server <URL>              Connect to live ef-clinic-server (e.g. http://127.0.0.1:8088)\n\
           --email <ADDR>              Clinician email for --server sign-in\n\
           --token <TOKEN>             Verification token for --server sign-in\n\
           --locale <BCP47>            Initial UI locale (en, he, ar, zh-Hans, ja, vi, hi, es, fr, pt, de, ko)\n\
           --theme <dark|light>        Initial WCAG 2.1 AA theme (default: dark)\n\
           --zoom <100|150|200>        Initial UI text zoom percent (default: 100)\n\
           --search <QUERY>            Pre-open Patient Roster Search with <QUERY> focused\n\
           --streaming-demo            Pre-seed progressive synthetic streaming draft and Cancel button\n\
           --view <VIEW>               Initial workstation tab (chat, today, messages, memory; default: chat)\n\
           --headless-smoke            Run deterministic headless UI smoke suite and print JSON\n\
           --screenshot <PATH.PNG>     Capture rendered GPU window PNG after layout settles and exit\n\
           --no-prefs                  Bypass reading and writing OS-native saved preferences\n\
           -h, --help                  Print this help message"
    }

    /// Returns true only for interactive windowed runs when `--no-prefs` was not passed.
    #[must_use]
    pub const fn should_use_prefs(&self) -> bool {
        matches!(self.prefs_mode, PreferenceMode::PersistOsConfig)
            && matches!(
                self.mode,
                ExecutionMode::Windowed {
                    screenshot_path: None
                }
            )
    }

    /// Applies saved OS-native preferences for any field not explicitly overridden on the CLI.
    pub fn apply_saved_preferences(&mut self, saved: DesktopPreferences) {
        if self.cli_overrides.locale.is_none() {
            self.locale = saved.locale;
        }
        if self.cli_overrides.theme.is_none() {
            self.theme = saved.theme;
        }
        if self.cli_overrides.zoom.is_none() {
            self.zoom = ZoomLevel::from_desktop_zoom(saved.zoom);
        }
    }

    /// Parses CLI arguments (excluding argv[0]) into a validated [`DesktopLaunchConfig`].
    pub fn parse_args<I, S>(args: I) -> Result<Self, DesktopConfigError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut iter = args.into_iter().map(Into::into);
        let mut demo_explicit = false;
        let mut server_url: Option<String> = None;
        let mut email: Option<String> = None;
        let mut token: Option<String> = None;
        let mut locale = Locale::En;
        let mut theme = ThemeMode::Dark;
        let mut zoom = ZoomLevel::Pct100;
        let mut headless_smoke = false;
        let mut screenshot_path: Option<PathBuf> = None;
        let mut prefs_mode = PreferenceMode::PersistOsConfig;
        let mut cli_overrides = DesktopPrefOverrides::default();
        let mut search_query: Option<String> = None;
        let mut streaming_demo = false;
        let mut initial_view = ClinicWorkspaceTab::Chat;

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-h" | "--help" => {
                    return Err(DesktopConfigError::HelpRequested(
                        Self::usage_text().to_owned(),
                    ));
                }
                "--demo" => {
                    demo_explicit = true;
                }
                "--headless-smoke" => {
                    headless_smoke = true;
                }
                "--no-prefs" => {
                    prefs_mode = PreferenceMode::Disabled;
                }
                "--streaming-demo" => {
                    streaming_demo = true;
                }
                "--search" => {
                    let val = iter
                        .next()
                        .ok_or_else(|| DesktopConfigError::MissingValue {
                            flag: "--search".to_owned(),
                        })?;
                    let trimmed = val.trim();
                    if trimmed.is_empty() {
                        return Err(DesktopConfigError::InvalidSearchQuery);
                    }
                    search_query = Some(trimmed.to_owned());
                }
                "--server" => {
                    let val = iter
                        .next()
                        .ok_or_else(|| DesktopConfigError::MissingValue {
                            flag: "--server".to_owned(),
                        })?;
                    server_url = Some(val);
                }
                "--email" => {
                    let val = iter
                        .next()
                        .ok_or_else(|| DesktopConfigError::MissingValue {
                            flag: "--email".to_owned(),
                        })?;
                    email = Some(val);
                }
                "--token" => {
                    let val = iter
                        .next()
                        .ok_or_else(|| DesktopConfigError::MissingValue {
                            flag: "--token".to_owned(),
                        })?;
                    token = Some(val);
                }
                "--locale" => {
                    let val = iter
                        .next()
                        .ok_or_else(|| DesktopConfigError::MissingValue {
                            flag: "--locale".to_owned(),
                        })?;
                    locale = Locale::parse_bcp47(&val)
                        .ok_or(DesktopConfigError::InvalidLocale { value: val })?;
                    cli_overrides.locale = Some(locale);
                }
                "--theme" => {
                    let val = iter
                        .next()
                        .ok_or_else(|| DesktopConfigError::MissingValue {
                            flag: "--theme".to_owned(),
                        })?;
                    theme = match val.trim().to_ascii_lowercase().as_str() {
                        "dark" => ThemeMode::Dark,
                        "light" => ThemeMode::Light,
                        _ => return Err(DesktopConfigError::InvalidTheme { value: val }),
                    };
                    cli_overrides.theme = Some(theme);
                }
                "--zoom" => {
                    let val = iter
                        .next()
                        .ok_or_else(|| DesktopConfigError::MissingValue {
                            flag: "--zoom".to_owned(),
                        })?;
                    zoom = ZoomLevel::parse(&val)?;
                    cli_overrides.zoom = Some(zoom);
                }
                "--screenshot" => {
                    let val = iter
                        .next()
                        .ok_or_else(|| DesktopConfigError::MissingValue {
                            flag: "--screenshot".to_owned(),
                        })?;
                    screenshot_path = Some(PathBuf::from(val));
                }
                "--view" => {
                    let val = iter
                        .next()
                        .ok_or_else(|| DesktopConfigError::MissingValue {
                            flag: "--view".to_owned(),
                        })?;
                    initial_view = ClinicWorkspaceTab::parse_cli(&val)
                        .ok_or(DesktopConfigError::InvalidView { value: val })?;
                }
                other => {
                    if let Some(val) = other.strip_prefix("--search=") {
                        let trimmed = val.trim();
                        if trimmed.is_empty() {
                            return Err(DesktopConfigError::InvalidSearchQuery);
                        }
                        search_query = Some(trimmed.to_owned());
                    } else if let Some(val) = other.strip_prefix("--view=") {
                        initial_view = ClinicWorkspaceTab::parse_cli(val).ok_or_else(|| {
                            DesktopConfigError::InvalidView {
                                value: val.to_owned(),
                            }
                        })?;
                    } else {
                        return Err(DesktopConfigError::UnknownFlag {
                            flag: other.to_owned(),
                        });
                    }
                }
            }
        }

        if demo_explicit && server_url.is_some() {
            return Err(DesktopConfigError::ConflictingBackendFlags);
        }

        let backend = match server_url {
            None => LaunchBackend::Demo,
            Some(url) => {
                let socket_addr = parse_server_socket_addr(&url)?;
                LaunchBackend::Server {
                    base_url: url,
                    socket_addr,
                    email,
                    token,
                }
            }
        };

        let mode = if headless_smoke {
            ExecutionMode::HeadlessSmoke
        } else {
            ExecutionMode::Windowed { screenshot_path }
        };

        if !matches!(
            mode,
            ExecutionMode::Windowed {
                screenshot_path: None
            }
        ) {
            prefs_mode = PreferenceMode::Disabled;
        }

        Ok(Self {
            backend,
            mode,
            locale,
            theme,
            zoom,
            prefs_mode,
            cli_overrides,
            search_query,
            streaming_demo,
            initial_view,
        })
    }
}

/// Parses an HTTP server origin (`http://127.0.0.1:8088` or `127.0.0.1:8088`) into a [`SocketAddr`].
pub fn parse_server_socket_addr(raw: &str) -> Result<SocketAddr, DesktopConfigError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(DesktopConfigError::InvalidServerUrl {
            url: raw.to_owned(),
            reason: "URL must not be empty".to_owned(),
        });
    }
    let without_scheme = if let Some(rest) = trimmed.strip_prefix("http://") {
        rest
    } else if trimmed.starts_with("https://") {
        return Err(DesktopConfigError::InvalidServerUrl {
            url: raw.to_owned(),
            reason: "live local TCP client supports http:// endpoints".to_owned(),
        });
    } else {
        trimmed
    };

    let authority = without_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    if authority.is_empty() {
        return Err(DesktopConfigError::InvalidServerUrl {
            url: raw.to_owned(),
            reason: "missing host:port authority".to_owned(),
        });
    }

    if let Ok(addr) = authority.parse::<SocketAddr>() {
        return Ok(addr);
    }

    let host_port = if authority.contains(':') {
        authority.to_owned()
    } else {
        format!("{authority}:80")
    };

    host_port
        .to_socket_addrs()
        .map_err(|err| DesktopConfigError::InvalidServerUrl {
            url: raw.to_owned(),
            reason: format!("could not resolve '{host_port}': {err}"),
        })?
        .next()
        .ok_or_else(|| DesktopConfigError::InvalidServerUrl {
            url: raw.to_owned(),
            reason: format!("no socket address resolved for '{host_port}'"),
        })
}
