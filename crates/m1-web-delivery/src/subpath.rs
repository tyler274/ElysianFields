//! Strict `/elysianfields` subpath normalization and URL/asset/API resolution.

use std::fmt;

/// Canonical browser mount prefix for all Elysian Fields web bundles, assets, and APIs.
pub const MOUNT_PREFIX: &str = "/elysianfields";

/// Errors produced when parsing origins or resolving paths under `/elysianfields`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubpathError {
    EmptyOrigin,
    UnsupportedOriginScheme,
    OriginContainsPathOrQuery,
    ExternalUrlNotAllowed,
    PathTraversalDetected,
    InvalidPathCharacter,
    OutsideSubpathMount { path: String },
}

impl fmt::Display for SubpathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyOrigin => f.write_str("origin must not be empty"),
            Self::UnsupportedOriginScheme => {
                f.write_str("origin must start with http:// or https://")
            }
            Self::OriginContainsPathOrQuery => f.write_str(
                "origin must contain only scheme and host[:port], with no path or query",
            ),
            Self::ExternalUrlNotAllowed => {
                f.write_str("relative target must not contain a scheme or protocol-relative prefix")
            }
            Self::PathTraversalDetected => {
                f.write_str("path traversal segments ('..') are forbidden")
            }
            Self::InvalidPathCharacter => {
                f.write_str("path contains control characters or backslashes")
            }
            Self::OutsideSubpathMount { path } => {
                write!(
                    f,
                    "path '{path}' is outside the required segment-bounded '{MOUNT_PREFIX}' mount"
                )
            }
        }
    }
}

impl std::error::Error for SubpathError {}

/// Validated HTTP(S) origin with no trailing slash, path, query, or fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedOrigin(String);

impl NormalizedOrigin {
    /// Parses and normalizes an origin such as `https://example.org` or `http://127.0.0.1:8080`.
    pub fn parse(raw: &str) -> Result<Self, SubpathError> {
        let trimmed = raw.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            return Err(SubpathError::EmptyOrigin);
        }

        let authority = if let Some(rest) = trimmed.strip_prefix("https://") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("http://") {
            rest
        } else {
            return Err(SubpathError::UnsupportedOriginScheme);
        };

        if authority.is_empty()
            || authority.contains('/')
            || authority.contains('?')
            || authority.contains('#')
            || authority
                .chars()
                .any(|ch| ch.is_control() || ch.is_whitespace())
        {
            return Err(SubpathError::OriginContainsPathOrQuery);
        }

        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the normalized origin string without a trailing slash.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Segment-bounded `/elysianfields` subpath mount resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubpathMount {
    origin: NormalizedOrigin,
}

impl SubpathMount {
    /// Creates a subpath mount bound to a validated origin under `/elysianfields`.
    #[must_use]
    pub const fn new(origin: NormalizedOrigin) -> Self {
        Self { origin }
    }

    /// Convenience constructor parsing the origin at the boundary.
    pub fn from_origin(raw_origin: &str) -> Result<Self, SubpathError> {
        Ok(Self::new(NormalizedOrigin::parse(raw_origin)?))
    }

    /// Returns the validated origin.
    #[must_use]
    pub const fn origin(&self) -> &NormalizedOrigin {
        &self.origin
    }

    /// Returns the canonical mount prefix (`/elysianfields`).
    #[must_use]
    pub const fn prefix(&self) -> &'static str {
        MOUNT_PREFIX
    }

    /// Single rule determining whether an incoming request path belongs to `/elysianfields`.
    ///
    /// Enforces a strict segment boundary so sibling prefixes like `/elysianfields-admin`
    /// or `/elysianfields2` are rejected.
    #[must_use]
    pub fn is_within_mount(path: &str) -> bool {
        let path_only = split_path_and_suffix(path).0;
        if path_only == MOUNT_PREFIX {
            return true;
        }
        path_only
            .strip_prefix(MOUNT_PREFIX)
            .is_some_and(|rest| rest.starts_with('/'))
    }

    /// Strips the segment-bounded `/elysianfields` prefix from an incoming browser path.
    pub fn strip_mount_prefix(incoming_path: &str) -> Result<String, SubpathError> {
        validate_no_control_or_backslash(incoming_path)?;
        let (path_only, suffix) = split_path_and_suffix(incoming_path);
        if !Self::is_within_mount(path_only) {
            return Err(SubpathError::OutsideSubpathMount {
                path: incoming_path.to_owned(),
            });
        }

        let remainder = path_only.strip_prefix(MOUNT_PREFIX).unwrap_or("");
        let normalized_segments = normalize_relative_segments(remainder)?;
        let mut relative = String::from("/");
        relative.push_str(&normalized_segments.join("/"));
        relative.push_str(suffix);
        Ok(relative)
    }

