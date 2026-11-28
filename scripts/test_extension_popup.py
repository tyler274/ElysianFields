#!/usr/bin/env python3
"""Unit tests for the Chrome MV3 extension popup (12-locale RTL, WCAG AA Light/Dark,
local-hash Quick-Capture Triage, strict CSP, zero inline scripts, < 20 KiB budget).
"""

from html.parser import HTMLParser
import json
from pathlib import Path
import re
import unittest

REPO_ROOT = Path(__file__).resolve().parent.parent
EXTENSION_DIR = REPO_ROOT / "extension"

REQUIRED_LOCALES = (
    "en",
    "es",
    "fr",
    "pt",
    "de",
    "zh-Hans",
    "ja",
    "ko",
    "ar",
    "he",
    "vi",
    "hi",
)
RTL_LOCALES = ("ar", "he")
POPUP_STRICT_BUDGET_BYTES = 20_480  # 20 KiB uncompressed (< 2% of 1.0 MiB MV3 cap)


def _srgb_luminance(hex_color: str) -> float:
    raw = hex_color.lstrip("#")
    r, g, b = (int(raw[i : i + 2], 16) / 255.0 for i in (0, 2, 4))

    def lin(c: float) -> float:
        return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4

    return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)


def _contrast_ratio(fg: str, bg: str) -> float:
    l1 = _srgb_luminance(fg)
    l2 = _srgb_luminance(bg)
    lighter, darker = (l1, l2) if l1 >= l2 else (l2, l1)
    return (lighter + 0.05) / (darker + 0.05)


