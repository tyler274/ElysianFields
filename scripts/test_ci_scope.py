#!/usr/bin/env python3
"""Unit tests for scripts/ci_scope.py."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

import ci_scope

REPO_ROOT = Path(__file__).resolve().parent.parent


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

    def test_client_web_and_ui_crates_set_client_only_and_wasm(self) -> None:
        for paths in (
            ["web/elysianfields/index.html"],
            ["crates/ef-ui-catalog/src/lib.rs"],
            ["crates/ef-clinic-client/src/lib.rs"],
            ["crates/ef-mobile-client/src/lib.rs"],
            ["web/index.html", "crates/elysian-web/src/lib.rs"],
            ["crates/m1-web-delivery/src/lib.rs"],
        ):
            scope = ci_scope.classify_paths(paths)
            self.assertFalse(scope["docs_only"], f"paths={paths}")
            self.assertTrue(scope["wasm"], f"paths={paths}")
            self.assertTrue(scope["client_only"], f"paths={paths}")
            self.assertFalse(scope["native"], f"paths={paths}")
            self.assertFalse(scope["server"], f"paths={paths}")
            self.assertFalse(scope["full"], f"paths={paths}")

    def test_headless_server_harness_boundaries_skip_wasm_and_client_only(self) -> None:
        for paths in (
            ["crates/ef-clinic-server/src/main.rs"],
            ["crates/ef-harness/src/lib.rs"],
            ["crates/ef-boundaries/src/lib.rs"],
            ["crates/ef-clinic-workflows/src/matching.rs"],
            ["crates/ef-mobile-domain/src/lib.rs"],
            ["crates/ef-mobile-server/src/main.rs"],
            ["crates/ef-connected-care/src/lib.rs"],
            ["crates/ef-clinical-lib/src/lib.rs"],
            ["crates/ef-encoder/src/lib.rs"],
            ["crates/ef-patient-bank/src/lib.rs"],
            ["crates/ef-extension/src/lib.rs"],
            ["crates/ef-public-site/src/lib.rs"],
            ["crates/ef-migration-rehearsal/src/lib.rs"],
            ["crates/elysian-server/src/main.rs"],
            ["infra/gcp/main.tf"],
        ):
            scope = ci_scope.classify_paths(paths)
            self.assertTrue(scope["server"], f"paths={paths}")
            self.assertTrue(scope["native"], f"paths={paths}")
            self.assertFalse(scope["wasm"], f"paths={paths}")
            self.assertFalse(scope["client_only"], f"paths={paths}")
            self.assertFalse(scope["full"], f"paths={paths}")

    def test_mixed_server_and_client_disables_client_only(self) -> None:
        scope = ci_scope.classify_paths(
            ["crates/ef-clinic-server/src/main.rs", "crates/ef-clinic-client/src/lib.rs"]
        )
        self.assertTrue(scope["server"])
        self.assertTrue(scope["native"])
        self.assertTrue(scope["wasm"])
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

        lifecycle_scope = ci_scope.classify_paths(["crates/m1-mobile-lifecycle/src/lib.rs"])
        self.assertTrue(lifecycle_scope["mobile"])
        self.assertTrue(lifecycle_scope["native"])
        self.assertFalse(lifecycle_scope["wasm"])

    def test_shared_domain_crates_run_native_and_wasm(self) -> None:
        for path in (
            "crates/elysian-smoke/src/lib.rs",
            "crates/ef-core/src/lib.rs",
            "crates/ef-clinic-domain/src/lib.rs",
        ):
            scope = ci_scope.classify_paths([path])
            self.assertFalse(scope["docs_only"], f"path={path}")
            self.assertTrue(scope["native"], f"path={path}")
            self.assertTrue(scope["wasm"], f"path={path}")
            self.assertFalse(scope["client_only"], f"path={path}")
            self.assertFalse(scope["full"], f"path={path}")

    def test_root_manifest_and_workflow_trigger_full(self) -> None:
        for path in ("Cargo.toml", "Cargo.lock", ".github/workflows/ci.yml", "scripts/ci_scope.py"):
            scope = ci_scope.classify_paths([path])
            self.assertTrue(scope["full"], f"path={path}")
            self.assertTrue(scope["native"], f"path={path}")
            self.assertTrue(scope["wasm"], f"path={path}")
            self.assertTrue(scope["desktop"], f"path={path}")
            self.assertTrue(scope["mobile"], f"path={path}")
            self.assertTrue(scope["server"], f"path={path}")
            self.assertFalse(scope["client_only"], f"path={path}")

    def test_parse_name_status_z_handles_renames(self) -> None:
        raw = b"R100\0docs/old.md\0crates/elysian-web/src/new.rs\0M\0README.md\0"
        parsed = ci_scope.parse_name_status_z(raw)
        self.assertEqual(
            parsed,
            ["docs/old.md", "crates/elysian-web/src/new.rs", "README.md"],
        )

    def test_wasm_build_crates_excludes_headless_server_and_mobile_only(self) -> None:
        crates = ci_scope.wasm_build_crates(REPO_ROOT)
        self.assertIn("elysian-smoke", crates)
        self.assertIn("elysian-bevy-probe", crates)
        self.assertIn("m1-web-delivery", crates)
        self.assertIn("ef-ui-catalog", crates)
        self.assertIn("ef-clinic-client", crates)
        self.assertNotIn("ef-clinic-server", crates)
        self.assertNotIn("ef-harness", crates)
        self.assertNotIn("ef-boundaries", crates)
        self.assertNotIn("m1-mobile-lifecycle", crates)


class TouchedCratesTests(unittest.TestCase):
    def test_pr_leaf_crate_edit_targets_crate_plus_boundaries(self) -> None:
        flags = ci_scope.compute_touched_crates(
            ["crates/ef-clinic-server/src/http.rs", "docs/program-plan.md"],
            event_name="pull_request",
            workspace_root=REPO_ROOT,
        )
        self.assertEqual(flags, "-p ef-boundaries -p ef-clinic-server")

    def test_pr_multiple_leaf_crates_are_sorted_and_include_boundaries(self) -> None:
        flags = ci_scope.compute_touched_crates(
            ["crates/ef-clinic-client/src/lib.rs", "crates/ef-harness/src/lib.rs"],
            event_name="pull_request",
            workspace_root=REPO_ROOT,
        )
        self.assertEqual(flags, "-p ef-boundaries -p ef-clinic-client -p ef-harness")

    def test_pr_shared_fanout_crate_runs_full_workspace(self) -> None:
        for shared in (
            "crates/ef-core/src/lib.rs",
            "crates/ef-ui-catalog/src/lib.rs",
            "crates/ef-clinic-domain/src/auth.rs",
            "crates/ef-mobile-domain/src/lib.rs",
            "crates/elysian-smoke/src/lib.rs",
        ):
            flags = ci_scope.compute_touched_crates(
                [shared],
                event_name="pull_request",
                workspace_root=REPO_ROOT,
            )
            self.assertEqual(flags, "", f"expected full workspace for {shared}")

    def test_pr_root_manifest_or_main_push_runs_full_workspace(self) -> None:
        self.assertEqual(
            ci_scope.compute_touched_crates(
                ["Cargo.toml", "crates/ef-clinic-server/src/http.rs"],
                event_name="pull_request",
                workspace_root=REPO_ROOT,
            ),
            "",
        )
        self.assertEqual(
            ci_scope.compute_touched_crates(
                ["crates/ef-clinic-server/src/http.rs"],
                event_name="push",
                workspace_root=REPO_ROOT,
            ),
            "",
        )
        self.assertEqual(
            ci_scope.compute_touched_crates(
                ["crates/ef-clinic-workflows/src/lib.rs"],
                event_name="pull_request",
                workspace_root=None,
            ),
            "-p ef-boundaries -p ef-clinic-workflows",
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

    def test_main_server_only_skips_wasm_and_deploy_client(self) -> None:
        needs = {
            "scope": {
                "result": "success",
                "outputs": {
                    "docs_only": "false",
                    "native": "true",
                    "wasm": "false",
                    "server": "true",
                    "client_only": "false",
                    "full": "false",
                },
            },
            "lightweight": {"result": "success"},
            "rust-native": {"result": "success"},
            "rust-wasm": {"result": "skipped"},
            "cargo-deny": {"result": "success"},
            "deploy-client": {"result": "skipped"},
        }
        self.assertEqual(
            ci_scope.verify_result(
                needs,
                event_name="push",
                ref_name="refs/heads/main",
            ),
            [],
        )

    def test_main_client_only_skips_native_and_requires_deploy_client(self) -> None:
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

    def test_pr_client_only_requires_native_tests_and_wasm(self) -> None:
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
            "deploy-client": {"result": "skipped"},
        }
        failures = ci_scope.verify_result(needs, event_name="pull_request")
        self.assertEqual(len(failures), 1)
        self.assertIn("rust-native", failures[0])

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


    def test_native_release_crates_selects_headless_server_binaries_only(self) -> None:
        crates = ci_scope.native_release_crates()
        self.assertEqual(crates, ["ef-clinic-server", "ef-mobile-server", "ef-public-site"])


if __name__ == "__main__":
    unittest.main()