    /// Resolves an application route into a subpath-mounted path (`/elysianfields/...`).
    pub fn resolve_app_path(&self, route: &str) -> Result<String, SubpathError> {
        let normalized = normalize_mounted_target(route)?;
        if normalized.is_empty() {
            return Ok(format!("{MOUNT_PREFIX}/"));
        }
        Ok(format!("{MOUNT_PREFIX}/{normalized}"))
    }

    /// Resolves an application route into a full origin-qualified URL.
    pub fn resolve_app_url(&self, route: &str) -> Result<String, SubpathError> {
        let path = self.resolve_app_path(route)?;
        Ok(format!("{}{path}", self.origin.as_str()))
    }

    /// Resolves a static asset path under `<origin>/elysianfields/assets/<asset>`.
    pub fn resolve_asset_url(&self, asset_path: &str) -> Result<String, SubpathError> {
        let normalized = normalize_mounted_target(asset_path)?;
        let stripped = normalized.strip_prefix("assets/").unwrap_or(&normalized);
        if stripped.is_empty() {
            return Err(SubpathError::InvalidPathCharacter);
        }
        Ok(format!(
            "{}{MOUNT_PREFIX}/assets/{stripped}",
            self.origin.as_str()
        ))
    }

    /// Resolves a bundle file (such as `pkg/m1_web_delivery_bg.wasm`) under `<origin>/elysianfields/<file>`.
    pub fn resolve_bundle_url(&self, bundle_path: &str) -> Result<String, SubpathError> {
        let normalized = normalize_mounted_target(bundle_path)?;
        if normalized.is_empty() {
            return Err(SubpathError::InvalidPathCharacter);
        }
        Ok(format!(
            "{}{MOUNT_PREFIX}/{normalized}",
            self.origin.as_str()
        ))
    }

    /// Resolves an API endpoint under `<origin>/elysianfields/api/<endpoint>`.
    pub fn resolve_api_url(&self, endpoint: &str) -> Result<String, SubpathError> {
        let normalized = normalize_mounted_target(endpoint)?;
        let stripped = normalized.strip_prefix("api/").unwrap_or(&normalized);
        if stripped.is_empty() {
            return Err(SubpathError::InvalidPathCharacter);
        }
        Ok(format!(
            "{}{MOUNT_PREFIX}/api/{stripped}",
            self.origin.as_str()
        ))
    }
}

fn validate_no_control_or_backslash(input: &str) -> Result<(), SubpathError> {
    if input.contains('\\') || input.chars().any(char::is_control) {
        return Err(SubpathError::InvalidPathCharacter);
    }
    Ok(())
}

fn split_path_and_suffix(input: &str) -> (&str, &str) {
    let query_idx = input.find('?');
    let frag_idx = input.find('#');
    let split_idx = match (query_idx, frag_idx) {
        (Some(q), Some(f)) => Some(q.min(f)),
        (Some(q), None) => Some(q),
        (None, Some(f)) => Some(f),
        (None, None) => None,
    };

    match split_idx {
        Some(idx) => (&input[..idx], &input[idx..]),
        None => (input, ""),
    }
}

fn normalize_mounted_target(raw: &str) -> Result<String, SubpathError> {
    let trimmed = raw.trim();
    validate_no_control_or_backslash(trimmed)?;

    if trimmed.starts_with("//") || trimmed.contains("://") {
        return Err(SubpathError::ExternalUrlNotAllowed);
    }

    let (path_only, suffix) = split_path_and_suffix(trimmed);

    // Strip the canonical mount prefix if the caller passed an already-mounted path.
    let relative = if SubpathMount::is_within_mount(path_only) {
        path_only.strip_prefix(MOUNT_PREFIX).unwrap_or("")
    } else {
        path_only
    };

    let segments = normalize_relative_segments(relative)?;
    let mut joined = segments.join("/");
    joined.push_str(suffix);
    Ok(joined)
}

fn normalize_relative_segments(path: &str) -> Result<Vec<&str>, SubpathError> {
    let mut segments = Vec::new();
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if is_traversal_segment(segment) {
            return Err(SubpathError::PathTraversalDetected);
        }
        segments.push(segment);
    }
    Ok(segments)
}

fn is_traversal_segment(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    lower == ".." || lower.contains("%2e%2e") || lower.contains("%2f") || lower.contains("%5c")
}
