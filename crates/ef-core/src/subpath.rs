//! Single owner for `/elysianfields` subpath mounting, loopback API override safety,
//! and static/clinical cache policy contracts.

use crate::error::SubpathError;

/// Canonical cache-control header strategy per route class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    /// Entry HTML (`index.html`): `Cache-Control: no-cache`, stripping `Last-Modified` / `If-Modified-Since`.
    EntryHtmlNoCache,
    /// Content-hashed Wasm, JS, and font assets: `public, max-age=31536000, immutable`.
    ImmutableHashedAsset,
    /// Clinical and authentication endpoints: `private, no-store, max-age=0` + `CDN-Cache-Control: no-store`.
    ClinicalNoStore,
}

impl CachePolicy {
    /// Returns the required `Cache-Control` response header value.
    #[must_use]
    pub const fn cache_control_header(self) -> &'static str {
        match self {
            Self::EntryHtmlNoCache => "no-cache",
            Self::ImmutableHashedAsset => "public, max-age=31536000, immutable",
            Self::ClinicalNoStore => "private, no-store, max-age=0",
        }
    }

    /// Returns the CDN cache header when required (`Some("no-store")` for clinical/auth routes).
    #[must_use]
    pub const fn cdn_cache_control_header(self) -> Option<&'static str> {
        match self {
            Self::ClinicalNoStore => Some("no-store"),
            Self::EntryHtmlNoCache | Self::ImmutableHashedAsset => None,
        }
    }

    /// Returns true when Nix-store `Last-Modified` / `If-Modified-Since` headers must be stripped.
    #[must_use]
    pub const fn strips_last_modified(self) -> bool {
        matches!(self, Self::EntryHtmlNoCache | Self::ImmutableHashedAsset)
    }
}

/// Segment-bounded `/elysianfields` subpath normalizer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subpath {
    mounted_path: String,
}

impl Subpath {
    /// Required subpath mount prefix for Elysian Fields web bundles and APIs.
    pub const CANONICAL_PREFIX: &'static str = "/elysianfields";

    /// Checks whether a pathname lies strictly on a segment boundary inside `/elysianfields`.
    #[must_use]
    pub fn is_within_mount(pathname: &str) -> bool {
        pathname == Self::CANONICAL_PREFIX
            || pathname
                .strip_prefix(Self::CANONICAL_PREFIX)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    /// Returns the canonical `308 Permanent Redirect` location for bare `/elysianfields` requests,
    /// preserving any query string (e.g., `/elysianfields?patient=pw-1` -> `/elysianfields/?patient=pw-1`).
    #[must_use]
    pub fn canonical_redirect_if_bare(path_and_query: &str) -> Option<String> {
        let (path, query) = match path_and_query.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (path_and_query, None),
        };
        if path != Self::CANONICAL_PREFIX {
            return None;
        }
        match query {
            Some(q) if !q.is_empty() => Some(format!("{}/?{q}", Self::CANONICAL_PREFIX)),
            _ => Some(format!("{}/", Self::CANONICAL_PREFIX)),
        }
    }

    /// Validates an already-mounted request path under `/elysianfields`.
    pub fn parse_mounted(raw: &str) -> Result<Self, SubpathError> {
        validate_no_external_or_traversal(raw)?;
        let (path, _) = raw.split_once('?').unwrap_or((raw, ""));
        if !Self::is_within_mount(path) {
            return Err(SubpathError::OutsideMount {
                path: path.to_owned(),
            });
        }
        Ok(Self {
            mounted_path: raw.to_owned(),
        })
    }

    /// Resolves a relative static asset path under `/elysianfields/assets/<rel>`.
    pub fn resolve_asset(relative: &str) -> Result<String, SubpathError> {
        validate_no_external_or_traversal(relative)?;
        let trimmed = relative.trim_start_matches('/');
        if trimmed.is_empty() {
            return Err(SubpathError::InvalidCharacters);
        }
        Ok(format!("{}/assets/{trimmed}", Self::CANONICAL_PREFIX))
    }

    /// Resolves a relative API route under `/elysianfields/api/<rel>`.
    pub fn resolve_api(relative: &str) -> Result<String, SubpathError> {
        validate_no_external_or_traversal(relative)?;
        let trimmed = relative.trim_start_matches('/');
        if trimmed.is_empty() {
            return Err(SubpathError::InvalidCharacters);
        }
        Ok(format!("{}/api/{trimmed}", Self::CANONICAL_PREFIX))
    }

    /// Resolves an application route under `/elysianfields/<rel>`.
    pub fn resolve_app_route(relative: &str) -> Result<String, SubpathError> {
        validate_no_external_or_traversal(relative)?;
        let trimmed = relative.trim_start_matches('/');
        if trimmed.is_empty() {
            return Ok(format!("{}/", Self::CANONICAL_PREFIX));
        }
        Ok(format!("{}/{trimmed}", Self::CANONICAL_PREFIX))
    }

    /// Returns the validated mounted path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.mounted_path
    }
}

/// Validated local-development `?api=` override restricted strictly to loopback hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopbackApiOverride(String);

impl LoopbackApiOverride {
    /// Parses an optional `?api=` override; accepts strictly `http://` or `https://`
    /// targeting `127.0.0.1`, `localhost`, or `[::1]` with zero userinfo or control chars.
    pub fn parse(raw: &str) -> Result<Self, SubpathError> {
        if raw.is_empty() || raw.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
            return Err(SubpathError::NonLoopbackOverrideRejected);
        }
        let rest = raw
            .strip_prefix("http://")
            .or_else(|| raw.strip_prefix("https://"))
            .ok_or(SubpathError::NonLoopbackOverrideRejected)?;

        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority.is_empty() || authority.contains('@') {
            return Err(SubpathError::NonLoopbackOverrideRejected);
        }

        let host = if let Some(after_bracket) = authority.strip_prefix('[') {
            let Some((ipv6, remainder)) = after_bracket.split_once(']') else {
                return Err(SubpathError::NonLoopbackOverrideRejected);
            };
            if !remainder.is_empty() && !remainder.starts_with(':') {
                return Err(SubpathError::NonLoopbackOverrideRejected);
            }
            if let Some(port) = remainder.strip_prefix(':')
                && (port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()))
            {
                return Err(SubpathError::NonLoopbackOverrideRejected);
            }
            format!("[{ipv6}]")
        } else {
            let (h, port_opt) = match authority.split_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (authority, None),
            };
            if let Some(port) = port_opt
                && (port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()))
            {
                return Err(SubpathError::NonLoopbackOverrideRejected);
            }
            h.to_ascii_lowercase()
        };

        if matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]") {
            Ok(Self(raw.trim_end_matches('/').to_owned()))
        } else {
            Err(SubpathError::NonLoopbackOverrideRejected)
        }
    }

    /// Returns the validated loopback base URL.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_no_external_or_traversal(raw: &str) -> Result<(), SubpathError> {
    if raw.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        return Err(SubpathError::InvalidCharacters);
    }
    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("javascript:")
        || lower.starts_with("//")
    {
        return Err(SubpathError::ExternalUrlNotAllowed);
    }
    if lower.contains("..") || lower.contains("%2e%2e") || lower.contains('\\') {
        return Err(SubpathError::PathTraversal);
    }
    Ok(())
}