class _PopupHtmlAuditor(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.inline_violations: list[str] = []
        self.script_srcs: list[str] = []
        self.locale_options: list[str] = []
        self.ids: set[str] = set()
        self._in_script = False
        self._in_locale_select = False

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attr_map = {k.lower(): (v or "") for k, v in attrs}
        if "id" in attr_map:
            self.ids.add(attr_map["id"])

        for key, value in attr_map.items():
            if key.startswith("on"):
                self.inline_violations.append(f"inline handler {key} on <{tag}>")
            if key == "style":
                self.inline_violations.append(f"inline style attribute on <{tag}>")
            if "javascript:" in value.lower():
                self.inline_violations.append(f"javascript: URI in {key} on <{tag}>")

        if tag.lower() == "style":
            self.inline_violations.append("inline <style> tag forbidden")
        if tag.lower() == "script":
            self._in_script = True
            src = attr_map.get("src", "")
            if not src:
                self.inline_violations.append("<script> missing src attribute")
            else:
                self.script_srcs.append(src)
        if tag.lower() == "select" and attr_map.get("id") == "locale-select":
            self._in_locale_select = True
        if tag.lower() == "option" and self._in_locale_select:
            self.locale_options.append(attr_map.get("value", ""))

    def handle_endtag(self, tag: str) -> None:
        if tag.lower() == "script":
            self._in_script = False
        if tag.lower() == "select":
            self._in_locale_select = False

    def handle_data(self, data: str) -> None:
        if self._in_script and data.strip():
            self.inline_violations.append("inline <script> body forbidden")


class TestExtensionPopupParity(unittest.TestCase):
    def test_mv3_manifest_validity_and_strict_csp(self) -> None:
        manifest = json.loads((EXTENSION_DIR / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest.get("manifest_version"), 3)
        self.assertNotIn("host_permissions", manifest)
        self.assertTrue(manifest.get("optional_host_permissions"))
        self.assertEqual(manifest.get("action", {}).get("default_popup"), "popup.html")

        csp = manifest.get("content_security_policy", {}).get("extension_pages", "")
        self.assertIn("script-src 'self'", csp)
        self.assertIn("object-src 'self'", csp)
        for forbidden in ("'unsafe-inline'", "'unsafe-eval'", "*", "http:", "https:"):
            self.assertNotIn(forbidden, csp)

    def test_popup_has_zero_inline_scripts_styles_or_event_handlers(self) -> None:
        html = (EXTENSION_DIR / "popup.html").read_text(encoding="utf-8")
        auditor = _PopupHtmlAuditor()
        auditor.feed(html)
        self.assertEqual(auditor.inline_violations, [])
        self.assertEqual(auditor.script_srcs, ["./popup.js"])
        for required_id in (
            "locale-select",
            "btn-theme",
            "connector-status",
            "triage-form",
            "triage-kind",
            "triage-priority",
            "triage-input",
            "triage-status",
            "btn-refresh",
            "btn-disconnect",
        ):
            self.assertIn(required_id, auditor.ids)

    def test_popup_uncompressed_bundle_strictly_under_20_kib(self) -> None:
        total = sum(
            (EXTENSION_DIR / name).stat().st_size
            for name in ("popup.html", "popup.css", "popup.js")
        )
        self.assertLess(
            total,
            POPUP_STRICT_BUDGET_BYTES,
            f"popup bundle ({total} B) must stay strictly < {POPUP_STRICT_BUDGET_BYTES} B (20 KiB)",
        )

    def test_all_12_locales_present_and_hebrew_arabic_rtl_logical_css(self) -> None:
        html = (EXTENSION_DIR / "popup.html").read_text(encoding="utf-8")
        css = (EXTENSION_DIR / "popup.css").read_text(encoding="utf-8")
        js = (EXTENSION_DIR / "popup.js").read_text(encoding="utf-8")

        auditor = _PopupHtmlAuditor()
        auditor.feed(html)
        self.assertEqual(tuple(auditor.locale_options), REQUIRED_LOCALES)

        for locale in REQUIRED_LOCALES:
            present = f"{locale}:" in js or f'"{locale}":' in js
            self.assertTrue(present, f"locale {locale!r} missing from popup.js catalog")

        for rtl in RTL_LOCALES:
            self.assertIn(f'"{rtl}"', js)
        self.assertIn("document.documentElement.lang", js)
        self.assertIn("document.documentElement.dir", js)

        for prop in ("inset-inline-", "margin-inline-", "padding-inline:", "text-align: start"):
            self.assertIn(prop, css, f"CSS logical property {prop!r} missing in popup.css")

    def test_wcag_aa_light_and_dark_theme_tokens(self) -> None:
        css = (EXTENSION_DIR / "popup.css").read_text(encoding="utf-8")
        js = (EXTENSION_DIR / "popup.js").read_text(encoding="utf-8")
        self.assertIn("dataset.theme", js)

        theme_blocks = {}
        for theme in ("light", "dark"):
            match = re.search(
                rf':root\[data-theme="{theme}"\]\s*\{{([^}}]+)\}}',
                css,
                flags=re.MULTILINE,
            )
            self.assertIsNotNone(match, f"missing :root[data-theme=\"{theme}\"] block")
            tokens = dict(re.findall(r"(--[a-z-]+)\s*:\s*(#[0-9a-fA-F]{6})", match.group(1)))
            theme_blocks[theme] = tokens

        for theme, tokens in theme_blocks.items():
            self.assertGreaterEqual(
                _contrast_ratio(tokens["--text-primary"], tokens["--bg-page"]), 4.5, theme
            )
            self.assertGreaterEqual(
                _contrast_ratio(tokens["--text-primary"], tokens["--bg-panel"]), 4.5, theme
            )
            self.assertGreaterEqual(
                _contrast_ratio(tokens["--text-secondary"], tokens["--bg-panel"]), 4.5, theme
            )
            self.assertGreaterEqual(
                _contrast_ratio(tokens["--accent-text"], tokens["--accent-bg"]), 4.5, theme
            )
            self.assertGreaterEqual(
                _contrast_ratio(tokens["--btn-secondary-text"], tokens["--btn-secondary-bg"]),
                4.5,
                theme,
            )
            self.assertGreaterEqual(
                _contrast_ratio(tokens["--badge-text"], tokens["--badge-bg"]), 4.5, theme
            )
            self.assertGreaterEqual(
                _contrast_ratio(tokens["--focus-ring"], tokens["--bg-panel"]), 3.0, theme
            )

    def test_quick_capture_triage_local_sha256_and_zero_raw_phi_persistence(self) -> None:
        js = (EXTENSION_DIR / "popup.js").read_text(encoding="utf-8")
        self.assertIn('crypto.subtle.digest("SHA-256"', js)
        self.assertIn("sha256FallbackHex", js)
        self.assertIn('triageInputEl.value = ""', js)
        self.assertIn("redactForHash", js)
        for forbidden in (
            "chrome.storage.local",
            "chrome.storage.sync",
            "chrome.storage.session.set",
            "localStorage",
            "sessionStorage",
            "innerHTML",
            "eval(",
        ):
            self.assertNotIn(forbidden, js)


if __name__ == "__main__":
    unittest.main()
