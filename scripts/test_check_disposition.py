#!/usr/bin/env python3
"""Unit tests for scripts/check_disposition.py (M5 Daybreak disposition gate)."""

from __future__ import annotations

import json
import shutil
import tempfile
import unittest
from pathlib import Path

import check_disposition
import check_wall

REPO_ROOT = Path(__file__).resolve().parent.parent


class TestCheckDisposition(unittest.TestCase):
    def _copy_minimal_repo(self, tmpdir: Path) -> Path:
        root = tmpdir / "repo"
        root.mkdir()
        (root / "docs" / "reference").mkdir(parents=True)
        (root / "docs" / "m5").mkdir(parents=True)
        (root / "specs").mkdir(parents=True)
        shutil.copy2(
            REPO_ROOT / "docs" / "reference" / "scope-inventory.json",
            root / "docs" / "reference" / "scope-inventory.json",
        )
        shutil.copy2(
            REPO_ROOT / "docs" / "m5" / "disposition.json",
            root / "docs" / "m5" / "disposition.json",
        )
        shutil.copy2(
            REPO_ROOT / "specs" / "ledger.toml",
            root / "specs" / "ledger.toml",
        )
        shutil.copy2(REPO_ROOT / "Cargo.toml", root / "Cargo.toml")

        # Create lightweight stub files/dirs for every referenced path in disposition.json.
        disp = json.loads((root / "docs" / "m5" / "disposition.json").read_text(encoding="utf-8"))
        paths_to_create: set[str] = set()
        for item in disp["items"]:
            paths_to_create.update(item["replacement_targets"])
            paths_to_create.update(item["evidence_paths"])
        for adapter in disp["approved_narrow_adapters"]:
            paths_to_create.update(adapter["paths"])
        for rel in paths_to_create:
            src = REPO_ROOT / rel
            dst = root / rel
            if src.is_dir():
                dst.mkdir(parents=True, exist_ok=True)
            else:
                dst.parent.mkdir(parents=True, exist_ok=True)
                if not dst.exists():
                    dst.write_text("stub\n", encoding="utf-8")
        return root

    def test_real_repository_disposition_passes(self) -> None:
        stats, errors = check_disposition.validate_disposition(REPO_ROOT)
        self.assertEqual(errors, [], f"Real repo disposition failed: {errors}")
        self.assertEqual(stats["items_total"], 43)
        self.assertEqual(stats["expected_items"], 43)
        self.assertEqual(
            stats["by_classification"],
            {"replace": 34, "retain-as-tooling": 6, "out-of-scope": 3},
        )
        self.assertEqual(stats["requirements_total"], 33)
        self.assertEqual(stats["requirements_covered"], 33)
        self.assertEqual(stats["ledger_unknown_cells"], 0)
        self.assertEqual(stats["ledger_passed_cells"], 64)
        self.assertEqual(stats["ledger_blocked_cells"], 20)

    def test_m5_docs_have_zero_wall_violations_across_all_scopes(self) -> None:
        m5_dir = REPO_ROOT / "docs" / "m5"
        self.assertTrue((m5_dir / "disposition.json").is_file())
        self.assertTrue((m5_dir / "disposition-and-retirement.md").is_file())
        for path in sorted(m5_dir.iterdir()):
            if not path.is_file():
                continue
            for scope in ("specs", "crates", "book"):
                violations = check_wall.scan_file(REPO_ROOT, scope, path)
                self.assertEqual(
                    violations,
                    [],
                    f"Wall violation in {path.name} under scope {scope}: {violations}",
                )

    def test_missing_inventory_item_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = self._copy_minimal_repo(Path(tmp))
            disp_path = root / "docs" / "m5" / "disposition.json"
            disp = json.loads(disp_path.read_text(encoding="utf-8"))
            disp["items"].pop()
            disp_path.write_text(json.dumps(disp, indent=2), encoding="utf-8")

            _, errors = check_disposition.validate_disposition(root)
            self.assertTrue(
                any("missing 1 inventory item" in e for e in errors),
                f"Expected missing item error, got: {errors}",
            )

    def test_classification_mismatch_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = self._copy_minimal_repo(Path(tmp))
            disp_path = root / "docs" / "m5" / "disposition.json"
            disp = json.loads(disp_path.read_text(encoding="utf-8"))
            disp["items"][0]["classification"] = "retain-as-tooling"
            disp_path.write_text(json.dumps(disp, indent=2), encoding="utf-8")

            _, errors = check_disposition.validate_disposition(root)
            self.assertTrue(
                any("classification mismatch" in e for e in errors),
                f"Expected classification mismatch error, got: {errors}",
            )

    def test_nonexistent_replacement_target_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = self._copy_minimal_repo(Path(tmp))
            disp_path = root / "docs" / "m5" / "disposition.json"
            disp = json.loads(disp_path.read_text(encoding="utf-8"))
            disp["items"][0]["replacement_targets"].append("crates/ef-does-not-exist")
            disp_path.write_text(json.dumps(disp, indent=2), encoding="utf-8")

            _, errors = check_disposition.validate_disposition(root)
            self.assertTrue(
                any("does not exist on disk" in e for e in errors),
                f"Expected missing target error, got: {errors}",
            )

    def test_unknown_ledger_cell_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = self._copy_minimal_repo(Path(tmp))
            ledger_path = root / "specs" / "ledger.toml"
            text = ledger_path.read_text(encoding="utf-8").replace(
                'state = "passed"', 'state = "unknown"', 1
            )
            ledger_path.write_text(text, encoding="utf-8")

            _, errors = check_disposition.validate_disposition(root)
            self.assertTrue(
                any("still 'unknown'" in e for e in errors),
                f"Expected unknown ledger cell error, got: {errors}",
            )

    def test_transitional_reference_crate_path_in_workspace_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = self._copy_minimal_repo(Path(tmp))
            (root / "crates" / "physician-sample").mkdir(parents=True, exist_ok=True)

            _, errors = check_disposition.validate_disposition(root)
            self.assertTrue(
                any("transitional reference product directory" in e for e in errors),
                f"Expected transitional path error, got: {errors}",
            )


if __name__ == "__main__":
    unittest.main()
