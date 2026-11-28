"""Unit tests validating deploy/ containers, compose stack, and run_local_stack.sh."""

from __future__ import annotations

import subprocess
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEPLOY_DIR = REPO_ROOT / "deploy"
LAUNCHER = REPO_ROOT / "scripts" / "run_local_stack.sh"
DESKTOP_LAUNCHER = REPO_ROOT / "scripts" / "run_desktop.sh"
COMPOSE_VERIFIER = REPO_ROOT / "scripts" / "verify_compose_stack.sh"


class DeployAndLocalStackTests(unittest.TestCase):
    def test_containerfiles_are_rootless_multistage_and_have_healthchecks(self) -> None:
        expected = {
            "Containerfile.clinic": ("ef-clinic-server", "/elysianfields/"),
            "Containerfile.mobile": ("ef-mobile-server", "/elysianfields/mobile-api/readyz"),
            "Containerfile.public": ("ef-public-site", "/readyz"),
        }
        for filename, (binary_name, health_path) in expected.items():
            path = DEPLOY_DIR / filename
            self.assertTrue(path.is_file(), f"missing {path}")
            text = path.read_text(encoding="utf-8")
            self.assertIn("AS builder", text, filename)
            self.assertIn("AS runtime", text, filename)
            self.assertIn(f"--bin {binary_name}", text, filename)
            self.assertIn("USER 10001:10001", text, filename)
            self.assertIn("HEALTHCHECK", text, filename)
            self.assertIn(health_path, text, filename)
            self.assertNotIn("SECRET", text.upper(), f"unexpected secret marker in {filename}")

    def test_docker_compose_defines_isolated_postgres18_and_three_services(self) -> None:
        compose = (DEPLOY_DIR / "docker-compose.yml").read_text(encoding="utf-8")
        self.assertIn("image: postgres:18", compose)
        self.assertIn("ef_clinic", compose)
        self.assertIn("ef_mobile", compose)
        self.assertIn("deploy/Containerfile.clinic", compose)
        self.assertIn("deploy/Containerfile.mobile", compose)
        self.assertIn("deploy/Containerfile.public", compose)
        self.assertIn("read_only: true", compose)
        self.assertIn("no-new-privileges:true", compose)

    def test_postgres_init_script_syntax_and_isolation(self) -> None:
        init_script = DEPLOY_DIR / "postgres-init" / "01-init-isolated-dbs.sh"
        self.assertTrue(init_script.is_file())
        proc = subprocess.run(["bash", "-n", str(init_script)], capture_output=True, text=True, check=False)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        text = init_script.read_text(encoding="utf-8")
        self.assertIn("CREATE DATABASE ef_clinic;", text)
        self.assertIn("CREATE DATABASE ef_mobile;", text)

    def test_run_local_stack_syntax_help_and_unknown_flag(self) -> None:
        syntax = subprocess.run(["bash", "-n", str(LAUNCHER)], capture_output=True, text=True, check=False)
        self.assertEqual(syntax.returncode, 0, syntax.stderr)

        help_res = subprocess.run(["bash", str(LAUNCHER), "--help"], capture_output=True, text=True, check=False)
        self.assertEqual(help_res.returncode, 0, help_res.stderr)
        self.assertIn("--smoke", help_res.stdout)

        bad_res = subprocess.run(["bash", str(LAUNCHER), "--bogus"], capture_output=True, text=True, check=False)
        self.assertEqual(bad_res.returncode, 2)

    def test_cross_platform_workflow_covers_wave9_proofs(self) -> None:
        wf = (REPO_ROOT / ".github" / "workflows" / "cross-platform.yml").read_text(encoding="utf-8")
        for needle in (
            "scripts/verify_pgwire_live.sh",
            "swift test --package-path platform/ios",
            "kotlin_test_execution",
            "--clinic-wasm",
            "--mobile-wasm",
            "oci-container-build:",
            "Rootless OCI Container Builds & Smoke (public, clinic, mobile)",
            "deploy/Containerfile.public",
            "deploy/Containerfile.clinic",
            "deploy/Containerfile.mobile",
            "--tmpfs /var/lib/elysian/clinic:uid=10001,gid=10001",
            "--tmpfs /var/lib/elysian/mobile-data:uid=10001,gid=10001",
            "--tmpfs /var/lib/elysian/mobile-journal:uid=10001,gid=10001",
            'uid" != "10001"',
            "scripts/qa_multilingual_gui.py",
            "scripts/qa_ux_accessibility.py",
            "web/elysianfields/**",
            "extension/**",
            "fonts-noto-core",
            "fonts-noto-cjk",
            "fonts-noto-extra",
            "fc-cache -f",
            "multilingual-gui-qa-report.json",
            "multilingual-screenshots/",
            "ux-accessibility-qa-report.json",
            "ux-screenshots/",
            "desktop-linux-native:",
            "Desktop Linux x86_64 Native Bevy Apps",
            "crates/ef-clinic-desktop/**",
            "crates/ef-mobile-desktop/**",
            "scripts/run_desktop.sh",
            "ef-clinic-desktop",
            "ef-mobile-desktop",
            "--headless-smoke",
            "macos-desktop-apps.json",
            "windows-desktop-apps.json",
            "desktop-linux-native.json",
        ):
            self.assertIn(needle, wf)

    def test_run_desktop_launcher_syntax_help_and_unknown_flag(self) -> None:
        self.assertTrue(DESKTOP_LAUNCHER.is_file())
        syntax = subprocess.run(["bash", "-n", str(DESKTOP_LAUNCHER)], capture_output=True, text=True, check=False)
        self.assertEqual(syntax.returncode, 0, syntax.stderr)

        help_res = subprocess.run(["bash", str(DESKTOP_LAUNCHER), "--help"], capture_output=True, text=True, check=False)
        self.assertEqual(help_res.returncode, 0, help_res.stderr)
        for needle in ("clinic", "mobile", "--demo", "--with-server", "--smoke", "--locale", "--theme"):
            self.assertIn(needle, help_res.stdout)

        bad_res = subprocess.run(["bash", str(DESKTOP_LAUNCHER), "--bogus"], capture_output=True, text=True, check=False)
        self.assertEqual(bad_res.returncode, 2)

    def test_flake_exposes_linux_bevy_desktop_gui_runtime_libs(self) -> None:
        flake_text = (REPO_ROOT / "flake.nix").read_text(encoding="utf-8")
        for lib_name in (
            "fontconfig.lib",
            "libxkbcommon",
            "wayland",
            "xorg.libX11",
            "xorg.libXcursor",
            "xorg.libXi",
            "xorg.libXrandr",
            "vulkan-loader",
            "LD_LIBRARY_PATH",
        ):
            self.assertIn(lib_name, flake_text)

    def test_caddyfile_and_compose_single_origin_gateway(self) -> None:
        caddyfile = DEPLOY_DIR / "Caddyfile"
        self.assertTrue(caddyfile.is_file(), f"missing {caddyfile}")
        caddy_text = caddyfile.read_text(encoding="utf-8")
        for needle in (
            ":8080",
            "/elysianfields/api/*",
            "/elysianfields/mobile-api/*",
            "/elysianfields/*",
            "/*",
            "clinic:8088",
            "mobile:8089",
            "public-site:8082",
            "X-Content-Type-Options",
            "nosniff",
            "X-Frame-Options",
            "DENY",
            "Referrer-Policy",
            "no-referrer",
        ):
            self.assertIn(needle, caddy_text, f"missing `{needle}` in deploy/Caddyfile")

        compose_text = (DEPLOY_DIR / "docker-compose.yml").read_text(encoding="utf-8")
        for needle in (
            "gateway:",
            "image: caddy:2-alpine",
            "profiles:",
            "- gateway",
            "./Caddyfile:/etc/caddy/Caddyfile:ro",
            "127.0.0.1:8080:8080",
        ):
            self.assertIn(needle, compose_text, f"missing `{needle}` in deploy/docker-compose.yml")

    def test_android_gradle_structure_and_manifest_security_invariants(self) -> None:
        android_dir = REPO_ROOT / "platform" / "android"
        settings_kts = android_dir / "settings.gradle.kts"
        build_kts = android_dir / "build.gradle.kts"
        config_kt = android_dir / "src" / "main" / "kotlin" / "fields" / "elysian" / "mobile" / "MobileAppConfig.kt"
        manifest = android_dir / "src" / "main" / "AndroidManifest.xml"

        for required in (settings_kts, build_kts, config_kt, manifest):
            self.assertTrue(required.is_file(), f"missing {required}")

        self.assertIn("elysian-mobile-android", settings_kts.read_text(encoding="utf-8"))
        self.assertIn("fields.elysian.mobile", build_kts.read_text(encoding="utf-8"))

        config_text = config_kt.read_text(encoding="utf-8")
        self.assertIn("package fields.elysian.mobile", config_text)
        self.assertNotIn("import android.", config_text)

        tree = ET.parse(manifest)
        root = tree.getroot()
        android_ns = "{http://schemas.android.com/apk/res/android}"
        self.assertEqual(root.attrib.get("package"), "fields.elysian.mobile")

        app = root.find("application")
        self.assertIsNotNone(app, "AndroidManifest.xml missing <application>")
        assert app is not None
        self.assertEqual(app.attrib.get(f"{android_ns}allowBackup"), "false")
        self.assertEqual(app.attrib.get(f"{android_ns}usesCleartextTraffic"), "false")

        queries = root.find("queries")
        self.assertIsNotNone(queries, "AndroidManifest.xml missing <queries> for tel: crisis intents")
        assert queries is not None
        tel_schemes = [
            data.attrib.get(f"{android_ns}scheme")
            for data in queries.findall(".//data")
        ]
        self.assertIn("tel", tel_schemes, "AndroidManifest.xml must declare explicit tel: crisis intent query")

    def test_verify_compose_stack_script_and_ci_wiring(self) -> None:
        self.assertTrue(COMPOSE_VERIFIER.is_file(), f"missing {COMPOSE_VERIFIER}")
        syntax = subprocess.run(["bash", "-n", str(COMPOSE_VERIFIER)], capture_output=True, text=True, check=False)
        self.assertEqual(syntax.returncode, 0, syntax.stderr)

        help_res = subprocess.run(["bash", str(COMPOSE_VERIFIER), "--help"], capture_output=True, text=True, check=False)
        self.assertEqual(help_res.returncode, 0, help_res.stderr)
        for needle in ("--no-build", "--clinic-image", "--mobile-image", "--public-image", "--report"):
            self.assertIn(needle, help_res.stdout)

        bad_res = subprocess.run(["bash", str(COMPOSE_VERIFIER), "--bogus"], capture_output=True, text=True, check=False)
        self.assertEqual(bad_res.returncode, 2)

        script_text = COMPOSE_VERIFIER.read_text(encoding="utf-8")
        for needle in (
            "deploy/docker-compose.yml",
            "--profile gateway",
            "http://127.0.0.1:8080",
            "EF_CLINIC_SEED_DEMO",
            "/dei?state=CA&service=crisis",
            "/dei?q=%3Cscript%3Ealert(1)%3C/script%3E",
            "&lt;script&gt;",
            "/elysianfields/api/today",
            "scheduled_visits",
            "/elysianfields/api/health",
            "/elysianfields/api/healthz",
            "/elysianfields/api/metrics",
            "/elysianfields/mobile/",
            "/elysianfields/mobile-api/readyz",
            "/elysianfields/mobile-api/healthz",
            "/elysianfields/mobile-api/metrics",
            "X-Request-Id",
            "ef-compose-trace-42",
            "ef_clinic_db_ready",
            "ef_mobile_db_ready",
            "ef_clinic_wal_events",
            "::error",
            "last_error=",
            "services=[",
            "down -v",
        ):
            self.assertIn(needle, script_text)

        wf = (REPO_ROOT / ".github" / "workflows" / "cross-platform.yml").read_text(encoding="utf-8")
        self.assertIn("scripts/verify_compose_stack.sh", wf)
        self.assertIn("compose-gateway-e2e.json", wf)

    def test_engineering_book_has_all_eight_chapters_and_zero_ticket_ids(self) -> None:
        book_src = REPO_ROOT / "book" / "src"
        summary = (book_src / "SUMMARY.md").read_text(encoding="utf-8")
        expected_chapters = (
            "clean-room-protocol.md",
            "architecture.md",
            "crate-map.md",
            "desktop-apps.md",
            "web-and-mobile.md",
            "servers-and-persistence.md",
            "deployment-and-releases.md",
            "ci-and-verification.md",
        )
        for chapter in expected_chapters:
            self.assertIn(f"./{chapter}", summary, f"missing {chapter} in book/src/SUMMARY.md")
            path = book_src / chapter
            self.assertTrue(path.is_file(), f"missing chapter file {path}")
            content = path.read_text(encoding="utf-8")
            self.assertGreater(len(content.strip()), 200, f"chapter {chapter} appears empty")
            self.assertNotRegex(content, r"\bTYL-[0-9]+\b")

    def test_release_workflow_cache_alignment_cargo_deny_and_sbom_wiring(self) -> None:
        release_wf = (REPO_ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        for needle in (
            "key: cross-desktop-linux-v2",
            "shared-key: cross-desktop-linux-v2",
            "key: cross-macos-arm64-v2",
            "shared-key: cross-macos-arm64-v2",
            "key: cross-windows-msvc-v2",
            "shared-key: cross-windows-msvc-v2",
            "key: wasm32-v3",
            "shared-key: wasm32-v3",
            "cargo-deny:",
            "EmbarkStudios/cargo-deny-action@v2",
            "- cargo-deny",
            "scripts/generate_sbom.py",
            "--verify",
            "-sbom.cdx.json",
            "*.cdx.json",
            "SHA256SUMS.txt",
        ):
            self.assertIn(needle, release_wf, f"missing `{needle}` in .github/workflows/release.yml")

        self.assertNotIn("key: wasm32-v2", release_wf)
        self.assertNotIn("shared-key: wasm32-v2", release_wf)
        self.assertNotRegex(release_wf, r"(?m)key: cross-macos-arm64\s*$")
        self.assertNotRegex(release_wf, r"(?m)key: cross-windows-msvc\s*$")

    def test_ci_and_dev_check_wire_check_book_verifier(self) -> None:
        ci_wf = (REPO_ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
        self.assertIn("scripts/check_book.py", ci_wf)

        dev_check = (REPO_ROOT / "scripts" / "dev-check.sh").read_text(encoding="utf-8")
        self.assertIn("scripts/check_book.py", dev_check)
        self.assertIn("mdbook build book", dev_check)
        syntax = subprocess.run(
            ["bash", "-n", str(REPO_ROOT / "scripts" / "dev-check.sh")],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(syntax.returncode, 0, syntax.stderr)


if __name__ == "__main__":
    unittest.main()
