//! Workspace architecture boundary verifier (`EF-PLATFORM-0004` / `SCN-M3-0020`).
//!
//! Reads workspace `Cargo.toml` manifests and static client assets to enforce:
//! 1. Zero `bevy` dependencies in headless domain, harness, boundary, extension, public-site, or server crates.
//! 2. Zero cross-product imports between `ef-clinic-*` and `ef-mobile-*` crates.
//! 3. Zero product imports in shared crates (`ef-core`, `ef-harness`, `ef-connected-care`, `ef-clinical-lib`, `ef-encoder`) and zero DB/KV crates in `ef-harness`.
//! 4. Zero direct native health bridge calls inside mobile UI crates (`ef-mobile-client`).
//! 5. Zero provider or tool secrets bundled in client crates, `web/`, or `extension/`.

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Violation detected by the workspace architecture boundary checker.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BoundaryViolation {
    #[error("headless crate '{crate_name}' must not depend on rendering engine 'bevy'")]
    HeadlessCrateDependsOnBevy { crate_name: String },
    #[error("cross-product import forbidden: '{crate_name}' depends on '{forbidden_dep}'")]
    CrossProductImport {
        crate_name: String,
        forbidden_dep: String,
    },
    #[error("shared crate '{crate_name}' must not depend on product crate '{forbidden_dep}'")]
    SharedCrateImportsProduct {
        crate_name: String,
        forbidden_dep: String,
    },
    #[error(
        "shared harness crate '{crate_name}' must not depend on persistence crate '{forbidden_dep}'"
    )]
    HarnessDependsOnPersistence {
        crate_name: String,
        forbidden_dep: String,
    },
    #[error(
        "mobile UI source '{path}' must not call native health bridge symbol '{symbol}' directly"
    )]
    MobileUiCallsNativeHealthBridge { path: String, symbol: &'static str },
    #[error("client asset '{path}' contains forbidden secret pattern '{pattern}'")]
    ClientSecretDetected { path: String, pattern: &'static str },
    #[error("I/O error reading '{path}': {message}")]
    Io { path: String, message: String },
}

/// Parsed summary of one workspace member's `Cargo.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateManifestSummary {
    pub name: String,
    pub path: PathBuf,
    pub dependencies: Vec<String>,
}

/// Parses workspace member paths from the root `Cargo.toml`.
pub fn load_workspace_manifests(
    workspace_root: &Path,
) -> Result<Vec<CrateManifestSummary>, BoundaryViolation> {
    let root_manifest_path = workspace_root.join("Cargo.toml");
    let raw = fs::read_to_string(&root_manifest_path).map_err(|err| BoundaryViolation::Io {
        path: root_manifest_path.display().to_string(),
        message: err.to_string(),
    })?;

    let member_paths = parse_workspace_members(&raw);
    let mut manifests = Vec::with_capacity(member_paths.len());
    for rel in member_paths {
        let cargo_toml = workspace_root.join(&rel).join("Cargo.toml");
        let content = fs::read_to_string(&cargo_toml).map_err(|err| BoundaryViolation::Io {
            path: cargo_toml.display().to_string(),
            message: err.to_string(),
        })?;
        let name = parse_package_name(&content).unwrap_or_else(|| rel.clone());
        let dependencies = parse_dependency_names(&content);
        manifests.push(CrateManifestSummary {
            name,
            path: cargo_toml,
            dependencies,
        });
    }
    Ok(manifests)
}

