//! Strict Content-Security-Policy (CSP) parser and verifier for web and MV3 surfaces.

use std::fmt;

/// Canonical strict CSP header for browser apps mounted under `/elysianfields/`.
pub const WEB_APP_CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; frame-ancestors 'none'";

/// Canonical strict CSP header for the Chrome Manifest V3 extension popup.
pub const MV3_POPUP_CSP: &str =
    "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; object-src 'self'";

/// Browser surface whose CSP requirements are being enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CspSurface {
    /// Static/server-hosted web application under `/elysianfields/`.
    WebApp,
    /// Chrome Extension Manifest V3 popup (`content_security_policy.extension_pages`).
    Mv3ExtensionPopup,
}

impl CspSurface {
    /// Returns the single-source canonical CSP policy string for this surface.
    #[must_use]
    pub const fn canonical_header(self) -> &'static str {
        match self {
            Self::WebApp => WEB_APP_CSP,
            Self::Mv3ExtensionPopup => MV3_POPUP_CSP,
        }
    }
}

/// One parsed CSP directive and its ordered source tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CspDirective {
    pub name: String,
    pub sources: Vec<String>,
}

/// A CSP header that passed strict parsing and security policy verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCspPolicy {
    surface: CspSurface,
    directives: Vec<CspDirective>,
}

impl VerifiedCspPolicy {
    /// Returns the surface this policy was verified for.
    #[must_use]
    pub const fn surface(&self) -> CspSurface {
        self.surface
    }

    /// Returns all parsed directives in declaration order.
    #[must_use]
    pub fn directives(&self) -> &[CspDirective] {
        &self.directives
    }

    /// Looks up a directive by lowercase name.
    #[must_use]
    pub fn directive(&self, name: &str) -> Option<&CspDirective> {
        self.directives.iter().find(|entry| entry.name == name)
    }
}

/// Individual policy defect reported by the strict CSP verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CspViolation {
    EmptyPolicy,
    DuplicateDirective {
        directive: String,
    },
    EmptyDirectiveSources {
        directive: String,
    },
    ForbiddenSource {
        directive: String,
        source: String,
    },
    MissingRequiredDirective {
        directive: &'static str,
    },
    MissingRequiredSource {
        directive: &'static str,
        source: &'static str,
    },
    UnexpectedSource {
        directive: &'static str,
        source: String,
    },
}

impl fmt::Display for CspViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPolicy => f.write_str("CSP header is empty"),
            Self::DuplicateDirective { directive } => {
                write!(f, "duplicate CSP directive '{directive}'")
            }
            Self::EmptyDirectiveSources { directive } => {
                write!(f, "CSP directive '{directive}' has no source expressions")
            }
            Self::ForbiddenSource { directive, source } => {
                write!(
                    f,
                    "forbidden CSP source {source} in directive '{directive}'"
                )
            }
            Self::MissingRequiredDirective { directive } => {
                write!(f, "missing required CSP directive '{directive}'")
            }
            Self::MissingRequiredSource { directive, source } => {
                write!(
                    f,
                    "directive '{directive}' is missing required source {source}"
                )
            }
            Self::UnexpectedSource { directive, source } => {
                write!(
                    f,
                    "directive '{directive}' contains disallowed source {source}"
                )
            }
        }
    }
}

/// Aggregated validation failure containing every CSP violation found in a policy header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CspValidationError {
    pub violations: Vec<CspViolation>,
}

