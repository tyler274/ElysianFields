//! Optional static web bundle file server mounted strictly under `/elysianfields/`.
//!
//! Enforces strict canonical path-traversal rejection, strict CSP/security headers,
//! explicit cache-control policies, and zero `Last-Modified` header emission.

use std::fs;
use std::path::{Component, Path, PathBuf};

use ef_core::{CachePolicy, Subpath};

use crate::http::{HttpMethod, HttpResponse};

/// Attempts to serve a static file from `web_dir` for a validated `subpath` under `/elysianfields`.
///
/// Resolves root (`/`) and trailing-slash subdirectory requests (e.g. `/mobile/`) to their
/// `index.html` entry file, redirects bare directory paths that contain an `index.html`
/// (e.g. `/mobile` -> `/elysianfields/mobile/`) with `308 Permanent Redirect` so relative
/// asset references resolve cleanly, and never emits directory listings.
///
/// Returns `Some(HttpResponse)` if a file/redirect was served or rejected as an invalid
/// method/traversal, or `None` if no matching regular file exists inside `web_dir`.
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
    if has_traversal_encoding(trimmed) || trimmed.contains("//") {
        return Some(HttpResponse::not_found());
    }

    let is_dir_index_request = trimmed.is_empty() || trimmed.ends_with('/');
    let path_without_trailing = trimmed.trim_end_matches('/');

    let mut clean_rel = PathBuf::new();
    if !path_without_trailing.is_empty() {
        for component in Path::new(path_without_trailing).components() {
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
    }
    if is_dir_index_request {
        clean_rel.push("index.html");
    }

    let canonical_root = fs::canonicalize(web_dir).ok()?;
    let candidate = canonical_root.join(&clean_rel);
    let canonical_target = fs::canonicalize(&candidate).ok()?;

    if !canonical_target.starts_with(&canonical_root) {
        return Some(HttpResponse::not_found());
    }
    let metadata = fs::metadata(&canonical_target).ok()?;
    if metadata.is_dir() {
        let dir_index = canonical_target.join("index.html");
        let canonical_index = fs::canonicalize(&dir_index).ok()?;
        if canonical_index.starts_with(&canonical_root)
            && fs::metadata(&canonical_index)
                .map(|m| m.is_file())
                .unwrap_or(false)
        {
            return Some(HttpResponse::permanent_redirect(format!(
                "{}/{path_without_trailing}/",
                Subpath::CANONICAL_PREFIX
            )));
        }
        return None;
    }
    if !metadata.is_file() {
        return None;
    }

    let bytes = fs::read(&canonical_target).ok()?;
    let content_type = mime_for_path(&canonical_target);
    let cache_policy = cache_policy_for_rel(&clean_rel, &canonical_target);
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
        Some("webmanifest") => "application/manifest+json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn cache_policy_for_rel(rel: &Path, path: &Path) -> CachePolicy {
    let ext = path.extension().and_then(|e| e.to_str());
    if matches!(ext, Some("html" | "htm")) {
        return CachePolicy::EntryHtmlNoCache;
    }
    let in_immutable_dir = rel.parent().is_some_and(|parent| {
        parent.components().any(|component| {
            matches!(
                component,
                Component::Normal(seg) if seg == "assets" || seg == "pkg"
            )
        })
    });
    if in_immutable_dir || matches!(ext, Some("wasm" | "woff2")) {
        return CachePolicy::ImmutableHashedAsset;
    }
    CachePolicy::EntryHtmlNoCache
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_web_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "ef-clinic-static-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(dir.join("mobile")).expect("create mobile dir");
        fs::create_dir_all(dir.join("empty-dir")).expect("create empty dir");
        fs::create_dir_all(dir.join("mobile/pkg")).expect("create mobile pkg dir");
        fs::write(dir.join("index.html"), b"<html>clinic</html>").expect("write root index");
        fs::write(dir.join("mobile/index.html"), b"<html>mobile</html>")
            .expect("write mobile index");
        fs::write(dir.join("mobile/sw.js"), b"// sw").expect("write sw");
        fs::write(
            dir.join("mobile/manifest.webmanifest"),
            br#"{"name":"Curie Mobile"}"#,
        )
        .expect("write manifest");
        fs::write(dir.join("mobile/pkg/app.wasm"), b"\0asm").expect("write wasm");
        dir
    }

    #[test]
    fn serves_root_and_subdirectory_index_html_with_strict_headers() {
        let dir = make_temp_web_dir("index");

        let root_resp = serve_static_asset(&dir, HttpMethod::Get, "/").expect("root index");
        assert_eq!(root_resp.status, 200);
        assert_eq!(
            root_resp.header("Content-Type"),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(root_resp.header("Cache-Control"), Some("no-cache"));
        assert!(root_resp.header("Last-Modified").is_none());
        assert_eq!(root_resp.body, b"<html>clinic</html>");

        let mobile_resp =
            serve_static_asset(&dir, HttpMethod::Get, "/mobile/").expect("mobile index");
        assert_eq!(mobile_resp.status, 200);
        assert_eq!(
            mobile_resp.header("Content-Type"),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(mobile_resp.header("Cache-Control"), Some("no-cache"));
        assert!(mobile_resp.header("Content-Security-Policy").is_some());
        assert!(mobile_resp.header("Last-Modified").is_none());
        assert_eq!(mobile_resp.body, b"<html>mobile</html>");

        let bare_mobile =
            serve_static_asset(&dir, HttpMethod::Get, "/mobile").expect("bare mobile redirect");
        assert_eq!(bare_mobile.status, 308);
        assert_eq!(
            bare_mobile.header("Location"),
            Some("/elysianfields/mobile/")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn serves_pwa_manifest_sw_and_immutable_nested_wasm_assets() {
        let dir = make_temp_web_dir("pwa");

        let manifest = serve_static_asset(&dir, HttpMethod::Get, "/mobile/manifest.webmanifest")
            .expect("manifest");
        assert_eq!(manifest.status, 200);
        assert_eq!(
            manifest.header("Content-Type"),
            Some("application/manifest+json; charset=utf-8")
        );
        assert_eq!(manifest.header("Cache-Control"), Some("no-cache"));

        let sw = serve_static_asset(&dir, HttpMethod::Get, "/mobile/sw.js").expect("sw");
        assert_eq!(sw.status, 200);
        assert_eq!(
            sw.header("Content-Type"),
            Some("application/javascript; charset=utf-8")
        );
        assert_eq!(sw.header("Cache-Control"), Some("no-cache"));

        let wasm = serve_static_asset(&dir, HttpMethod::Get, "/mobile/pkg/app.wasm").expect("wasm");
        assert_eq!(wasm.status, 200);
        assert_eq!(wasm.header("Content-Type"), Some("application/wasm"));
        assert_eq!(
            wasm.header("Cache-Control"),
            Some("public, max-age=31536000, immutable")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_traversal_directory_listing_and_non_get_methods() {
        let dir = make_temp_web_dir("security");

        let traversal = serve_static_asset(&dir, HttpMethod::Get, "/mobile/../../Cargo.toml")
            .expect("traversal 404");
        assert_eq!(traversal.status, 404);

        let double_slash =
            serve_static_asset(&dir, HttpMethod::Get, "/mobile//index.html").expect("double slash");
        assert_eq!(double_slash.status, 404);

        assert!(serve_static_asset(&dir, HttpMethod::Get, "/empty-dir").is_none());
        assert!(serve_static_asset(&dir, HttpMethod::Get, "/empty-dir/").is_none());

        let post = serve_static_asset(&dir, HttpMethod::Post, "/mobile/").expect("post 405");
        assert_eq!(post.status, 405);

        let _ = fs::remove_dir_all(&dir);
    }
}
