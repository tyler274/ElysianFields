#!/usr/bin/env python3
"""Multi-browser Playwright E2E qualification suite for Elysian Fields.

Validates across Chromium, Firefox, and WebKit:
1. `/elysianfields/` clinic browser shell (HTTP 200, 0 console/page errors, 0 CSP
   violations under strict CSP, non-blank viewport, real single-owner ARIA tree via
   `a11y-bridge.js`, and keyboard `Tab` navigation).
2. Real WebAssembly compilation (`WebAssembly.compile` / `WebAssembly.instantiate`) under
   strict CSP (`script-src 'self' 'wasm-unsafe-eval'`) with 0 CSP violations (using an
   optional `--wasm-file` or a built-in minimal valid 41-byte Wasm module).
3. Chrome MV3 `extension/popup.html` (Decision D2 semantic-HTML popup: 0 console/page
   errors, 0 CSP violations, semantic landmarks, and cold start <= 250 ms).
4. Optional `--mobile-emulation` running Playwright Mobile WebKit (`iPhone 14`: 390x844,
   `has_touch=True`, `is_mobile=True`) and Mobile Chromium (`Pixel 7`: 412x915,
   `has_touch=True`, `is_mobile=True`) against `/elysianfields/mobile/` verifying zero
   horizontal overflow, touch ARIA landmarks, Arabic RTL (`?lang=ar`), and offline reload
   resilience retaining crisis lines (`988`, `911`, `SAMHSA`).
5. Optional live `ef-public-site` binary with JavaScript disabled (`java_script_enabled=False`):
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
import shutil
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
DESKTOP_WASM_BROTLI11_BUDGET_BYTES = int(3.5 * 1024 * 1024)
MOBILE_WASM_BROTLI11_BUDGET_BYTES = int(2.5 * 1024 * 1024)
BEVY_RUST_WASM_RUNTIME_ID = "bevy-rust-wasm"

# Minimal valid WebAssembly v1 binary module exporting `add(i32, i32) -> i32` (41 bytes).
MINIMAL_VALID_WASM_BYTES = bytes(
    [
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,  # magic + version 1
        0x01, 0x07, 0x01, 0x60, 0x02, 0x7F, 0x7F, 0x01, 0x7F,  # type: (i32, i32) -> i32
        0x03, 0x02, 0x01, 0x00,  # function section: func 0 -> type 0
        0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64, 0x00, 0x00,  # export "add" -> func 0
        0x0A, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6A, 0x0B,  # code body
    ]
)

MOBILE_PROFILES: dict[str, dict[str, Any]] = {
    "iphone_14_webkit": {
        "name": "iPhone 14 (Mobile WebKit)",
        "browser": "webkit",
        "viewport": {"width": 390, "height": 844},
        "has_touch": True,
        "is_mobile": True,
        "device_scale_factor": 3,
    },
    "pixel_7_chromium": {
        "name": "Pixel 7 (Mobile Chromium)",
        "browser": "chromium",
        "viewport": {"width": 412, "height": 915},
        "has_touch": True,
        "is_mobile": True,
        "device_scale_factor": 2.625,
    },
}


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


def measure_brotli11_bytes(payload: bytes) -> int | None:
    """Computes Brotli quality-11 compressed byte size if brotli module or CLI is available."""
    try:
        import brotli  # type: ignore[import-not-found]

        return len(brotli.compress(payload, quality=11))
    except ImportError:
        pass

    brotli_cli = shutil.which("brotli")
    if brotli_cli:
        try:
            proc = subprocess.run(
                [brotli_cli, "-q", "11", "-c"],
                input=payload,
                capture_output=True,
                check=True,
                timeout=30,
            )
            return len(proc.stdout)
        except (subprocess.SubprocessError, OSError):
            return None
    return None


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


def verify_crisis_lines_present(text_or_html: str) -> bool:
    """Checks that 988, 911, and SAMHSA are all visibly present in rendered text or HTML."""
    return "988" in text_or_html and "911" in text_or_html and "SAMHSA" in text_or_html


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


def evaluate_wasm_brotli11_budget(
    brotli11_bytes: int | None,
    max_bytes: int,
) -> bool:
    """Checks that a compiled `.wasm` binary's Brotli-11 size was measured and fits within budget."""
    return brotli11_bytes is not None and 0 < brotli11_bytes <= max_bytes


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
    if candidate.is_dir():
        candidate = candidate / "index.html"
    return candidate


