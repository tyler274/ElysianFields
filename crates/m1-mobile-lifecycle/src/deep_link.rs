//! Deep-link URI parser and router for Android App Links, iOS Universal Links,
//! and the `curie://` custom application scheme.
//!
//! Enforces the `/elysianfields` subpath requirement for HTTPS links, rejects
//! untrusted schemes or hosts, and keeps sign-in tokens redacted in `Debug`
//! output.

use std::fmt;

use crate::state_restore::{LaunchMode, RecordId, RecordIdError, ScreenRoute, SignInOrigin};

const ALLOWED_CUSTOM_SCHEME: &str = "curie";
const ALLOWED_HTTPS_SCHEME: &str = "https";
const ALLOWED_HTTPS_HOST: &str = "dabney.moe";
const REQUIRED_HTTPS_SUBPATH_PREFIX: &str = "/elysianfields/";

const MIN_TOKEN_LEN: usize = 16;
const MAX_TOKEN_LEN: usize = 256;

/// Validated sign-in exchange token parsed from a trusted deep link.
///
/// Always prints `[REDACTED]` in [`fmt::Debug`] so tokens never leak into logs.
#[derive(Clone, PartialEq, Eq)]
pub struct SignInToken(String);

impl fmt::Debug for SignInToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SignInToken([REDACTED])")
    }
}

/// Validation failure for a [`SignInToken`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenValidationError {
    Empty,
    TooShort { len: usize, min: usize },
    TooLong { len: usize, max: usize },
    InvalidCharacter { byte: u8 },
}

impl fmt::Display for TokenValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "sign-in token must not be empty"),
            Self::TooShort { len, min } => {
                write!(f, "sign-in token length {len} is below minimum {min}")
            }
            Self::TooLong { len, max } => {
                write!(f, "sign-in token length {len} exceeds maximum {max}")
            }
            Self::InvalidCharacter { byte } => {
                write!(f, "sign-in token contains invalid byte 0x{byte:02x}")
            }
        }
    }
}

impl std::error::Error for TokenValidationError {}

impl SignInToken {
    /// Parses and validates a sign-in token (`[A-Za-z0-9._-]`, 16..=256 bytes).
    pub fn parse(raw: &str) -> Result<Self, TokenValidationError> {
        if raw.is_empty() {
            return Err(TokenValidationError::Empty);
        }
        if raw.len() < MIN_TOKEN_LEN {
            return Err(TokenValidationError::TooShort {
                len: raw.len(),
                min: MIN_TOKEN_LEN,
            });
        }
        if raw.len() > MAX_TOKEN_LEN {
            return Err(TokenValidationError::TooLong {
                len: raw.len(),
                max: MAX_TOKEN_LEN,
            });
        }
        for &byte in raw.as_bytes() {
            let valid =
                byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte == b'.';
            if !valid {
                return Err(TokenValidationError::InvalidCharacter { byte });
            }
        }
        Ok(Self(raw.to_owned()))
    }

    /// Exposes the raw token strictly for secure-storage or auth-exchange boundaries.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

/// Typed deep-link routes accepted by the mobile client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkRoute {
    SignIn { token: SignInToken },
    Record { record_id: RecordId },
    Vitals,
}

/// Incoming deep-link intent paired with whether the app launched cold or warm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepLinkIntent {
    pub route: DeepLinkRoute,
    pub launch_mode: LaunchMode,
}

/// Routed screen transition derived from a validated [`DeepLinkIntent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkResolution {
    SignIn {
        screen: ScreenRoute,
        token: SignInToken,
    },
    Record {
        screen: ScreenRoute,
        record_id: RecordId,
    },
    Vitals {
        screen: ScreenRoute,
    },
}

/// Typed rejection reasons when parsing an untrusted or malformed deep-link URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkError {
    EmptyUri,
    MissingScheme,
    UntrustedScheme { scheme: String },
    UntrustedHost { host: String },
    InvalidSubpath { path: String },
    PathTraversalNotAllowed,
    UnknownRoute { route: String },
    MissingQueryParameter { name: &'static str },
    DuplicateQueryParameter { name: &'static str },
    InvalidToken(TokenValidationError),
    InvalidRecordId(RecordIdError),
}

impl fmt::Display for DeepLinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyUri => write!(f, "deep-link URI is empty"),
            Self::MissingScheme => write!(f, "deep-link URI is missing a scheme delimiter"),
            Self::UntrustedScheme { scheme } => {
                write!(f, "deep-link scheme `{scheme}` is not trusted")
            }
            Self::UntrustedHost { host } => {
                write!(f, "deep-link host `{host}` is not trusted")
            }
            Self::InvalidSubpath { path } => write!(
                f,
                "HTTPS deep-link path `{path}` is outside `{REQUIRED_HTTPS_SUBPATH_PREFIX}`"
            ),
            Self::PathTraversalNotAllowed => {
                write!(f, "deep-link path must not contain traversal segments")
            }
            Self::UnknownRoute { route } => write!(f, "unknown deep-link route `{route}`"),
            Self::MissingQueryParameter { name } => {
                write!(f, "deep-link is missing required query parameter `{name}`")
            }
            Self::DuplicateQueryParameter { name } => {
                write!(f, "deep-link contains duplicate query parameter `{name}`")
            }
            Self::InvalidToken(err) => write!(f, "invalid deep-link sign-in token: {err}"),
            Self::InvalidRecordId(err) => write!(f, "invalid deep-link record ID: {err}"),
        }
    }
}

