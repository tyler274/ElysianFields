#!/usr/bin/env python3
"""Unit tests for scripts/ci_scope.py."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

import ci_scope


class ClassifyPathsTests(unittest.TestCase):
    def test_empty_paths_is_docs_only(self) -> None:
        scope = ci_scope.classify_paths([])
        self.assertTrue(scope["docs_only"])
        for key in ("native", "wasm", "desktop", "mobile", "server", "client_only", "full"):
            self.assertFalse(scope[key], f"expected {key}=False")

    def test_docs_only_paths(self) -> None:
        scope = ci_scope.classify_paths(
            ["README.md", "AGENTS.md", "docs/program-plan.md", "docs/ci/budgets.md"]
        )
        self.assertTrue(scope["docs_only"])
        self.assertFalse(scope["full"])
        self.assertFalse(scope["native"])
        self.assertFalse(scope["wasm"])

    def test_budgets_json_triggers_full(self) -> None:
        scope = ci_scope.classify_paths(["docs/ci/budgets.json"])
        self.assertFalse(scope["docs_only"])
        self.assertTrue(scope["full"])
        self.assertTrue(scope["native"])
        self.assertTrue(scope["wasm"])

    def test_client_web_only_sets_client_only_and_wasm(self) -> None:
        scope = ci_scope.classify_paths(["web/index.html", "crates/elysian-web/src/lib.rs"])
        self.assertFalse(scope["docs_only"])
        self.assertTrue(scope["wasm"])
        self.assertTrue(scope["client_only"])
        self.assertFalse(scope["native"])
        self.assertFalse(scope["server"])
        self.assertFalse(scope["full"])

    def test_server_only_skips_wasm_and_client_only(self) -> None:
        scope = ci_scope.classify_paths(["crates/elysian-server/src/main.rs"])
        self.assertTrue(scope["server"])
        self.assertTrue(scope["native"])
        self.assertFalse(scope["wasm"])
        self.assertFalse(scope["client_only"])
        self.assertFalse(scope["full"])

    def test_desktop_and_mobile_targets(self) -> None:
        desktop_scope = ci_scope.classify_paths(["crates/client-desktop/src/main.rs"])
        self.assertTrue(desktop_scope["desktop"])
        self.assertTrue(desktop_scope["native"])
        self.assertFalse(desktop_scope["mobile"])
        self.assertFalse(desktop_scope["wasm"])

        mobile_scope = ci_scope.classify_paths(["platform/ios/App.swift"])
        self.assertTrue(mobile_scope["mobile"])
        self.assertTrue(mobile_scope["native"])
        self.assertFalse(mobile_scope["desktop"])
        self.assertFalse(mobile_scope["client_only"])

    def test_shared_crate_runs_native_and_wasm(self) -> None:
        scope = ci_scope.classify_paths(["crates/elysian-smoke/src/lib.rs"])
        self.assertFalse(scope["docs_only"])
        self.assertTrue(scope["native"])
        self.assertTrue(scope["wasm"])
        self.assertFalse(scope["client_only"])
        self.assertFalse(scope["full"])

    def test_root_manifest_triggers_full(self) -> None:
        scope = ci_scope.classify_paths(["Cargo.toml"])
        self.assertTrue(scope["full"])
        self.assertTrue(scope["native"])
        self.assertTrue(scope["wasm"])
        self.assertTrue(scope["desktop"])
        self.assertTrue(scope["mobile"])
        self.assertTrue(scope["server"])
        self.assertFalse(scope["client_only"])

    def test_parse_name_status_z_handles_renames(self) -> None:
        raw = b"R100\0docs/old.md\0crates/elysian-web/src/new.rs\0M\0README.md\0"
        parsed = ci_scope.parse_name_status_z(raw)
        self.assertEqual(
            parsed,
            ["docs/old.md", "crates/elysian-web/src/new.rs", "README.md"],
        )


class SelectScopeGitTests(unittest.TestCase):
    def _run_git(self, cwd: Path, *args: str) -> str:
        proc = subprocess.run(
            ["git", *args],
            cwd=cwd,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        return proc.stdout.strip()

    def test_initial_push_zero_sha_fails_closed(self) -> None:
        scope, reason = ci_scope.select_scope(
            "push",
            {"before": ci_scope.ZERO_SHA, "after": "abc123"},
        )
        self.assertTrue(scope["full"])
        self.assertIn("initial/empty base", reason)

    def test_pr_diff_and_main_push_diff(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            self._run_git(repo, "init", "-b", "main")
            self._run_git(repo, "config", "user.email", "ci@example.test")
            self._run_git(repo, "config", "user.name", "CI Test")
            (repo / "README.md").write_text("# init\n", encoding="utf-8")
            self._run_git(repo, "add", "README.md")
            self._run_git(repo, "commit", "-m", "initial")
            base_sha = self._run_git(repo, "rev-parse", "HEAD")

            (repo / "web").mkdir()
            (repo / "web" / "index.html").write_text("<h1>hi</h1>\n", encoding="utf-8")
            self._run_git(repo, "add", "web/index.html")
            self._run_git(repo, "commit", "-m", "web change")
            head_sha = self._run_git(repo, "rev-parse", "HEAD")

            pr_scope, _ = ci_scope.select_scope(
                "pull_request",
                {
                    "pull_request": {
                        "base": {"sha": base_sha},
                        "head": {"sha": head_sha},
                    }
                },
                cwd=repo,
            )
            self.assertTrue(pr_scope["client_only"])
            self.assertTrue(pr_scope["wasm"])
            self.assertFalse(pr_scope["native"])

            push_scope, _ = ci_scope.select_scope(
                "push",
                {"before": base_sha, "after": head_sha},
                cwd=repo,
            )
            self.assertTrue(push_scope["client_only"])
            self.assertTrue(push_scope["wasm"])
            self.assertFalse(push_scope["full"])


class VerifyResultTests(unittest.TestCase):
    def test_docs_only_requires_scope_and_lightweight_only(self) -> None:
        needs = {
            "scope": {
                "result": "success",
                "outputs": {
                    "docs_only": "true",
                    "native": "false",
                    "wasm": "false",
                    "client_only": "false",
                    "full": "false",
                },
            },
            "lightweight": {"result": "success"},
            "rust-native": {"result": "skipped"},
            "rust-wasm": {"result": "skipped"},
            "cargo-deny": {"result": "skipped"},
            "deploy-client": {"result": "skipped"},
        }
        self.assertEqual(
            ci_scope.verify_result(needs, event_name="pull_request"),
            [],
        )

    def test_main_client_only_requires_wasm_deny_and_deploy_client(self) -> None:
        needs = {
            "scope": {
                "result": "success",
                "outputs": {
                    "docs_only": "false",
                    "native": "false",
                    "wasm": "true",
                    "client_only": "true",
                    "full": "false",
                },
            },
            "lightweight": {"result": "success"},
            "rust-native": {"result": "skipped"},
            "rust-wasm": {"result": "success"},
            "cargo-deny": {"result": "success"},
            "deploy-client": {"result": "failure"},
        }
        failures = ci_scope.verify_result(
            needs,
            event_name="push",
            ref_name="refs/heads/main",
        )
        self.assertEqual(len(failures), 1)
        self.assertIn("deploy-client", failures[0])

    def test_pr_full_requires_native_wasm_deny_not_deploy_client(self) -> None:
        needs = {
            "scope": {
                "result": "success",
                "outputs": {
                    "docs_only": "false",
                    "native": "true",
                    "wasm": "true",
                    "client_only": "false",
                    "full": "true",
                },
            },
            "lightweight": {"result": "success"},
            "rust-native": {"result": "success"},
            "rust-wasm": {"result": "success"},
            "cargo-deny": {"result": "success"},
            "deploy-client": {"result": "skipped"},
        }
        self.assertEqual(
            ci_scope.verify_result(needs, event_name="pull_request"),
            [],
        )


if __name__ == "__main__":
    unittest.main()