class _ReusableTcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


class StrictStaticPreviewServer:
    """Localhost HTTP server enforcing strict CSP, `/elysianfields/` (including `/mobile/`
    and `/wasm-probe.wasm`), `/mv3-popup/`, and optional reverse-proxying of `/elysianfields/api/*`."""

    def __init__(
        self,
        web_dir: Path,
        extension_dir: Path,
        clinic_api_origin: str | None = None,
        wasm_file: Path | None = None,
        clinic_wasm: Path | None = None,
        mobile_wasm: Path | None = None,
    ) -> None:
        self.web_dir = web_dir.resolve()
        self.extension_dir = extension_dir.resolve()
        self.clinic_api_origin = clinic_api_origin.rstrip("/") if clinic_api_origin else None
        self.wasm_file = wasm_file.resolve() if wasm_file is not None else None
        self.clinic_wasm = clinic_wasm.resolve() if clinic_wasm is not None else None
        self.mobile_wasm = mobile_wasm.resolve() if mobile_wasm is not None else None
        self._wasm_bytes = (
            self.wasm_file.read_bytes() if self.wasm_file is not None else MINIMAL_VALID_WASM_BYTES
        )
        self._clinic_wasm_bytes = (
            self.clinic_wasm.read_bytes() if self.clinic_wasm is not None else None
        )
        self._mobile_wasm_bytes = (
            self.mobile_wasm.read_bytes() if self.mobile_wasm is not None else None
        )
        self._httpd: _ReusableTcpServer | None = None
        self._thread: threading.Thread | None = None
        self.port: int = 0

    @property
    def origin(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    @property
    def wasm_bytes(self) -> bytes:
        return self._wasm_bytes

    @property
    def clinic_wasm_bytes(self) -> bytes | None:
        return self._clinic_wasm_bytes

    @property
    def mobile_wasm_bytes(self) -> bytes | None:
        return self._mobile_wasm_bytes

    def start(self) -> None:
        web_dir = self.web_dir
        extension_dir = self.extension_dir
        clinic_api_origin = self.clinic_api_origin
        wasm_bytes = self._wasm_bytes
        clinic_wasm_bytes = self._clinic_wasm_bytes
        mobile_wasm_bytes = self._mobile_wasm_bytes
        wasm_manifest_bytes = json.dumps(
            {
                "clinicWasm": clinic_wasm_bytes is not None,
                "mobileWasm": mobile_wasm_bytes is not None,
            }
        ).encode("utf-8")

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

                if path_only == "/elysianfields/mobile":
                    self.send_response(308)
                    self.send_header("Location", "/elysianfields/mobile/")
                    self.send_header("Content-Security-Policy", CLINIC_STRICT_CSP)
                    self.end_headers()
                    return

                if path_only == "/elysianfields/wasm-probe.wasm":
                    self._send_bytes(200, "application/wasm", wasm_bytes, CLINIC_STRICT_CSP)
                    return

                if path_only in (
                    "/elysianfields/wasm-manifest.json",
                    "/elysianfields/mobile/wasm-manifest.json",
                ):
                    self._send_bytes(
                        200,
                        "application/json; charset=utf-8",
                        wasm_manifest_bytes,
                        CLINIC_STRICT_CSP,
                    )
                    return

                if path_only == "/elysianfields/ef_clinic_client.wasm":
                    if clinic_wasm_bytes is not None:
                        self._send_bytes(
                            200, "application/wasm", clinic_wasm_bytes, CLINIC_STRICT_CSP
                        )
                    else:
                        self._send_bytes(
                            404,
                            "text/plain; charset=utf-8",
                            b"Clinic Wasm Not Configured",
                            CLINIC_STRICT_CSP,
                        )
                    return

                if path_only == "/elysianfields/mobile/ef_mobile_client.wasm":
                    if mobile_wasm_bytes is not None:
                        self._send_bytes(
                            200, "application/wasm", mobile_wasm_bytes, CLINIC_STRICT_CSP
                        )
                    else:
                        self._send_bytes(
                            404,
                            "text/plain; charset=utf-8",
                            b"Mobile Wasm Not Configured",
                            CLINIC_STRICT_CSP,
                        )
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
                    extra = None
                    if path_only == "/elysianfields/mobile/sw.js":
                        extra = {"Service-Worker-Allowed": "/elysianfields/mobile/"}
                    self._serve_file(target, CLINIC_STRICT_CSP, extra_headers=extra)
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

            def _serve_file(
                self,
                file_path: Path,
                csp: str,
                extra_headers: dict[str, str] | None = None,
            ) -> None:
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
                elif ext == ".webmanifest":
                    ctype = "application/manifest+json; charset=utf-8"
                else:
                    guessed, _ = mimetypes.guess_type(str(file_path))
                    ctype = guessed or "application/octet-stream"
                self._send_bytes(200, ctype, file_path.read_bytes(), csp, extra_headers=extra_headers)

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


def check_clinic_shell_page(
    page: Any,
    clinic_shell_url: str,
    expect_bevy_wasm: bool = False,
    clinic_wasm_raw_bytes: int | None = None,
    clinic_wasm_brotli11_bytes: int | None = None,
) -> dict[str, Any]:
    """Exercises `/elysianfields/` on an open Playwright page (and `?lang=ar` RTL when Wasm is loaded)."""
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
        timeout=20_000,
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
                wasmRuntime: document.documentElement.dataset.efWasmRuntime || "static-shell",
                a11yOwner: document.documentElement.dataset.efA11yOwner || "shell",
                dir: document.documentElement.dir || "ltr",
                lang: document.documentElement.lang || "en",
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

    rtl_check: dict[str, Any] | None = None
    rtl_ok = True
    wasm_runtime_ok = True
    wasm_within_budget: bool | None = None

    if expect_bevy_wasm:
        rtl_url = f"{clinic_shell_url}?lang=ar"
        rtl_resp = page.goto(rtl_url, wait_until="networkidle")
        rtl_status = rtl_resp.status if rtl_resp is not None else 0
        page.wait_for_function(
            "() => document.documentElement.dataset.efReady === 'true'",
            timeout=20_000,
        )
        rtl_metrics = page.evaluate(
            """() => {
                const overlay = document.getElementById("ef-a11y-overlay");
                const bridgeNodes = overlay ? overlay.querySelectorAll("[data-ef-node-id]").length : 0;
                return {
                    dir: document.documentElement.dir || "ltr",
                    lang: document.documentElement.lang || "en",
                    wasmRuntime: document.documentElement.dataset.efWasmRuntime || "static-shell",
                    a11yOwner: document.documentElement.dataset.efA11yOwner || "shell",
                    cspViolations: Number(document.documentElement.dataset.efCspViolations || "0"),
                    bridgeNodes,
                };
            }"""
        )
        rtl_ok = (
            rtl_status == 200
            and rtl_metrics["dir"] == "rtl"
            and rtl_metrics["lang"] == "ar"
            and rtl_metrics["wasmRuntime"] == BEVY_RUST_WASM_RUNTIME_ID
            and rtl_metrics["a11yOwner"] == "rust-accesskit"
            and rtl_metrics["bridgeNodes"] >= 5
            and rtl_metrics["cspViolations"] == 0
        )
        rtl_check = {
            "url": rtl_url,
            "http_status": rtl_status,
            "passed": rtl_ok,
            **rtl_metrics,
        }
        wasm_within_budget = evaluate_wasm_brotli11_budget(
            clinic_wasm_brotli11_bytes,
            DESKTOP_WASM_BROTLI11_BUDGET_BYTES,
        )
        wasm_runtime_ok = (
            dom_metrics.get("wasmRuntime") == BEVY_RUST_WASM_RUNTIME_ID
            and dom_metrics.get("a11yOwner") == "rust-accesskit"
            and rtl_ok
            and wasm_within_budget
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
        and wasm_runtime_ok
    )

    result: dict[str, Any] = {
        "passed": passed,
        "url": clinic_shell_url,
        "http_status": http_status,
        "load_ms": round(load_ms, 2),
        "console_errors": console_errors,
        "page_errors": page_errors,
        "csp_violations": dom_metrics["cspViolations"],
        "non_blank": dom_metrics["nonBlank"],
        "backend": dom_metrics["backend"],
        "wasm_runtime": dom_metrics.get("wasmRuntime", "static-shell"),
        "a11y_owner": dom_metrics.get("a11yOwner", "shell"),
        "a11y_bridge_nodes": dom_metrics["bridgeNodes"],
        "keyboard_focus": focused_info,
    }
    if expect_bevy_wasm:
        result["wasm_raw_bytes"] = clinic_wasm_raw_bytes
        result["wasm_brotli11_bytes"] = clinic_wasm_brotli11_bytes
        result["wasm_budget_bytes"] = DESKTOP_WASM_BROTLI11_BUDGET_BYTES
        result["wasm_within_budget"] = wasm_within_budget
        result["rtl"] = rtl_check
    return result


def check_wasm_csp_compile(
    page: Any,
    wasm_host_url: str,
    wasm_probe_url: str,
    is_minimal_builtin: bool,
    raw_bytes: int | None = None,
    brotli11_bytes: int | None = None,
) -> dict[str, Any]:
    """Verifies real `WebAssembly.compile` / `WebAssembly.instantiate` succeeds under strict CSP
    (`'wasm-unsafe-eval'`, zero `'unsafe-eval'`) with 0 CSP violations."""
    console_errors: list[str] = []
    page_errors: list[str] = []

    page.on(
        "console",
        lambda msg: console_errors.append(msg.text) if msg.type == "error" else None,
    )
    page.on("pageerror", lambda err: page_errors.append(str(err)))

    response = page.goto(wasm_host_url, wait_until="domcontentloaded")
    http_status = response.status if response is not None else 0

    result = page.evaluate(
        """async ({ wasmUrl, isMinimal }) => {
            const start = performance.now();
            let cspViolations = 0;
            const listener = () => { cspViolations += 1; };
            document.addEventListener("securitypolicyviolation", listener);
            try {
                const resp = await fetch(wasmUrl);
                const bytes = await resp.arrayBuffer();
                const mod = await WebAssembly.compile(bytes);
                let instantiated = false;
                let addResult = null;
                if (isMinimal) {
                    const inst = await WebAssembly.instantiate(mod);
                    instantiated = true;
                    addResult = typeof inst.exports.add === "function" ? inst.exports.add(20, 22) : null;
                }
                const elapsedMs = performance.now() - start;
                return {
                    fetchStatus: resp.status,
                    byteLength: bytes.byteLength,
                    compiled: mod instanceof WebAssembly.Module,
                    instantiated,
                    addResult,
                    cspViolations,
                    compileMs: elapsedMs,
                    error: null,
                };
            } catch (err) {
                return {
                    fetchStatus: 0,
                    byteLength: 0,
                    compiled: false,
                    instantiated: false,
                    addResult: null,
                    cspViolations,
                    compileMs: performance.now() - start,
                    error: err instanceof Error ? err.message : String(err),
                };
            } finally {
                document.removeEventListener("securitypolicyviolation", listener);
            }
        }""",
        {"wasmUrl": wasm_probe_url, "isMinimal": is_minimal_builtin},
    )

    minimal_ok = (not is_minimal_builtin) or (
        result["instantiated"] and result["addResult"] == 42
    )
    passed = (
        http_status == 200
        and result["fetchStatus"] == 200
        and result["compiled"]
        and minimal_ok
        and result["cspViolations"] == 0
        and result["error"] is None
        and len(console_errors) == 0
        and len(page_errors) == 0
    )

    return {
        "passed": passed,
        "wasm_url": wasm_probe_url,
        "mode": "minimal-builtin" if is_minimal_builtin else "custom-wasm-file",
        "compiled": result["compiled"],
        "instantiated": result["instantiated"],
        "add_result": result["addResult"],
        "byte_length": result["byteLength"],
        "raw_bytes": raw_bytes if raw_bytes is not None else result["byteLength"],
        "brotli11_bytes": brotli11_bytes,
        "compile_ms": round(float(result["compileMs"]), 2),
        "csp_violations": result["cspViolations"],
        "console_errors": console_errors,
        "page_errors": page_errors,
        "error": result["error"],
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


def check_mobile_pwa_profile(
    browser: Any,
    profile_key: str,
    profile_spec: dict[str, Any],
    mobile_shell_url: str,
    expect_bevy_wasm: bool = False,
    mobile_wasm_raw_bytes: int | None = None,
    mobile_wasm_brotli11_bytes: int | None = None,
) -> dict[str, Any]:
    """Exercises `/elysianfields/mobile/` under touch mobile emulation (LTR, Arabic RTL, and offline reload)."""
    context = browser.new_context(
        viewport=profile_spec["viewport"],
        has_touch=profile_spec["has_touch"],
        is_mobile=profile_spec["is_mobile"],
        device_scale_factor=profile_spec["device_scale_factor"],
    )
    try:
        page = context.new_page()
        console_errors: list[str] = []
        page_errors: list[str] = []

        page.on(
            "console",
            lambda msg: console_errors.append(msg.text) if msg.type == "error" else None,
        )
        page.on("pageerror", lambda err: page_errors.append(str(err)))

        # 1. Online LTR boot check
        resp = page.goto(mobile_shell_url, wait_until="networkidle")
        http_status = resp.status if resp is not None else 0
        page.wait_for_function(
            "() => document.documentElement.dataset.efMobileReady === 'true'",
            timeout=20_000,
        )

        ltr_metrics = page.evaluate(
            """() => {
                const overlay = document.getElementById("ef-a11y-overlay");
                const nodes = overlay ? overlay.querySelectorAll("[data-ef-node-id]").length : 0;
                const text = document.body.innerText || "";
                return {
                    scrollWidth: document.documentElement.scrollWidth,
                    innerWidth: window.innerWidth,
                    zeroOverflow: document.documentElement.scrollWidth <= window.innerWidth,
                    cspViolations: Number(document.documentElement.dataset.efCspViolations || "0"),
                    dir: document.documentElement.dir,
                    lang: document.documentElement.lang,
                    wasmRuntime: document.documentElement.dataset.efWasmRuntime || "static-shell",
                    a11yOwner: document.documentElement.dataset.efA11yOwner || "shell",
                    overlayPresent: overlay !== null,
                    bridgeNodes: nodes,
                    hasNav: document.querySelector("nav") !== null,
                    hasMain: document.querySelector("main") !== null,
                    crisisPresent: text.includes("988") && text.includes("911") && text.includes("SAMHSA"),
                    swState: document.documentElement.dataset.efSwState || "unknown",
                    hasController: Boolean(navigator.serviceWorker && navigator.serviceWorker.controller),
                };
            }"""
        )

        # 2. Arabic RTL check (`?lang=ar`)
        rtl_url = f"{mobile_shell_url}?lang=ar"
        rtl_resp = page.goto(rtl_url, wait_until="networkidle")
        rtl_status = rtl_resp.status if rtl_resp is not None else 0
        page.wait_for_function(
            "() => document.documentElement.dataset.efMobileReady === 'true'",
            timeout=20_000,
        )
        rtl_metrics = page.evaluate(
            """() => ({
                dir: document.documentElement.dir,
                lang: document.documentElement.lang,
                wasmRuntime: document.documentElement.dataset.efWasmRuntime || "static-shell",
                a11yOwner: document.documentElement.dataset.efA11yOwner || "shell",
                scrollWidth: document.documentElement.scrollWidth,
                innerWidth: window.innerWidth,
                zeroOverflow: document.documentElement.scrollWidth <= window.innerWidth,
                cspViolations: Number(document.documentElement.dataset.efCspViolations || "0"),
            })"""
        )

        # 3. Offline reload resilience: return to canonical URL while online so SW claims client,
        # then go offline and verify crisis lines (988/911/SAMHSA) + non-blank UI survive reload.
        page.goto(mobile_shell_url, wait_until="networkidle")
        page.wait_for_function(
            "() => document.documentElement.dataset.efMobileReady === 'true'",
            timeout=20_000,
        )
        sw_controlling = page.evaluate(
            """async () => {
                if (!("serviceWorker" in navigator)) return false;
                try {
                    const timeout = new Promise((resolve) => setTimeout(() => resolve(null), 3000));
                    const reg = await Promise.race([navigator.serviceWorker.ready, timeout]);
                    if (!reg) return false;
                    return Boolean(navigator.serviceWorker.controller);
                } catch (_err) {
                    return false;
                }
            }"""
        )
        if not sw_controlling:
            page.reload(wait_until="networkidle")
            page.wait_for_function(
                "() => document.documentElement.dataset.efMobileReady === 'true'",
                timeout=20_000,
            )

        page_errors_before_offline = len(page_errors)
        offline_reload_ok = False
        offline_non_blank = False
        offline_crisis_present = False
        offline_error: str | None = None

        context.set_offline(True)
        try:
            try:
                page.reload(wait_until="domcontentloaded", timeout=10_000)
                offline_metrics = page.evaluate(
                    """() => {
                        const text = (document.body && document.body.innerText) ? document.body.innerText : "";
                        return {
                            nonBlank: text.trim().length > 0,
                            crisisPresent: text.includes("988") && text.includes("911") && text.includes("SAMHSA"),
                        };
                    }"""
                )
                offline_non_blank = bool(offline_metrics["nonBlank"])
                offline_crisis_present = bool(offline_metrics["crisisPresent"])
                offline_reload_ok = offline_non_blank and offline_crisis_present
            except Exception as exc:  # noqa: BLE001
                offline_error = str(exc)
        finally:
            context.set_offline(False)

        offline_page_errors = page_errors[page_errors_before_offline:]

        wasm_within_budget: bool | None = None
        wasm_ok = True
        if expect_bevy_wasm:
            wasm_within_budget = evaluate_wasm_brotli11_budget(
                mobile_wasm_brotli11_bytes,
                MOBILE_WASM_BROTLI11_BUDGET_BYTES,
            )
            wasm_ok = (
                ltr_metrics.get("wasmRuntime") == BEVY_RUST_WASM_RUNTIME_ID
                and ltr_metrics.get("a11yOwner") == "rust-accesskit"
                and rtl_metrics.get("wasmRuntime") == BEVY_RUST_WASM_RUNTIME_ID
                and rtl_metrics.get("a11yOwner") == "rust-accesskit"
                and wasm_within_budget
            )

        passed = (
            http_status == 200
            and rtl_status == 200
            and len(console_errors) == 0
            and len(page_errors) == 0
            and ltr_metrics["cspViolations"] == 0
            and ltr_metrics["zeroOverflow"]
            and ltr_metrics["overlayPresent"]
            and ltr_metrics["bridgeNodes"] >= 5
            and ltr_metrics["hasNav"]
            and ltr_metrics["hasMain"]
            and ltr_metrics["crisisPresent"]
            and rtl_metrics["dir"] == "rtl"
            and rtl_metrics["lang"] == "ar"
            and rtl_metrics["zeroOverflow"]
            and rtl_metrics["cspViolations"] == 0
            and offline_reload_ok
            and len(offline_page_errors) == 0
            and wasm_ok
        )

        profile_report: dict[str, Any] = {
            "profile": profile_key,
            "name": profile_spec["name"],
            "browser": profile_spec["browser"],
            "viewport": profile_spec["viewport"],
            "passed": passed,
            "http_status": http_status,
            "console_errors": console_errors,
            "page_errors": page_errors,
            "wasm_runtime": ltr_metrics.get("wasmRuntime", "static-shell"),
            "a11y_owner": ltr_metrics.get("a11yOwner", "shell"),
            "ltr": ltr_metrics,
            "rtl": rtl_metrics,
            "offline": {
                "reloaded": offline_reload_ok,
                "non_blank": offline_non_blank,
                "crisis_lines_present": offline_crisis_present,
                "error": offline_error,
            },
        }
        if expect_bevy_wasm:
            profile_report["wasm_raw_bytes"] = mobile_wasm_raw_bytes
            profile_report["wasm_brotli11_bytes"] = mobile_wasm_brotli11_bytes
            profile_report["wasm_budget_bytes"] = MOBILE_WASM_BROTLI11_BUDGET_BYTES
            profile_report["wasm_within_budget"] = wasm_within_budget
        return profile_report
    finally:
        context.close()


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

        # Check `/dei` first; fall back cleanly to `/recovery/wa` if `/dei` is 404.
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
    wasm_host_url: str | None = None,
    wasm_probe_url: str | None = None,
    wasm_is_minimal_builtin: bool = True,
    wasm_raw_bytes: int | None = None,
    wasm_brotli11_bytes: int | None = None,
    mobile_emulation: bool = False,
    mobile_shell_url: str | None = None,
    expect_clinic_bevy_wasm: bool = False,
    clinic_wasm_raw_bytes: int | None = None,
    clinic_wasm_brotli11_bytes: int | None = None,
    expect_mobile_bevy_wasm: bool = False,
    mobile_wasm_raw_bytes: int | None = None,
    mobile_wasm_brotli11_bytes: int | None = None,
) -> dict[str, Any]:
    """Executes all browser checks for each requested engine and returns a structured report."""
    results: dict[str, Any] = {}
    mobile_results: dict[str, Any] = {}

    for engine_name in browsers:
        browser_type = getattr(playwright_obj, engine_name)
        browser = browser_type.launch(headless=True)
        try:
            version = browser.version

            clinic_ctx = browser.new_context(viewport={"width": 1280, "height": 800})
            try:
                clinic_page = clinic_ctx.new_page()
                clinic_check = check_clinic_shell_page(
                    clinic_page,
                    clinic_shell_url,
                    expect_bevy_wasm=expect_clinic_bevy_wasm,
                    clinic_wasm_raw_bytes=clinic_wasm_raw_bytes,
                    clinic_wasm_brotli11_bytes=clinic_wasm_brotli11_bytes,
                )
            finally:
                clinic_ctx.close()

            if wasm_probe_url is not None:
                wasm_ctx = browser.new_context(viewport={"width": 1280, "height": 800})
                try:
                    wasm_page = wasm_ctx.new_page()
                    wasm_check = check_wasm_csp_compile(
                        wasm_page,
                        wasm_host_url=wasm_host_url or clinic_shell_url,
                        wasm_probe_url=wasm_probe_url,
                        is_minimal_builtin=wasm_is_minimal_builtin,
                        raw_bytes=wasm_raw_bytes,
                        brotli11_bytes=wasm_brotli11_bytes,
                    )
                finally:
                    wasm_ctx.close()
            else:
                wasm_check = {
                    "passed": True,
                    "skipped": True,
                    "reason": "wasm_probe_url not configured",
                }

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

            if mobile_emulation and mobile_shell_url is not None:
                for profile_key, profile_spec in MOBILE_PROFILES.items():
                    if profile_spec["browser"] == engine_name:
                        mobile_results[profile_key] = check_mobile_pwa_profile(
                            browser,
                            profile_key=profile_key,
                            profile_spec=profile_spec,
                            mobile_shell_url=mobile_shell_url,
                            expect_bevy_wasm=expect_mobile_bevy_wasm,
                            mobile_wasm_raw_bytes=mobile_wasm_raw_bytes,
                            mobile_wasm_brotli11_bytes=mobile_wasm_brotli11_bytes,
                        )

            engine_passed = (
                clinic_check["passed"]
                and wasm_check["passed"]
                and popup_check["passed"]
                and public_check["passed"]
            )
            results[engine_name] = {
                "browser": engine_name,
                "version": version,
                "passed": engine_passed,
                "checks": {
                    "clinic_shell": clinic_check,
                    "wasm_csp": wasm_check,
                    "mv3_popup": popup_check,
                    "public_site_no_js": public_check,
                },
            }
        finally:
            browser.close()

    wasm_bundles: dict[str, Any] = {}
    wasm_budgets_ok = True
    if expect_clinic_bevy_wasm:
        clinic_budget_ok = evaluate_wasm_brotli11_budget(
            clinic_wasm_brotli11_bytes, DESKTOP_WASM_BROTLI11_BUDGET_BYTES
        )
        wasm_bundles["clinic"] = {
            "runtime": BEVY_RUST_WASM_RUNTIME_ID,
            "raw_bytes": clinic_wasm_raw_bytes,
            "brotli11_bytes": clinic_wasm_brotli11_bytes,
            "budget_bytes": DESKTOP_WASM_BROTLI11_BUDGET_BYTES,
            "within_budget": clinic_budget_ok,
        }
        wasm_budgets_ok = wasm_budgets_ok and clinic_budget_ok
    if expect_mobile_bevy_wasm:
        mobile_budget_ok = evaluate_wasm_brotli11_budget(
            mobile_wasm_brotli11_bytes, MOBILE_WASM_BROTLI11_BUDGET_BYTES
        )
        wasm_bundles["mobile"] = {
            "runtime": BEVY_RUST_WASM_RUNTIME_ID,
            "raw_bytes": mobile_wasm_raw_bytes,
            "brotli11_bytes": mobile_wasm_brotli11_bytes,
            "budget_bytes": MOBILE_WASM_BROTLI11_BUDGET_BYTES,
            "within_budget": mobile_budget_ok,
        }
        wasm_budgets_ok = wasm_budgets_ok and mobile_budget_ok

    mobile_passed = all(entry["passed"] for entry in mobile_results.values())
    overall_passed = (
        all(entry["passed"] for entry in results.values()) and mobile_passed and wasm_budgets_ok
    )

    report: dict[str, Any] = {
        "schema_version": 1,
        "overall_passed": overall_passed,
        "browsers_requested": browsers,
        "results": results,
    }
    if wasm_bundles:
        report["wasm_bundles"] = wasm_bundles
    if mobile_emulation:
        report["mobile_emulation"] = {
            "enabled": True,
            "passed": mobile_passed,
            "profiles": mobile_results,
        }
    return report


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
        "--wasm-file",
        type=Path,
        default=None,
        help="Optional path to a compiled `.wasm` binary to compile under strict CSP (`'wasm-unsafe-eval'`).",
    )
    parser.add_argument(
        "--clinic-wasm",
        type=Path,
        default=None,
        help="Optional path to compiled `ef_clinic_client.wasm` served under `/elysianfields/ef_clinic_client.wasm`.",
    )
    parser.add_argument(
        "--mobile-wasm",
        type=Path,
        default=None,
        help="Optional path to compiled `ef_mobile_client.wasm` served under `/elysianfields/mobile/ef_mobile_client.wasm`.",
    )
    parser.add_argument(
        "--mobile-emulation",
        action="store_true",
        help="Run Mobile WebKit (iPhone 14) and Mobile Chromium (Pixel 7) PWA, RTL, and offline reload checks.",
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

    if args.wasm_file is not None and not args.wasm_file.is_file():
        print(f"ERROR: --wasm-file not found: {args.wasm_file}", file=sys.stderr)
        return 2
    if args.clinic_wasm is not None and not args.clinic_wasm.is_file():
        print(f"ERROR: --clinic-wasm not found: {args.clinic_wasm}", file=sys.stderr)
        return 2
    if args.mobile_wasm is not None and not args.mobile_wasm.is_file():
        print(f"ERROR: --mobile-wasm not found: {args.mobile_wasm}", file=sys.stderr)
        return 2

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
            wasm_file=args.wasm_file,
            clinic_wasm=args.clinic_wasm,
            mobile_wasm=args.mobile_wasm,
        ) as preview:
            if args.clinic_wasm is not None or clinic_shell_url is None:
                clinic_shell_url = f"{preview.origin}/elysianfields/"
            mv3_popup_url = f"{preview.origin}/mv3-popup/popup.html"
            wasm_host_url = f"{preview.origin}/elysianfields/"
            wasm_probe_url = f"{preview.origin}/elysianfields/wasm-probe.wasm"
            mobile_shell_url = f"{preview.origin}/elysianfields/mobile/"
            wasm_raw_bytes = len(preview.wasm_bytes)
            wasm_brotli11_bytes = measure_brotli11_bytes(preview.wasm_bytes)
            clinic_wasm_raw_bytes = (
                len(preview.clinic_wasm_bytes) if preview.clinic_wasm_bytes is not None else None
            )
            clinic_wasm_brotli11_bytes = (
                measure_brotli11_bytes(preview.clinic_wasm_bytes)
                if preview.clinic_wasm_bytes is not None
                else None
            )
            mobile_wasm_raw_bytes = (
                len(preview.mobile_wasm_bytes) if preview.mobile_wasm_bytes is not None else None
            )
            mobile_wasm_brotli11_bytes = (
                measure_brotli11_bytes(preview.mobile_wasm_bytes)
                if preview.mobile_wasm_bytes is not None
                else None
            )

            with sync_playwright() as pw:
                report = run_browser_qualification(
                    pw,
                    browsers=browsers,
                    clinic_shell_url=clinic_shell_url,
                    mv3_popup_url=mv3_popup_url,
                    public_origin=public_origin,
                    mv3_budget_ms=args.mv3_budget_ms,
                    wasm_host_url=wasm_host_url,
                    wasm_probe_url=wasm_probe_url,
                    wasm_is_minimal_builtin=args.wasm_file is None,
                    wasm_raw_bytes=wasm_raw_bytes,
                    wasm_brotli11_bytes=wasm_brotli11_bytes,
                    mobile_emulation=args.mobile_emulation,
                    mobile_shell_url=mobile_shell_url,
                    expect_clinic_bevy_wasm=args.clinic_wasm is not None,
                    clinic_wasm_raw_bytes=clinic_wasm_raw_bytes,
                    clinic_wasm_brotli11_bytes=clinic_wasm_brotli11_bytes,
                    expect_mobile_bevy_wasm=args.mobile_wasm is not None,
                    mobile_wasm_raw_bytes=mobile_wasm_raw_bytes,
                    mobile_wasm_brotli11_bytes=mobile_wasm_brotli11_bytes,
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
