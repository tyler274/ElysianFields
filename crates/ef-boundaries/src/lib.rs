//! Workspace architecture boundary verifier.
//!
//! Reads workspace `Cargo.toml` manifests and static client assets to enforce:
//! 1. Zero `bevy` dependencies in headless domain, harness, boundary, or server crates.
//! 2. Zero cross-product imports between `ef-clinic-*` and `ef-mobile-*` crates.
//! 3. Zero provider or tool secrets bundled in client crates or static web harnesses.

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

/// Verifies all 3 architectural boundary rules across the workspace.
pub fn audit_workspace_boundaries(workspace_root: &Path) -> Vec<BoundaryViolation> {
    let mut violations = Vec::new();
    let manifests = match load_workspace_manifests(workspace_root) {
        Ok(m) => m,
        Err(err) => {
            violations.push(err);
            return violations;
        }
    };

    for manifest in &manifests {
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

        if is_client_crate(&manifest.name)
            && let Some(crate_dir) = manifest.path.parent()
        {
            scan_dir_for_secrets(crate_dir, workspace_root, &mut violations);
        }
    }

    let web_dir = workspace_root.join("web");
    if web_dir.is_dir() {
        scan_dir_for_secrets(&web_dir, workspace_root, &mut violations);
    }

    violations
}

/// Identifies headless domain, harness, boundary, and backend server crates that MUST NOT depend on `bevy`.
#[must_use]
pub fn is_headless_crate(name: &str) -> bool {
    matches!(
        name,
        "ef-core" | "ef-clinic-domain" | "ef-harness" | "ef-boundaries" | "elysian-smoke"
    ) || name.ends_with("-domain")
        || name.ends_with("-server")
        || name.ends_with("-worker")
        || name.ends_with("-harness")
        || name.ends_with("-core")
}

/// Identifies client/browser crates where provider and tool secrets must never be bundled.
#[must_use]
pub fn is_client_crate(name: &str) -> bool {
    name.starts_with("m1-")
        || name.starts_with("ef-ui-")
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
