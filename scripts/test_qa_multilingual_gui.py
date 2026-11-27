#!/usr/bin/env python3
"""Unit tests for `scripts/qa_multilingual_gui.py` and the upgraded multilingual web shells."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

import qa_multilingual_gui as qa  # noqa: E402


def _make_desktop_metrics(scenario: dict[str, str], **overrides: Any) -> dict[str, Any]:
    is_rtl = scenario["dir"] == "rtl"
    base: dict[str, Any] = {
        "lang": scenario["lang"],
        "dir": scenario["dir"],
        "computedDirection": scenario["dir"],
        "script": scenario["script"],
        "wasmRuntime": "static-shell",
        "cspViolations": 0,
        "scrollWidth": 1280,
        "innerWidth": 1280,
        "navRect": (
            {"left": 980.0, "right": 1256.0, "width": 276.0, "height": 520.0}
            if is_rtl
            else {"left": 24.0, "right": 300.0, "width": 276.0, "height": 520.0}
        ),
        "mainRect": (
            {"left": 24.0, "right": 964.0, "width": 940.0, "height": 520.0}
            if is_rtl
            else {"left": 316.0, "right": 1256.0, "width": 940.0, "height": 520.0}
        ),
        "clippedElements": [],
        "fontFamily": '"Noto Sans", "Noto Sans Hebrew", "Noto Sans Arabic", sans-serif',
        "glyphProbe": {
            "sampleInkedPixels": 412,
            "sampleRasterHash": "a1b2c3d4",
            "tofuRasterHash": "deadbeef",
            "distinctFromTofu": True,
            "boundingWidth": 182.5,
            "boundingHeight": 26.0,
        },
        "composerRoundTrip": True,
        "renderedTranscript": scenario["composer_sample"],
        "transcriptBidi": "plaintext",
        "bridgeNodes": 6,
        "bidiIsolationOk": True,
    }
    base.update(overrides)
    return base


def _make_mobile_metrics(scenario: dict[str, str], **overrides: Any) -> dict[str, Any]:
    base: dict[str, Any] = {
        "lang": scenario["lang"],
        "dir": scenario["dir"],
        "computedDirection": scenario["dir"],
        "script": scenario["script"],
        "wasmRuntime": "static-shell",
        "cspViolations": 0,
        "scrollWidth": 390,
        "innerWidth": 390,
        "clippedElements": [],
        "touchTargets": [
            {"id": "locale-select", "height": 44.0, "width": 180.0},
            {"id": "btn-toggle-lang", "height": 44.0, "width": 120.0},
            {"id": "btn-send-doctor", "height": 46.0, "width": 320.0},
        ],
        "fontFamily": '"Noto Sans", "Noto Sans Devanagari", sans-serif',
        "glyphProbe": {
            "sampleInkedPixels": 398,
            "sampleRasterHash": "11223344",
            "tofuRasterHash": "99887766",
            "distinctFromTofu": True,
            "boundingWidth": 164.0,
            "boundingHeight": 25.0,
        },
        "composerRoundTrip": True,
        "renderedNote": scenario["composer_sample"],
        "crisisLinesPresent": True,
        "bridgeNodes": 6,
        "bidiIsolationOk": True,
    }
    base.update(overrides)
    return base


class _FakePage:
    def __init__(self, surface: str) -> None:
        self.surface = surface
        self._lang = "en"

    def on(self, _event: str, _handler: Any) -> None:
        pass

    def goto(self, url: str, wait_until: str = "networkidle") -> Any:
        if "?lang=" in url:
            self._lang = url.split("?lang=", 1)[1]

        class _Resp:
            status = 200

        return _Resp()

    def wait_for_function(self, _fn: str, timeout: int = 20_000) -> None:
        pass

    def evaluate(self, _expr: str) -> dict[str, Any]:
        scenario = next(s for s in qa.MULTILINGUAL_SCENARIOS if s["lang"] == self._lang)
        if self.surface == "desktop":
            return _make_desktop_metrics(scenario)
        return _make_mobile_metrics(scenario)

    def screenshot(self, path: str, full_page: bool = True) -> None:
        Path(path).write_bytes(b"\x89PNG\r\n\x1a\nfake")


class _FakeContext:
    def __init__(self, surface: str) -> None:
        self.surface = surface

    def new_page(self) -> _FakePage:
        return _FakePage(self.surface)

    def close(self) -> None:
        pass


class _FakeBrowser:
    version = "mock-1.0"

    def new_context(self, **kwargs: Any) -> _FakeContext:
        surface = "mobile" if kwargs.get("is_mobile") else "desktop"
        return _FakeContext(surface)

    def close(self) -> None:
        pass


class _FakeBrowserType:
    def launch(self, headless: bool = True) -> _FakeBrowser:
        return _FakeBrowser()


class _FakePlaywright:
    chromium = _FakeBrowserType()
    firefox = _FakeBrowserType()
    webkit = _FakeBrowserType()


class MultilingualQaTests(unittest.TestCase):
    def test_covers_all_seven_required_writing_systems(self) -> None:
        langs = [s["lang"] for s in qa.MULTILINGUAL_SCENARIOS]
        self.assertEqual(langs, ["en", "he", "ar", "zh-Hans", "ja", "vi", "hi"])
        rtl_langs = [s["lang"] for s in qa.MULTILINGUAL_SCENARIOS if s["dir"] == "rtl"]
        self.assertEqual(rtl_langs, ["he", "ar"])

    def test_desktop_rtl_mirror_and_tofu_and_overflow_invariants(self) -> None:
        he_scenario = next(s for s in qa.MULTILINGUAL_SCENARIOS if s["lang"] == "he")
        good = qa.evaluate_desktop_view(he_scenario, _make_desktop_metrics(he_scenario), 200, [], [])
        self.assertTrue(good["passed"])
        self.assertTrue(good["rtl_mirror_ok"])

        # Non-mirrored RTL geometry must fail.
        unmirrored = _make_desktop_metrics(
            he_scenario,
            navRect={"left": 24.0, "right": 300.0, "width": 276.0, "height": 500.0},
            mainRect={"left": 316.0, "right": 1256.0, "width": 940.0, "height": 500.0},
        )
        bad_mirror = qa.evaluate_desktop_view(he_scenario, unmirrored, 200, [], [])
        self.assertFalse(bad_mirror["passed"])
        self.assertFalse(bad_mirror["rtl_mirror_ok"])

        # Tofu raster collision must fail.
        tofu_metrics = _make_desktop_metrics(
            he_scenario,
            glyphProbe={
                "sampleInkedPixels": 100,
                "sampleRasterHash": "deadbeef",
                "tofuRasterHash": "deadbeef",
                "distinctFromTofu": False,
            },
        )
        bad_tofu = qa.evaluate_desktop_view(he_scenario, tofu_metrics, 200, [], [])
        self.assertFalse(bad_tofu["passed"])
        self.assertFalse(bad_tofu["non_tofu_ok"])

        # Horizontal overflow must fail.
        overflow = _make_desktop_metrics(he_scenario, scrollWidth=1320, innerWidth=1280)
        bad_overflow = qa.evaluate_desktop_view(he_scenario, overflow, 200, [], [])
        self.assertFalse(bad_overflow["passed"])
        self.assertFalse(bad_overflow["zero_overflow"])

        # Missing BiDi isolation on diagnostic <dd> or structured email input must fail.
        bad_bidi = qa.evaluate_desktop_view(
            he_scenario, _make_desktop_metrics(he_scenario, bidiIsolationOk=False), 200, [], []
        )
        self.assertFalse(bad_bidi["passed"])
        self.assertFalse(bad_bidi["bidi_isolation_ok"])

    def test_mobile_touch_targets_and_crisis_lines_invariants(self) -> None:
        hi_scenario = next(s for s in qa.MULTILINGUAL_SCENARIOS if s["lang"] == "hi")
        good = qa.evaluate_mobile_view(hi_scenario, _make_mobile_metrics(hi_scenario), 200, [], [])
        self.assertTrue(good["passed"])
        self.assertTrue(good["touch_targets_ok"])

        undersized = _make_mobile_metrics(
            hi_scenario,
            touchTargets=[{"id": "btn-small", "height": 32.0, "width": 100.0}],
        )
        bad_touch = qa.evaluate_mobile_view(hi_scenario, undersized, 200, [], [])
        self.assertFalse(bad_touch["passed"])
        self.assertFalse(bad_touch["touch_targets_ok"])

        missing_crisis = _make_mobile_metrics(hi_scenario, crisisLinesPresent=False)
        bad_crisis = qa.evaluate_mobile_view(hi_scenario, missing_crisis, 200, [], [])
        self.assertFalse(bad_crisis["passed"])

        bad_mobile_bidi = qa.evaluate_mobile_view(
            hi_scenario, _make_mobile_metrics(hi_scenario, bidiIsolationOk=False), 200, [], []
        )
        self.assertFalse(bad_mobile_bidi["passed"])
        self.assertFalse(bad_mobile_bidi["bidi_isolation_ok"])

    def test_mocked_three_engine_playwright_qa_captures_14_views_per_engine(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            shot_dir = Path(tmp) / "shots"
            report = qa.run_playwright_multilingual_qa(
                _FakePlaywright(),
                browsers=["chromium", "firefox", "webkit"],
                clinic_base_url="http://127.0.0.1:9999/elysianfields/",
                mobile_base_url="http://127.0.0.1:9999/elysianfields/mobile/",
                screenshot_dir=shot_dir,
            )
            self.assertTrue(report["overall_passed"])
            for engine in ("chromium", "firefox", "webkit"):
                self.assertTrue(report["browsers"][engine]["passed"])
                self.assertEqual(report["browsers"][engine]["total_views"], 14)
                self.assertEqual(report["browsers"][engine]["passed_views"], 14)
            self.assertTrue((shot_dir / "desktop_he.png").is_file())
            self.assertTrue((shot_dir / "mobile_hi.png").is_file())
            self.assertTrue((shot_dir / "firefox_desktop_ar.png").is_file())

    def test_web_shells_have_locale_select_logical_css_noto_fonts_and_sw_precache(self) -> None:
        root = qa.repo_root()
        desktop_html = (root / "web/elysianfields/index.html").read_text(encoding="utf-8")
        mobile_html = (root / "web/elysianfields/mobile/index.html").read_text(encoding="utf-8")
        desktop_css = (root / "web/elysianfields/styles.css").read_text(encoding="utf-8")
        mobile_css = (root / "web/elysianfields/mobile/mobile.css").read_text(encoding="utf-8")
        sw_js = (root / "web/elysianfields/mobile/sw.js").read_text(encoding="utf-8")

        for html in (desktop_html, mobile_html):
            self.assertIn('id="locale-select"', html)
            for lang in ("en", "he", "ar", "zh-Hans", "ja", "vi", "hi"):
                self.assertIn(f'value="{lang}"', html)
            self.assertNotIn("<script>", html)
            self.assertNotIn("style=", html)

        for css in (desktop_css, mobile_css):
            self.assertIn("Noto Sans Hebrew", css)
            self.assertIn("Noto Sans Arabic", css)
            self.assertIn("Noto Sans Devanagari", css)
            self.assertIn("Noto Sans CJK SC", css)
            self.assertIn("Noto Sans CJK JP", css)
            self.assertIn("text-align: start", css)
            self.assertIn("unicode-bidi: plaintext", css)
            self.assertNotIn("text-align: left", css)

        self.assertIn("../locales.js", sw_js)


if __name__ == "__main__":
    unittest.main()