/// Audits parsed workspace crate manifests for dependency-level boundary violations.
#[must_use]
pub fn audit_manifest_dependencies(manifests: &[CrateManifestSummary]) -> Vec<BoundaryViolation> {
    let mut violations = Vec::new();

    for manifest in manifests {
        if is_headless_crate(&manifest.name)
            && manifest
                .dependencies
                .iter()
                .any(|dep| dep == "bevy" || dep.starts_with("bevy_"))
        {
            violations.push(BoundaryViolation::HeadlessCrateDependsOnBevy {
                crate_name: manifest.name.clone(),
            });
        }

        if manifest.name.starts_with("ef-clinic-") {
            for dep in &manifest.dependencies {
                if dep.starts_with("ef-mobile-") {
                    violations.push(BoundaryViolation::CrossProductImport {
                        crate_name: manifest.name.clone(),
                        forbidden_dep: dep.clone(),
                    });
                }
            }
        }

        if manifest.name.starts_with("ef-mobile-") {
            for dep in &manifest.dependencies {
                if dep.starts_with("ef-clinic-") {
                    violations.push(BoundaryViolation::CrossProductImport {
                        crate_name: manifest.name.clone(),
                        forbidden_dep: dep.clone(),
                    });
                }
            }
        }

        if is_shared_neutral_crate(&manifest.name) {
            for dep in &manifest.dependencies {
                if dep.starts_with("ef-clinic-") || dep.starts_with("ef-mobile-") {
                    violations.push(BoundaryViolation::SharedCrateImportsProduct {
                        crate_name: manifest.name.clone(),
                        forbidden_dep: dep.clone(),
                    });
                }
            }
        }

        if manifest.name == "ef-harness" {
            for dep in &manifest.dependencies {
                if dep.starts_with("ef-mobile-") {
                    violations.push(BoundaryViolation::CrossProductImport {
                        crate_name: manifest.name.clone(),
                        forbidden_dep: dep.clone(),
                    });
                }
                if is_forbidden_harness_persistence_crate(dep) {
                    violations.push(BoundaryViolation::HarnessDependsOnPersistence {
                        crate_name: manifest.name.clone(),
                        forbidden_dep: dep.clone(),
                    });
                }
            }
        }
    }

    violations
}

/// Verifies all architectural boundary rules across workspace manifests, client sources, `web/`, and `extension/`.
#[must_use]
pub fn audit_workspace_boundaries(workspace_root: &Path) -> Vec<BoundaryViolation> {
    let manifests = match load_workspace_manifests(workspace_root) {
        Ok(m) => m,
        Err(err) => return vec![err],
    };

    let mut violations = audit_manifest_dependencies(&manifests);

    for manifest in &manifests {
        let Some(crate_dir) = manifest.path.parent() else {
            continue;
        };
        if is_client_crate(&manifest.name) {
            scan_dir_for_secrets(crate_dir, workspace_root, &mut violations);
        }
        if manifest.name == "ef-mobile-client" {
            scan_dir_for_mobile_ui_health_bridges(crate_dir, workspace_root, &mut violations);
        }
    }

    for static_dir_name in ["web", "extension"] {
        let static_dir = workspace_root.join(static_dir_name);
        if static_dir.is_dir() {
            scan_dir_for_secrets(&static_dir, workspace_root, &mut violations);
        }
    }

    violations
}

/// Identifies headless domain, harness, boundary, extension, public-site, and backend crates
/// that MUST NOT depend on `bevy`.
#[must_use]
pub fn is_headless_crate(name: &str) -> bool {
    matches!(
        name,
        "ef-core"
            | "ef-clinic-domain"
            | "ef-harness"
            | "ef-boundaries"
            | "elysian-smoke"
            | "ef-connected-care"
            | "ef-clinical-lib"
            | "ef-encoder"
            | "ef-patient-bank"
            | "ef-extension"
            | "ef-public-site"
            | "ef-migration-rehearsal"
    ) || name.ends_with("-domain")
        || name.ends_with("-server")
        || name.ends_with("-worker")
        || name.ends_with("-harness")
        || name.ends_with("-core")
        || name.ends_with("-workflows")
        || name.ends_with("-lib")
}

/// Identifies shared neutral crates that must never import product-specific crates.
#[must_use]
pub fn is_shared_neutral_crate(name: &str) -> bool {
    matches!(
        name,
        "ef-core"
            | "ef-connected-care"
            | "ef-clinical-lib"
            | "ef-encoder"
            | "ef-patient-bank"
            | "ef-extension"
            | "ef-public-site"
    )
}

/// Identifies database/KV persistence crates forbidden inside the shared execution harness.
#[must_use]
pub fn is_forbidden_harness_persistence_crate(dep: &str) -> bool {
    matches!(
        dep,
        "sqlx"
            | "postgres"
            | "tokio-postgres"
            | "deadpool-postgres"
            | "diesel"
            | "sea-orm"
            | "rusqlite"
            | "redis"
            | "fred"
            | "valkey"
    )
}

