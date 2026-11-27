#!/usr/bin/env python3
"""Unit tests for scripts/browser_e2e.py (pure stdlib, no Playwright required)."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import browser_e2e  # noqa: E402


class ParseBrowsersTests(unittest.TestCase):
    def test_parses_all_three_engines_and_deduplicates(self) -> None:
        self.assertEqual(
            browser_e2e.parse_browsers("chromium, firefox, webkit, chromium"),
            ["chromium", "firefox", "webkit"],
        )

    def test_rejects_unknown_or_empty_browser(self) -> None:
        with self.assertRaises(ValueError):
            browser_e2e.parse_browsers("chromium,opera")
        with self.assertRaises(ValueError):
            browser_e2e.parse_browsers(" , ")

    def test_cli_flags_keep_defaults_and_support_mobile_and_wasm(self) -> None:
        parser = browser_e2e.build_arg_parser()
        default_args = parser.parse_args([])
        self.assertEqual(default_args.browsers, "chromium,firefox,webkit")
        self.assertFalse(default_args.mobile_emulation)
        self.assertIsNone(default_args.wasm_file)

        extended = parser.parse_args(["--mobile-emulation", "--wasm-file", "/tmp/demo.wasm"])
        self.assertTrue(extended.mobile_emulation)
        self.assertEqual(extended.wasm_file, Path("/tmp/demo.wasm"))


class HtmlAndWasmInvariantTests(unittest.TestCase):
    def test_minimal_wasm_binary_has_valid_magic_and_version(self) -> None:
        self.assertEqual(browser_e2e.MINIMAL_VALID_WASM_BYTES[:8], b"\x00asm\x01\x00\x00\x00")
        self.assertEqual(len(browser_e2e.MINIMAL_VALID_WASM_BYTES), 41)

    def test_crisis_lines_before_facilities_passes_when_ordered(self) -> None:
        html = (
            "<main>"
            "<section id='crisis'>988 Suicide Lifeline, 911 Emergency, SAMHSA Helpline</section>"
            '<section id="facilities"><ul><li data-facility-id="wa-1">Clinic</li></ul></section>'
            "</main>"
        )
        ok, info = browser_e2e.verify_crisis_lines_before_facilities(html)
        self.assertTrue(ok)
        self.assertLess(info["idx_988"], info["first_listing_idx"])
        self.assertLess(info["idx_911"], info["first_listing_idx"])
        self.assertLess(info["idx_samhsa"], info["first_listing_idx"])

    def test_crisis_lines_after_facilities_fails(self) -> None:
        html = (
            '<section id="facilities"><ul><li>Clinic</li></ul></section>'
            "<section>988 / 911 / SAMHSA</section>"
        )
        ok, _ = browser_e2e.verify_crisis_lines_before_facilities(html)
        self.assertFalse(ok)

    def test_verify_invite_noindex_checks_meta_or_header(self) -> None:
        self.assertTrue(
            browser_e2e.verify_invite_noindex(
                '<meta name="robots" content="noindex, nofollow">', {}
            )
        )
        self.assertTrue(
            browser_e2e.verify_invite_noindex(
                "<html></html>", {"X-Robots-Tag": "noindex, nofollow"}
            )
        )
        self.assertFalse(
            browser_e2e.verify_invite_noindex(
                '<meta name="robots" content="index, follow">', {}
            )
        )

    def test_mv3_popup_budget(self) -> None:
        self.assertTrue(browser_e2e.evaluate_mv3_popup_budget(42.5, 250.0))
        self.assertTrue(browser_e2e.evaluate_mv3_popup_budget(250.0, 250.0))
        self.assertFalse(browser_e2e.evaluate_mv3_popup_budget(250.1, 250.0))


class StrictStaticPreviewServerTests(unittest.TestCase):
    def test_serves_strict_csp_subpath_mobile_pwa_wasm_and_blocks_traversal(self) -> None:
        root = browser_e2e.repo_root()
        with browser_e2e.StrictStaticPreviewServer(
            web_dir=root / "web" / "elysianfields",
            extension_dir=root / "extension",
        ) as srv:
            # Valid `/elysianfields/` shell + `boot.js` + `a11y-bridge.js`
            with urllib.request.urlopen(f"{srv.origin}/elysianfields/") as resp:
                self.assertEqual(resp.status, 200)
                self.assertEqual(
                    resp.headers.get("Content-Security-Policy"),
                    browser_e2e.CLINIC_STRICT_CSP,
                )
                self.assertIn("./boot.js", resp.read().decode("utf-8"))

            # Valid `/elysianfields/mobile/` PWA + manifest + service worker
            with urllib.request.urlopen(f"{srv.origin}/elysianfields/mobile/") as resp:
                self.assertEqual(resp.status, 200)
                self.assertEqual(
                    resp.headers.get("Content-Security-Policy"),
                    browser_e2e.CLINIC_STRICT_CSP,
                )
                mobile_html = resp.read().decode("utf-8")
                self.assertIn("./mobile-boot.js", mobile_html)
                self.assertTrue(browser_e2e.verify_crisis_lines_present(mobile_html))

            with urllib.request.urlopen(f"{srv.origin}/elysianfields/mobile/sw.js") as resp:
                self.assertEqual(resp.status, 200)
                self.assertEqual(
                    resp.headers.get("Service-Worker-Allowed"),
                    "/elysianfields/mobile/",
                )

            with urllib.request.urlopen(
                f"{srv.origin}/elysianfields/mobile/manifest.webmanifest"
            ) as resp:
                self.assertEqual(resp.status, 200)
                self.assertIn("application/manifest+json", resp.headers.get("Content-Type", ""))

            # Built-in minimal valid Wasm probe served with application/wasm under strict CSP
            with urllib.request.urlopen(f"{srv.origin}/elysianfields/wasm-probe.wasm") as resp:
                self.assertEqual(resp.status, 200)
                self.assertEqual(resp.headers.get("Content-Type"), "application/wasm")
                self.assertEqual(resp.read(), browser_e2e.MINIMAL_VALID_WASM_BYTES)

            # Valid MV3 popup under strict MV3 CSP
            with urllib.request.urlopen(f"{srv.origin}/mv3-popup/popup.html") as resp:
                self.assertEqual(resp.status, 200)
                self.assertEqual(
                    resp.headers.get("Content-Security-Policy"),
                    browser_e2e.MV3_STRICT_CSP,
                )

            # Domain root `/` and prefix-confusion `/elysianfields-evil/` return 404
            for forbidden in ("/", "/elysianfields-evil/", "/elysianfields/../../Cargo.toml"):
                try:
                    urllib.request.urlopen(f"{srv.origin}{forbidden}")
                    self.fail(f"expected HTTP 404 for {forbidden}")
                except urllib.error.HTTPError as exc:
                    self.assertEqual(exc.code, 404)
                    exc.close()


class _FakeResponse:
    def __init__(self, status: int, headers: dict[str, str] | None = None) -> None:
        self.status = status
        self.headers = headers or {}


class _FakeKeyboard:
    def press(self, _key: str) -> None:
        pass


class _FakePage:
    def __init__(self, mode: str) -> None:
        self.mode = mode
        self.url = ""
        self.keyboard = _FakeKeyboard()
        self._eval_calls = 0

    def on(self, _event: str, _cb: Any) -> None:
        pass

    def add_init_script(self, _script: str) -> None:
        pass

    def goto(self, url: str, wait_until: str = "load") -> _FakeResponse:  # noqa: ARG002
        self.url = url
        if url.endswith("/invite"):
            return _FakeResponse(200, {"X-Robots-Tag": "noindex, nofollow"})
        return _FakeResponse(200)

    def reload(self, wait_until: str = "load", timeout: int = 5000) -> _FakeResponse:  # noqa: ARG002
        return _FakeResponse(200)

    def wait_for_function(self, _fn: str, timeout: int = 5000) -> None:  # noqa: ARG002
        pass

    def content(self) -> str:
        if self.url.endswith("/invite"):
            return '<main><meta name="robots" content="noindex, nofollow">Invite</main>'
        if self.url.endswith("/dei"):
            return (
                "<main><section>988 / 911 / SAMHSA National Helpline</section>"
                '<section id="facilities"><ul><li data-facility-id="wa-1">Center</li></ul></section></main>'
            )
        return "<main><h1>Elysian Fields</h1></main>"

    def evaluate(self, _script: str, arg: Any = None) -> Any:  # noqa: ARG002
        self._eval_calls += 1
        if self.mode == "clinic":
            if self._eval_calls == 1:
                return {
                    "nonBlank": True,
                    "cspViolations": 0,
                    "subpathValid": True,
                    "backend": "webgl2",
                    "overlayPresent": True,
                    "overlaySubpath": "/elysianfields",
                    "bridgeNodes": 6,
                    "hasNav": True,
                    "hasMain": True,
                    "hasTextbox": True,
                }
            return {"tag": "input", "id": "signin-email", "movedOffBody": True}
        if self.mode == "wasm":
            return {
                "fetchStatus": 200,
                "byteLength": 41,
                "compiled": True,
                "instantiated": True,
                "addResult": 42,
                "cspViolations": 0,
                "compileMs": 1.25,
                "error": None,
            }
        if self.mode == "mobile":
            if self._eval_calls == 1:
                return {
                    "scrollWidth": 390,
                    "innerWidth": 390,
                    "zeroOverflow": True,
                    "cspViolations": 0,
                    "dir": "ltr",
                    "lang": "en",
                    "overlayPresent": True,
                    "bridgeNodes": 6,
                    "hasNav": True,
                    "hasMain": True,
                    "crisisPresent": True,
                    "swState": "controlling",
                    "hasController": True,
                }
            if self._eval_calls == 2:
                return {
                    "dir": "rtl",
                    "lang": "ar",
                    "scrollWidth": 390,
                    "innerWidth": 390,
                    "zeroOverflow": True,
                    "cspViolations": 0,
                }
            if self._eval_calls == 3:
                return True
            return {"nonBlank": True, "crisisPresent": True}
        return {
            "dclMs": 18.4,
            "cspViolations": 0,
            "hasMain": True,
            "hasHeader": True,
            "hasStatus": True,
            "buttonCount": 2,
        }


class _FakeContext:
    def __init__(self, mode: str) -> None:
        self.mode = mode
        self.offline_transitions: list[bool] = []

    def new_page(self) -> _FakePage:
        return _FakePage(self.mode)

    def set_offline(self, offline: bool) -> None:
        self.offline_transitions.append(offline)

    def close(self) -> None:
        pass


class _FakeBrowser:
    def __init__(self, version: str) -> None:
        self.version = version
        self._ctx_idx = 0

    def new_context(self, **kwargs: Any) -> _FakeContext:
        if kwargs.get("java_script_enabled") is False:
            return _FakeContext("public")
        if kwargs.get("is_mobile") is True:
            return _FakeContext("mobile")
        self._ctx_idx += 1
        if self._ctx_idx == 1:
            return _FakeContext("clinic")
        if self._ctx_idx == 2:
            return _FakeContext("wasm")
        return _FakeContext("popup")

    def close(self) -> None:
        pass


class _FakeBrowserType:
    def __init__(self, version: str) -> None:
        self._version = version

    def launch(self, headless: bool = True) -> _FakeBrowser:  # noqa: ARG002
        return _FakeBrowser(self._version)


class _FakePlaywright:
    chromium = _FakeBrowserType("157.0")
    firefox = _FakeBrowserType("159.0")
    webkit = _FakeBrowserType("18.4")


class MockedPlaywrightFlowTests(unittest.TestCase):
    def test_run_browser_qualification_covers_desktop_wasm_csp_and_mobile_emulation(self) -> None:
        report = browser_e2e.run_browser_qualification(
            _FakePlaywright(),
            browsers=["chromium", "firefox", "webkit"],
            clinic_shell_url="http://127.0.0.1:9999/elysianfields/",
            mv3_popup_url="http://127.0.0.1:9999/mv3-popup/popup.html",
            public_origin="http://127.0.0.1:9998",
            wasm_host_url="http://127.0.0.1:9999/elysianfields/",
            wasm_probe_url="http://127.0.0.1:9999/elysianfields/wasm-probe.wasm",
            wasm_is_minimal_builtin=True,
            wasm_raw_bytes=41,
            wasm_brotli11_bytes=45,
            mobile_emulation=True,
            mobile_shell_url="http://127.0.0.1:9999/elysianfields/mobile/",
        )
        self.assertTrue(report["overall_passed"])
        self.assertEqual(report["browsers_requested"], ["chromium", "firefox", "webkit"])
        for engine in ("chromium", "firefox", "webkit"):
            entry = report["results"][engine]
            self.assertTrue(entry["passed"])
            self.assertTrue(entry["checks"]["clinic_shell"]["passed"])
            self.assertTrue(entry["checks"]["wasm_csp"]["passed"])
            self.assertEqual(entry["checks"]["wasm_csp"]["add_result"], 42)
            self.assertTrue(entry["checks"]["mv3_popup"]["passed"])
            self.assertTrue(entry["checks"]["public_site_no_js"]["passed"])

        mobile_section = report["mobile_emulation"]
        self.assertTrue(mobile_section["enabled"])
        self.assertTrue(mobile_section["passed"])
        self.assertIn("iphone_14_webkit", mobile_section["profiles"])
        self.assertIn("pixel_7_chromium", mobile_section["profiles"])
        for profile in mobile_section["profiles"].values():
            self.assertTrue(profile["passed"])
            self.assertEqual(profile["rtl"]["dir"], "rtl")
            self.assertTrue(profile["offline"]["crisis_lines_present"])

        with tempfile.TemporaryDirectory() as tmp:
            out_path = Path(tmp) / "report.json"
            out_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
            reloaded = json.loads(out_path.read_text(encoding="utf-8"))
            self.assertTrue(reloaded["overall_passed"])


if __name__ == "__main__":
    unittest.main()