impl std::error::Error for DeepLinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidToken(err) => Some(err),
            Self::InvalidRecordId(err) => Some(err),
            Self::EmptyUri
            | Self::MissingScheme
            | Self::UntrustedScheme { .. }
            | Self::UntrustedHost { .. }
            | Self::InvalidSubpath { .. }
            | Self::PathTraversalNotAllowed
            | Self::UnknownRoute { .. }
            | Self::MissingQueryParameter { .. }
            | Self::DuplicateQueryParameter { .. } => None,
        }
    }
}

impl DeepLinkRoute {
    /// Parses and validates a deep-link URI from Android App Links, iOS Universal
    /// Links, or the `curie://` custom scheme.
    ///
    /// Accepted forms:
    /// - `curie://sign-in?token=<token>`
    /// - `curie://record?id=<record_id>`
    /// - `curie://vitals`
    /// - `https://dabney.moe/elysianfields/sign-in?token=<token>`
    /// - `https://dabney.moe/elysianfields/record?id=<record_id>`
    /// - `https://dabney.moe/elysianfields/vitals`
    pub fn parse(raw_uri: &str) -> Result<Self, DeepLinkError> {
        let trimmed = raw_uri.trim();
        if trimmed.is_empty() {
            return Err(DeepLinkError::EmptyUri);
        }

        // Strip any URL fragment before inspecting path and query.
        let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);

        let Some((scheme_raw, after_scheme)) = without_fragment.split_once("://") else {
            return Err(DeepLinkError::MissingScheme);
        };
        let scheme = scheme_raw.to_ascii_lowercase();

        let (route_segment, query) = match scheme.as_str() {
            ALLOWED_CUSTOM_SCHEME => split_custom_scheme_route(after_scheme)?,
            ALLOWED_HTTPS_SCHEME => split_https_route(after_scheme)?,
            _ => return Err(DeepLinkError::UntrustedScheme { scheme }),
        };

        if route_segment.contains("..") {
            return Err(DeepLinkError::PathTraversalNotAllowed);
        }

        match route_segment {
            "sign-in" => {
                let raw_token = extract_unique_query_param(query, "token")?;
                let token = SignInToken::parse(raw_token).map_err(DeepLinkError::InvalidToken)?;
                Ok(Self::SignIn { token })
            }
            "record" => {
                let raw_id = extract_unique_query_param(query, "id")?;
                let record_id = RecordId::parse(raw_id).map_err(DeepLinkError::InvalidRecordId)?;
                Ok(Self::Record { record_id })
            }
            "vitals" => Ok(Self::Vitals),
            unknown => Err(DeepLinkError::UnknownRoute {
                route: unknown.to_owned(),
            }),
        }
    }

    /// Maps a validated deep-link route + launch mode to a screen transition.
    #[must_use]
    pub fn resolve(self, launch_mode: LaunchMode) -> DeepLinkResolution {
        match self {
            Self::SignIn { token } => DeepLinkResolution::SignIn {
                screen: ScreenRoute::SignIn {
                    origin: SignInOrigin::DeepLink { launch_mode },
                },
                token,
            },
            Self::Record { record_id } => DeepLinkResolution::Record {
                screen: ScreenRoute::RecordDetail {
                    record_id: record_id.clone(),
                },
                record_id,
            },
            Self::Vitals => DeepLinkResolution::Vitals {
                screen: ScreenRoute::VitalsOverview,
            },
        }
    }
}

fn split_custom_scheme_route(after_scheme: &str) -> Result<(&str, &str), DeepLinkError> {
    let (route_part, query) = after_scheme.split_once('?').unwrap_or((after_scheme, ""));
    let normalized = route_part.trim_end_matches('/');
    if normalized.is_empty() {
        return Err(DeepLinkError::UnknownRoute {
            route: String::new(),
        });
    }
    Ok((normalized, query))
}

fn split_https_route(after_scheme: &str) -> Result<(&str, &str), DeepLinkError> {
    let (authority_and_path, query) = after_scheme.split_once('?').unwrap_or((after_scheme, ""));
    let (host_raw, path) = match authority_and_path.find('/') {
        Some(idx) => (&authority_and_path[..idx], &authority_and_path[idx..]),
        None => (authority_and_path, "/"),
    };

    if !host_raw.eq_ignore_ascii_case(ALLOWED_HTTPS_HOST) {
        return Err(DeepLinkError::UntrustedHost {
            host: host_raw.to_owned(),
        });
    }

    let Some(subroute) = path.strip_prefix(REQUIRED_HTTPS_SUBPATH_PREFIX) else {
        return Err(DeepLinkError::InvalidSubpath {
            path: path.to_owned(),
        });
    };

    let normalized = subroute.trim_end_matches('/');
    if normalized.is_empty() {
        return Err(DeepLinkError::UnknownRoute {
            route: String::new(),
        });
    }

    Ok((normalized, query))
}

fn extract_unique_query_param<'a>(
    query: &'a str,
    name: &'static str,
) -> Result<&'a str, DeepLinkError> {
    let mut found: Option<&'a str> = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key == name {
            if found.is_some() {
                return Err(DeepLinkError::DuplicateQueryParameter { name });
            }
            found = Some(value);
        }
    }

    found.ok_or(DeepLinkError::MissingQueryParameter { name })
}
