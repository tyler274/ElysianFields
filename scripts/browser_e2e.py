#!/usr/bin/env python3
"""Multi-browser Playwright E2E qualification suite for Elysian Fields.

Validates across Chromium, Firefox, and WebKit:
1. `/elysianfields/` clinic browser shell (HTTP 200, 0 console/page errors, 0 CSP
   violations under strict CSP, non-blank viewport, real single-owner ARIA tree via
   `a11y-bridge.js`, and keyboard `Tab` navigation).
2. Chrome MV3 `extension/popup.html` (Decision D2 semantic-HTML popup: 0 console/page
   errors, 0 CSP violations, semantic landmarks, and cold start <= 250 ms).
3. Optional live `ef-public-site` binary with JavaScript disabled (`java_script_enabled=False`):
   `/`, `/privacy`, `/terms`, `/invite` (`noindex, nofollow`), and `/dei` (or `/recovery/wa`)
   with crisis lines (`988`, `911`, `SAMHSA`) strictly before facility listings.
"""

from __future__ import annotations

import argparse
import http.server
import json
import mimetypes
import os
import re
import socket
import socketserver
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

SUPPORTED_BROWSERS = ("chromium", "firefox", "webkit")

CLINIC_STRICT_CSP = (
    "default-src 'self'; "
    "script-src 'self' 'wasm-unsafe-eval'; "
    "style-src 'self'; "
    "connect-src 'self'; "
    "object-src 'none'"
)

MV3_STRICT_CSP = (
    "default-src 'self'; "
    "script-src 'self' 'wasm-unsafe-eval'; "
    "style-src 'self'; "
    "object-src 'self'"
)

DEFAULT_MV3_COLD_START_BUDGET_MS = 250.0


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def parse_browsers(raw: str) -> list[str]:
    """Parses and validates a comma-separated list of Playwright browser engines."""
    parsed: list[str] = []
    for item in raw.split(","):
        name = item.strip().lower()
        if not name:
            continue
        if name not in SUPPORTED_BROWSERS:
            supported = ", ".join(SUPPORTED_BROWSERS)
            raise ValueError(f"Unsupported browser '{name}'; expected one of: {supported}")
        if name not in parsed:
            parsed.append(name)
    if not parsed:
        raise ValueError("At least one browser engine must be specified")
    return parsed


def find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def wait_for_tcp_port(host: str, port: int, timeout_secs: float = 15.0) -> bool:
    deadline = time.monotonic() + timeout_secs
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def verify_crisis_lines_before_facilities(html: str) -> tuple[bool, dict[str, Any]]:
    """Checks that 988, 911, and SAMHSA appear strictly before facility listings in HTML/DOM."""
    idx_988 = html.find("988")
    idx_911 = html.find("911")
    idx_samhsa = html.find("SAMHSA")

    facility_markers = [
        'id="facilities"',
        "id='facilities'",
        'data-facility-id="',
        'id="state-directory"',
        'class="facility',
        "Listed Facilities",
    ]
    listing_positions = [html.find(marker) for marker in facility_markers if html.find(marker) >= 0]
    first_listing_idx = min(listing_positions) if listing_positions else -1

    all_present = idx_988 >= 0 and idx_911 >= 0 and idx_samhsa >= 0
    if first_listing_idx >= 0:
        ordered_before = (
            all_present
            and idx_988 < first_listing_idx
            and idx_911 < first_listing_idx
            and idx_samhsa < first_listing_idx
        )
    else:
        ordered_before = False

    return ordered_before, {
        "idx_988": idx_988,
        "idx_911": idx_911,
        "idx_samhsa": idx_samhsa,
        "first_listing_idx": first_listing_idx,
    }


