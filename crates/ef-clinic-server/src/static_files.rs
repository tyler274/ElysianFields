//! Optional static web bundle file server mounted strictly under `/elysianfields/`.
//!
//! Enforces strict canonical path-traversal rejection, strict CSP/security headers,
//! explicit cache-control policies, and zero `Last-Modified` header emission.

use std::fs;
use std::path::{Component, Path, PathBuf};

use ef_core::CachePolicy;

use crate::http::{HttpMethod, HttpResponse};

/// Attempts to serve a static file from `web_dir` for a validated `subpath` under `/elysianfields`.
///
/// Returns `Some(HttpResponse)` if the file was served or rejected as an invalid method/traversal,
/// or `None` if no matching regular file exists inside `web_dir`.
#[must_use]
pub fn serve_static_asset(
    web_dir: &Path,
    method: HttpMethod,
    subpath: &str,
) -> Option<HttpResponse> {
    if !matches!(method, HttpMethod::Get | HttpMethod::Head) {
        return Some(HttpResponse::method_not_allowed());
    }

    let trimmed = subpath.trim_start_matches('/');
    let rel_str = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };

    if has_traversal_encoding(rel_str) {
        return Some(HttpResponse::not_found());
    }

    let rel_path = Path::new(rel_str);
    let mut clean_rel = PathBuf::new();
    for component in rel_path.components() {
        match component {
            Component::Normal(seg) => clean_rel.push(seg),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Some(HttpResponse::not_found());
            }
        }
    }

    let canonical_root = fs::canonicalize(web_dir).ok()?;
    let candidate = canonical_root.join(&clean_rel);
    let canonical_file = fs::canonicalize(&candidate).ok()?;

    if !canonical_file.starts_with(&canonical_root) {
        return Some(HttpResponse::not_found());
    }
    let metadata = fs::metadata(&canonical_file).ok()?;
    if !metadata.is_file() {
        return None;
    }

    let bytes = fs::read(&canonical_file).ok()?;
    let content_type = mime_for_path(&canonical_file);
    let cache_policy = cache_policy_for_rel(rel_str, &canonical_file);
    Some(HttpResponse::static_asset(
        content_type,
        cache_policy,
        bytes,
    ))
}

fn has_traversal_encoding(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    raw.contains('\0')
        || raw.contains('\\')
        || lower.contains("..")
        || lower.contains("%2e")
        || lower.contains("%2f")
        || lower.contains("%5c")
}

fn mime_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn cache_policy_for_rel(rel: &str, path: &Path) -> CachePolicy {
    let ext = path.extension().and_then(|e| e.to_str());
    if matches!(ext, Some("html" | "htm")) {
        return CachePolicy::EntryHtmlNoCache;
    }
    if rel.starts_with("assets/")
        || rel.starts_with("pkg/")
        || matches!(ext, Some("wasm" | "woff2"))
    {
        return CachePolicy::ImmutableHashedAsset;
    }
    CachePolicy::EntryHtmlNoCache
}
