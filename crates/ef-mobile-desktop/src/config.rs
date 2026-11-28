//! Typed CLI configuration for `ef-mobile-desktop`.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;

use ef_ui_catalog::{Locale, ThemeMode};
use thiserror::Error;

/// Canonical mobile-proportioned desktop window dimensions.
pub const MOBILE_WINDOW_WIDTH: f32 = 420.0;
pub const MOBILE_WINDOW_HEIGHT: f32 = 860.0;
pub const MOBILE_WINDOW_TITLE: &str = "Elysian Fields — Curie Mobile";
pub const DEFAULT_DEMO_EMAIL: &str = "patient.demo@example.invalid";

/// Backend connectivity mode selected at launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMode {
    /// Deterministic local in-process transport pre-seeded with a synthetic demo invitation.
    Demo,
    /// Live TCP HTTP/1.1 connection to `ef-mobile-server` via `LiveHttpMobileTransport`.
    LiveServer { addr: SocketAddr, raw_url: String },
}

/// Execution mode for the desktop binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunTarget {
    /// Native GPU window (`WinitSettings::desktop_app()`, `420x860`).
    Windowed { screenshot_path: Option<PathBuf> },
    /// Headless Bevy verification runner printing JSON summary and exiting 0.
    HeadlessSmoke,
    /// Print CLI help and exit 0.
    Help,
}

/// Complete validated launch configuration for `ef-mobile-desktop`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileDesktopConfig {
    pub mode: LaunchMode,
    pub target: RunTarget,
    pub locale: Locale,
    pub theme: ThemeMode,
    pub email: String,
}

impl Default for MobileDesktopConfig {
    fn default() -> Self {
        Self {
            mode: LaunchMode::Demo,
            target: RunTarget::Windowed {
                screenshot_path: None,
            },
            locale: Locale::En,
            theme: ThemeMode::Dark,
            email: DEFAULT_DEMO_EMAIL.to_owned(),
        }
    }
}

/// Typed CLI parsing error preserving the exact rejected flag or value.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    #[error("missing value for CLI flag '{flag}'")]
    MissingValue { flag: String },
    #[error("unknown CLI argument '{arg}' (use --help for usage)")]
    UnknownArgument { arg: String },
    #[error(
        "invalid locale '{value}' (expected BCP-47 tag such as en, he, ar, zh-Hans, ja, vi, hi)"
    )]
    InvalidLocale { value: String },
    #[error("invalid theme '{value}' (expected 'dark' or 'light')")]
    InvalidTheme { value: String },
    #[error("invalid server address '{value}': {detail}")]
    InvalidServerAddress { value: String, detail: String },
    #[error("invalid email address '{value}'")]
    InvalidEmail { value: String },
    #[error("cannot combine --demo with --server")]
    ConflictingLaunchModes,
}