def verify_invite_noindex(html: str, headers: dict[str, str]) -> bool:
    """Checks `/invite` emits `noindex` + `nofollow` via meta robots or X-Robots-Tag."""
    lower_html = html.lower()
    meta_match = re.search(
        r'<meta[^>]+name=["\']robots["\'][^>]+content=["\']([^"\']+)["\']',
        lower_html,
    )
    meta_ok = bool(
        meta_match and "noindex" in meta_match.group(1) and "nofollow" in meta_match.group(1)
    )
    header_val = ""
    for key, value in headers.items():
        if key.lower() == "x-robots-tag":
            header_val = value.lower()
            break
    header_ok = "noindex" in header_val and "nofollow" in header_val
    return meta_ok or header_ok


def evaluate_mv3_popup_budget(
    cold_start_ms: float,
    max_ms: float = DEFAULT_MV3_COLD_START_BUDGET_MS,
) -> bool:
    return cold_start_ms >= 0.0 and cold_start_ms <= max_ms


def _safe_resolve_under(base_dir: Path, rel_path: str) -> Path | None:
    if "\\" in rel_path or "\x00" in rel_path:
        return None
    cleaned = rel_path.lstrip("/")
    if not cleaned:
        cleaned = "index.html"
    parts = [part for part in cleaned.split("/") if part not in ("", ".")]
    if any(part == ".." for part in parts):
        return None
    base_resolved = base_dir.resolve()
    candidate = (base_resolved / Path(*parts)).resolve()
    try:
        candidate.relative_to(base_resolved)
    except ValueError:
        return None
    return candidate


class _ReusableTcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


