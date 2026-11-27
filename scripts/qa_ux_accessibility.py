#!/usr/bin/env python3
"""WCAG 2.1 AA Theme Contrast, 200% Zoom Reflow, Privacy Cover & Dei UX QA Harness.

Validates live rendered UX and accessibility invariants across:
  1. Desktop Clinic Light & Dark themes (computed WCAG 2.1 contrast >= 4.5:1 across body,
     panels, secondary diagnostic labels, primary CTA buttons, plus interactive theme toggle).
  2. Desktop Clinic 200% text zoom reflow (`html.style.fontSize = '32px'` at 1280x800:
     zero horizontal overflow, zero clipped visible buttons, non-zero bounds).
  3. Desktop Clinic live Privacy Cover (`Control+Shift+L` -> modal visible, `aria-modal="true"`,
     focus moved to `#btn-dismiss-privacy` -> `Escape` dismisses) and Shortcuts dialog
     (`#btn-shortcuts` -> `Escape` closes).
  4. Mobile PWA Light & Dark themes at 390x844 (crisis banner text & `tel:` link contrast
     >= 4.5:1 in both themes, touch targets >= 44px, zero horizontal overflow).
  5. Chrome MV3 Extension Popup (`extension/popup.html`: 0 CSP/console/page errors,
     >= 4.5:1 contrast, visible interactive buttons, DCL <= 250ms).
  6. Public Site Dei Crisis Finder (`/dei`, `/dei/search`, `/invite`) when `--public-bin`
     is supplied or auto-discovered at `target/debug/ef-public-site`: crisis numbers
     (`988`, `911`, `SAMHSA`) physically above facility listings (`crisisRect.bottom <=
     facilityRect.top + 4`), `/invite` has `noindex`, and 0 console/page errors.
  7. Optional full-page PNG screenshots (`clinic_light.png`, `clinic_dark.png`,
     `clinic_zoom200.png`, `clinic_privacy_cover.png`, `mobile_light.png`,
     `extension_popup.png`, `public_dei.png`).

Supports Playwright (`chromium`, `firefox`, `webkit`) for CI and a stdlib Chromium CDP
fallback when `playwright` is unavailable and only `chromium` is requested.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from browser_e2e import (  # noqa: E402
    StrictStaticPreviewServer,
    evaluate_mv3_popup_budget,
    find_free_port,
    parse_browsers,
    repo_root,
    verify_invite_noindex,
    wait_for_tcp_port,
)
from qa_multilingual_gui import (  # noqa: E402
    DESKTOP_VIEWPORT,
    MIN_MOBILE_TOUCH_TARGET_PX,
    MOBILE_VIEWPORT,
    _CdpSession,
    _start_optional_binary,
    locate_chromium_binary,
)

POPUP_VIEWPORT = {"width": 360, "height": 420}
WCAG_AA_NORMAL_TEXT_MIN_RATIO = 4.5
MV3_POPUP_DCL_BUDGET_MS = 250.0
_RGB_RE = re.compile(
    r"rgba?\(\s*([0-9.]+)\s*[,\s]\s*([0-9.]+)\s*[,\s]\s*([0-9.]+)(?:\s*[,/]\s*([0-9.]+))?\s*\)"
)


def parse_css_rgb(color_str: str) -> tuple[int, int, int, float]:
    """Parses a CSS `rgb(...)` or `rgba(...)` color string into `(r, g, b, alpha)`."""
    match = _RGB_RE.search(str(color_str or "").strip())
    if not match:
        raise ValueError(f"Unsupported CSS RGB color string: {color_str!r}")
    r = max(0, min(255, int(round(float(match.group(1))))))
    g = max(0, min(255, int(round(float(match.group(2))))))
    b = max(0, min(255, int(round(float(match.group(3))))))
    alpha = float(match.group(4)) if match.group(4) is not None else 1.0
    alpha = max(0.0, min(1.0, alpha))
    return (r, g, b, alpha)


def _linearize_channel(channel_8bit: int) -> float:
    srgb = channel_8bit / 255.0
    if srgb <= 0.04045:
        return srgb / 12.92
    return ((srgb + 0.055) / 1.055) ** 2.4


def relative_luminance(rgb: tuple[int, int, int]) -> float:
    """Computes WCAG 2.1 relative luminance for an 8-bit sRGB triple."""
    r_lin = _linearize_channel(rgb[0])
    g_lin = _linearize_channel(rgb[1])
    b_lin = _linearize_channel(rgb[2])
    return 0.2126 * r_lin + 0.7152 * g_lin + 0.0722 * b_lin


def wcag_contrast_ratio(fg_rgb: tuple[int, int, int], bg_rgb: tuple[int, int, int]) -> float:
    """Computes the WCAG 2.1 contrast ratio between foreground and background sRGB colors."""
    lum1 = relative_luminance(fg_rgb)
    lum2 = relative_luminance(bg_rgb)
    lighter = max(lum1, lum2)
    darker = min(lum1, lum2)
    return (lighter + 0.05) / (darker + 0.05)


def evaluate_contrast_samples(
    samples: list[dict[str, Any]],
    min_ratio: float = WCAG_AA_NORMAL_TEXT_MIN_RATIO,
) -> tuple[bool, list[dict[str, Any]]]:
    """Evaluates computed foreground/background color pairs against WCAG 2.1 AA."""
    evaluated: list[dict[str, Any]] = []
    all_ok = len(samples) > 0
    for sample in samples:
        label = str(sample.get("label", "unknown"))
        fg_raw = str(sample.get("fg", ""))
        bg_raw = str(sample.get("bg", ""))
        try:
            fg_r, fg_g, fg_b, fg_a = parse_css_rgb(fg_raw)
            bg_r, bg_g, bg_b, bg_a = parse_css_rgb(bg_raw)
            if fg_a < 0.99:
                fg_r = int(round(fg_r * fg_a + bg_r * (1.0 - fg_a)))
                fg_g = int(round(fg_g * fg_a + bg_g * (1.0 - fg_a)))
                fg_b = int(round(fg_b * fg_a + bg_b * (1.0 - fg_a)))
            ratio = wcag_contrast_ratio((fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b))
            passed = bg_a > 0.0 and ratio >= min_ratio
            evaluated.append(
                {
                    "label": label,
                    "fg": fg_raw,
                    "bg": bg_raw,
                    "ratio": round(ratio, 2),
                    "min_ratio": min_ratio,
                    "passed": passed,
                }
            )
            if not passed:
                all_ok = False
        except ValueError as exc:
            all_ok = False
            evaluated.append(
                {
                    "label": label,
                    "fg": fg_raw,
                    "bg": bg_raw,
                    "ratio": 0.0,
                    "min_ratio": min_ratio,
                    "passed": False,
                    "error": str(exc),
                }
            )
    return all_ok, evaluated


_COLOR_HELPER_JS = """
function isOpaqueBg(colorStr) {
    if (!colorStr || colorStr === "transparent") return false;
    const m = colorStr.match(/rgba?\\(\\s*([0-9.]+)\\s*[,\\s]\\s*([0-9.]+)\\s*[,\\s]\\s*([0-9.]+)(?:\\s*[,/]\\s*([0-9.]+))?\\s*\\)/);
    if (!m) return false;
    const alpha = m[4] === undefined ? 1.0 : parseFloat(m[4]);
    return alpha > 0.05;
}
function effectiveBg(el) {
    let cur = el;
    while (cur && cur.nodeType === 1) {
        const bg = getComputedStyle(cur).backgroundColor;
        if (isOpaqueBg(bg)) return bg;
        cur = cur.parentElement;
    }
    return getComputedStyle(document.documentElement).backgroundColor || "rgb(255, 255, 255)";
}
function sampleColor(label, selector) {
    const el = document.querySelector(selector);
    if (!el) return { label, selector, found: false, fg: "", bg: "" };
    const st = getComputedStyle(el);
    return { label, selector, found: true, fg: st.color, bg: effectiveBg(el) };
}
"""


def _clinic_theme_probe_js() -> str:
    return f"""(() => {{
        {_COLOR_HELPER_JS}
        const html = document.documentElement;
        const samples = [
            sampleColor("body_text", "body"),
            sampleColor("panel_heading", ".panel h2"),
            sampleColor("status_banner", "#status-banner"),
            sampleColor("secondary_diagnostic_label", ".metric-list dt"),
            sampleColor("primary_cta_button", "#btn-send-prompt"),
        ];
        const toggleBtn = document.getElementById("btn-toggle-theme");
        const beforeTheme = html.dataset.theme || "";
        const beforePressed = toggleBtn ? toggleBtn.getAttribute("aria-pressed") : null;
        return {{
            theme: beforeTheme,
            togglePresent: Boolean(toggleBtn),
            ariaPressed: beforePressed,
            cspViolations: Number(html.dataset.efCspViolations || "0"),
            scrollWidth: html.scrollWidth,
            innerWidth: window.innerWidth,
            samples,
        }};
    }})()"""


def _clinic_theme_toggle_probe_js() -> str:
    return """(() => {
        const html = document.documentElement;
        const btn = document.getElementById("btn-toggle-theme");
        if (!btn) return { ok: false, reason: "missing_toggle_btn" };
        const initialTheme = html.dataset.theme || "";
        const initialPressed = btn.getAttribute("aria-pressed");
        btn.click();
        const afterTheme = html.dataset.theme || "";
        const afterPressed = btn.getAttribute("aria-pressed");
        let stored = null;
        try {
            stored = window.localStorage.getItem("curie.theme.v1");
        } catch (_err) {
            stored = null;
        }
        return {
            ok:
                initialTheme !== afterTheme &&
                initialPressed !== afterPressed &&
                stored === afterTheme,
            initialTheme,
            afterTheme,
            initialPressed,
            afterPressed,
            stored,
        };
    })()"""


def _clinic_zoom200_probe_js() -> str:
    return """(() => {
        const html = document.documentElement;
        html.style.fontSize = "32px";
        // Force synchronous reflow after changing root font size.
        void document.body.offsetHeight;
        const clippedButtons = [];
        const checkedButtons = [];
        for (const el of document.querySelectorAll("button")) {
            const st = getComputedStyle(el);
            if (st.display === "none" || st.visibility === "hidden" || el.closest("[hidden], #ef-a11y-overlay, .ef-sr-only")) {
                continue;
            }
            const rect = el.getBoundingClientRect();
            checkedButtons.push({
                id: el.id || "",
                width: Number(rect.width.toFixed(1)),
                height: Number(rect.height.toFixed(1)),
                clientHeight: el.clientHeight,
                scrollHeight: el.scrollHeight,
            });
            if (rect.width <= 0 || rect.height <= 0) {
                clippedButtons.push({ id: el.id || "", reason: "zero_bounds" });
            } else if (el.scrollHeight > el.clientHeight + 6) {
                clippedButtons.push({
                    id: el.id || "",
                    reason: "vertical_clip",
                    scrollHeight: el.scrollHeight,
                    clientHeight: el.clientHeight,
                });
            } else if (el.clientWidth > 0 && el.scrollWidth > el.clientWidth + 6) {
                clippedButtons.push({
                    id: el.id || "",
                    reason: "horizontal_clip",
                    scrollWidth: el.scrollWidth,
                    clientWidth: el.clientWidth,
                });
            } else if (rect.left < -2 || rect.right > window.innerWidth + 2) {
                clippedButtons.push({
                    id: el.id || "",
                    reason: "viewport_overflow",
                    left: Number(rect.left.toFixed(1)),
                    right: Number(rect.right.toFixed(1)),
                });
            }
        }
        return {
            rootFontSize: getComputedStyle(html).fontSize,
            scrollWidth: html.scrollWidth,
            innerWidth: window.innerWidth,
            checkedButtonCount: checkedButtons.length,
            clippedButtons,
            cspViolations: Number(html.dataset.efCspViolations || "0"),
        };
    })()"""


def _privacy_state_js() -> str:
    return """(() => {
        const cover = document.getElementById("privacy-cover");
        const grid = document.querySelector(".workspace-grid");
        const shortcuts = document.getElementById("shortcuts-dialog");
        return {
            privacyPresent: Boolean(cover),
            privacyHidden: cover ? Boolean(cover.hidden) : true,
            privacyAriaModal: cover ? cover.getAttribute("aria-modal") : null,
            privacyRole: cover ? cover.getAttribute("role") : null,
            gridAriaHidden: grid ? grid.getAttribute("aria-hidden") : null,
            shortcutsPresent: Boolean(shortcuts),
            shortcutsHidden: shortcuts ? Boolean(shortcuts.hidden) : true,
            shortcutsAriaModal: shortcuts ? shortcuts.getAttribute("aria-modal") : null,
            activeElementId: document.activeElement ? document.activeElement.id || "" : "",
            cspViolations: Number(document.documentElement.dataset.efCspViolations || "0"),
        };
    })()"""


def _mobile_theme_probe_js() -> str:
    return f"""(() => {{
        {_COLOR_HELPER_JS}
        const html = document.documentElement;
        const samples = [
            sampleColor("body_text", "body"),
            sampleColor("card_heading", ".card h2"),
            sampleColor("secondary_diagnostic_label", ".metric-grid dt"),
            sampleColor("primary_cta_button", "#btn-mobile-signin"),
            sampleColor("crisis_banner_heading", "#crisis-heading"),
            sampleColor("crisis_banner_text", "#crisis-988-label"),
            sampleColor("crisis_tel_link", "#crisis-resources a.crisis-tel"),
        ];
        const touchTargets = [];
        for (const el of document.querySelectorAll(".touch-btn, .touch-select, .touch-input, .touch-textarea")) {{
            const st = getComputedStyle(el);
            if (st.display === "none" || st.visibility === "hidden" || el.closest("[hidden], #ef-a11y-overlay, .ef-sr-only")) continue;
            const rect = el.getBoundingClientRect();
            touchTargets.push({{
                id: el.id || el.tagName.toLowerCase(),
                height: Number(rect.height.toFixed(2)),
                width: Number(rect.width.toFixed(2)),
            }});
        }}
        const toggleBtn = document.getElementById("btn-toggle-theme");
        return {{
            theme: html.dataset.theme || "",
            togglePresent: Boolean(toggleBtn),
            ariaPressed: toggleBtn ? toggleBtn.getAttribute("aria-pressed") : null,
            scrollWidth: html.scrollWidth,
            innerWidth: window.innerWidth,
            touchTargets,
            samples,
            cspViolations: Number(html.dataset.efCspViolations || "0"),
        }};
    }})()"""


def _extension_popup_probe_js() -> str:
    return f"""(() => {{
        {_COLOR_HELPER_JS}
        const nav = performance.getEntriesByType("navigation")[0];
        const dclMs = nav && nav.domContentLoadedEventEnd > 0
            ? Number((nav.domContentLoadedEventEnd - (nav.startTime || 0)).toFixed(2))
            : Number(performance.now().toFixed(2));
        const samples = [
            sampleColor("popup_heading", ".popup-header h1"),
            sampleColor("status_pill", "#connector-status"),
            sampleColor("secondary_label", ".detail-grid dt"),
            sampleColor("primary_button", "#btn-refresh"),
            sampleColor("secondary_button", "#btn-disconnect"),
        ];
        const buttons = Array.from(document.querySelectorAll("button")).map((btn) => {{
            const rect = btn.getBoundingClientRect();
            return {{
                id: btn.id || "",
                visible: rect.width > 0 && rect.height > 0,
                width: Number(rect.width.toFixed(1)),
                height: Number(rect.height.toFixed(1)),
            }};
        }});
        return {{
            dclMs,
            samples,
            buttons,
            scrollWidth: document.documentElement.scrollWidth,
            innerWidth: window.innerWidth,
        }};
    }})()"""


def _public_dei_probe_js() -> str:
    return """(() => {
        const crisis = document.getElementById("crisis-resources");
        const listing = document.getElementById("facilities") || document.getElementById("state-directory");
        const crisisRect = crisis ? crisis.getBoundingClientRect() : null;
        const listingRect = listing ? listing.getBoundingClientRect() : null;
        const crisisText = crisis ? (crisis.innerText || "") : "";
        const facilityCount = document.querySelectorAll("[data-facility-id]").length;
        return {
            crisisPresent: Boolean(crisis),
            listingPresent: Boolean(listing),
            listingId: listing ? listing.id : "",
            has988: crisisText.includes("988"),
            has911: crisisText.includes("911"),
            hasSamhsa: crisisText.toUpperCase().includes("SAMHSA"),
            crisisRect: crisisRect
                ? {
                      top: Number(crisisRect.top.toFixed(1)),
                      bottom: Number(crisisRect.bottom.toFixed(1)),
                      height: Number(crisisRect.height.toFixed(1)),
                  }
                : null,
            listingRect: listingRect
                ? {
                      top: Number(listingRect.top.toFixed(1)),
                      bottom: Number(listingRect.bottom.toFixed(1)),
                      height: Number(listingRect.height.toFixed(1)),
                  }
                : null,
            facilityCount,
        };
    })()"""


def evaluate_clinic_theme_check(
    expected_theme: str,
    metrics: dict[str, Any],
    http_status: int,
    console_errors: list[str],
    page_errors: list[str],
    toggle_result: dict[str, Any] | None = None,
) -> dict[str, Any]:
    contrast_ok, contrast_details = evaluate_contrast_samples(metrics.get("samples", []))
    zero_overflow = int(metrics.get("scrollWidth", 99999)) <= int(metrics.get("innerWidth", 0)) + 1
    toggle_ok = True if toggle_result is None else bool(toggle_result.get("ok"))
    passed = (
        http_status == 200
        and len(console_errors) == 0
        and len(page_errors) == 0
        and int(metrics.get("cspViolations", 1)) == 0
        and metrics.get("theme") == expected_theme
        and bool(metrics.get("togglePresent"))
        and contrast_ok
        and zero_overflow
        and toggle_ok
    )
    return {
        "passed": passed,
        "expected_theme": expected_theme,
        "actual_theme": metrics.get("theme"),
        "aria_pressed": metrics.get("ariaPressed"),
        "contrast_ok": contrast_ok,
        "contrast_samples": contrast_details,
        "zero_overflow": zero_overflow,
        "toggle_check": toggle_result,
        "csp_violations": metrics.get("cspViolations"),
        "console_errors": list(console_errors),
        "page_errors": list(page_errors),
    }


def evaluate_zoom200_check(
    metrics: dict[str, Any],
    console_errors: list[str],
    page_errors: list[str],
) -> dict[str, Any]:
    zero_overflow = int(metrics.get("scrollWidth", 99999)) <= int(metrics.get("innerWidth", 0)) + 1
    clipped = metrics.get("clippedButtons", [])
    zero_clipped_buttons = len(clipped) == 0
    checked_count = int(metrics.get("checkedButtonCount", 0))
    root_font_ok = str(metrics.get("rootFontSize", "")).startswith("32")
    passed = (
        len(console_errors) == 0
        and len(page_errors) == 0
        and int(metrics.get("cspViolations", 1)) == 0
        and root_font_ok
        and zero_overflow
        and zero_clipped_buttons
        and checked_count >= 4
    )
    return {
        "passed": passed,
        "root_font_size": metrics.get("rootFontSize"),
        "scroll_width": metrics.get("scrollWidth"),
        "inner_width": metrics.get("innerWidth"),
        "zero_overflow": zero_overflow,
        "checked_button_count": checked_count,
        "clipped_buttons": clipped,
        "csp_violations": metrics.get("cspViolations"),
        "console_errors": list(console_errors),
        "page_errors": list(page_errors),
    }


def evaluate_privacy_and_shortcuts_check(
    after_open_privacy: dict[str, Any],
    after_escape_privacy: dict[str, Any],
    after_open_shortcuts: dict[str, Any],
    after_escape_shortcuts: dict[str, Any],
    console_errors: list[str],
    page_errors: list[str],
) -> dict[str, Any]:
    privacy_open_ok = (
        bool(after_open_privacy.get("privacyPresent"))
        and after_open_privacy.get("privacyHidden") is False
        and after_open_privacy.get("privacyAriaModal") == "true"
        and after_open_privacy.get("gridAriaHidden") == "true"
        and after_open_privacy.get("activeElementId") == "btn-dismiss-privacy"
    )
    privacy_close_ok = (
        after_escape_privacy.get("privacyHidden") is True
        and after_escape_privacy.get("gridAriaHidden") is None
    )
    shortcuts_open_ok = (
        bool(after_open_shortcuts.get("shortcutsPresent"))
        and after_open_shortcuts.get("shortcutsHidden") is False
        and after_open_shortcuts.get("shortcutsAriaModal") == "true"
        and after_open_shortcuts.get("activeElementId") == "btn-close-shortcuts"
    )
    shortcuts_close_ok = after_escape_shortcuts.get("shortcutsHidden") is True

    passed = (
        len(console_errors) == 0
        and len(page_errors) == 0
        and int(after_escape_shortcuts.get("cspViolations", 1)) == 0
        and privacy_open_ok
        and privacy_close_ok
        and shortcuts_open_ok
        and shortcuts_close_ok
    )
    return {
        "passed": passed,
        "privacy_open_ok": privacy_open_ok,
        "privacy_close_ok": privacy_close_ok,
        "shortcuts_open_ok": shortcuts_open_ok,
        "shortcuts_close_ok": shortcuts_close_ok,
        "after_open_privacy": after_open_privacy,
        "after_escape_privacy": after_escape_privacy,
        "after_open_shortcuts": after_open_shortcuts,
        "after_escape_shortcuts": after_escape_shortcuts,
        "console_errors": list(console_errors),
        "page_errors": list(page_errors),
    }


def evaluate_mobile_theme_check(
    expected_theme: str,
    metrics: dict[str, Any],
    http_status: int,
    console_errors: list[str],
    page_errors: list[str],
) -> dict[str, Any]:
    contrast_ok, contrast_details = evaluate_contrast_samples(metrics.get("samples", []))
    zero_overflow = int(metrics.get("scrollWidth", 99999)) <= int(metrics.get("innerWidth", 0)) + 1
    touch_targets = metrics.get("touchTargets", [])
    undersized = [
        t for t in touch_targets if float(t.get("height", 0.0)) < (MIN_MOBILE_TOUCH_TARGET_PX - 0.5)
    ]
    touch_targets_ok = len(touch_targets) > 0 and len(undersized) == 0
    passed = (
        http_status == 200
        and len(console_errors) == 0
        and len(page_errors) == 0
        and int(metrics.get("cspViolations", 1)) == 0
        and metrics.get("theme") == expected_theme
        and bool(metrics.get("togglePresent"))
        and contrast_ok
        and zero_overflow
        and touch_targets_ok
    )
    return {
        "passed": passed,
        "expected_theme": expected_theme,
        "actual_theme": metrics.get("theme"),
        "aria_pressed": metrics.get("ariaPressed"),
        "contrast_ok": contrast_ok,
        "contrast_samples": contrast_details,
        "zero_overflow": zero_overflow,
        "touch_targets_ok": touch_targets_ok,
        "touch_target_count": len(touch_targets),
        "undersized_touch_targets": undersized,
        "csp_violations": metrics.get("cspViolations"),
        "console_errors": list(console_errors),
        "page_errors": list(page_errors),
    }


def evaluate_extension_popup_check(
    metrics: dict[str, Any],
    http_status: int,
    csp_violations: int,
    console_errors: list[str],
    page_errors: list[str],
) -> dict[str, Any]:
    contrast_ok, contrast_details = evaluate_contrast_samples(metrics.get("samples", []))
    dcl_ms = float(metrics.get("dclMs", 9999.0))
    dcl_ok = evaluate_mv3_popup_budget(dcl_ms, MV3_POPUP_DCL_BUDGET_MS)
    buttons = metrics.get("buttons", [])
    visible_buttons = [b for b in buttons if b.get("visible")]
    buttons_ok = len(visible_buttons) >= 2
    passed = (
        http_status == 200
        and csp_violations == 0
        and len(console_errors) == 0
        and len(page_errors) == 0
        and contrast_ok
        and dcl_ok
        and buttons_ok
    )
    return {
        "passed": passed,
        "http_status": http_status,
        "dcl_ms": dcl_ms,
        "dcl_budget_ms": MV3_POPUP_DCL_BUDGET_MS,
        "dcl_ok": dcl_ok,
        "contrast_ok": contrast_ok,
        "contrast_samples": contrast_details,
        "visible_button_count": len(visible_buttons),
        "csp_violations": csp_violations,
        "console_errors": list(console_errors),
        "page_errors": list(page_errors),
    }


def evaluate_public_dei_check(
    dei_index_metrics: dict[str, Any],
    dei_index_status: int,
    dei_search_metrics: dict[str, Any],
    dei_search_status: int,
    invite_status: int,
    invite_html: str,
    invite_headers: dict[str, str],
    console_errors: list[str],
    page_errors: list[str],
) -> dict[str, Any]:
    def _crisis_above(m: dict[str, Any]) -> bool:
        c = m.get("crisisRect")
        l = m.get("listingRect")
        if not c or not l:
            return False
        return (
            bool(m.get("has988"))
            and bool(m.get("has911"))
            and bool(m.get("hasSamhsa"))
            and float(c["bottom"]) <= float(l["top"]) + 4.0
        )

    index_above_ok = _crisis_above(dei_index_metrics)
    search_above_ok = _crisis_above(dei_search_metrics) and int(
        dei_search_metrics.get("facilityCount", 0)
    ) > 0
    invite_noindex_ok = verify_invite_noindex(invite_html, invite_headers)
    passed = (
        dei_index_status == 200
        and dei_search_status == 200
        and invite_status == 200
        and len(console_errors) == 0
        and len(page_errors) == 0
        and index_above_ok
        and search_above_ok
        and invite_noindex_ok
    )
    return {
        "passed": passed,
        "dei_index_status": dei_index_status,
        "dei_index_crisis_above": index_above_ok,
        "dei_index": dei_index_metrics,
        "dei_search_status": dei_search_status,
        "dei_search_crisis_above": search_above_ok,
        "dei_search": dei_search_metrics,
        "invite_status": invite_status,
        "invite_noindex_ok": invite_noindex_ok,
        "console_errors": list(console_errors),
        "page_errors": list(page_errors),
    }


def _shot_name(engine: str, multi_engine: bool, stem: str) -> str:
    if multi_engine and engine != "chromium":
        return f"{engine}_{stem}.png"
    return f"{stem}.png"


def run_playwright_ux_qa(
    playwright_obj: Any,
    browsers: list[str],
    clinic_base_url: str,
    mobile_base_url: str,
    popup_url: str,
    public_origin: str | None = None,
    screenshot_dir: Path | None = None,
) -> dict[str, Any]:
    """Runs the WCAG 2.1 AA theme, 200% zoom, Privacy Cover, Popup & Dei QA via Playwright."""
    if screenshot_dir is not None:
        screenshot_dir.mkdir(parents=True, exist_ok=True)

    multi_engine = len(browsers) > 1
    engine_reports: dict[str, Any] = {}

    for engine in browsers:
        browser_type = getattr(playwright_obj, engine)
        browser = browser_type.launch(headless=True)
        try:
            checks: dict[str, Any] = {}

            # 1. Desktop Clinic Dark & Light Themes + Interactive Toggle
            for theme in ("dark", "light"):
                ctx = browser.new_context(viewport=DESKTOP_VIEWPORT)
                try:
                    page = ctx.new_page()
                    console_errors: list[str] = []
                    page_errors: list[str] = []
                    page.on(
                        "console",
                        lambda m, errs=console_errors: errs.append(m.text) if m.type == "error" else None,
                    )
                    page.on("pageerror", lambda e, errs=page_errors: errs.append(str(e)))
                    resp = page.goto(f"{clinic_base_url}?theme={theme}", wait_until="networkidle")
                    status = resp.status if resp is not None else 0
                    page.wait_for_function(
                        "() => document.documentElement.dataset.efReady === 'true'",
                        timeout=20_000,
                    )
                    metrics = page.evaluate(_clinic_theme_probe_js())
                    if screenshot_dir is not None:
                        shot = screenshot_dir / _shot_name(engine, multi_engine, f"clinic_{theme}")
                        page.screenshot(path=str(shot), full_page=True)
                    toggle_res = page.evaluate(_clinic_theme_toggle_probe_js()) if theme == "dark" else None
                    checks[f"clinic_theme_{theme}"] = evaluate_clinic_theme_check(
                        theme, metrics, status, console_errors, page_errors, toggle_res
                    )
                finally:
                    ctx.close()

            # 2. Desktop Clinic 200% Text Zoom Reflow
            ctx = browser.new_context(viewport=DESKTOP_VIEWPORT)
            try:
                page = ctx.new_page()
                console_errors = []
                page_errors = []
                page.on(
                    "console",
                    lambda m, errs=console_errors: errs.append(m.text) if m.type == "error" else None,
                )
                page.on("pageerror", lambda e, errs=page_errors: errs.append(str(e)))
                page.goto(f"{clinic_base_url}?theme=light", wait_until="networkidle")
                page.wait_for_function(
                    "() => document.documentElement.dataset.efReady === 'true'",
                    timeout=20_000,
                )
                zoom_metrics = page.evaluate(_clinic_zoom200_probe_js())
                if screenshot_dir is not None:
                    shot = screenshot_dir / _shot_name(engine, multi_engine, "clinic_zoom200")
                    page.screenshot(path=str(shot), full_page=True)
                checks["clinic_zoom_200"] = evaluate_zoom200_check(
                    zoom_metrics, console_errors, page_errors
                )
            finally:
                ctx.close()

            # 3. Desktop Clinic Live Privacy Cover (Ctrl+Shift+L / Escape) & Shortcuts Modal
            ctx = browser.new_context(viewport=DESKTOP_VIEWPORT)
            try:
                page = ctx.new_page()
                console_errors = []
                page_errors = []
                page.on(
                    "console",
                    lambda m, errs=console_errors: errs.append(m.text) if m.type == "error" else None,
                )
                page.on("pageerror", lambda e, errs=page_errors: errs.append(str(e)))
                page.goto(f"{clinic_base_url}?theme=dark", wait_until="networkidle")
                page.wait_for_function(
                    "() => document.documentElement.dataset.efReady === 'true'",
                    timeout=20_000,
                )
                page.click("h1")
                page.keyboard.press("Control+Shift+KeyL")
                after_open_privacy = page.evaluate(_privacy_state_js())
                if after_open_privacy.get("privacyHidden") is True:
                    page.keyboard.press("Control+Shift+L")
                    after_open_privacy = page.evaluate(_privacy_state_js())
                if screenshot_dir is not None:
                    shot = screenshot_dir / _shot_name(engine, multi_engine, "clinic_privacy_cover")
                    page.screenshot(path=str(shot), full_page=False)
                page.keyboard.press("Escape")
                after_escape_privacy = page.evaluate(_privacy_state_js())
                page.click("#btn-shortcuts")
                after_open_shortcuts = page.evaluate(_privacy_state_js())
                page.keyboard.press("Escape")
                after_escape_shortcuts = page.evaluate(_privacy_state_js())
                checks["clinic_privacy_and_shortcuts"] = evaluate_privacy_and_shortcuts_check(
                    after_open_privacy,
                    after_escape_privacy,
                    after_open_shortcuts,
                    after_escape_shortcuts,
                    console_errors,
                    page_errors,
                )
            finally:
                ctx.close()

            # 4. Mobile PWA Light & Dark Theme + 390x844 Reflow + Crisis Banner Contrast
            for theme in ("dark", "light"):
                mobile_ctx_kwargs: dict[str, Any] = {
                    "viewport": MOBILE_VIEWPORT,
                    "has_touch": True,
                }
                if engine != "firefox":
                    mobile_ctx_kwargs["is_mobile"] = True
                ctx = browser.new_context(**mobile_ctx_kwargs)
                try:
                    page = ctx.new_page()
                    console_errors = []
                    page_errors = []
                    page.on(
                        "console",
                        lambda m, errs=console_errors: errs.append(m.text) if m.type == "error" else None,
                    )
                    page.on("pageerror", lambda e, errs=page_errors: errs.append(str(e)))
                    resp = page.goto(f"{mobile_base_url}?theme={theme}", wait_until="networkidle")
                    status = resp.status if resp is not None else 0
                    page.wait_for_function(
                        "() => document.documentElement.dataset.efMobileReady === 'true'",
                        timeout=20_000,
                    )
                    metrics = page.evaluate(_mobile_theme_probe_js())
                    if screenshot_dir is not None and theme == "light":
                        shot = screenshot_dir / _shot_name(engine, multi_engine, "mobile_light")
                        page.screenshot(path=str(shot), full_page=True)
                    checks[f"mobile_theme_{theme}"] = evaluate_mobile_theme_check(
                        theme, metrics, status, console_errors, page_errors
                    )
                finally:
                    ctx.close()

            # 5. Chrome MV3 Extension Popup (Dark & Light)
            for theme in ("dark", "light"):
                ctx = browser.new_context(viewport=POPUP_VIEWPORT)
                try:
                    page = ctx.new_page()
                    console_errors = []
                    page_errors = []
                    page.add_init_script(
                        "window.__efCspViolations = 0; "
                        "document.addEventListener('securitypolicyviolation', () => { window.__efCspViolations += 1; });"
                    )
                    page.on(
                        "console",
                        lambda m, errs=console_errors: errs.append(m.text) if m.type == "error" else None,
                    )
                    page.on("pageerror", lambda e, errs=page_errors: errs.append(str(e)))
                    resp = page.goto(f"{popup_url}?theme={theme}", wait_until="networkidle")
                    status = resp.status if resp is not None else 0
                    page.wait_for_function(
                        f"() => document.documentElement.dataset.theme === '{theme}'",
                        timeout=10_000,
                    )
                    metrics = page.evaluate(_extension_popup_probe_js())
                    csp_violations = int(page.evaluate("() => window.__efCspViolations || 0"))
                    if screenshot_dir is not None and theme == "light":
                        shot = screenshot_dir / _shot_name(engine, multi_engine, "extension_popup")
                        page.screenshot(path=str(shot), full_page=True)
                    checks[f"extension_popup_{theme}"] = evaluate_extension_popup_check(
                        metrics, status, csp_violations, console_errors, page_errors
                    )
                finally:
                    ctx.close()

            # 6. Public Site Dei Crisis Finder & /invite noindex
            if public_origin is not None:
                ctx = browser.new_context(viewport=DESKTOP_VIEWPORT)
                try:
                    page = ctx.new_page()
                    console_errors = []
                    page_errors = []
                    page.on(
                        "console",
                        lambda m, errs=console_errors: errs.append(m.text) if m.type == "error" else None,
                    )
                    page.on("pageerror", lambda e, errs=page_errors: errs.append(str(e)))

                    r_idx = page.goto(f"{public_origin}/dei", wait_until="load")
                    idx_status = r_idx.status if r_idx is not None else 0
                    idx_metrics = page.evaluate(_public_dei_probe_js())

                    r_search = page.goto(f"{public_origin}/dei/search", wait_until="load")
                    search_status = r_search.status if r_search is not None else 0
                    search_metrics = page.evaluate(_public_dei_probe_js())
                    if screenshot_dir is not None:
                        shot = screenshot_dir / _shot_name(engine, multi_engine, "public_dei")
                        page.screenshot(path=str(shot), full_page=True)

                    r_inv = page.goto(f"{public_origin}/invite", wait_until="load")
                    inv_status = r_inv.status if r_inv is not None else 0
                    inv_headers = dict(r_inv.headers) if r_inv is not None else {}
                    inv_html = page.content()

                    checks["public_dei_and_invite"] = evaluate_public_dei_check(
                        idx_metrics,
                        idx_status,
                        search_metrics,
                        search_status,
                        inv_status,
                        inv_html,
                        inv_headers,
                        console_errors,
                        page_errors,
                    )
                finally:
                    ctx.close()

            engine_reports[engine] = {
                "driver": "playwright",
                "version": getattr(browser, "version", "unknown"),
                "passed": all(c["passed"] for c in checks.values()),
                "total_checks": len(checks),
                "passed_checks": sum(1 for c in checks.values() if c["passed"]),
                "checks": checks,
            }
        finally:
            browser.close()

    overall_passed = bool(engine_reports) and all(r["passed"] for r in engine_reports.values())
    return {
        "schema_version": 1,
        "overall_passed": overall_passed,
        "browsers": engine_reports,
    }


def _cdp_press_key(
    cdp: _CdpSession,
    key: str,
    code: str,
    vk: int,
    modifiers: int = 0,
) -> None:
    cdp.call(
        "Input.dispatchKeyEvent",
        {
            "type": "rawKeyDown",
            "key": key,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk,
            "modifiers": modifiers,
        },
    )
    cdp.call(
        "Input.dispatchKeyEvent",
        {
            "type": "keyUp",
            "key": key,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk,
            "modifiers": modifiers,
        },
    )


def run_cdp_chromium_ux_qa(
    chromium_bin: Path,
    clinic_base_url: str,
    mobile_base_url: str,
    popup_url: str,
    public_origin: str | None = None,
    screenshot_dir: Path | None = None,
) -> dict[str, Any]:
    """Runs the full WCAG 2.1 AA UX & Accessibility QA via stdlib Chromium CDP."""
    if screenshot_dir is not None:
        screenshot_dir.mkdir(parents=True, exist_ok=True)

    cdp_port = find_free_port()
    user_data_dir = Path(tempfile.mkdtemp(prefix="ef-ux-a11y-cdp-"))
    cmd = [
        str(chromium_bin),
        "--headless=new",
        "--no-sandbox",
        "--disable-gpu-sandbox",
        "--disable-dev-shm-usage",
        f"--remote-debugging-port={cdp_port}",
        f"--user-data-dir={user_data_dir}",
        "about:blank",
    ]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        if not wait_for_tcp_port("127.0.0.1", cdp_port, timeout_secs=15.0):
            raise RuntimeError("Headless Chromium CDP port did not become ready")
        with urllib.request.urlopen(f"http://127.0.0.1:{cdp_port}/json/version", timeout=5) as resp:
            version_info = json.loads(resp.read().decode("utf-8"))
        with urllib.request.urlopen(f"http://127.0.0.1:{cdp_port}/json/list", timeout=5) as resp:
            targets = json.loads(resp.read().decode("utf-8"))
        page_target = next((t for t in targets if t.get("type") == "page"), None)
        if page_target is None or "webSocketDebuggerUrl" not in page_target:
            raise RuntimeError("No debuggable Chromium page target available")

        cdp = _CdpSession(page_target["webSocketDebuggerUrl"])
        try:
            cdp.call("Page.enable")
            cdp.call("Runtime.enable")
            cdp.call("Network.enable")

            checks: dict[str, Any] = {}

            def _set_viewport(w: int, h: int, mobile: bool = False, scale: int = 1) -> None:
                cdp.call(
                    "Emulation.setDeviceMetricsOverride",
                    {
                        "width": w,
                        "height": h,
                        "deviceScaleFactor": scale,
                        "mobile": mobile,
                    },
                )

            def _capture(filename: str, full_page: bool = True) -> None:
                if screenshot_dir is None:
                    return
                shot_data = cdp.call(
                    "Page.captureScreenshot",
                    {"format": "png", "captureBeyondViewport": full_page},
                )
                (screenshot_dir / filename).write_bytes(base64.b64decode(shot_data["data"]))

            # 1. Desktop Clinic Dark & Light Themes + Toggle
            for theme in ("dark", "light"):
                cdp.console_errors.clear()
                cdp.page_errors.clear()
                cdp.last_doc_status = 0
                _set_viewport(DESKTOP_VIEWPORT["width"], DESKTOP_VIEWPORT["height"])
                cdp.call("Page.navigate", {"url": f"{clinic_base_url}?theme={theme}"})
                cdp.wait_for_ready("document.documentElement.dataset.efReady === 'true'")
                metrics = cdp.evaluate(_clinic_theme_probe_js())
                status = cdp.last_doc_status or 200
                _capture(f"clinic_{theme}.png", full_page=True)
                toggle_res = cdp.evaluate(_clinic_theme_toggle_probe_js()) if theme == "dark" else None
                checks[f"clinic_theme_{theme}"] = evaluate_clinic_theme_check(
                    theme, metrics, status, cdp.console_errors, cdp.page_errors, toggle_res
                )

            # 2. Desktop Clinic 200% Text Zoom Reflow
            cdp.console_errors.clear()
            cdp.page_errors.clear()
            cdp.last_doc_status = 0
            _set_viewport(DESKTOP_VIEWPORT["width"], DESKTOP_VIEWPORT["height"])
            cdp.call("Page.navigate", {"url": f"{clinic_base_url}?theme=light"})
            cdp.wait_for_ready("document.documentElement.dataset.efReady === 'true'")
            zoom_metrics = cdp.evaluate(_clinic_zoom200_probe_js())
            _capture("clinic_zoom200.png", full_page=True)
            checks["clinic_zoom_200"] = evaluate_zoom200_check(
                zoom_metrics, cdp.console_errors, cdp.page_errors
            )

            # 3. Desktop Clinic Privacy Cover (Ctrl+Shift+L / Escape) & Shortcuts Modal
            cdp.console_errors.clear()
            cdp.page_errors.clear()
            cdp.last_doc_status = 0
            _set_viewport(DESKTOP_VIEWPORT["width"], DESKTOP_VIEWPORT["height"])
            cdp.call("Page.navigate", {"url": f"{clinic_base_url}?theme=dark"})
            cdp.wait_for_ready("document.documentElement.dataset.efReady === 'true'")
            # Modifiers: Ctrl=2, Shift=8 -> 10
            _cdp_press_key(cdp, "L", "KeyL", 76, modifiers=10)
            after_open_privacy = cdp.evaluate(_privacy_state_js())
            _capture("clinic_privacy_cover.png", full_page=False)
            _cdp_press_key(cdp, "Escape", "Escape", 27, modifiers=0)
            after_escape_privacy = cdp.evaluate(_privacy_state_js())
            cdp.evaluate("document.getElementById('btn-shortcuts').click()")
            after_open_shortcuts = cdp.evaluate(_privacy_state_js())
            _cdp_press_key(cdp, "Escape", "Escape", 27, modifiers=0)
            after_escape_shortcuts = cdp.evaluate(_privacy_state_js())
            checks["clinic_privacy_and_shortcuts"] = evaluate_privacy_and_shortcuts_check(
                after_open_privacy,
                after_escape_privacy,
                after_open_shortcuts,
                after_escape_shortcuts,
                cdp.console_errors,
                cdp.page_errors,
            )

            # 4. Mobile PWA Light & Dark Theme + 390x844 Reflow + Crisis Banner Contrast
            for theme in ("dark", "light"):
                cdp.console_errors.clear()
                cdp.page_errors.clear()
                cdp.last_doc_status = 0
                _set_viewport(MOBILE_VIEWPORT["width"], MOBILE_VIEWPORT["height"], mobile=True, scale=2)
                cdp.call("Page.navigate", {"url": f"{mobile_base_url}?theme={theme}"})
                cdp.wait_for_ready("document.documentElement.dataset.efMobileReady === 'true'")
                metrics = cdp.evaluate(_mobile_theme_probe_js())
                status = cdp.last_doc_status or 200
                if theme == "light":
                    _capture("mobile_light.png", full_page=True)
                checks[f"mobile_theme_{theme}"] = evaluate_mobile_theme_check(
                    theme, metrics, status, cdp.console_errors, cdp.page_errors
                )

            # 5. Chrome MV3 Extension Popup (Dark & Light)
            for theme in ("dark", "light"):
                cdp.console_errors.clear()
                cdp.page_errors.clear()
                cdp.last_doc_status = 0
                _set_viewport(POPUP_VIEWPORT["width"], POPUP_VIEWPORT["height"])
                cdp.call("Page.navigate", {"url": f"{popup_url}?theme={theme}"})
                cdp.wait_for_ready("document.readyState === 'complete' && Boolean(document.getElementById('btn-refresh'))")
                metrics = cdp.evaluate(_extension_popup_probe_js())
                status = cdp.last_doc_status or 200
                if theme == "light":
                    _capture("extension_popup.png", full_page=True)
                checks[f"extension_popup_{theme}"] = evaluate_extension_popup_check(
                    metrics, status, 0, cdp.console_errors, cdp.page_errors
                )

            # 6. Public Site Dei Crisis Finder & /invite noindex
            if public_origin is not None:
                cdp.console_errors.clear()
                cdp.page_errors.clear()
                _set_viewport(DESKTOP_VIEWPORT["width"], DESKTOP_VIEWPORT["height"])

                cdp.last_doc_status = 0
                cdp.call("Page.navigate", {"url": f"{public_origin}/dei"})
                cdp.wait_for_ready("document.readyState === 'complete' && Boolean(document.getElementById('crisis-resources'))")
                idx_status = cdp.last_doc_status or 200
                idx_metrics = cdp.evaluate(_public_dei_probe_js())

                cdp.last_doc_status = 0
                cdp.call("Page.navigate", {"url": f"{public_origin}/dei/search"})
                cdp.wait_for_ready("document.readyState === 'complete' && Boolean(document.getElementById('facilities'))")
                search_status = cdp.last_doc_status or 200
                search_metrics = cdp.evaluate(_public_dei_probe_js())
                _capture("public_dei.png", full_page=True)

                cdp.last_doc_status = 0
                cdp.call("Page.navigate", {"url": f"{public_origin}/invite"})
                cdp.wait_for_ready("document.readyState === 'complete' && Boolean(document.querySelector('meta[name=\"robots\"]'))")
                inv_status = cdp.last_doc_status or 200
                inv_html = str(cdp.evaluate("document.documentElement.outerHTML") or "")
                inv_headers: dict[str, str] = {}

                checks["public_dei_and_invite"] = evaluate_public_dei_check(
                    idx_metrics,
                    idx_status,
                    search_metrics,
                    search_status,
                    inv_status,
                    inv_html,
                    inv_headers,
                    cdp.console_errors,
                    cdp.page_errors,
                )

            engine_report = {
                "driver": "chromium-cdp",
                "version": version_info.get("Browser", "chromium"),
                "passed": all(c["passed"] for c in checks.values()),
                "total_checks": len(checks),
                "passed_checks": sum(1 for c in checks.values() if c["passed"]),
                "checks": checks,
            }
            return {
                "schema_version": 1,
                "overall_passed": engine_report["passed"],
                "browsers": {"chromium": engine_report},
            }
        finally:
            cdp.close()
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
        shutil.rmtree(user_data_dir, ignore_errors=True)


def resolve_public_binary(explicit: Path | None, root: Path) -> Path | None:
    """Returns `--public-bin` if provided, or auto-discovers `target/debug/ef-public-site`."""
    if explicit is not None:
        return explicit
    candidate = root / "target" / "debug" / "ef-public-site"
    if candidate.is_file() and os.access(candidate, os.X_OK):
        return candidate
    return None


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    root = repo_root()
    parser = argparse.ArgumentParser(
        description="Run WCAG 2.1 AA theme contrast, 200% zoom, Privacy Cover, Popup & Dei UX QA."
    )
    parser.add_argument(
        "--browsers",
        default="chromium",
        help="Comma-separated browser engines: chromium,firefox,webkit",
    )
    parser.add_argument("--report", type=Path, default=None, help="Output JSON report path")
    parser.add_argument(
        "--screenshot-dir",
        type=Path,
        default=None,
        help="Optional directory for PNG screenshots",
    )
    parser.add_argument("--web-dir", type=Path, default=root / "web" / "elysianfields")
    parser.add_argument("--extension-dir", type=Path, default=root / "extension")
    parser.add_argument("--clinic-bin", type=Path, default=None)
    parser.add_argument("--mobile-bin", type=Path, default=None)
    parser.add_argument("--public-bin", type=Path, default=None)
    parser.add_argument("--chromium-bin", type=Path, default=None)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    browsers = parse_browsers(args.browsers)
    root = repo_root()

    spawned: list[subprocess.Popen[bytes]] = []
    temp_dirs: list[Path] = []
    clinic_api_origin: str | None = None
    public_origin: str | None = None

    try:
        if args.clinic_bin is not None:
            wal_dir = Path(tempfile.mkdtemp(prefix="ef-ux-clinic-"))
            temp_dirs.append(wal_dir)
            proc, port = _start_optional_binary(
                args.clinic_bin,
                "EF_CLINIC_BIND",
                {"EF_CLINIC_DATA_DIR": str(wal_dir)},
            )
            spawned.append(proc)
            clinic_api_origin = f"http://127.0.0.1:{port}"

        if args.mobile_bin is not None:
            mob_dir = Path(tempfile.mkdtemp(prefix="ef-ux-mobile-"))
            journal_dir = Path(tempfile.mkdtemp(prefix="ef-ux-journal-"))
            temp_dirs.extend([mob_dir, journal_dir])
            proc, _port = _start_optional_binary(
                args.mobile_bin,
                "EF_MOBILE_BIND",
                {
                    "EF_MOBILE_DATA_DIR": str(mob_dir),
                    "EF_MOBILE_JOURNAL_DIR": str(journal_dir),
                },
            )
            spawned.append(proc)

        public_bin = resolve_public_binary(args.public_bin, root)
        if public_bin is not None:
            proc, port = _start_optional_binary(
                public_bin,
                "EF_PUBLIC_BIND",
                {"EF_PUBLIC_MODE": "production"},
            )
            spawned.append(proc)
            public_origin = f"http://127.0.0.1:{port}"

        preview = StrictStaticPreviewServer(
            web_dir=args.web_dir,
            extension_dir=args.extension_dir,
            clinic_api_origin=clinic_api_origin,
        )
        preview.start()
        try:
            clinic_url = f"{preview.origin}/elysianfields/"
            mobile_url = f"{preview.origin}/elysianfields/mobile/"
            popup_url = f"{preview.origin}/mv3-popup/popup.html"

            try:
                from playwright.sync_api import sync_playwright  # type: ignore[import-not-found]

                with sync_playwright() as pw:
                    report = run_playwright_ux_qa(
                        pw,
                        browsers=browsers,
                        clinic_base_url=clinic_url,
                        mobile_base_url=mobile_url,
                        popup_url=popup_url,
                        public_origin=public_origin,
                        screenshot_dir=args.screenshot_dir,
                    )
            except ImportError:
                if browsers != ["chromium"]:
                    raise RuntimeError(
                        "playwright.sync_api is required for multi-browser execution (firefox/webkit)"
                    )
                chromium_bin = locate_chromium_binary(args.chromium_bin)
                if chromium_bin is None:
                    raise RuntimeError(
                        "Neither playwright.sync_api nor a local Chromium binary was found"
                    )
                report = run_cdp_chromium_ux_qa(
                    chromium_bin=chromium_bin,
                    clinic_base_url=clinic_url,
                    mobile_base_url=mobile_url,
                    popup_url=popup_url,
                    public_origin=public_origin,
                    screenshot_dir=args.screenshot_dir,
                )
        finally:
            preview.stop()

        if args.report is not None:
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(
                json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
            )

        for eng, eng_rep in report["browsers"].items():
            status = "PASS" if eng_rep["passed"] else "FAIL"
            print(
                f"[qa_ux_accessibility] {eng} ({eng_rep['driver']}): {status} "
                f"({eng_rep['passed_checks']}/{eng_rep['total_checks']} checks)"
            )
            for name, chk in eng_rep["checks"].items():
                if not chk["passed"]:
                    detail = json.dumps(chk, ensure_ascii=False)
                    print(f"  FAIL {name}: {detail}")
                    if os.environ.get("GITHUB_ACTIONS") == "true":
                        print(f"::error title=UX QA {eng} {name}::{detail[:600]}")

        return 0 if report["overall_passed"] else 1
    finally:
        for proc in spawned:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
        for tmp in temp_dirs:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