impl MobileDesktopConfig {
    /// Parses typed CLI arguments (excluding argv[0]).
    pub fn parse_args<I, S>(args: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut config = Self::default();
        let mut saw_demo = false;
        let mut saw_server = false;
        let mut headless_smoke = false;
        let mut screenshot_path: Option<PathBuf> = None;

        let mut iter = args.into_iter().map(Into::into);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-h" | "--help" => {
                    config.target = RunTarget::Help;
                    return Ok(config);
                }
                "--demo" => {
                    saw_demo = true;
                    config.mode = LaunchMode::Demo;
                }
                "--headless-smoke" => {
                    headless_smoke = true;
                }
                "--server" => {
                    let raw = next_value(&mut iter, "--server")?;
                    let addr = parse_server_addr(&raw)?;
                    saw_server = true;
                    config.mode = LaunchMode::LiveServer { addr, raw_url: raw };
                }
                "--email" => {
                    let email = next_value(&mut iter, "--email")?;
                    if email.trim().is_empty() || !email.contains('@') {
                        return Err(ConfigError::InvalidEmail { value: email });
                    }
                    config.email = email.trim().to_owned();
                }
                "--locale" => {
                    let raw = next_value(&mut iter, "--locale")?;
                    let locale = Locale::parse_bcp47(&raw)
                        .ok_or(ConfigError::InvalidLocale { value: raw })?;
                    config.locale = locale;
                }
                "--theme" => {
                    let raw = next_value(&mut iter, "--theme")?;
                    config.theme = match raw.trim().to_ascii_lowercase().as_str() {
                        "dark" => ThemeMode::Dark,
                        "light" => ThemeMode::Light,
                        _ => return Err(ConfigError::InvalidTheme { value: raw }),
                    };
                }
                "--screenshot" => {
                    let raw = next_value(&mut iter, "--screenshot")?;
                    screenshot_path = Some(PathBuf::from(raw));
                }
                _ => {
                    if let Some(val) = arg.strip_prefix("--server=") {
                        let addr = parse_server_addr(val)?;
                        saw_server = true;
                        config.mode = LaunchMode::LiveServer {
                            addr,
                            raw_url: val.to_owned(),
                        };
                    } else if let Some(val) = arg.strip_prefix("--email=") {
                        if val.trim().is_empty() || !val.contains('@') {
                            return Err(ConfigError::InvalidEmail {
                                value: val.to_owned(),
                            });
                        }
                        config.email = val.trim().to_owned();
                    } else if let Some(val) = arg.strip_prefix("--locale=") {
                        let locale =
                            Locale::parse_bcp47(val).ok_or_else(|| ConfigError::InvalidLocale {
                                value: val.to_owned(),
                            })?;
                        config.locale = locale;
                    } else if let Some(val) = arg.strip_prefix("--theme=") {
                        config.theme = match val.trim().to_ascii_lowercase().as_str() {
                            "dark" => ThemeMode::Dark,
                            "light" => ThemeMode::Light,
                            _ => {
                                return Err(ConfigError::InvalidTheme {
                                    value: val.to_owned(),
                                });
                            }
                        };
                    } else if let Some(val) = arg.strip_prefix("--screenshot=") {
                        screenshot_path = Some(PathBuf::from(val));
                    } else {
                        return Err(ConfigError::UnknownArgument { arg });
                    }
                }
            }
        }

        if saw_demo && saw_server {
            return Err(ConfigError::ConflictingLaunchModes);
        }

        config.target = if headless_smoke {
            RunTarget::HeadlessSmoke
        } else {
            RunTarget::Windowed { screenshot_path }
        };

        Ok(config)
    }

    /// Human-readable CLI help text.
    #[must_use]
    pub const fn usage() -> &'static str {
        "Usage: ef-mobile-desktop [OPTIONS]\n\
         \n\
         Standalone, product-isolated native Bevy 0.20.1 Curie Mobile desktop app (420x860).\n\
         \n\
         Options:\n\
           --demo                      Run with synthetic in-process demo transport (default)\n\
           --server <URL|HOST:PORT>    Connect live to ef-mobile-server (e.g. http://127.0.0.1:8089)\n\
           --email <ADDRESS>           Patient email for standalone account sign-in\n\
           --locale <BCP47>            Initial locale (en, es, fr, pt, de, zh-Hans, ja, ko, ar, he, vi, hi)\n\
           --theme <dark|light>        Initial WCAG 2.1 AA theme (default: dark)\n\
           --headless-smoke            Execute headless Bevy verification suite and emit JSON\n\
           --screenshot <PATH.PNG>     Capture primary window PNG after initial render and exit\n\
           -h, --help                  Print this help message"
    }
}

fn next_value(iter: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, ConfigError> {
    iter.next().ok_or_else(|| ConfigError::MissingValue {
        flag: flag.to_owned(),
    })
}

/// Parses `http://127.0.0.1:8089`, `https://...`, or `127.0.0.1:8089` into a [`SocketAddr`].
pub fn parse_server_addr(raw: &str) -> Result<SocketAddr, ConfigError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::InvalidServerAddress {
            value: raw.to_owned(),
            detail: "address is empty".to_owned(),
        });
    }
    let without_scheme = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    let authority = without_scheme.split('/').next().unwrap_or("").trim();
    if authority.is_empty() {
        return Err(ConfigError::InvalidServerAddress {
            value: raw.to_owned(),
            detail: "missing host:port".to_owned(),
        });
    }
    if let Ok(addr) = authority.parse::<SocketAddr>() {
        return Ok(addr);
    }
    authority
        .to_socket_addrs()
        .map_err(|err| ConfigError::InvalidServerAddress {
            value: raw.to_owned(),
            detail: err.to_string(),
        })?
        .next()
        .ok_or_else(|| ConfigError::InvalidServerAddress {
            value: raw.to_owned(),
            detail: "could not resolve socket address".to_owned(),
        })
}
