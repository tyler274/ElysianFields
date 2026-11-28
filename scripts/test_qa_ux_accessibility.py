#!/usr/bin/env python3
"""Unit tests for scripts/qa_ux_accessibility.py (pure stdlib, no live browser required)."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import qa_ux_accessibility as qa  # noqa: E402


class WcagContrastMathTests(unittest.TestCase):
    def test_black_on_white_is_21_to_1(self) -> None:
        ratio = qa.wcag_contrast_ratio((0, 0, 0), (255, 255, 255))
        self.assertAlmostEqual(ratio, 21.0, places=2)

    def test_light_and_dark_theme_tokens_clear_wcag_aa(self) -> None:
        samples = [
            {"label": "light_body", "fg": "rgb(15, 23, 42)", "bg": "rgb(248, 250, 252)"},
            {"label": "light_secondary", "fg": "rgb(71, 85, 105)", "bg": "rgb(255, 255, 255)"},
            {"label": "light_primary_btn", "fg": "rgb(255, 255, 255)", "bg": "rgb(29, 78, 216)"},
            {"label": "light_crisis_link", "fg": "rgb(136, 19, 55)", "bg": "rgb(255, 241, 242)"},
            {"label": "dark_body", "fg": "rgb(241, 245, 249)", "bg": "rgb(15, 23, 42)"},
            {"label": "dark_secondary", "fg": "rgb(203, 213, 225)", "bg": "rgb(30, 41, 59)"},
            {"label": "dark_crisis_link", "fg": "rgb(253, 230, 138)", "bg": "rgb(63, 29, 46)"},
        ]
        ok, evaluated = qa.evaluate_contrast_samples(samples)
        self.assertTrue(ok)
        self.assertEqual(len(evaluated), len(samples))
        for item in evaluated:
            self.assertGreaterEqual(item["ratio"], 4.5)

    def test_low_contrast_pair_fails_wcag_aa(self) -> None:
        ok, evaluated = qa.evaluate_contrast_samples(
            [{"label": "low_contrast", "fg": "rgb(148, 163, 184)", "bg": "rgb(255, 255, 255)"}]
        )
        self.assertFalse(ok)
        self.assertFalse(evaluated[0]["passed"])
        self.assertLess(evaluated[0]["ratio"], 4.5)

    def test_parse_css_rgb_rejects_malformed(self) -> None:
        self.assertEqual(qa.parse_css_rgb("rgba(15, 23, 42, 0.5)"), (15, 23, 42, 0.5))
        with self.assertRaises(ValueError):
            qa.parse_css_rgb("not-a-color")


class EvaluatorContractTests(unittest.TestCase):
    def test_zoom_200_rejects_clipped_button_or_overflow(self) -> None:
        good = qa.evaluate_zoom200_check(
            {
                "rootFontSize": "32px",
                "scrollWidth": 1280,
                "innerWidth": 1280,
                "checkedButtonCount": 6,
                "clippedButtons": [],
                "cspViolations": 0,
            },
            [],
            [],
        )
        self.assertTrue(good["passed"])

        bad_clip = qa.evaluate_zoom200_check(
            {
                "rootFontSize": "32px",
                "scrollWidth": 1280,
                "innerWidth": 1280,
                "checkedButtonCount": 6,
                "clippedButtons": [{"id": "btn-send-prompt", "reason": "vertical_clip"}],
                "cspViolations": 0,
            },
            [],
            [],
        )
        self.assertFalse(bad_clip["passed"])

        bad_overflow = qa.evaluate_zoom200_check(
            {
                "rootFontSize": "32px",
                "scrollWidth": 1340,
                "innerWidth": 1280,
                "checkedButtonCount": 6,
                "clippedButtons": [],
                "cspViolations": 0,
            },
            [],
            [],
        )
        self.assertFalse(bad_overflow["passed"])

    def test_privacy_cover_and_shortcuts_require_modal_focus_and_escape(self) -> None:
        open_priv = {
            "privacyPresent": True,
            "privacyHidden": False,
            "privacyAriaModal": "true",
            "gridAriaHidden": "true",
            "activeElementId": "btn-dismiss-privacy",
            "cspViolations": 0,
        }
        close_priv = {
            "privacyPresent": True,
            "privacyHidden": True,
            "gridAriaHidden": None,
            "cspViolations": 0,
        }
        open_short = {
            "shortcutsPresent": True,
            "shortcutsHidden": False,
            "shortcutsAriaModal": "true",
            "activeElementId": "btn-close-shortcuts",
            "cspViolations": 0,
        }
        close_short = {
            "shortcutsPresent": True,
            "shortcutsHidden": True,
            "cspViolations": 0,
        }
        self.assertTrue(
            qa.evaluate_privacy_and_shortcuts_check(
                open_priv, close_priv, open_short, close_short, [], []
            )["passed"]
        )
        wrong_focus = dict(open_priv, activeElementId="composer-draft")
        self.assertFalse(
            qa.evaluate_privacy_and_shortcuts_check(
                wrong_focus, close_priv, open_short, close_short, [], []
            )["passed"]
        )

    def test_public_dei_requires_crisis_physically_above_facilities_and_invite_noindex(self) -> None:
        idx_ok = {
            "has988": True,
            "has911": True,
            "hasSamhsa": True,
            "crisisRect": {"top": 40.0, "bottom": 180.0, "height": 140.0},
            "listingRect": {"top": 200.0, "bottom": 500.0, "height": 300.0},
        }
        search_ok = {
            "has988": True,
            "has911": True,
            "hasSamhsa": True,
            "crisisRect": {"top": 40.0, "bottom": 180.0, "height": 140.0},
            "listingRect": {"top": 360.0, "bottom": 900.0, "height": 540.0},
            "facilityCount": 3,
        }
        res = qa.evaluate_public_dei_check(
            idx_ok,
            200,
            search_ok,
            200,
            200,
            '<meta name="robots" content="noindex, nofollow">',
            {},
            [],
            [],
        )
        self.assertTrue(res["passed"])

        inverted = dict(
            search_ok,
            crisisRect={"top": 520.0, "bottom": 640.0, "height": 120.0},
            listingRect={"top": 120.0, "bottom": 480.0, "height": 360.0},
        )
        bad = qa.evaluate_public_dei_check(
            idx_ok,
            200,
            inverted,
            200,
            200,
            '<meta name="robots" content="noindex, nofollow">',
            {},
            [],
            [],
        )
        self.assertFalse(bad["passed"])


class _FakeResp:
    def __init__(self, status: int = 200, headers: dict[str, str] | None = None) -> None:
        self.status = status
        self.headers = headers or {}


class _FakeKeyboard:
    def __init__(self, page: _FakeUxPage) -> None:
        self._page = page

    def press(self, key: str) -> None:
        if key == "Control+Shift+L":
            self._page.privacy_open = True
        elif key == "Escape":
            if self._page.privacy_open:
                self._page.privacy_open = False
            elif self._page.shortcuts_open:
                self._page.shortcuts_open = False


class _FakeUxPage:
    def __init__(self) -> None:
        self.url = ""
        self.privacy_open = False
        self.shortcuts_open = False
        self.keyboard = _FakeKeyboard(self)
        self.screenshots: list[Path] = []

    def on(self, _evt: str, _cb: Any) -> None:
        pass

    def add_init_script(self, _script: str) -> None:
        pass

    def goto(self, url: str, wait_until: str = "load") -> _FakeResp:  # noqa: ARG002
        self.url = url
        if url.endswith("/invite"):
            return _FakeResp(200, {"X-Robots-Tag": "noindex, nofollow"})
        return _FakeResp(200)

    def wait_for_function(self, _expr: str, timeout: int = 5000) -> None:  # noqa: ARG002
        pass

    def click(self, selector: str) -> None:
        if selector == "#btn-shortcuts":
            self.shortcuts_open = True

    def screenshot(self, path: str, full_page: bool = True) -> None:  # noqa: ARG002
        p = Path(path)
        p.write_bytes(b"\x89PNG\r\n\x1a\n")
        self.screenshots.append(p)

    def content(self) -> str:
        return '<meta name="robots" content="noindex, nofollow">'

    def evaluate(self, script: str) -> Any:
        if "__efCspViolations" in script:
            return 0
        if "html.style.fontSize = \"32px\"" in script:
            return {
                "rootFontSize": "32px",
                "scrollWidth": 1280,
                "innerWidth": 1280,
                "checkedButtonCount": 7,
                "clippedButtons": [],
                "cspViolations": 0,
            }
        if "curie.theme.v1" in script:
            return {
                "ok": True,
                "initialTheme": "dark",
                "afterTheme": "light",
                "initialPressed": "false",
                "afterPressed": "true",
                "stored": "light",
            }
        if "privacyPresent" in script:
            return {
                "privacyPresent": True,
                "privacyHidden": not self.privacy_open,
                "privacyAriaModal": "true",
                "gridAriaHidden": "true" if self.privacy_open else None,
                "shortcutsPresent": True,
                "shortcutsHidden": not self.shortcuts_open,
                "shortcutsAriaModal": "true",
                "activeElementId": (
                    "btn-dismiss-privacy"
                    if self.privacy_open
                    else ("btn-close-shortcuts" if self.shortcuts_open else "body")
                ),
                "cspViolations": 0,
            }
        if "crisis_banner_heading" in script:
            theme = "light" if "theme=light" in self.url else "dark"
            return {
                "theme": theme,
                "togglePresent": True,
                "ariaPressed": "true" if theme == "light" else "false",
                "scrollWidth": 390,
                "innerWidth": 390,
                "touchTargets": [{"id": "btn-toggle-theme", "height": 44.0, "width": 120.0}],
                "samples": [
                    {"label": "body_text", "fg": "rgb(15, 23, 42)", "bg": "rgb(248, 250, 252)"},
                    {"label": "crisis_tel_link", "fg": "rgb(136, 19, 55)", "bg": "rgb(255, 241, 242)"},
                ],
                "cspViolations": 0,
            }
        if "popup_heading" in script:
            return {
                "dclMs": 14.2,
                "samples": [
                    {"label": "popup_heading", "fg": "rgb(15, 23, 42)", "bg": "rgb(248, 250, 252)"},
                    {"label": "primary_button", "fg": "rgb(255, 255, 255)", "bg": "rgb(29, 78, 216)"},
                ],
                "buttons": [
                    {"id": "btn-refresh", "visible": True, "width": 150.0, "height": 40.0},
                    {"id": "btn-disconnect", "visible": True, "width": 150.0, "height": 40.0},
                ],
            }
        if "crisisPresent" in script:
            return {
                "crisisPresent": True,
                "listingPresent": True,
                "listingId": "facilities",
                "has988": True,
                "has911": True,
                "hasSamhsa": True,
                "crisisRect": {"top": 40.0, "bottom": 160.0, "height": 120.0},
                "listingRect": {"top": 220.0, "bottom": 700.0, "height": 480.0},
                "facilityCount": 2,
            }
        theme = "light" if "theme=light" in self.url else "dark"
        return {
            "theme": theme,
            "togglePresent": True,
            "ariaPressed": "true" if theme == "light" else "false",
            "cspViolations": 0,
            "scrollWidth": 1280,
            "innerWidth": 1280,
            "samples": [
                {"label": "body_text", "fg": "rgb(15, 23, 42)", "bg": "rgb(248, 250, 252)"},
                {"label": "primary_cta_button", "fg": "rgb(255, 255, 255)", "bg": "rgb(29, 78, 216)"},
            ],
        }


class _FakeUxContext:
    def new_page(self) -> _FakeUxPage:
        return _FakeUxPage()

    def close(self) -> None:
        pass


class _FakeUxBrowser:
    version = "157.0"

    def new_context(self, **_kwargs: Any) -> _FakeUxContext:
        return _FakeUxContext()

    def close(self) -> None:
        pass


class _FakeUxBrowserType:
    def launch(self, headless: bool = True) -> _FakeUxBrowser:  # noqa: ARG002
        return _FakeUxBrowser()


class _FakePlaywright:
    chromium = _FakeUxBrowserType()
    firefox = _FakeUxBrowserType()
    webkit = _FakeUxBrowserType()


class MockedPlaywrightUxTests(unittest.TestCase):
    def test_playwright_runner_captures_all_seven_screenshots_and_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            shots = Path(tmp) / "shots"
            report = qa.run_playwright_ux_qa(
                _FakePlaywright(),
                browsers=["chromium", "firefox", "webkit"],
                clinic_base_url="http://127.0.0.1:9999/elysianfields/",
                mobile_base_url="http://127.0.0.1:9999/elysianfields/mobile/",
                popup_url="http://127.0.0.1:9999/mv3-popup/popup.html",
                public_origin="http://127.0.0.1:9998",
                screenshot_dir=shots,
            )
            self.assertTrue(report["overall_passed"])
            for engine in ("chromium", "firefox", "webkit"):
                self.assertTrue(report["browsers"][engine]["passed"])
                self.assertEqual(report["browsers"][engine]["total_checks"], 9)
            for name in (
                "clinic_light.png",
                "clinic_dark.png",
                "clinic_zoom200.png",
                "clinic_privacy_cover.png",
                "mobile_light.png",
                "extension_popup.png",
                "public_dei.png",
            ):
                self.assertTrue((shots / name).is_file(), f"missing screenshot {name}")

    def test_cli_parse_args_supports_all_flags(self) -> None:
        args = qa.parse_args(
            [
                "--browsers",
                "chromium,firefox,webkit",
                "--report",
                "/tmp/ux.json",
                "--screenshot-dir",
                "/tmp/shots",
                "--clinic-bin",
                "/tmp/clinic",
                "--mobile-bin",
                "/tmp/mobile",
                "--public-bin",
                "/tmp/public",
            ]
        )
        self.assertEqual(args.browsers, "chromium,firefox,webkit")
        self.assertEqual(args.report, Path("/tmp/ux.json"))
        self.assertEqual(args.screenshot_dir, Path("/tmp/shots"))
        self.assertEqual(args.public_bin, Path("/tmp/public"))


if __name__ == "__main__":
    unittest.main()