class StrictStaticPreviewServer:
    """Localhost HTTP server enforcing strict CSP, `/elysianfields/` mount, `/mv3-popup/` mount,
    and optional reverse-proxying of `/elysianfields/api/*` to a live `ef-clinic-server`."""

    def __init__(
        self,
        web_dir: Path,
        extension_dir: Path,
        clinic_api_origin: str | None = None,
    ) -> None:
        self.web_dir = web_dir.resolve()
        self.extension_dir = extension_dir.resolve()
        self.clinic_api_origin = clinic_api_origin.rstrip("/") if clinic_api_origin else None
        self._httpd: _ReusableTcpServer | None = None
        self._thread: threading.Thread | None = None
        self.port: int = 0

    @property
    def origin(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def start(self) -> None:
        web_dir = self.web_dir
        extension_dir = self.extension_dir
        clinic_api_origin = self.clinic_api_origin

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, format: str, *args: Any) -> None:  # noqa: A002
                pass

            def do_GET(self) -> None:  # noqa: N802
                self._dispatch("GET")

            def do_POST(self) -> None:  # noqa: N802
                self._dispatch("POST")

            def do_PUT(self) -> None:  # noqa: N802
                self._dispatch("PUT")

            def _send_bytes(
                self,
                status: int,
                content_type: str,
                body: bytes,
                csp: str,
                extra_headers: dict[str, str] | None = None,
            ) -> None:
                self.send_response(status)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Content-Security-Policy", csp)
                self.send_header("X-Content-Type-Options", "nosniff")
                self.send_header("Cache-Control", "no-store")
                if extra_headers:
                    for k, v in extra_headers.items():
                        self.send_header(k, v)
                self.end_headers()
                self.wfile.write(body)

            def _proxy_clinic_api(self, method: str) -> None:
                if not clinic_api_origin:
                    self._send_bytes(
                        404,
                        "application/json; charset=utf-8",
                        b'{"error":"clinic_api_not_configured"}',
                        CLINIC_STRICT_CSP,
                    )
                    return

                content_len = int(self.headers.get("Content-Length", "0") or "0")
                req_body = self.rfile.read(content_len) if content_len > 0 else None
                fwd_headers: dict[str, str] = {}
                for h in ("Authorization", "Content-Type", "Accept"):
                    val = self.headers.get(h)
                    if val:
                        fwd_headers[h] = val

                req = urllib.request.Request(
                    f"{clinic_api_origin}{self.path}",
                    data=req_body,
                    headers=fwd_headers,
                    method=method,
                )
                try:
                    with urllib.request.urlopen(req, timeout=10) as resp:
                        data = resp.read()
                        ctype = resp.headers.get("Content-Type", "application/json")
                        self._send_bytes(resp.status, ctype, data, CLINIC_STRICT_CSP)
                except urllib.error.HTTPError as err:
                    data = err.read()
                    ctype = err.headers.get("Content-Type", "application/json")
                    self._send_bytes(err.code, ctype, data, CLINIC_STRICT_CSP)

            def _dispatch(self, method: str) -> None:
                path_only = self.path.split("?", 1)[0]

                if path_only.startswith("/elysianfields/api/"):
                    self._proxy_clinic_api(method)
                    return

                if method != "GET":
                    self._send_bytes(
                        405,
                        "text/plain; charset=utf-8",
                        b"Method Not Allowed",
                        CLINIC_STRICT_CSP,
                    )
                    return

                if path_only == "/elysianfields":
                    self.send_response(308)
                    self.send_header("Location", "/elysianfields/")
                    self.send_header("Content-Security-Policy", CLINIC_STRICT_CSP)
                    self.end_headers()
                    return

                if path_only.startswith("/elysianfields/"):
                    rel = path_only[len("/elysianfields/") :]
                    target = _safe_resolve_under(web_dir, rel or "index.html")
                    if target is None or not target.is_file():
                        self._send_bytes(
                            404,
                            "text/plain; charset=utf-8",
                            b"Not Found",
                            CLINIC_STRICT_CSP,
                        )
                        return
                    self._serve_file(target, CLINIC_STRICT_CSP)
                    return

                if path_only.startswith("/mv3-popup/"):
                    rel = path_only[len("/mv3-popup/") :]
                    target = _safe_resolve_under(extension_dir, rel or "popup.html")
                    if target is None or not target.is_file():
                        self._send_bytes(
                            404,
                            "text/plain; charset=utf-8",
                            b"Not Found",
                            MV3_STRICT_CSP,
                        )
                        return
                    self._serve_file(target, MV3_STRICT_CSP)
                    return

                self._send_bytes(
                    404,
                    "text/plain; charset=utf-8",
                    b"Not Found (outside /elysianfields mount)",
                    CLINIC_STRICT_CSP,
                )

            def _serve_file(self, file_path: Path, csp: str) -> None:
                ext = file_path.suffix.lower()
                if ext == ".js":
                    ctype = "application/javascript; charset=utf-8"
                elif ext == ".wasm":
                    ctype = "application/wasm"
                elif ext == ".css":
                    ctype = "text/css; charset=utf-8"
                elif ext == ".html":
                    ctype = "text/html; charset=utf-8"
                elif ext == ".json":
                    ctype = "application/json; charset=utf-8"
                else:
                    guessed, _ = mimetypes.guess_type(str(file_path))
                    ctype = guessed or "application/octet-stream"
                self._send_bytes(200, ctype, file_path.read_bytes(), csp)

        self._httpd = _ReusableTcpServer(("127.0.0.1", 0), Handler)
        self.port = int(self._httpd.server_address[1])
        self._thread = threading.Thread(target=self._httpd.serve_forever, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        if self._httpd is not None:
            self._httpd.shutdown()
            self._httpd.server_close()
            self._httpd = None
        if self._thread is not None:
            self._thread.join(timeout=5)
            self._thread = None

    def __enter__(self) -> StrictStaticPreviewServer:
        self.start()
        return self

    def __exit__(self, *_exc: object) -> None:
        self.stop()


class ManagedServerProcess:
    """Starts and tears down a native Rust binary on an ephemeral localhost port."""

    def __init__(self, binary: Path, env_overrides: dict[str, str], port: int) -> None:
        self.binary = binary
        self.env_overrides = env_overrides
        self.port = port
        self._proc: subprocess.Popen[str] | None = None

    @property
    def origin(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def start(self) -> None:
        env = os.environ.copy()
        env.update(self.env_overrides)
        self._proc = subprocess.Popen(
            [str(self.binary)],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        if not wait_for_tcp_port("127.0.0.1", self.port, timeout_secs=15.0):
            self.stop()
            raise RuntimeError(
                f"Timed out waiting for {self.binary} on 127.0.0.1:{self.port}"
            )

    def stop(self) -> None:
        if self._proc is not None:
            self._proc.terminate()
            try:
                self._proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self._proc.kill()
                self._proc.wait(timeout=5)
            self._proc = None

    def __enter__(self) -> ManagedServerProcess:
        self.start()
        return self

    def __exit__(self, *_exc: object) -> None:
        self.stop()


def _probe_clinic_serves_shell(origin: str) -> bool:
    try:
        with urllib.request.urlopen(f"{origin}/elysianfields/boot.js", timeout=3) as resp:
            return resp.status == 200
    except (urllib.error.URLError, OSError):
        return False


def check_clinic_shell_page(page: Any, clinic_shell_url: str) -> dict[str, Any]:
    """Exercises `/elysianfields/` on an open Playwright page."""
    console_errors: list[str] = []
    page_errors: list[str] = []

    page.on(
        "console",
        lambda msg: console_errors.append(msg.text) if msg.type == "error" else None,
    )
    page.on("pageerror", lambda err: page_errors.append(str(err)))

    t0 = time.perf_counter()
    response = page.goto(clinic_shell_url, wait_until="networkidle")
    http_status = response.status if response is not None else 0
    page.wait_for_function(
        "() => document.documentElement.dataset.efReady === 'true'",
        timeout=10_000,
    )
    load_ms = (time.perf_counter() - t0) * 1000.0

    dom_metrics = page.evaluate(
        """() => {
            const root = document.getElementById("elysian-app");
            const rect = root ? root.getBoundingClientRect() : { width: 0, height: 0 };
            const overlay = document.getElementById("ef-a11y-overlay");
            const bridgeNodes = overlay ? overlay.querySelectorAll("[data-ef-node-id]").length : 0;
            const hasNav = overlay ? overlay.querySelector('[role="navigation"]') !== null : false;
            const hasMain = overlay ? overlay.querySelector('[role="main"]') !== null : false;
            const hasTextbox = overlay ? overlay.querySelector('[role="textbox"]') !== null : false;
            return {
                nonBlank: (document.body.innerText || "").trim().length > 0 && rect.width > 0 && rect.height > 0,
                cspViolations: Number(document.documentElement.dataset.efCspViolations || "0"),
                subpathValid: document.documentElement.dataset.efSubpathValid === "true",
                backend: document.documentElement.dataset.efBackend || "unknown",
                overlayPresent: overlay !== null,
                overlaySubpath: overlay ? overlay.dataset.efSubpath : null,
                bridgeNodes,
                hasNav,
                hasMain,
                hasTextbox,
            };
        }"""
    )

    page.keyboard.press("Tab")
    focused_info = page.evaluate(
        """() => {
            const active = document.activeElement;
            return {
                tag: active ? active.tagName.toLowerCase() : null,
                id: active ? (active.id || null) : null,
                movedOffBody: active !== null && active !== document.body && active !== document.documentElement,
            };
        }"""
    )

    passed = (
        http_status == 200
        and len(console_errors) == 0
        and len(page_errors) == 0
        and dom_metrics["cspViolations"] == 0
        and dom_metrics["nonBlank"]
        and dom_metrics["subpathValid"]
        and dom_metrics["overlayPresent"]
        and dom_metrics["overlaySubpath"] == "/elysianfields"
        and dom_metrics["bridgeNodes"] >= 5
        and dom_metrics["hasNav"]
        and dom_metrics["hasMain"]
        and dom_metrics["hasTextbox"]
        and focused_info["movedOffBody"]
    )

    return {
        "passed": passed,
        "url": clinic_shell_url,
        "http_status": http_status,
        "load_ms": round(load_ms, 2),
        "console_errors": console_errors,
        "page_errors": page_errors,
        "csp_violations": dom_metrics["cspViolations"],
        "non_blank": dom_metrics["nonBlank"],
        "backend": dom_metrics["backend"],
        "a11y_bridge_nodes": dom_metrics["bridgeNodes"],
        "keyboard_focus": focused_info,
    }


def check_mv3_popup_page(
    page: Any,
    popup_url: str,
    max_cold_start_ms: float = DEFAULT_MV3_COLD_START_BUDGET_MS,
) -> dict[str, Any]:
    """Exercises `extension/popup.html` under strict MV3 CSP and <= 250 ms cold-start budget."""
    console_errors: list[str] = []
    page_errors: list[str] = []

    page.on(
        "console",
        lambda msg: console_errors.append(msg.text) if msg.type == "error" else None,
    )
    page.on("pageerror", lambda err: page_errors.append(str(err)))

    # Attach CSP listener before document script runs.
    page.add_init_script(
        """() => {
            window.__efPopupCspViolations = 0;
            document.addEventListener("securitypolicyviolation", () => {
                window.__efPopupCspViolations += 1;
            });
        }"""
    )

    t0 = time.perf_counter()
    response = page.goto(popup_url, wait_until="domcontentloaded")
    wall_ms = (time.perf_counter() - t0) * 1000.0
    http_status = response.status if response is not None else 0

    metrics = page.evaluate(
        """() => {
            const nav = performance.getEntriesByType("navigation")[0];
            const dclMs = nav && nav.domContentLoadedEventEnd > 0
                ? (nav.domContentLoadedEventEnd - nav.startTime)
                : -1;
            const hasMain = document.querySelector("main") !== null;
            const hasHeader = document.querySelector("header") !== null;
            const hasStatus = document.querySelector('[role="status"]') !== null;
            const buttonCount = document.querySelectorAll("button").length;
            return {
                dclMs,
                cspViolations: Number(window.__efPopupCspViolations || 0),
                hasMain,
                hasHeader,
                hasStatus,
                buttonCount,
            };
        }"""
    )

    cold_start_ms = metrics["dclMs"] if metrics["dclMs"] >= 0 else wall_ms
    within_budget = evaluate_mv3_popup_budget(cold_start_ms, max_cold_start_ms)
    has_landmarks = (
        metrics["hasMain"]
        and metrics["hasHeader"]
        and metrics["hasStatus"]
        and metrics["buttonCount"] >= 2
    )

    passed = (
        http_status == 200
        and len(console_errors) == 0
        and len(page_errors) == 0
        and metrics["cspViolations"] == 0
        and has_landmarks
        and within_budget
    )

    return {
        "passed": passed,
        "url": popup_url,
        "http_status": http_status,
        "console_errors": console_errors,
        "page_errors": page_errors,
        "csp_violations": metrics["cspViolations"],
        "has_landmarks": has_landmarks,
        "cold_start_ms": round(cold_start_ms, 2),
        "budget_ms": max_cold_start_ms,
        "within_budget": within_budget,
    }


def check_public_site_no_js(browser: Any, public_origin: str) -> dict[str, Any]:
    """Verifies live `ef-public-site` with JavaScript disabled (`java_script_enabled=False`)."""
    context = browser.new_context(java_script_enabled=False)
    try:
        page = context.new_page()
        route_results: dict[str, Any] = {}

        for route in ("/", "/privacy", "/terms"):
            resp = page.goto(f"{public_origin}{route}", wait_until="domcontentloaded")
            status = resp.status if resp is not None else 0
            html = page.content()
            has_main = "<main" in html.lower()
            has_canvas = "<canvas" in html.lower()
            route_results[route] = {
                "http_status": status,
                "has_main": has_main,
                "zero_canvas": not has_canvas,
                "passed": status == 200 and has_main and not has_canvas,
            }

        invite_resp = page.goto(f"{public_origin}/invite", wait_until="domcontentloaded")
        invite_status = invite_resp.status if invite_resp is not None else 0
        invite_headers = invite_resp.headers if invite_resp is not None else {}
        invite_html = page.content()
        invite_noindex = verify_invite_noindex(invite_html, invite_headers)
        route_results["/invite"] = {
            "http_status": invite_status,
            "noindex_nofollow": invite_noindex,
            "passed": invite_status == 200 and invite_noindex,
        }

        # Check `/dei` first; fall back cleanly to `/recovery/wa` or `/recovery` if `/dei` is 404.
        dei_route_used = "/dei"
        dei_resp = page.goto(f"{public_origin}/dei", wait_until="domcontentloaded")
        if dei_resp is None or dei_resp.status == 404:
            dei_route_used = "/recovery/wa"
            dei_resp = page.goto(f"{public_origin}/recovery/wa", wait_until="domcontentloaded")
        dei_status = dei_resp.status if dei_resp is not None else 0
        dei_html = page.content()
        crisis_ok, crisis_offsets = verify_crisis_lines_before_facilities(dei_html)
        route_results[dei_route_used] = {
            "http_status": dei_status,
            "crisis_lines_before_facilities": crisis_ok,
            "offsets": crisis_offsets,
            "passed": dei_status == 200 and crisis_ok,
        }

        all_passed = all(entry["passed"] for entry in route_results.values())
        return {
            "passed": all_passed,
            "skipped": False,
            "public_origin": public_origin,
            "routes": route_results,
        }
    finally:
        context.close()


def run_browser_qualification(
    playwright_obj: Any,
    browsers: list[str],
    clinic_shell_url: str,
    mv3_popup_url: str,
    public_origin: str | None = None,
    mv3_budget_ms: float = DEFAULT_MV3_COLD_START_BUDGET_MS,
) -> dict[str, Any]:
    """Executes all browser checks for each requested engine and returns a structured report."""
    results: dict[str, Any] = {}

    for engine_name in browsers:
        browser_type = getattr(playwright_obj, engine_name)
        browser = browser_type.launch(headless=True)
        try:
            version = browser.version

            clinic_ctx = browser.new_context(viewport={"width": 1280, "height": 800})
            try:
                clinic_page = clinic_ctx.new_page()
                clinic_check = check_clinic_shell_page(clinic_page, clinic_shell_url)
            finally:
                clinic_ctx.close()

            popup_ctx = browser.new_context(viewport={"width": 360, "height": 520})
            try:
                popup_page = popup_ctx.new_page()
                popup_check = check_mv3_popup_page(
                    popup_page, mv3_popup_url, max_cold_start_ms=mv3_budget_ms
                )
            finally:
                popup_ctx.close()

            if public_origin is not None:
                public_check = check_public_site_no_js(browser, public_origin)
            else:
                public_check = {
                    "passed": True,
                    "skipped": True,
                    "reason": "--public-bin not provided",
                }

            engine_passed = (
                clinic_check["passed"] and popup_check["passed"] and public_check["passed"]
            )
            results[engine_name] = {
                "browser": engine_name,
                "version": version,
                "passed": engine_passed,
                "checks": {
                    "clinic_shell": clinic_check,
                    "mv3_popup": popup_check,
                    "public_site_no_js": public_check,
                },
            }
        finally:
            browser.close()

    overall_passed = all(entry["passed"] for entry in results.values())
    return {
        "schema_version": 1,
        "overall_passed": overall_passed,
        "browsers_requested": browsers,
        "results": results,
    }


def build_arg_parser() -> argparse.ArgumentParser:
    root = repo_root()
    parser = argparse.ArgumentParser(
        description="Run multi-browser Playwright E2E checks for Elysian Fields."
    )
    parser.add_argument(
        "--browsers",
        default="chromium,firefox,webkit",
        help="Comma-separated browser engines (chromium,firefox,webkit).",
    )
    parser.add_argument(
        "--web-dir",
        type=Path,
        default=root / "web" / "elysianfields",
        help="Path to the static `/elysianfields/` web shell directory.",
    )
    parser.add_argument(
        "--extension-dir",
        type=Path,
        default=root / "extension",
        help="Path to the Chrome MV3 extension directory containing `popup.html`.",
    )
    parser.add_argument(
        "--clinic-bin",
        type=Path,
        default=None,
        help="Optional path to compiled `ef-clinic-server` binary.",
    )
    parser.add_argument(
        "--public-bin",
        type=Path,
        default=None,
        help="Optional path to compiled `ef-public-site` binary.",
    )
    parser.add_argument(
        "--report",
        type=Path,
        default=None,
        help="Optional path to write the structured JSON E2E report.",
    )
    parser.add_argument(
        "--mv3-budget-ms",
        type=float,
        default=DEFAULT_MV3_COLD_START_BUDGET_MS,
        help="Maximum allowed cold-start time (ms) for MV3 extension popup.",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_arg_parser()
    args = parser.parse_args(argv)
    browsers = parse_browsers(args.browsers)

    try:
        from playwright.sync_api import sync_playwright
    except ImportError as exc:
        print(
            f"ERROR: Playwright Python package is required to run browser E2E checks: {exc}",
            file=sys.stderr,
        )
        return 2

    clinic_proc: ManagedServerProcess | None = None
    public_proc: ManagedServerProcess | None = None
    temp_dir = tempfile.TemporaryDirectory(prefix="ef-browser-e2e-")

    try:
        clinic_api_origin: str | None = None
        clinic_shell_url: str | None = None

        if args.clinic_bin is not None:
            clinic_port = find_free_port()
            clinic_proc = ManagedServerProcess(
                args.clinic_bin,
                {
                    "EF_CLINIC_BIND": f"127.0.0.1:{clinic_port}",
                    "EF_CLINIC_DATA_DIR": str(Path(temp_dir.name) / "clinic-wal"),
                    "EF_CLINIC_WEB_DIR": str(args.web_dir.resolve()),
                },
                clinic_port,
            )
            clinic_proc.start()
            clinic_api_origin = clinic_proc.origin
            if _probe_clinic_serves_shell(clinic_proc.origin):
                clinic_shell_url = f"{clinic_proc.origin}/elysianfields/"

        public_origin: str | None = None
        if args.public_bin is not None:
            public_port = find_free_port()
            public_proc = ManagedServerProcess(
                args.public_bin,
                {"EF_PUBLIC_BIND": f"127.0.0.1:{public_port}"},
                public_port,
            )
            public_proc.start()
            public_origin = public_proc.origin

        with StrictStaticPreviewServer(
            web_dir=args.web_dir,
            extension_dir=args.extension_dir,
            clinic_api_origin=clinic_api_origin,
        ) as preview:
            if clinic_shell_url is None:
                clinic_shell_url = f"{preview.origin}/elysianfields/"
            mv3_popup_url = f"{preview.origin}/mv3-popup/popup.html"

            with sync_playwright() as pw:
                report = run_browser_qualification(
                    pw,
                    browsers=browsers,
                    clinic_shell_url=clinic_shell_url,
                    mv3_popup_url=mv3_popup_url,
                    public_origin=public_origin,
                    mv3_budget_ms=args.mv3_budget_ms,
                )

        if args.report is not None:
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

        print(json.dumps(report, indent=2))
        return 0 if report["overall_passed"] else 1
    finally:
        if public_proc is not None:
            public_proc.stop()
        if clinic_proc is not None:
            clinic_proc.stop()
        temp_dir.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())