impl fmt::Display for CspValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CSP validation failed: ")?;
        for (idx, violation) in self.violations.iter().enumerate() {
            if idx > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CspValidationError {}

const FORBIDDEN_SOURCES: &[&str] = &[
    "'unsafe-inline'",
    "'unsafe-eval'",
    "'unsafe-hashes'",
    "*",
    "http:",
    "https:",
    "data:",
];

/// Parses and validates a Content-Security-Policy header against the strict surface contract.
pub fn parse_and_verify_csp(
    header_value: &str,
    surface: CspSurface,
) -> Result<VerifiedCspPolicy, CspValidationError> {
    let mut violations = Vec::new();
    let mut directives: Vec<CspDirective> = Vec::new();

    for raw_directive in header_value.split(';') {
        let trimmed = raw_directive.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let Some(name_raw) = parts.next() else {
            continue;
        };
        let name = name_raw.to_ascii_lowercase();

        if directives.iter().any(|existing| existing.name == name) {
            violations.push(CspViolation::DuplicateDirective { directive: name });
            continue;
        }

        let sources: Vec<String> = parts.map(str::to_ascii_lowercase).collect();
        if sources.is_empty() {
            violations.push(CspViolation::EmptyDirectiveSources {
                directive: name.clone(),
            });
        }

        for source in &sources {
            if is_forbidden_source(source) {
                violations.push(CspViolation::ForbiddenSource {
                    directive: name.clone(),
                    source: source.clone(),
                });
            }
        }

        directives.push(CspDirective { name, sources });
    }

    if directives.is_empty() {
        return Err(CspValidationError {
            violations: vec![CspViolation::EmptyPolicy],
        });
    }

    match surface {
        CspSurface::WebApp => verify_web_app_directives(&directives, &mut violations),
        CspSurface::Mv3ExtensionPopup => verify_mv3_popup_directives(&directives, &mut violations),
    }

    if !violations.is_empty() {
        return Err(CspValidationError { violations });
    }

    Ok(VerifiedCspPolicy {
        surface,
        directives,
    })
}

fn is_forbidden_source(source: &str) -> bool {
    FORBIDDEN_SOURCES.contains(&source)
        || source.starts_with("http://")
        || source.starts_with("https://")
}

fn verify_web_app_directives(directives: &[CspDirective], violations: &mut Vec<CspViolation>) {
    require_exact_sources(directives, "default-src", &["'self'"], violations);
    require_exact_sources(
        directives,
        "script-src",
        &["'self'", "'wasm-unsafe-eval'"],
        violations,
    );
    require_exact_sources(directives, "style-src", &["'self'"], violations);
    require_exact_sources(directives, "frame-ancestors", &["'none'"], violations);
}

fn verify_mv3_popup_directives(directives: &[CspDirective], violations: &mut Vec<CspViolation>) {
    require_exact_sources(
        directives,
        "script-src",
        &["'self'", "'wasm-unsafe-eval'"],
        violations,
    );
    require_exact_sources(directives, "object-src", &["'self'"], violations);

    if let Some(default_src) = directives.iter().find(|entry| entry.name == "default-src") {
        check_allowed_sources(default_src, "default-src", &["'self'"], violations);
    }
    if let Some(style_src) = directives.iter().find(|entry| entry.name == "style-src") {
        check_allowed_sources(style_src, "style-src", &["'self'"], violations);
    }
}

fn require_exact_sources(
    directives: &[CspDirective],
    directive_name: &'static str,
    required_sources: &[&'static str],
    violations: &mut Vec<CspViolation>,
) {
    let Some(directive) = directives.iter().find(|entry| entry.name == directive_name) else {
        violations.push(CspViolation::MissingRequiredDirective {
            directive: directive_name,
        });
        return;
    };

    for &required in required_sources {
        if !directive.sources.iter().any(|src| src == required) {
            violations.push(CspViolation::MissingRequiredSource {
                directive: directive_name,
                source: required,
            });
        }
    }

    check_allowed_sources(directive, directive_name, required_sources, violations);
}

fn check_allowed_sources(
    directive: &CspDirective,
    directive_name: &'static str,
    allowed_sources: &[&'static str],
    violations: &mut Vec<CspViolation>,
) {
    for source in &directive.sources {
        if !is_forbidden_source(source) && !allowed_sources.contains(&source.as_str()) {
            violations.push(CspViolation::UnexpectedSource {
                directive: directive_name,
                source: source.clone(),
            });
        }
    }
}
