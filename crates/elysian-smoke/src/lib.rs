#![no_std]

//! Minimal `no_std` and Wasm-compatible smoke crate for Elysian Realm CI.
//!
//! Validates workspace lints, native unit tests, and `wasm32-unknown-unknown`
//! compilation without third-party dependencies.

/// Canonical web deployment subpath for Elysian Fields.
pub const WEB_SUBPATH: &str = "/elysianfields";

/// Target platforms supported by the Elysian Fields workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildTarget {
    NativeLinux,
    NativeMacOs,
    NativeWindows,
    BrowserWasm,
    MobileAndroid,
    MobileIos,
}

impl BuildTarget {
    /// Returns the short target classification key used by CI scope rules.
    #[must_use]
    pub const fn scope_key(self) -> &'static str {
        match self {
            Self::NativeLinux | Self::NativeMacOs | Self::NativeWindows => "desktop",
            Self::BrowserWasm => "wasm",
            Self::MobileAndroid | Self::MobileIos => "mobile",
        }
    }

    /// Returns whether this target ships static browser assets under [`WEB_SUBPATH`].
    #[must_use]
    pub const fn uses_web_subpath(self) -> bool {
        matches!(self, Self::BrowserWasm)
    }
}

/// Errors produced when validating a subpath-relative route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubpathError {
    /// The input path did not begin with the required `/elysianfields` prefix.
    OutsideMountPrefix,
    /// The input path contained a parent-directory traversal segment (`..`).
    ParentTraversal,
}

/// Validates that `route` sits under [`WEB_SUBPATH`] without path traversal
/// and returns the normalized suffix after [`WEB_SUBPATH`].
///
/// # Errors
///
/// Returns [`SubpathError::OutsideMountPrefix`] if `route` does not start at
/// `/elysianfields` (exact match or followed by `/`), or
/// [`SubpathError::ParentTraversal`] if any segment equals `..`.
pub fn strip_web_subpath(route: &str) -> Result<&str, SubpathError> {
    let suffix = if route == WEB_SUBPATH {
        ""
    } else if let Some(rest) = route.strip_prefix(WEB_SUBPATH) {
        if rest.starts_with('/') {
            rest
        } else {
            return Err(SubpathError::OutsideMountPrefix);
        }
    } else {
        return Err(SubpathError::OutsideMountPrefix);
    };

    if suffix.split('/').any(|segment| segment == "..") {
        return Err(SubpathError::ParentTraversal);
    }

    Ok(suffix)
}

#[cfg(test)]
mod tests {
    use super::{BuildTarget, SubpathError, WEB_SUBPATH, strip_web_subpath};

    #[test]
    fn subpath_accepts_mount_root_and_nested_assets() {
        assert_eq!(strip_web_subpath(WEB_SUBPATH), Ok(""));
        assert_eq!(strip_web_subpath("/elysianfields/"), Ok("/"));
        assert_eq!(
            strip_web_subpath("/elysianfields/assets/app.wasm"),
            Ok("/assets/app.wasm")
        );
    }

    #[test]
    fn subpath_rejects_sibling_prefix_and_traversal() {
        assert_eq!(
            strip_web_subpath("/elysianfields-other/app.wasm"),
            Err(SubpathError::OutsideMountPrefix)
        );
        assert_eq!(
            strip_web_subpath("/other"),
            Err(SubpathError::OutsideMountPrefix)
        );
        assert_eq!(
            strip_web_subpath("/elysianfields/../secret"),
            Err(SubpathError::ParentTraversal)
        );
    }

    #[test]
    fn target_classifications_are_exhaustive() {
        assert_eq!(BuildTarget::NativeLinux.scope_key(), "desktop");
        assert_eq!(BuildTarget::NativeMacOs.scope_key(), "desktop");
        assert_eq!(BuildTarget::NativeWindows.scope_key(), "desktop");
        assert_eq!(BuildTarget::BrowserWasm.scope_key(), "wasm");
        assert_eq!(BuildTarget::MobileAndroid.scope_key(), "mobile");
        assert_eq!(BuildTarget::MobileIos.scope_key(), "mobile");
        assert!(BuildTarget::BrowserWasm.uses_web_subpath());
        assert!(!BuildTarget::NativeLinux.uses_web_subpath());
    }
}
