#!/usr/bin/env python3
"""Unit tests for scripts/check_ledger.py."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_ledger

REPO_ROOT = Path(__file__).resolve().parent.parent


def _write_valid_fixture(root: Path, coverage_override: str | None = None) -> None:
    pkg = root / "specs" / "packages" / "sample-pkg"
    pkg.mkdir(parents=True, exist_ok=True)
    (pkg / "spec.md").write_text(
        "# Sample Spec\n\n## Requirement `EF-CLINIC-0001`\n\nBehavior.\n",
        encoding="utf-8",
    )
    (pkg / "scenarios.toml").write_text(
        """schema_version = 1
package = "specs/packages/sample-pkg"

[[scenarios]]
id = "SCN-CLINIC-0001"
requirement_id = "EF-CLINIC-0001"
title = "Sample scenario"
platforms = ["browser-wasm-desktop", "desktop-linux"]

[[scenarios.steps]]
step = 1
action = "Open application"
expected_visible = "Boundary renders"
expected_effects = "None"
""",
        encoding="utf-8",
    )

    coverage_block = coverage_override or """[requirements.coverage.browser-wasm-desktop]
state = "unknown"
revision = ""
evidence = ""

[requirements.coverage.desktop-linux]
state = "unknown"
revision = ""
evidence = ""
"""
    (root / "specs" / "ledger.toml").write_text(
        f"""schema_version = 1

[[requirements]]
id = "EF-CLINIC-0001"
title = "Sample requirement"
domain = "CLINIC"
product = "curie-clinic"
platforms = ["browser-wasm-desktop", "desktop-linux"]
status = "observed-and-required"
package = "specs/packages/sample-pkg"
scenario_id = "SCN-CLINIC-0001"

{coverage_block}
""",
        encoding="utf-8",
    )


class TestCheckLedger(unittest.TestCase):
    def test_real_repository_ledger_passes(self) -> None:
        errors = check_ledger.validate_ledger(REPO_ROOT)
        self.assertEqual(errors, [])

    def test_valid_fixture_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_valid_fixture(root)
            self.assertEqual(check_ledger.validate_ledger(root), [])

    def test_duplicate_id_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_valid_fixture(root)
            ledger = root / "specs" / "ledger.toml"
            original = ledger.read_text(encoding="utf-8")
            dup_entry = original.split("[[requirements]]", 1)[1]
            ledger.write_text(original + "\n[[requirements]]" + dup_entry, encoding="utf-8")
            errors = check_ledger.validate_ledger(root)
            self.assertTrue(any("duplicate requirement ID" in e for e in errors), errors)

    def test_domain_mismatch_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_valid_fixture(root)
            ledger = root / "specs" / "ledger.toml"
            ledger.write_text(
                ledger.read_text(encoding="utf-8").replace('domain = "CLINIC"', 'domain = "MOBILE"'),
                encoding="utf-8",
            )
            errors = check_ledger.validate_ledger(root)
            self.assertTrue(any("does not match ID domain" in e for e in errors), errors)

    def test_invalid_status_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_valid_fixture(root)
            ledger = root / "specs" / "ledger.toml"
            ledger.write_text(
                ledger.read_text(encoding="utf-8").replace(
                    'status = "observed-and-required"',
                    'status = "passed"',
                ),
                encoding="utf-8",
            )
            errors = check_ledger.validate_ledger(root)
            self.assertTrue(any("invalid status" in e for e in errors), errors)

    def test_invalid_coverage_state_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_valid_fixture(
                root,
                coverage_override="""[requirements.coverage.browser-wasm-desktop]
state = "observed-and-required"
revision = ""
evidence = ""

[requirements.coverage.desktop-linux]
state = "unknown"
revision = ""
evidence = ""
""",
            )
            errors = check_ledger.validate_ledger(root)
            self.assertTrue(any("invalid state" in e for e in errors), errors)

    def test_missing_platform_coverage_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_valid_fixture(
                root,
                coverage_override="""[requirements.coverage.browser-wasm-desktop]
state = "unknown"
revision = ""
evidence = ""
""",
            )
            errors = check_ledger.validate_ledger(root)
            self.assertTrue(any("missing coverage entries" in e for e in errors), errors)

    def test_passed_requires_revision_and_existing_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_valid_fixture(
                root,
                coverage_override="""[requirements.coverage.browser-wasm-desktop]
state = "passed"
revision = "abc1234"
evidence = "evidence/missing.log"

[requirements.coverage.desktop-linux]
state = "blocked"
revision = ""
evidence = ""
reason = "No Linux GUI runner provisioned yet"
""",
            )
            errors = check_ledger.validate_ledger(root)
            self.assertTrue(any("does not exist on disk" in e for e in errors), errors)

            ev = root / "evidence" / "missing.log"
            ev.parent.mkdir(parents=True, exist_ok=True)
            ev.write_text("ok\n", encoding="utf-8")
            self.assertEqual(check_ledger.validate_ledger(root), [])

    def test_blocked_without_reason_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_valid_fixture(
                root,
                coverage_override="""[requirements.coverage.browser-wasm-desktop]
state = "blocked"
revision = ""
evidence = ""

[requirements.coverage.desktop-linux]
state = "unknown"
revision = ""
evidence = ""
""",
            )
            errors = check_ledger.validate_ledger(root)
            self.assertTrue(any("requires a non-empty reason" in e for e in errors), errors)

    def test_missing_scenario_id_on_disk_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_valid_fixture(root)
            scn_file = root / "specs" / "packages" / "sample-pkg" / "scenarios.toml"
            scn_file.write_text(
                scn_file.read_text(encoding="utf-8").replace("SCN-CLINIC-0001", "SCN-CLINIC-9999"),
                encoding="utf-8",
            )
            errors = check_ledger.validate_ledger(root)
            self.assertTrue(any("not found in" in e for e in errors), errors)

    def test_missing_package_directory_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_valid_fixture(root)
            ledger = root / "specs" / "ledger.toml"
            ledger.write_text(
                ledger.read_text(encoding="utf-8").replace(
                    "specs/packages/sample-pkg",
                    "specs/packages/does-not-exist",
                ),
                encoding="utf-8",
            )
            errors = check_ledger.validate_ledger(root)
            self.assertTrue(any("does not exist" in e for e in errors), errors)

    def test_milestone_scenario_id_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_valid_fixture(root)
            for rel in (
                "specs/ledger.toml",
                "specs/packages/sample-pkg/scenarios.toml",
            ):
                p = root / rel
                p.write_text(
                    p.read_text(encoding="utf-8").replace("SCN-CLINIC-0001", "SCN-M2-0001"),
                    encoding="utf-8",
                )
            self.assertEqual(check_ledger.validate_ledger(root), [])

    def test_spec_missing_requirement_reference_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_valid_fixture(root)
            spec = root / "specs" / "packages" / "sample-pkg" / "spec.md"
            spec.write_text("# Spec without ID\n", encoding="utf-8")
            errors = check_ledger.validate_ledger(root)
            self.assertTrue(
                any("does not reference requirement ID" in e for e in errors), errors
            )


if __name__ == "__main__":
    unittest.main()
