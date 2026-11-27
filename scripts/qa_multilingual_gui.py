#!/usr/bin/env python3
"""Multilingual GUI/UX & Complex-Script Rendering QA Harness for Elysian Fields.

Validates 7 writing systems (`en`, `he`, `ar`, `zh-Hans`, `ja`, `vi`, `hi`) across BOTH
Desktop Clinic (`1280x800`) and Mobile PWA (`390x844`) — 14 rendered GUI views per engine:
  1. `html[lang]`, `html[dir]`, and `getComputedStyle(document.body).direction`.
  2. Real rendered RTL geometry mirroring on Desktop (`nav` right of `main` in `he` & `ar`;
     `nav` left of `main` in LTR scripts).
  3. Zero horizontal overflow (`scrollWidth <= innerWidth + 1`), zero button/heading clipping,
     and Mobile touch targets >= 44 px.
  4. Real non-tofu glyph rendering via Canvas 2D pixel probe comparing script-native clinical
     glyphs against `\\uFFFD` tofu boxes plus bounding-box and computed font telemetry.
  5. Live multilingual clinical composer round-trip & BiDi preservation with 0 console, page,
     and CSP errors.
  6. Optional full-page PNG screenshots (`desktop_<lang>.png`, `mobile_<lang>.png`).

Uses `playwright.sync_api` for multi-engine (`chromium`, `firefox`, `webkit`) qualification
and falls back cleanly to a stdlib Chromium CDP driver when `playwright` is unavailable and
only `chromium` is requested.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import shutil
import socket
import struct
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
    find_free_port,
    parse_browsers,
    repo_root,
    wait_for_tcp_port,
)

DESKTOP_VIEWPORT = {"width": 1280, "height": 800}
MOBILE_VIEWPORT = {"width": 390, "height": 844}
MIN_MOBILE_TOUCH_TARGET_PX = 44.0

MULTILINGUAL_SCENARIOS: tuple[dict[str, str], ...] = (
    {
        "lang": "en",
        "dir": "ltr",
        "script": "Latn",
        "label": "English (Latin LTR)",
        "native_probe": "Glycemic control",
        "composer_sample": "Glycemic control assessment · HbA1c 6.8% · BP 122/78 mmHg",
    },
    {
        "lang": "he",
        "dir": "rtl",
        "script": "Hebr",
        "label": "Hebrew (RTL + Nikud + BiDi)",
        "native_probe": "שָׁלוֹם מְטוּפָּל",
        "composer_sample": "שָׁלוֹם מְטוּפָּל · בדיקת HbA1c 6.8% — BP 122/78",
    },
    {
        "lang": "ar",
        "dir": "rtl",
        "script": "Arab",
        "label": "Arabic (RTL + Harakat + BiDi)",
        "native_probe": "فحصُ السكّري وضغط الدم",
        "composer_sample": "فحصُ السكّري HbA1c 6.8% — ضغط الدم BP 122/78",
    },
    {
        "lang": "zh-Hans",
        "dir": "ltr",
        "script": "Hans",
        "label": "Simplified Chinese (Han CJK)",
        "native_probe": "血糖控制临床评估",
        "composer_sample": "血糖控制临床评估 · HbA1c 6.8% · 血压 122/78 mmHg",
    },
    {
        "lang": "ja",
        "dir": "ltr",
        "script": "Jpan",
        "label": "Japanese (Kanji + Kana)",
        "native_probe": "血糖コントロール評価",
        "composer_sample": "血糖コントロール評価 · HbA1c 6.8% · 血圧 122/78 mmHg",
    },
    {
        "lang": "vi",
        "dir": "ltr",
        "script": "Latn-VN",
        "label": "Vietnamese (Stacked Diacritics)",
        "native_probe": "Nguyễn Thị Hương đĐữệ",
        "composer_sample": "Nguyễn Thị Hương · Đánh giá đường huyết ữ/ệ · HbA1c 6.8%",
    },
    {
        "lang": "hi",
        "dir": "ltr",
        "script": "Deva",
        "label": "Modern Hindi (Devanagari Conjuncts)",
        "native_probe": "चिकित्सा मूल्यांकन रक्तचाप",
        "composer_sample": "चिकित्सा मूल्यांकन · रक्तचाप BP 122/78 · HbA1c 6.8%",
    },
)


def _desktop_probe_js(native_probe: str, composer_sample: str) -> str:
    payload = json.dumps({"nativeProbe": native_probe, "composerSample": composer_sample})
    return f"""(() => {{
        const cfg = {payload};
        const html = document.documentElement;
        const bodyStyle = getComputedStyle(document.body);
        const nav = document.querySelector("nav");
        const main = document.querySelector("main");
        const navRect = nav ? nav.getBoundingClientRect() : null;
        const mainRect = main ? main.getBoundingClientRect() : null;

        const clippedElements = [];
        for (const el of document.querySelectorAll("h1, h2, h3, button, select")) {{
            const st = getComputedStyle(el);
            if (st.display === "none" || st.visibility === "hidden" || el.closest("[hidden], #ef-a11y-overlay, .ef-sr-only")) continue;
            const rect = el.getBoundingClientRect();
            if (rect.width <= 0 || rect.height <= 0) {{
                clippedElements.push({{ tag: el.tagName.toLowerCase(), id: el.id || "", reason: "zero_box" }});
                continue;
            }}
            if (rect.left < -2 || rect.right > window.innerWidth + 2) {{
                clippedElements.push({{ tag: el.tagName.toLowerCase(), id: el.id || "", reason: "viewport_overflow", left: rect.left, right: rect.right }});
            }} else if (el.tagName.toLowerCase() !== "select" && el.clientWidth > 0 && el.scrollWidth > el.clientWidth + 4) {{
                clippedElements.push({{ tag: el.tagName.toLowerCase(), id: el.id || "", reason: "inner_clip", scrollWidth: el.scrollWidth, clientWidth: el.clientWidth }});
            }}
        }}

        function probeCanvas(text, fontFamily) {{
            const c = document.createElement("canvas");
            c.width = 480;
            c.height = 64;
            const ctx = c.getContext("2d");
            ctx.fillStyle = "#ffffff";
            ctx.font = `24px ${{fontFamily}}`;
            ctx.textBaseline = "middle";
            ctx.fillText(text, 8, 32);
            const metrics = ctx.measureText(text);
            const data = ctx.getImageData(0, 0, c.width, c.height).data;
            let inked = 0;
            let hash = 0x811c9dc5 >>> 0;
            for (let i = 3; i < data.length; i += 4) {{
                const alpha = data[i];
                if (alpha > 20) inked += 1;
                hash ^= alpha;
                hash = Math.imul(hash, 0x01000193) >>> 0;
            }}
            return {{
                inkedPixels: inked,
                rasterHash: hash.toString(16).padStart(8, "0"),
                width: Number(metrics.width.toFixed(2)),
                height: Number(((metrics.actualBoundingBoxAscent || 18) + (metrics.actualBoundingBoxDescent || 6)).toFixed(2)),
            }};
        }}

        const fontFamily = bodyStyle.fontFamily || "sans-serif";
        const sampleGlyph = probeCanvas(cfg.nativeProbe, fontFamily);
        const tofuText = "\\uFFFD".repeat(Math.max(4, Math.min(12, Array.from(cfg.nativeProbe).length)));
        const tofuGlyph = probeCanvas(tofuText, fontFamily);

        // Live composer round-trip & BiDi preservation.
        const composer = document.getElementById("composer-draft");
        const form = document.getElementById("composer-form");
        let composerRoundTrip = false;
        let renderedTranscript = "";
        let transcriptBidi = "";
        if (composer && form) {{
            composer.value = cfg.composerSample;
            composer.dispatchEvent(new Event("input", {{ bubbles: true }}));
            form.dispatchEvent(new Event("submit", {{ bubbles: true, cancelable: true }}));
            const items = document.querySelectorAll("#transcript-list li");
            const last = items.length > 0 ? items[items.length - 1] : null;
            renderedTranscript = last ? (last.textContent || "") : "";
            transcriptBidi = last ? (getComputedStyle(last).unicodeBidi || "") : "";
            composerRoundTrip = renderedTranscript.includes(cfg.composerSample);
        }}

        const overlay = document.getElementById("ef-a11y-overlay");
        const bridgeNodes = overlay ? overlay.querySelectorAll("[data-ef-node-id]").length : 0;

        const metricDdDirs = Array.from(document.querySelectorAll(".metric-list dd")).map((el) => getComputedStyle(el).direction);
        const emailEl = document.querySelector('input[type="email"]');
        const emailDir = emailEl ? getComputedStyle(emailEl).direction : "ltr";
        const bidiIsolationOk =
            metricDdDirs.length > 0 &&
            metricDdDirs.every((d) => d === "ltr") &&
            emailDir === "ltr";

        return {{
            lang: html.lang || "",
            dir: html.dir || "",
            computedDirection: bodyStyle.direction || "",
            script: html.dataset.efScript || "",
            wasmRuntime: html.dataset.efWasmRuntime || "static-shell",
            cspViolations: Number(html.dataset.efCspViolations || "0"),
            scrollWidth: html.scrollWidth,
            innerWidth: window.innerWidth,
            navRect: navRect ? {{ left: navRect.left, right: navRect.right, width: navRect.width, height: navRect.height }} : null,
            mainRect: mainRect ? {{ left: mainRect.left, right: mainRect.right, width: mainRect.width, height: mainRect.height }} : null,
            clippedElements,
            fontFamily,
            glyphProbe: {{
                sampleInkedPixels: sampleGlyph.inkedPixels,
                sampleRasterHash: sampleGlyph.rasterHash,
                tofuRasterHash: tofuGlyph.rasterHash,
                distinctFromTofu: sampleGlyph.inkedPixels > 0 && sampleGlyph.rasterHash !== tofuGlyph.rasterHash,
                boundingWidth: sampleGlyph.width,
                boundingHeight: sampleGlyph.height,
            }},
            composerRoundTrip,
            renderedTranscript,
            transcriptBidi,
            bridgeNodes,
            bidiIsolationOk,
        }};
    }})()"""


def _mobile_probe_js(native_probe: str, composer_sample: str) -> str:
    payload = json.dumps({"nativeProbe": native_probe, "composerSample": composer_sample})
    return f"""(() => {{
        const cfg = {payload};
        const html = document.documentElement;
        const bodyStyle = getComputedStyle(document.body);

        const clippedElements = [];
        for (const el of document.querySelectorAll("h1, h2, button, a.touch-btn, select")) {{
            const st = getComputedStyle(el);
            if (st.display === "none" || st.visibility === "hidden" || el.closest("[hidden], #ef-a11y-overlay, .ef-sr-only")) continue;
            const rect = el.getBoundingClientRect();
            if (rect.width <= 0 || rect.height <= 0) {{
                clippedElements.push({{ tag: el.tagName.toLowerCase(), id: el.id || "", reason: "zero_box" }});
                continue;
            }}
            if (rect.left < -2 || rect.right > window.innerWidth + 2) {{
                clippedElements.push({{ tag: el.tagName.toLowerCase(), id: el.id || "", reason: "viewport_overflow", left: rect.left, right: rect.right }});
            }} else if (el.tagName.toLowerCase() !== "select" && el.clientWidth > 0 && el.scrollWidth > el.clientWidth + 4) {{
                clippedElements.push({{ tag: el.tagName.toLowerCase(), id: el.id || "", reason: "inner_clip", scrollWidth: el.scrollWidth, clientWidth: el.clientWidth }});
            }}
        }}

        const touchTargets = [];
        for (const el of document.querySelectorAll(".touch-btn, .touch-select, .touch-input, .touch-textarea")) {{
            const st = getComputedStyle(el);
            if (st.display === "none" || st.visibility === "hidden") continue;
            const rect = el.getBoundingClientRect();
            touchTargets.push({{
                id: el.id || el.tagName.toLowerCase(),
                height: Number(rect.height.toFixed(2)),
                width: Number(rect.width.toFixed(2)),
            }});
        }}

        function probeCanvas(text, fontFamily) {{
            const c = document.createElement("canvas");
            c.width = 480;
            c.height = 64;
            const ctx = c.getContext("2d");
            ctx.fillStyle = "#ffffff";
            ctx.font = `24px ${{fontFamily}}`;
            ctx.textBaseline = "middle";
            ctx.fillText(text, 8, 32);
            const metrics = ctx.measureText(text);
            const data = ctx.getImageData(0, 0, c.width, c.height).data;
            let inked = 0;
            let hash = 0x811c9dc5 >>> 0;
            for (let i = 3; i < data.length; i += 4) {{
                const alpha = data[i];
                if (alpha > 20) inked += 1;
                hash ^= alpha;
                hash = Math.imul(hash, 0x01000193) >>> 0;
            }}
            return {{
                inkedPixels: inked,
                rasterHash: hash.toString(16).padStart(8, "0"),
                width: Number(metrics.width.toFixed(2)),
                height: Number(((metrics.actualBoundingBoxAscent || 18) + (metrics.actualBoundingBoxDescent || 6)).toFixed(2)),
            }};
        }}

        const fontFamily = bodyStyle.fontFamily || "sans-serif";
        const sampleGlyph = probeCanvas(cfg.nativeProbe, fontFamily);
        const tofuText = "\\uFFFD".repeat(Math.max(4, Math.min(12, Array.from(cfg.nativeProbe).length)));
        const tofuGlyph = probeCanvas(tofuText, fontFamily);

        // Live mobile composer round-trip & BiDi preservation.
        const note = document.getElementById("doctor-note");
        const form = document.getElementById("doctor-note-form");
        let composerRoundTrip = false;
        let renderedNote = "";
        if (note && form) {{
            note.value = cfg.composerSample;
            note.dispatchEvent(new Event("input", {{ bubbles: true }}));
            form.dispatchEvent(new Event("submit", {{ bubbles: true, cancelable: true }}));
            const items = document.querySelectorAll("#doctor-sent-list li");
            const last = items.length > 0 ? items[items.length - 1] : null;
            renderedNote = last ? (last.textContent || "") : "";
            composerRoundTrip = renderedNote.includes(cfg.composerSample);
        }}

        const bodyText = document.body.innerText || "";
        const crisisLinesPresent = bodyText.includes("988") && bodyText.includes("911") && bodyText.includes("SAMHSA");
        const overlay = document.getElementById("ef-a11y-overlay");
        const bridgeNodes = overlay ? overlay.querySelectorAll("[data-ef-node-id]").length : 0;

        const metricDdDirs = Array.from(document.querySelectorAll(".metric-grid dd")).map((el) => getComputedStyle(el).direction);
        const telLinkDirs = Array.from(document.querySelectorAll('a[href^="tel:"]')).map(
            (el) => el.getAttribute("dir") === "ltr" && getComputedStyle(el).direction === "ltr",
        );
        const structuredInputDirs = Array.from(
            document.querySelectorAll('input[type="email"], #invite-code, #verify-dob'),
        ).map((el) => getComputedStyle(el).direction === "ltr");
        const bidiIsolationOk =
            metricDdDirs.length > 0 &&
            metricDdDirs.every((d) => d === "ltr") &&
            telLinkDirs.length >= 3 &&
            telLinkDirs.every(Boolean) &&
            structuredInputDirs.length >= 3 &&
            structuredInputDirs.every(Boolean);

        return {{
            lang: html.lang || "",
            dir: html.dir || "",
            computedDirection: bodyStyle.direction || "",
            script: html.dataset.efScript || "",
            wasmRuntime: html.dataset.efWasmRuntime || "static-shell",
            cspViolations: Number(html.dataset.efCspViolations || "0"),
            scrollWidth: html.scrollWidth,
            innerWidth: window.innerWidth,
            clippedElements,
            touchTargets,
            fontFamily,
            glyphProbe: {{
                sampleInkedPixels: sampleGlyph.inkedPixels,
                sampleRasterHash: sampleGlyph.rasterHash,
                tofuRasterHash: tofuGlyph.rasterHash,
                distinctFromTofu: sampleGlyph.inkedPixels > 0 && sampleGlyph.rasterHash !== tofuGlyph.rasterHash,
                boundingWidth: sampleGlyph.width,
                boundingHeight: sampleGlyph.height,
            }},
            composerRoundTrip,
            renderedNote,
            crisisLinesPresent,
            bridgeNodes,
            bidiIsolationOk,
        }};
    }})()"""


def evaluate_desktop_view(
    scenario: dict[str, str],
    metrics: dict[str, Any],
    http_status: int,
    console_errors: list[str],
    page_errors: list[str],
) -> dict[str, Any]:
    """Evaluates all multilingual desktop Clinic GUI/UX invariants for one script."""
    expected_lang = scenario["lang"]
    expected_dir = scenario["dir"]

    nav_rect = metrics.get("navRect")
    main_rect = metrics.get("mainRect")
    if nav_rect and main_rect:
        if expected_dir == "rtl":
            rtl_mirror_ok = float(nav_rect["left"]) >= float(main_rect["right"]) - 2.0
        else:
            rtl_mirror_ok = float(nav_rect["right"]) <= float(main_rect["left"]) + 2.0
    else:
        rtl_mirror_ok = False

    zero_overflow = int(metrics.get("scrollWidth", 99999)) <= int(metrics.get("innerWidth", 0)) + 1
    clipped = metrics.get("clippedElements", [])
    zero_clipping = len(clipped) == 0
    glyph = metrics.get("glyphProbe", {})
    non_tofu_ok = bool(glyph.get("distinctFromTofu")) and int(glyph.get("sampleInkedPixels", 0)) > 0
    composer_ok = bool(metrics.get("composerRoundTrip"))
    bidi_isolation_ok = bool(metrics.get("bidiIsolationOk"))

    passed = (
        http_status == 200
        and len(console_errors) == 0
        and len(page_errors) == 0
        and int(metrics.get("cspViolations", 1)) == 0
        and metrics.get("lang") == expected_lang
        and metrics.get("dir") == expected_dir
        and metrics.get("computedDirection") == expected_dir
        and rtl_mirror_ok
        and zero_overflow
        and zero_clipping
        and non_tofu_ok
        and composer_ok
        and bidi_isolation_ok
        and int(metrics.get("bridgeNodes", 0)) >= 5
    )

    return {
        "surface": "desktop",
        "lang": expected_lang,
        "expected_dir": expected_dir,
        "script": scenario["script"],
        "label": scenario["label"],
        "passed": passed,
        "http_status": http_status,
        "actual_lang": metrics.get("lang"),
        "actual_dir": metrics.get("dir"),
        "computed_direction": metrics.get("computedDirection"),
        "rtl_mirror_ok": rtl_mirror_ok,
        "bidi_isolation_ok": bidi_isolation_ok,
        "nav_rect": nav_rect,
        "main_rect": main_rect,
        "zero_overflow": zero_overflow,
        "scroll_width": metrics.get("scrollWidth"),
        "inner_width": metrics.get("innerWidth"),
        "zero_clipping": zero_clipping,
        "clipped_elements": clipped,
        "font_family": metrics.get("fontFamily"),
        "glyph_probe": glyph,
        "non_tofu_ok": non_tofu_ok,
        "composer_round_trip": composer_ok,
        "transcript_bidi": metrics.get("transcriptBidi"),
        "bridge_nodes": metrics.get("bridgeNodes"),
        "wasm_runtime": metrics.get("wasmRuntime"),
        "csp_violations": metrics.get("cspViolations"),
        "console_errors": list(console_errors),
        "page_errors": list(page_errors),
    }


def evaluate_mobile_view(
    scenario: dict[str, str],
    metrics: dict[str, Any],
    http_status: int,
    console_errors: list[str],
    page_errors: list[str],
) -> dict[str, Any]:
    """Evaluates all multilingual mobile PWA GUI/UX invariants for one script."""
    expected_lang = scenario["lang"]
    expected_dir = scenario["dir"]

    zero_overflow = int(metrics.get("scrollWidth", 99999)) <= int(metrics.get("innerWidth", 0)) + 1
    clipped = metrics.get("clippedElements", [])
    zero_clipping = len(clipped) == 0

    touch_targets = metrics.get("touchTargets", [])
    undersized_targets = [
        t for t in touch_targets if float(t.get("height", 0.0)) < (MIN_MOBILE_TOUCH_TARGET_PX - 0.5)
    ]
    touch_targets_ok = len(touch_targets) > 0 and len(undersized_targets) == 0

    glyph = metrics.get("glyphProbe", {})
    non_tofu_ok = bool(glyph.get("distinctFromTofu")) and int(glyph.get("sampleInkedPixels", 0)) > 0
    composer_ok = bool(metrics.get("composerRoundTrip"))
    crisis_ok = bool(metrics.get("crisisLinesPresent"))
    bidi_isolation_ok = bool(metrics.get("bidiIsolationOk"))

    passed = (
        http_status == 200
        and len(console_errors) == 0
        and len(page_errors) == 0
        and int(metrics.get("cspViolations", 1)) == 0
        and metrics.get("lang") == expected_lang
        and metrics.get("dir") == expected_dir
        and metrics.get("computedDirection") == expected_dir
        and zero_overflow
        and zero_clipping
        and touch_targets_ok
        and non_tofu_ok
        and composer_ok
        and crisis_ok
        and bidi_isolation_ok
        and int(metrics.get("bridgeNodes", 0)) >= 5
    )

    return {
        "surface": "mobile",
        "lang": expected_lang,
        "expected_dir": expected_dir,
        "script": scenario["script"],
        "label": scenario["label"],
        "passed": passed,
        "http_status": http_status,
        "actual_lang": metrics.get("lang"),
        "actual_dir": metrics.get("dir"),
        "computed_direction": metrics.get("computedDirection"),
        "bidi_isolation_ok": bidi_isolation_ok,
        "zero_overflow": zero_overflow,
        "scroll_width": metrics.get("scrollWidth"),
        "inner_width": metrics.get("innerWidth"),
        "zero_clipping": zero_clipping,
        "clipped_elements": clipped,
        "touch_targets_ok": touch_targets_ok,
        "touch_target_count": len(touch_targets),
        "undersized_touch_targets": undersized_targets,
        "font_family": metrics.get("fontFamily"),
        "glyph_probe": glyph,
        "non_tofu_ok": non_tofu_ok,
        "composer_round_trip": composer_ok,
        "crisis_lines_present": crisis_ok,
        "bridge_nodes": metrics.get("bridgeNodes"),
        "wasm_runtime": metrics.get("wasmRuntime"),
        "csp_violations": metrics.get("cspViolations"),
        "console_errors": list(console_errors),
        "page_errors": list(page_errors),
    }


def _screenshot_filename(engine: str, multi_engine: bool, surface: str, lang: str) -> str:
    safe_lang = lang.replace("/", "_")
    if multi_engine and engine != "chromium":
        return f"{engine}_{surface}_{safe_lang}.png"
    return f"{surface}_{safe_lang}.png"


def run_playwright_multilingual_qa(
    playwright_obj: Any,
    browsers: list[str],
    clinic_base_url: str,
    mobile_base_url: str,
    screenshot_dir: Path | None = None,
) -> dict[str, Any]:
    """Runs the 7-script × Desktop/Mobile QA suite across all requested Playwright engines."""
    if screenshot_dir is not None:
        screenshot_dir.mkdir(parents=True, exist_ok=True)

    multi_engine = len(browsers) > 1
    engine_reports: dict[str, Any] = {}

    for engine in browsers:
        browser_type = getattr(playwright_obj, engine)
        browser = browser_type.launch(headless=True)
        try:
            desktop_views: list[dict[str, Any]] = []
            mobile_views: list[dict[str, Any]] = []

            for scenario in MULTILINGUAL_SCENARIOS:
                lang = scenario["lang"]

                # Desktop Clinic view (1280x800)
                d_ctx = browser.new_context(viewport=DESKTOP_VIEWPORT)
                try:
                    page = d_ctx.new_page()
                    console_errors: list[str] = []
                    page_errors: list[str] = []
                    page.on(
                        "console",
                        lambda m, errs=console_errors: errs.append(m.text) if m.type == "error" else None,
                    )
                    page.on("pageerror", lambda e, errs=page_errors: errs.append(str(e)))
                    url = f"{clinic_base_url}?lang={lang}"
                    resp = page.goto(url, wait_until="networkidle")
                    status = resp.status if resp is not None else 0
                    page.wait_for_function(
                        "() => document.documentElement.dataset.efReady === 'true'",
                        timeout=20_000,
                    )
                    metrics = page.evaluate(
                        _desktop_probe_js(scenario["native_probe"], scenario["composer_sample"])
                    )
                    view = evaluate_desktop_view(scenario, metrics, status, console_errors, page_errors)
                    if screenshot_dir is not None:
                        shot_path = screenshot_dir / _screenshot_filename(engine, multi_engine, "desktop", lang)
                        page.screenshot(path=str(shot_path), full_page=True)
                        view["screenshot"] = str(shot_path)
                    desktop_views.append(view)
                finally:
                    d_ctx.close()

                # Mobile PWA view (390x844)
                m_ctx = browser.new_context(
                    viewport=MOBILE_VIEWPORT,
                    has_touch=True,
                    is_mobile=True,
                )
                try:
                    page = m_ctx.new_page()
                    console_errors = []
                    page_errors = []
                    page.on(
                        "console",
                        lambda m, errs=console_errors: errs.append(m.text) if m.type == "error" else None,
                    )
                    page.on("pageerror", lambda e, errs=page_errors: errs.append(str(e)))
                    url = f"{mobile_base_url}?lang={lang}"
                    resp = page.goto(url, wait_until="networkidle")
                    status = resp.status if resp is not None else 0
                    page.wait_for_function(
                        "() => document.documentElement.dataset.efMobileReady === 'true'",
                        timeout=20_000,
                    )
                    metrics = page.evaluate(
                        _mobile_probe_js(scenario["native_probe"], scenario["composer_sample"])
                    )
                    view = evaluate_mobile_view(scenario, metrics, status, console_errors, page_errors)
                    if screenshot_dir is not None:
                        shot_path = screenshot_dir / _screenshot_filename(engine, multi_engine, "mobile", lang)
                        page.screenshot(path=str(shot_path), full_page=True)
                        view["screenshot"] = str(shot_path)
                    mobile_views.append(view)
                finally:
                    m_ctx.close()

            all_views = desktop_views + mobile_views
            engine_reports[engine] = {
                "driver": "playwright",
                "version": getattr(browser, "version", "unknown"),
                "passed": all(v["passed"] for v in all_views),
                "total_views": len(all_views),
                "passed_views": sum(1 for v in all_views if v["passed"]),
                "desktop": desktop_views,
                "mobile": mobile_views,
            }
        finally:
            browser.close()

    overall_passed = bool(engine_reports) and all(r["passed"] for r in engine_reports.values())
    return {
        "schema_version": 1,
        "scripts_tested": [s["lang"] for s in MULTILINGUAL_SCENARIOS],
        "overall_passed": overall_passed,
        "browsers": engine_reports,
    }


def locate_chromium_binary(explicit: Path | None = None) -> Path | None:
    """Locates a local Chromium binary (explicit path, PATH, or Playwright cache)."""
    if explicit is not None and explicit.is_file():
        return explicit
    for name in ("chromium", "chromium-browser", "google-chrome", "microsoft-edge-stable"):
        found = shutil.which(name)
        if found:
            return Path(found)
    cache_root = Path.home() / ".cache" / "ms-playwright"
    if cache_root.is_dir():
        for pattern in ("chromium-*/chrome-linux/chrome", "chromium_headless_shell-*/chrome-linux/headless_shell"):
            matches = sorted(cache_root.glob(pattern), reverse=True)
            for candidate in matches:
                if candidate.is_file() and os.access(candidate, os.X_OK):
                    return candidate
    return None


class _MinimalWsClient:
    """Minimal RFC 6455 localhost WebSocket client using Python stdlib only (for CDP fallback)."""

    def __init__(self, ws_url: str, timeout: float = 20.0) -> None:
        if not ws_url.startswith("ws://"):
            raise ValueError(f"Unsupported WebSocket URL: {ws_url}")
        rest = ws_url[5:]
        host_port, path = rest.split("/", 1)
        path = "/" + path
        host, port_s = host_port.split(":", 1)
        self._sock = socket.create_connection((host, int(port_s)), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        req = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {host_port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        self._sock.sendall(req.encode("ascii"))
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = self._sock.recv(4096)
            if not chunk:
                raise RuntimeError("CDP WebSocket handshake closed unexpectedly")
            buf += chunk
        header_end = buf.index(b"\r\n\r\n") + 4
        head = buf[:header_end].decode("ascii", errors="replace")
        if " 101 " not in head.splitlines()[0]:
            raise RuntimeError(f"CDP WebSocket upgrade failed: {head.splitlines()[0]}")
        self._leftover = buf[header_end:]

    def _read_exact(self, n: int) -> bytes:
        chunks = []
        if self._leftover:
            take = min(n, len(self._leftover))
            chunks.append(self._leftover[:take])
            self._leftover = self._leftover[take:]
            n -= take
        while n > 0:
            chunk = self._sock.recv(n)
            if not chunk:
                raise RuntimeError("CDP WebSocket closed while reading frame")
            chunks.append(chunk)
            n -= len(chunk)
        return b"".join(chunks)

    def send_text(self, text: str) -> None:
        payload = text.encode("utf-8")
        mask = os.urandom(4)
        length = len(payload)
        header = bytearray([0x81])
        if length < 126:
            header.append(0x80 | length)
        elif length < 65536:
            header.append(0x80 | 126)
            header.extend(struct.pack("!H", length))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack("!Q", length))
        header.extend(mask)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self._sock.sendall(bytes(header) + masked)

    def recv_text(self) -> str:
        fragments: list[bytes] = []
        while True:
            hdr = self._read_exact(2)
            fin = (hdr[0] & 0x80) != 0
            opcode = hdr[0] & 0x0F
            masked = (hdr[1] & 0x80) != 0
            length = hdr[1] & 0x7F
            if length == 126:
                length = struct.unpack("!H", self._read_exact(2))[0]
            elif length == 127:
                length = struct.unpack("!Q", self._read_exact(8))[0]
            mask = self._read_exact(4) if masked else b""
            payload = self._read_exact(length)
            if masked:
                payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
            if opcode == 0x9:  # ping -> pong
                self._sock.sendall(b"\x8a\x00")
                continue
            if opcode == 0x8:  # close
                raise RuntimeError("CDP WebSocket closed by browser")
            fragments.append(payload)
            if fin:
                return b"".join(fragments).decode("utf-8")

    def close(self) -> None:
        try:
            self._sock.close()
        except OSError:
            pass


class _CdpSession:
    """Thin stdlib Chrome DevTools Protocol client for local headless Chromium QA."""

    def __init__(self, ws_url: str) -> None:
        self._ws = _MinimalWsClient(ws_url)
        self._next_id = 0
        self.console_errors: list[str] = []
        self.page_errors: list[str] = []
        self.last_doc_status: int = 0

    def call(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        self._next_id += 1
        req_id = self._next_id
        self._ws.send_text(json.dumps({"id": req_id, "method": method, "params": params or {}}))
        while True:
            msg = json.loads(self._ws.recv_text())
            if msg.get("id") == req_id:
                if "error" in msg:
                    raise RuntimeError(f"CDP error on {method}: {msg['error']}")
                return msg.get("result", {})
            self._handle_event(msg)

    def _handle_event(self, msg: dict[str, Any]) -> None:
        method = msg.get("method")
        params = msg.get("params", {})
        if method == "Runtime.consoleAPICalled" and params.get("type") == "error":
            args = [str(a.get("value", a.get("description", ""))) for a in params.get("args", [])]
            self.console_errors.append(" ".join(args))
        elif method == "Runtime.exceptionThrown":
            details = params.get("exceptionDetails", {})
            self.page_errors.append(str(details.get("text", "uncaught exception")))
        elif method == "Network.responseReceived" and params.get("type") == "Document":
            status = params.get("response", {}).get("status")
            if isinstance(status, int):
                self.last_doc_status = status

    def evaluate(self, expression: str) -> Any:
        res = self.call(
            "Runtime.evaluate",
            {"expression": expression, "returnByValue": True, "awaitPromise": True},
        )
        if "exceptionDetails" in res:
            raise RuntimeError(f"JS exception: {res['exceptionDetails']}")
        return res.get("result", {}).get("value")

    def wait_for_ready(self, flag_expr: str, timeout_secs: float = 15.0) -> None:
        deadline = time.monotonic() + timeout_secs
        while time.monotonic() < deadline:
            if self.evaluate(flag_expr):
                return
            time.sleep(0.08)
        raise TimeoutError(f"Timed out waiting for {flag_expr}")

    def close(self) -> None:
        self._ws.close()


def run_cdp_chromium_multilingual_qa(
    chromium_bin: Path,
    clinic_base_url: str,
    mobile_base_url: str,
    screenshot_dir: Path | None = None,
) -> dict[str, Any]:
    """Executes all 7 scripts × Desktop/Mobile live via stdlib CDP against local Chromium."""
    if screenshot_dir is not None:
        screenshot_dir.mkdir(parents=True, exist_ok=True)

    cdp_port = find_free_port()
    user_data_dir = Path(tempfile.mkdtemp(prefix="ef-multilingual-cdp-"))
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

            desktop_views: list[dict[str, Any]] = []
            mobile_views: list[dict[str, Any]] = []

            for scenario in MULTILINGUAL_SCENARIOS:
                lang = scenario["lang"]

                # Desktop Clinic (1280x800)
                cdp.console_errors.clear()
                cdp.page_errors.clear()
                cdp.last_doc_status = 0
                cdp.call(
                    "Emulation.setDeviceMetricsOverride",
                    {
                        "width": DESKTOP_VIEWPORT["width"],
                        "height": DESKTOP_VIEWPORT["height"],
                        "deviceScaleFactor": 1,
                        "mobile": False,
                    },
                )
                cdp.call("Page.navigate", {"url": f"{clinic_base_url}?lang={lang}"})
                cdp.wait_for_ready("document.documentElement.dataset.efReady === 'true'")
                metrics = cdp.evaluate(_desktop_probe_js(scenario["native_probe"], scenario["composer_sample"]))
                status = cdp.last_doc_status or 200
                view = evaluate_desktop_view(scenario, metrics, status, cdp.console_errors, cdp.page_errors)
                if screenshot_dir is not None:
                    shot_data = cdp.call(
                        "Page.captureScreenshot",
                        {"format": "png", "captureBeyondViewport": True},
                    )
                    shot_path = screenshot_dir / f"desktop_{lang}.png"
                    shot_path.write_bytes(base64.b64decode(shot_data["data"]))
                    view["screenshot"] = str(shot_path)
                desktop_views.append(view)

                # Mobile PWA (390x844)
                cdp.console_errors.clear()
                cdp.page_errors.clear()
                cdp.last_doc_status = 0
                cdp.call(
                    "Emulation.setDeviceMetricsOverride",
                    {
                        "width": MOBILE_VIEWPORT["width"],
                        "height": MOBILE_VIEWPORT["height"],
                        "deviceScaleFactor": 2,
                        "mobile": True,
                    },
                )
                cdp.call("Page.navigate", {"url": f"{mobile_base_url}?lang={lang}"})
                cdp.wait_for_ready("document.documentElement.dataset.efMobileReady === 'true'")
                metrics = cdp.evaluate(_mobile_probe_js(scenario["native_probe"], scenario["composer_sample"]))
                status = cdp.last_doc_status or 200
                view = evaluate_mobile_view(scenario, metrics, status, cdp.console_errors, cdp.page_errors)
                if screenshot_dir is not None:
                    shot_data = cdp.call(
                        "Page.captureScreenshot",
                        {"format": "png", "captureBeyondViewport": True},
                    )
                    shot_path = screenshot_dir / f"mobile_{lang}.png"
                    shot_path.write_bytes(base64.b64decode(shot_data["data"]))
                    view["screenshot"] = str(shot_path)
                mobile_views.append(view)

            all_views = desktop_views + mobile_views
            engine_report = {
                "driver": "chromium-cdp",
                "version": version_info.get("Browser", "chromium"),
                "passed": all(v["passed"] for v in all_views),
                "total_views": len(all_views),
                "passed_views": sum(1 for v in all_views if v["passed"]),
                "desktop": desktop_views,
                "mobile": mobile_views,
            }
            return {
                "schema_version": 1,
                "scripts_tested": [s["lang"] for s in MULTILINGUAL_SCENARIOS],
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


def _start_optional_binary(
    binary: Path,
    env_bind_key: str,
    extra_env: dict[str, str],
) -> tuple[subprocess.Popen[bytes], int]:
    port = find_free_port()
    env = os.environ.copy()
    env[env_bind_key] = f"127.0.0.1:{port}"
    env.update(extra_env)
    proc = subprocess.Popen(
        [str(binary)],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if not wait_for_tcp_port("127.0.0.1", port, timeout_secs=15.0):
        proc.terminate()
        raise RuntimeError(f"Server binary {binary} did not bind on 127.0.0.1:{port}")
    return proc, port


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    root = repo_root()
    parser = argparse.ArgumentParser(
        description="Run 7-script multilingual GUI/UX & complex-script rendering QA."
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
        help="Optional directory for full-page PNG screenshots (desktop_<lang>.png, mobile_<lang>.png)",
    )
    parser.add_argument("--web-dir", type=Path, default=root / "web" / "elysianfields")
    parser.add_argument("--extension-dir", type=Path, default=root / "extension")
    parser.add_argument("--clinic-wasm", type=Path, default=None)
    parser.add_argument("--mobile-wasm", type=Path, default=None)
    parser.add_argument("--clinic-bin", type=Path, default=None)
    parser.add_argument("--mobile-bin", type=Path, default=None)
    parser.add_argument("--chromium-bin", type=Path, default=None)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    browsers = parse_browsers(args.browsers)

    spawned: list[subprocess.Popen[bytes]] = []
    temp_dirs: list[Path] = []
    clinic_api_origin: str | None = None

    try:
        if args.clinic_bin is not None:
            wal_dir = Path(tempfile.mkdtemp(prefix="ef-qa-clinic-"))
            temp_dirs.append(wal_dir)
            proc, port = _start_optional_binary(
                args.clinic_bin,
                "EF_CLINIC_BIND",
                {"EF_CLINIC_DATA_DIR": str(wal_dir)},
            )
            spawned.append(proc)
            clinic_api_origin = f"http://127.0.0.1:{port}"

        if args.mobile_bin is not None:
            mob_dir = Path(tempfile.mkdtemp(prefix="ef-qa-mobile-"))
            journal_dir = Path(tempfile.mkdtemp(prefix="ef-qa-journal-"))
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

        preview = StrictStaticPreviewServer(
            web_dir=args.web_dir,
            extension_dir=args.extension_dir,
            clinic_api_origin=clinic_api_origin,
            clinic_wasm=args.clinic_wasm,
            mobile_wasm=args.mobile_wasm,
        )
        preview.start()
        try:
            clinic_url = f"{preview.origin}/elysianfields/"
            mobile_url = f"{preview.origin}/elysianfields/mobile/"

            try:
                from playwright.sync_api import sync_playwright  # type: ignore[import-not-found]

                with sync_playwright() as pw:
                    report = run_playwright_multilingual_qa(
                        pw,
                        browsers=browsers,
                        clinic_base_url=clinic_url,
                        mobile_base_url=mobile_url,
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
                report = run_cdp_chromium_multilingual_qa(
                    chromium_bin=chromium_bin,
                    clinic_base_url=clinic_url,
                    mobile_base_url=mobile_url,
                    screenshot_dir=args.screenshot_dir,
                )
        finally:
            preview.stop()

        if args.report is not None:
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

        for eng, eng_rep in report["browsers"].items():
            status = "PASS" if eng_rep["passed"] else "FAIL"
            print(
                f"[qa_multilingual_gui] {eng} ({eng_rep['driver']}): {status} "
                f"({eng_rep['passed_views']}/{eng_rep['total_views']} views)"
            )
            for view in eng_rep["desktop"] + eng_rep["mobile"]:
                if not view["passed"]:
                    print(f"  FAIL {view['surface']} lang={view['lang']}: {json.dumps(view, ensure_ascii=False)}")

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
