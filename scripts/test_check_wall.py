#!/usr/bin/env python3
"""Unit tests for scripts/check_wall.py."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_wall

REPO_ROOT = Path(__file__).resolve().parent.parent


class TestCheckWall(unittest.TestCase):
    def test_real_repository_passes_wall_lint(self) -> None:
        scanned, violations = check_wall.scan_repo(REPO_ROOT)
        self.assertGreater(scanned, 0)
        self.assertEqual(violations, [])

    def test_clean_specs_crates_and_book_pass(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "specs" / "packages" / "demo").mkdir(parents=True)
            (root / "specs" / "packages" / "demo" / "spec.md").write_text(
                "# Clean Spec\n\nRequirement EF-CLINIC-0001 verifies sign-in behavior.\n"
                "See [protocol](../../book/src/clean-room-protocol.md#authority-order).\n",
                encoding="utf-8",
            )
            (root / "crates" / "demo-crate" / "src").mkdir(parents=True)
            (root / "crates" / "demo-crate" / "src" / "lib.rs").write_text(
                "pub(crate) fn internal_helper() -> bool { true }\n",
                encoding="utf-8",
            )
            (root / "book" / "src").mkdir(parents=True)
            (root / "book" / "src" / "protocol.md").write_text(
                "# Clean-Room Protocol\n\nObserver (Su) and Gatekeeper (Aponia).\n",
                encoding="utf-8",
            )

            scanned, violations = check_wall.scan_repo(root)
            self.assertEqual(scanned, 3)
            self.assertEqual(violations, [])

    def test_observer_docs_reference_is_excluded(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "docs" / "reference").mkdir(parents=True)
            (root / "docs" / "reference" / "pin.md").write_text(
                "Observed /home/luluco/code/dabney-moe crates/physician-app dioxus TYL-27\n",
                encoding="utf-8",
            )
            scanned, violations = check_wall.scan_repo(root)
            self.assertEqual(scanned, 0)
            self.assertEqual(violations, [])

    def test_book_build_output_is_excluded(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "book" / "book").mkdir(parents=True)
            (root / "book" / "book" / "index.html").write_text(
                '<div class="content" data-testid="generated">TYL-99</div>\n',
                encoding="utf-8",
            )
            scanned, violations = check_wall.scan_repo(root)
            self.assertEqual(scanned, 0)
            self.assertEqual(violations, [])

    def test_reference_host_path_leak_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "specs").mkdir(parents=True)
            (root / "specs" / "bad.md").write_text(
                "Copied from /home/luluco/code/dabney-moe/README.md\n",
                encoding="utf-8",
            )
            _, violations = check_wall.scan_repo(root)
            self.assertEqual([v.rule_id for v in violations], ["WALL-001"])

    def test_reference_crate_path_leak_fails_in_specs_and_crates(self) -> None:
        for rel_path in (
            "specs/bad.md",
            "crates/new-ui/src/lib.rs",
            "book/src/bad.md",
        ):
            with self.subTest(rel_path=rel_path):
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    target = root / rel_path
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text(
                        "See crates/physician-app and crates/mobile-app and crates/physician-sample\n",
                        encoding="utf-8",
                    )
                    _, violations = check_wall.scan_repo(root)
                    self.assertEqual(len(violations), 3)
                    self.assertTrue(all(v.rule_id == "WALL-002" for v in violations))

    def test_dioxus_leak_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "crates" / "ui" / "src").mkdir(parents=True)
            (root / "crates" / "ui" / "src" / "lib.rs").write_text(
                "// Ported from Dioxus component tree\n",
                encoding="utf-8",
            )
            _, violations = check_wall.scan_repo(root)
            self.assertEqual([v.rule_id for v in violations], ["WALL-003"])

    def test_linear_ticket_id_fails_in_book_and_specs(self) -> None:
        for rel_path in ("book/src/chapter.md", "specs/packages/p/spec.md"):
            with self.subTest(rel_path=rel_path):
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    target = root / rel_path
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text("Tracked in TYL-32.\n", encoding="utf-8")
                    _, violations = check_wall.scan_repo(root)
                    self.assertEqual([v.rule_id for v in violations], ["WALL-006"])

    def test_pub_crate_forbidden_in_specs_but_allowed_in_crates(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "specs").mkdir(parents=True)
            (root / "specs" / "leaky.md").write_text(
                "Function signature: `pub(crate) fn load_session()`\n",
                encoding="utf-8",
            )
            (root / "crates" / "app" / "src").mkdir(parents=True)
            (root / "crates" / "app" / "src" / "lib.rs").write_text(
                "pub(crate) fn load_session() {}\n",
                encoding="utf-8",
            )
            _, violations = check_wall.scan_repo(root)
            self.assertEqual(len(violations), 1)
            self.assertEqual(violations[0].file, "specs/leaky.md")
            self.assertEqual(violations[0].rule_id, "WALL-007")

    def test_dom_and_css_selectors_fail_in_specs(self) -> None:
        cases = [
            ('Click element with `data-testid="submit-login"`.\n', "WALL-008"),
            ("Locate via querySelector('#chat-input').\n", "WALL-008"),
            ('Render `<button class="btn-primary">`.\n', "WALL-008"),
            ("Focus moves to `#chat-input` on open.\n", "WALL-009"),
            ("Style matches `.chat-bubble` state.\n", "WALL-009"),
        ]
        for text, expected_rule in cases:
            with self.subTest(text=text):
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    (root / "specs").mkdir(parents=True)
                    (root / "specs" / "selectors.md").write_text(text, encoding="utf-8")
                    _, violations = check_wall.scan_repo(root)
                    rule_ids = [v.rule_id for v in violations]
                    self.assertIn(expected_rule, rule_ids, violations)

    def test_pinned_reference_sha_leak_fails_in_specs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "specs").mkdir(parents=True)
            (root / "specs" / "sha.md").write_text(
                "Observed at 01b762838d639a51f82e9d7d15e788e02bac772f.\n",
                encoding="utf-8",
            )
            _, violations = check_wall.scan_repo(root)
            self.assertEqual([v.rule_id for v in violations], ["WALL-005"])


if __name__ == "__main__":
    unittest.main()
