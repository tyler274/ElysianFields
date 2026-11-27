//! Strict MV3 manifest, CSP, connector-neutrality, and Decision D2 bundle-size auditor.

use std::fs;
use std::path::Path;

use serde_json::Value;
use thiserror::Error;

/// Decision D2 hard upper bound (1.0 MiB) on extension popup asset bundle size.
pub const MV3_POPUP_MAX_BYTES: usize = 1_048_576;

/// Forbidden vendor, host, and chart-part names that must never appear in core extension files.
const FORBIDDEN_NEUTRALITY_TOKENS: &[&str] = &[
    "epic",
    "cerner",
    "athenahealth",
    "eclinicalworks",
    "nextgen",
    "allscripts",
    "drchrono",
    "kipu",
    "elation",
    "practicefusion",
    "meditech",
    "problem_list",
    "medication_reconciliation",
    "progress_note",
    "chrome.storage.local",
    "chrome.storage.sync",
];

/// Security or neutrality defect detected in the MV3 extension package.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExtensionAuditViolation {
    #[error("I/O error reading '{path}': {message}")]
    Io { path: String, message: String },
    #[error("manifest.json is invalid JSON: {message}")]
    InvalidJson { message: String },
    #[error("manifest_version must be 3")]
    InvalidManifestVersion,
    #[error("pre-granted host_permissions are forbidden; use optional_host_permissions only")]
    PreGrantedHostPermissionsForbidden,
    #[error("optional_host_permissions must declare least-privilege optional patterns")]
    MissingOptionalHostPermissions,
    #[error("strict MV3 CSP missing or invalid: {reason}")]
    InvalidCsp { reason: String },
    #[error(
        "core file '{file}' violated connector neutrality / storage policy with token '{token}'"
    )]
    NeutralityOrStorageViolation { file: String, token: &'static str },
    #[error("HTML file '{file}' contains forbidden inline script or inline handler")]
    InlineScriptDetected { file: String },
    #[error("MV3 popup bundle size {actual_bytes} B exceeds 1.0 MiB Decision D2 budget")]
    PopupBudgetExceeded { actual_bytes: usize },
}

/// Summary produced when an MV3 extension package passes every security and budget audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionAuditReport {
    pub manifest_version: u64,
    pub popup_total_bytes: usize,
    pub audited_files: Vec<String>,
}

/// Audits an MV3 extension directory (`manifest.json`, `worker.js`, `connector.js`, `popup.html`, `popup.css`, `popup.js`).
pub fn audit_extension_package(
    extension_dir: &Path,
) -> Result<ExtensionAuditReport, Vec<ExtensionAuditViolation>> {
    let mut violations = Vec::new();
    let manifest_path = extension_dir.join("manifest.json");
    let manifest_raw = match fs::read_to_string(&manifest_path) {
        Ok(content) => content,
        Err(err) => {
            return Err(vec![ExtensionAuditViolation::Io {
                path: manifest_path.display().to_string(),
                message: err.to_string(),
            }]);
        }
    };

    let manifest: Value = match serde_json::from_str(&manifest_raw) {
        Ok(v) => v,
        Err(err) => {
            return Err(vec![ExtensionAuditViolation::InvalidJson {
                message: err.to_string(),
            }]);
        }
    };

    let manifest_version = manifest
        .get("manifest_version")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if manifest_version != 3 {
        violations.push(ExtensionAuditViolation::InvalidManifestVersion);
    }

    if manifest.get("host_permissions").is_some() {
        violations.push(ExtensionAuditViolation::PreGrantedHostPermissionsForbidden);
    }

    let has_optional_hosts = manifest
        .get("optional_host_permissions")
        .and_then(Value::as_array)
        .is_some_and(|arr| !arr.is_empty());
    if !has_optional_hosts {
        violations.push(ExtensionAuditViolation::MissingOptionalHostPermissions);
    }

    validate_mv3_csp(&manifest, &mut violations);

    let core_files = [
        "manifest.json",
        "worker.js",
        "connector.js",
        "popup.html",
        "popup.css",
        "popup.js",
    ];
    let mut audited_files = Vec::new();
    let mut popup_total_bytes = 0usize;

    for rel in core_files {
        let full_path = extension_dir.join(rel);
        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(err) => {
                violations.push(ExtensionAuditViolation::Io {
                    path: full_path.display().to_string(),
                    message: err.to_string(),
                });
                continue;
            }
        };
        audited_files.push(rel.to_owned());

        if rel.starts_with("popup.") {
            popup_total_bytes += content.len();
        }

        let lower = content.to_ascii_lowercase();
        for token in FORBIDDEN_NEUTRALITY_TOKENS {
            if lower.contains(token) {
                violations.push(ExtensionAuditViolation::NeutralityOrStorageViolation {
                    file: rel.to_owned(),
                    token,
                });
            }
        }

        if rel.ends_with(".html") && has_inline_script_or_handler(&lower) {
            violations.push(ExtensionAuditViolation::InlineScriptDetected {
                file: rel.to_owned(),
            });
        }
    }

    if popup_total_bytes > MV3_POPUP_MAX_BYTES {
        violations.push(ExtensionAuditViolation::PopupBudgetExceeded {
            actual_bytes: popup_total_bytes,
        });
    }

    if !violations.is_empty() {
        return Err(violations);
    }

    Ok(ExtensionAuditReport {
        manifest_version,
        popup_total_bytes,
        audited_files,
    })
}

fn validate_mv3_csp(manifest: &Value, violations: &mut Vec<ExtensionAuditViolation>) {
    let Some(csp) = manifest
        .get("content_security_policy")
        .and_then(|v| v.get("extension_pages"))
        .and_then(Value::as_str)
    else {
        violations.push(ExtensionAuditViolation::InvalidCsp {
            reason: "missing content_security_policy.extension_pages".to_owned(),
        });
        return;
    };

    let lower = csp.to_ascii_lowercase();
    for forbidden in ["'unsafe-inline'", "'unsafe-eval'", "*", "http:", "https:"] {
        if lower.contains(forbidden) {
            violations.push(ExtensionAuditViolation::InvalidCsp {
                reason: format!("contains forbidden CSP token {forbidden}"),
            });
        }
    }
    if !lower.contains("script-src 'self' 'wasm-unsafe-eval'") {
        violations.push(ExtensionAuditViolation::InvalidCsp {
            reason: "script-src must require 'self' 'wasm-unsafe-eval'".to_owned(),
        });
    }
    if !lower.contains("object-src 'self'") {
        violations.push(ExtensionAuditViolation::InvalidCsp {
            reason: "object-src must be 'self'".to_owned(),
        });
    }
}

fn has_inline_script_or_handler(lower_html: &str) -> bool {
    for forbidden in ["onclick=", "onload=", "onerror=", "style=", "javascript:"] {
        if lower_html.contains(forbidden) {
            return true;
        }
    }
    let mut rest = lower_html;
    while let Some(start) = rest.find("<script") {
        let after = &rest[start..];
        let Some(tag_end) = after.find('>') else {
            return true;
        };
        let open_tag = &after[..=tag_end];
        if !open_tag.contains("src=") {
            return true;
        }
        let body_start = &after[tag_end + 1..];
        let Some(close_idx) = body_start.find("</script>") else {
            return true;
        };
        if !body_start[..close_idx].trim().is_empty() {
            return true;
        }
        rest = &body_start[close_idx + "</script>".len()..];
    }
    false
}
