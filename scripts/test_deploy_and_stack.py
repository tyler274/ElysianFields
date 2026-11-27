"""Unit tests validating deploy/ containers, compose stack, and run_local_stack.sh."""

from __future__ import annotations

import subprocess
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEPLOY_DIR = REPO_ROOT / "deploy"
LAUNCHER = REPO_ROOT / "scripts" / "run_local_stack.sh"


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
            "deploy/Containerfile.public",
            'container_uid" != "10001"',
            "scripts/qa_multilingual_gui.py",
            "web/elysianfields/**",
            "fonts-noto-core",
            "fonts-noto-cjk",
            "fonts-noto-extra",
            "fc-cache -f",
            "multilingual-gui-qa-report.json",
            "multilingual-screenshots/",
        ):
            self.assertIn(needle, wf)


if __name__ == "__main__":
    unittest.main()