/// Identifies client/browser crates where provider and tool secrets must never be bundled.
#[must_use]
pub fn is_client_crate(name: &str) -> bool {
    name.starts_with("m1-")
        || name.starts_with("ef-ui-")
        || name == "ef-extension"
        || name.ends_with("-client")
        || name.ends_with("-web")
}

const FORBIDDEN_CLIENT_SECRET_PATTERNS: &[&str] = &[
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "DOPPLER_TOKEN",
    "DATABASE_URL",
    "BEGIN PRIVATE KEY",
    "BEGIN RSA PRIVATE KEY",
];

const FORBIDDEN_MOBILE_UI_HEALTH_SYMBOLS: &[&str] = &[
    "HKHealthStore",
    "HealthConnectClient",
    "androidx.health.connect",
    "HealthKitAdapter",
];

fn scan_dir_for_secrets(dir: &Path, root: &Path, violations: &mut Vec<BoundaryViolation>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n == "target" || n == "pkg")
            {
                continue;
            }
            scan_dir_for_secrets(&path, root, violations);
        } else if path.is_file()
            && let Ok(content) = fs::read_to_string(&path)
        {
            for pattern in FORBIDDEN_CLIENT_SECRET_PATTERNS {
                if content.contains(pattern) {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    violations.push(BoundaryViolation::ClientSecretDetected { path: rel, pattern });
                }
            }
        }
    }
}

/// Scans a mobile client source directory to ensure UI code never calls native health bridges directly.
pub fn scan_dir_for_mobile_ui_health_bridges(
    dir: &Path,
    root: &Path,
    violations: &mut Vec<BoundaryViolation>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n == "target" || n == "pkg" || n == "tests")
            {
                continue;
            }
            scan_dir_for_mobile_ui_health_bridges(&path, root, violations);
        } else if path.is_file()
            && path.extension().is_some_and(|ext| ext == "rs")
            && let Ok(content) = fs::read_to_string(&path)
        {
            for symbol in FORBIDDEN_MOBILE_UI_HEALTH_SYMBOLS {
                if content.contains(symbol) {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    violations.push(BoundaryViolation::MobileUiCallsNativeHealthBridge {
                        path: rel,
                        symbol,
                    });
                }
            }
        }
    }
}

fn parse_workspace_members(cargo_toml: &str) -> Vec<String> {
    let mut in_members = false;
    let mut members = Vec::new();
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("members") && trimmed.contains('[') {
            in_members = true;
            if trimmed.contains(']') {
                extract_quoted_strings(trimmed, &mut members);
                in_members = false;
            }
            continue;
        }
        if in_members {
            if trimmed.starts_with(']') {
                in_members = false;
                continue;
            }
            extract_quoted_strings(trimmed, &mut members);
        }
    }
    members
}

fn extract_quoted_strings(line: &str, out: &mut Vec<String>) {
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else {
            break;
        };
        let value = &after[..end];
        if !value.is_empty() {
            out.push(value.to_owned());
        }
        rest = &after[end + 1..];
    }
}

fn parse_package_name(cargo_toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package
            && let Some(rest) = trimmed.strip_prefix("name")
            && let Some(val) = rest.trim_start().strip_prefix('=')
        {
            let name = val.trim().trim_matches('"');
            if !name.is_empty() {
                return Some(name.to_owned());
            }
        }
    }
    None
}

fn parse_dependency_names(cargo_toml: &str) -> Vec<String> {
    let mut in_dep_section = false;
    let mut deps = Vec::new();
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') {
            in_dep_section = trimmed == "[dependencies]"
                || trimmed == "[build-dependencies]"
                || trimmed.ends_with(".dependencies]");
            continue;
        }
        if in_dep_section && let Some((raw_key, _)) = trimmed.split_once('=') {
            let key = raw_key.trim().trim_matches('"');
            if !key.is_empty() {
                deps.push(key.to_owned());
            }
        }
    }
    deps
}
