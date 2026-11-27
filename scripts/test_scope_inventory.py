#!/usr/bin/env python3
"""Validate M0 observer reference pin, scope inventory, and clean-room M1 specs."""

from __future__ import annotations

import json
import re
import tomllib
import unittest
from pathlib import Path

import check_wall

REPO_ROOT = Path(__file__).resolve().parent.parent
REF_DIR = REPO_ROOT / "docs" / "reference"
M1_DIR = REPO_ROOT / "docs" / "m1"
SPECS_DIR = REPO_ROOT / "specs"

VALID_CLASSES = frozenset({
    "replace",
    "retain-as-tooling",
    "reuse-as-approved-asset",
    "out-of-scope",
})

EXPECTED_DABNEY_MOE_SHA = "01b762838d639a51f82e9d7d15e788e02bac772f"
EXPECTED_WEBSITE_SHA = "194624690007f82c21be73ba5cde285938917582"
LINEAR_ID_RE = re.compile(r"\bTYL-[0-9]+\b")


class TestScopeInventoryAndPin(unittest.TestCase):
    def test_required_reference_and_m1_files_exist(self) -> None:
        required_files = [
            REF_DIR / "pin.toml",
            REF_DIR / "pin.md",
            REF_DIR / "scope-inventory.json",
            REF_DIR / "scope-inventory.md",
            REF_DIR / "allowed-inputs.md",
            REF_DIR / "platform-register.md",
            M1_DIR / "stutter-baseline.md",
            SPECS_DIR / "m1-feasibility-budgets.md",
            SPECS_DIR / "m1-platforms.md",
        ]
        for path in required_files:
            with self.subTest(path=path.relative_to(REPO_ROOT).as_posix()):
                self.assertTrue(path.is_file(), f"Missing required file: {path}")

    def test_pin_toml_parses_and_pins_both_deployed_mains(self) -> None:
        pin = tomllib.loads((REF_DIR / "pin.toml").read_text(encoding="utf-8"))
        self.assertEqual(pin["schema_version"], 1)
        self.assertEqual(pin["reference"]["git_sha"], EXPECTED_DABNEY_MOE_SHA)
        self.assertEqual(pin["reference_website"]["git_sha"], EXPECTED_WEBSITE_SHA)
        self.assertTrue(pin["fixtures"]["eval_excluded"])
        self.assertFalse(pin["reference"]["eval_dirs_present"])

        members = pin["workspace"]["members"]
        self.assertEqual(pin["workspace"]["member_count"], 43)
        self.assertEqual(len(members), 43)
        paths = [m["path"] for m in members]
        packages = [m["package"] for m in members]
        self.assertEqual(len(set(paths)), 43)
        self.assertEqual(len(set(packages)), 43)

    def test_scope_inventory_matches_pin_1_to_1_and_uses_valid_classes(self) -> None:
        pin = tomllib.loads((REF_DIR / "pin.toml").read_text(encoding="utf-8"))
        inv = json.loads((REF_DIR / "scope-inventory.json").read_text(encoding="utf-8"))

        self.assertEqual(inv["schema_version"], 1)
        self.assertEqual(inv["reference_sha"], EXPECTED_DABNEY_MOE_SHA)
        self.assertEqual(set(inv["allowed_classes"]), VALID_CLASSES)

        pin_pairs = {(m["path"], m["package"]) for m in pin["workspace"]["members"]}
        inv_members = inv["workspace_members"]
        self.assertEqual(len(inv_members), 43)
        inv_pairs = [(m["path"], m["package"]) for m in inv_members]
        self.assertEqual(len(set(inv_pairs)), 43, "Duplicate workspace member in scope-inventory.json")
        self.assertEqual(set(inv_pairs), pin_pairs, "Workspace members do not match pin.toml 1-to-1")

        # Validate classifications and non-empty justifications across all inventory sections.
        section_keys = (
            "workspace_members",
            "platform_bridges",
            "chrome_extension_components",
            "public_route_families",
            "background_services_and_workers",
            "infrastructure_and_dev_tooling",
        )
        for key in section_keys:
            items = inv.get(key)
            self.assertIsInstance(items, list, f"Section {key} must be a list")
            self.assertGreater(len(items), 0, f"Section {key} must be non-empty")
            for idx, item in enumerate(items):
                with self.subTest(section=key, idx=idx):
                    cls = item.get("classification")
                    self.assertIn(cls, VALID_CLASSES, f"Invalid class {cls!r} in {key}[{idx}]")
                    justification = item.get("justification", "")
                    self.assertIsInstance(justification, str)
                    self.assertTrue(
                        justification.strip(),
                        f"Empty justification in {key}[{idx}]",
                    )

        # Verify summary counts match actual workspace_members counts.
        summary = inv["summary"]
        self.assertEqual(summary["workspace_members_total"], 43)
        counts = {cls: 0 for cls in VALID_CLASSES}
        for member in inv_members:
            counts[member["classification"]] += 1
        self.assertEqual(summary["workspace_members_by_class"], counts)

    def test_no_linear_ticket_ids_in_reference_or_m1_docs(self) -> None:
        docs_to_check = sorted(REF_DIR.iterdir()) + [M1_DIR / "stutter-baseline.md"]
        for path in docs_to_check:
            if not path.is_file():
                continue
            text = path.read_text(encoding="utf-8")
            matches = LINEAR_ID_RE.findall(text)
            self.assertEqual(
                matches,
                [],
                f"Found Linear ticket IDs {matches} in {path.relative_to(REPO_ROOT)}",
            )

    def test_ancestry_subpath_reconciled_to_dux(self) -> None:
        for name in ("scope-inventory.json", "scope-inventory.md"):
            text = (REF_DIR / name).read_text(encoding="utf-8")
            self.assertNotIn(
                "dabney.moe/ancestry/",
                text,
                f"Stale dabney.moe/ancestry/ path in {name}; deployed subpath is dabney.moe/dux/",
            )

    def test_clean_room_m1_specs_pass_wall_lint(self) -> None:
        for name in ("m1-feasibility-budgets.md", "m1-platforms.md"):
            path = SPECS_DIR / name
            violations = check_wall.scan_file(REPO_ROOT, "specs", path)
            self.assertEqual(
                violations,
                [],
                f"Clean-room wall violations in specs/{name}: {violations}",
            )


if __name__ == "__main__":
    unittest.main()
