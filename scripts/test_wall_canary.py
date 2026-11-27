#!/usr/bin/env python3
"""Unit and negative tests for scripts/wall_canary.py."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPTS_DIR.parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import wall_canary  # noqa: E402


class WallCanaryTests(unittest.TestCase):
    def test_scrubbed_env_strips_tokens_and_isolates_paths(self) -> None:
        with tempfile.TemporaryDirectory(prefix="canary-env-test-") as tmp:
            root = Path(tmp)
            realm = root / "realm"
            bin_dir = root / "bin"
            tc_bin = root / "tc" / "bin"
            realm.mkdir()
            bin_dir.mkdir()
            tc_bin.mkdir(parents=True)

            old_env = os.environ.copy()
            try:
                os.environ["GITHUB_TOKEN"] = "ghp_secret_should_not_leak"
                os.environ["GH_TOKEN"] = "gho_secret_should_not_leak"
                os.environ["LINEAR_API_KEY"] = "lin_api_secret_should_not_leak"
                scrubbed = wall_canary.build_scrubbed_env(root, realm, bin_dir, tc_bin)
            finally:
                os.environ.clear()
                os.environ.update(old_env)

            for var in wall_canary.FORBIDDEN_ENV_VARS:
                self.assertNotIn(var, scrubbed)
            self.assertEqual(scrubbed["HOME"], str(root / "home"))
            self.assertEqual(scrubbed["CARGO_TARGET_DIR"], str(root / "cargo-target"))
            self.assertEqual(scrubbed["PATH"], str(bin_dir))
            self.assertEqual(scrubbed["CARGO_NET_OFFLINE"], "true")

    def test_end_to_end_wall_canary_passes_all_four_probes(self) -> None:
        receipt = wall_canary.run_wall_canary(source_repo=REPO_ROOT)
        self.assertTrue(receipt["all_passed"], json.dumps(receipt, indent=2))
        self.assertEqual(len(receipt["probes"]), 4)
        by_name = {p["name"]: p for p in receipt["probes"]}
        self.assertTrue(by_name["reference_file_locate"]["passed"])
        self.assertTrue(by_name["reference_git_history"]["passed"])
        self.assertTrue(by_name["linear_github_connectors"]["passed"])
        self.assertTrue(by_name["trivial_crate_build_lint"]["passed"])

        transcript = wall_canary.render_markdown_transcript(receipt)
        self.assertIn("**PASS**", transcript)
        self.assertIn("reference_file_locate", transcript)
        self.assertIn("trivial_crate_build_lint", transcript)

    def test_probe1_fails_closed_when_reference_file_is_readable(self) -> None:
        with tempfile.TemporaryDirectory(prefix="canary-leak-p1-") as tmp:
            leak_file = Path(tmp) / "leaked_ref.rs"
            leak_file.write_text("pub fn leaked() {}\n", encoding="utf-8")

            res = wall_canary._probe_reference_file_locate(
                {
                    "reference_targets": [
                        {
                            "label": "injected-leak",
                            "path": str(leak_file),
                            "kind": "file",
                        }
                    ]
                }
            )
            self.assertFalse(res.passed)
            self.assertIn("WALL BREACH", res.summary)

    def test_probe2_fails_closed_when_reference_remote_or_alternates_present(self) -> None:
        def inject_reference_remote(realm_dir: Path) -> None:
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(realm_dir),
                    "remote",
                    "add",
                    "upstream-ref",
                    "https://github.com/example/dabney-moe.git",
                ],
                check=True,
                capture_output=True,
            )

        receipt = wall_canary.run_wall_canary(
            source_repo=REPO_ROOT,
            post_clone_hook=inject_reference_remote,
        )
        self.assertFalse(receipt["all_passed"])
        by_name = {p["name"]: p for p in receipt["probes"]}
        self.assertFalse(by_name["reference_git_history"]["passed"])
        self.assertIn("WALL BREACH", by_name["reference_git_history"]["summary"])

    def test_probe3_fails_closed_when_token_leaks_into_child_env(self) -> None:
        receipt = wall_canary.run_wall_canary(
            source_repo=REPO_ROOT,
            extra_child_env={"LINEAR_API_KEY": "lin_api_leaked_test_value"},
        )
        self.assertFalse(receipt["all_passed"])
        by_name = {p["name"]: p for p in receipt["probes"]}
        self.assertFalse(by_name["linear_github_connectors"]["passed"])
        self.assertIn("LINEAR_API_KEY", by_name["linear_github_connectors"]["details"]["leaked_env_vars"])

    def test_fallback_env_and_dac_sandbox_mode_also_passes(self) -> None:
        receipt = wall_canary.run_wall_canary(
            source_repo=REPO_ROOT,
            force_disable_mount_ns=True,
        )
        self.assertEqual(receipt["isolation_mode"], "env-and-dac-sandbox")
        self.assertTrue(receipt["all_passed"], json.dumps(receipt, indent=2))


if __name__ == "__main__":
    unittest.main()
