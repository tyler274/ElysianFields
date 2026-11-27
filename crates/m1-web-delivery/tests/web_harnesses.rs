//! Integration tests verifying static web and MV3 popup harnesses uphold strict CSP
//! and `/elysianfields` subpath invariants.

use std::fs;
use std::path::PathBuf;

use m1_web_delivery::csp::{CspSurface, parse_and_verify_csp};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates_dir| crates_dir.parent())
        .expect("crate must live under <workspace>/crates/<name>")
        .to_path_buf()
}

fn assert_no_inline_scripts_or_root_links(html: &str, label: &str) {
    let lower = html.to_ascii_lowercase();

    // Verify every <script ...> tag references an external src and has an empty body.
    let mut rest = lower.as_str();
    while let Some(start) = rest.find("<script") {
        let after_open = &rest[start..];
        let tag_end = after_open.find('>').expect("script opening tag must close");
        let open_tag = &after_open[..=tag_end];
        assert!(
            open_tag.contains("src="),
            "{label}: inline <script> without external src is forbidden by strict CSP"
        );

        let body_start = &after_open[tag_end + 1..];
        let close_idx = body_start
            .find("</script>")
            .expect("script tag must have closing </script>");
        let body = body_start[..close_idx].trim();
        assert!(
            body.is_empty(),
            "{label}: <script> body must be empty under strict CSP, found: {body}"
        );

        rest = &body_start[close_idx + "</script>".len()..];
    }

    for forbidden in ["onclick=", "onload=", "onerror=", "style=", "javascript:"] {
        assert!(
            !lower.contains(forbidden),
            "{label}: forbidden inline attribute/scheme '{forbidden}' detected"
        );
    }

    for domain_root_attr in ["src=\"/", "href=\"/"] {
        assert!(
            !lower.contains(domain_root_attr),
            "{label}: domain-root asset reference '{domain_root_attr}' violates /elysianfields subpath rule"
        );
    }
}

#[test]
fn mv3_manifest_declares_v3_and_passes_strict_csp_verifier() {
    let manifest_path = workspace_root().join("web/mv3-popup/manifest.json");
    let raw = fs::read_to_string(&manifest_path).expect("MV3 manifest.json must exist");

    assert!(
        raw.contains("\"manifest_version\": 3"),
        "manifest must target Manifest V3"
    );

    let marker = "\"extension_pages\": \"";
    let start = raw
        .find(marker)
        .expect("manifest must declare content_security_policy.extension_pages")
        + marker.len();
    let end = raw[start..]
        .find('"')
        .expect("extension_pages CSP value must be quoted");
    let csp_value = &raw[start..start + end];

    let verified = parse_and_verify_csp(csp_value, CspSurface::Mv3ExtensionPopup)
        .expect("MV3 manifest CSP must satisfy strict verifier");
    assert_eq!(verified.surface(), CspSurface::Mv3ExtensionPopup);
}

#[test]
fn static_web_and_mv3_html_harnesses_have_zero_inline_scripts_or_domain_root_assumptions() {
    let root = workspace_root();

    let web_html =
        fs::read_to_string(root.join("web/elysianfields/index.html")).expect("web index.html");
    assert_no_inline_scripts_or_root_links(&web_html, "web/elysianfields/index.html");

    let popup_html =
        fs::read_to_string(root.join("web/mv3-popup/popup.html")).expect("mv3 popup.html");
    assert_no_inline_scripts_or_root_links(&popup_html, "web/mv3-popup/popup.html");
}
