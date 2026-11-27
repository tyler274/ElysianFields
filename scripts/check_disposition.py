#!/usr/bin/env python3
"""Validate the M5 Daybreak 43-item replacement disposition ledger and retirement gate.

Verifies:
1. Exact 1-to-1 bijection between all 43 M0 inventory items in
   `docs/reference/scope-inventory.json` and `docs/m5/disposition.json`
   (0 missing, 0 extras, exact classification and zone match).
2. Every `replace` item maps to >= 1 existing workspace target path,
   >= 1 valid requirement ID in `specs/ledger.toml`, and >= 1 existing evidence path.
3. Every requirement in `specs/ledger.toml` (33/33) has zero `unknown` coverage cells
   and is accounted for by at least one disposition entry.
4. Zero transitional reference product paths exist in the replacement workspace
   (`Cargo.toml` members or `crates/`), and `docs/m5/` stays free of clean-room wall leaks.
"""

from __future__ import annotations

import argparse
import collections
import json
import re
import sys
import tomllib
from pathlib import Path
from typing import Any

import check_wall

VALID_CLASSIFICATIONS = frozenset({
    "replace",
    "retain-as-tooling",
    "out-of-scope",
})

VALID_RETIREMENT_STATUSES = frozenset({
    "verified-ready-for-staged-cutover",
    "verified-with-hardware-signoff-gate",
    "retained-as-tooling",
    "out-of-scope",
})

REPLACE_STATUSES = frozenset({
    "verified-ready-for-staged-cutover",
    "verified-with-hardware-signoff-gate",
})

INV_ID_RE = re.compile(r"^INV-([0-9]{2})$")
LINEAR_ID_RE = re.compile(r"\bTYL-[0-9]+\b")


def _is_within_root(repo_root: Path, candidate: Path) -> bool:
    try:
        candidate.resolve().relative_to(repo_root.resolve())
        return True
    except ValueError:
        return False


def _validate_existing_paths(
    prefix: str,
    field: str,
    paths: Any,
    repo_root: Path,
    require_non_empty: bool,
) -> list[str]:
    errors: list[str] = []
    if not isinstance(paths, list) or not all(isinstance(p, str) for p in paths):
        return [f"{prefix}: '{field}' must be a list of relative path strings"]
    if require_non_empty and not paths:
        return [f"{prefix}: '{field}' must be non-empty"]
    if len(set(paths)) != len(paths):
        errors.append(f"{prefix}: '{field}' contains duplicate paths")
    for rel in paths:
        if not rel.strip() or Path(rel).is_absolute():
            errors.append(f"{prefix}: '{field}' entry {rel!r} must be a non-empty relative path")
            continue
        resolved = repo_root / rel
        if not _is_within_root(repo_root, resolved):
            errors.append(f"{prefix}: '{field}' path {rel!r} escapes repository root")
        elif not resolved.exists():
            errors.append(f"{prefix}: '{field}' path {rel!r} does not exist on disk")
    return errors


def _validate_m5_wall_cleanliness(repo_root: Path) -> list[str]:
    errors: list[str] = []
    m5_dir = repo_root / "docs" / "m5"
    if not m5_dir.is_dir():
        return ["Missing docs/m5 directory"]
    for path in sorted(m5_dir.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(repo_root).as_posix()
        text = path.read_text(encoding="utf-8")
        if LINEAR_ID_RE.search(text):
            errors.append(f"{rel}: contains forbidden Linear ticket ID")
        for scope in ("specs", "crates", "book"):
            for violation in check_wall.scan_file(repo_root, scope, path):
                errors.append(
                    f"{rel}:{violation.line}: wall violation [{violation.rule_id}] "
                    f"matched {violation.match!r}"
                )
    return errors


def _validate_zero_transitional_paths(
    repo_root: Path,
    inventory_members: list[dict[str, Any]],
) -> list[str]:
    errors: list[str] = []
    forbidden_rel_paths = {
        m["path"]
        for m in inventory_members
        if m.get("classification") == "replace" and isinstance(m.get("path"), str)
    }

    cargo_toml_path = repo_root / "Cargo.toml"
    if not cargo_toml_path.is_file():
        return ["Missing root Cargo.toml"]

    cargo_data = tomllib.loads(cargo_toml_path.read_text(encoding="utf-8"))
    workspace_members = cargo_data.get("workspace", {}).get("members", [])
    if not isinstance(workspace_members, list):
        errors.append("Cargo.toml: [workspace].members must be a list")
    else:
        for member in workspace_members:
            if member in forbidden_rel_paths:
                errors.append(
                    f"Cargo.toml: transitional reference product path {member!r} "
                    "is present in [workspace].members"
                )

    crates_dir = repo_root / "crates"
    if crates_dir.is_dir():
        for child in sorted(crates_dir.iterdir()):
            if not child.is_dir():
                continue
            rel = f"crates/{child.name}"
            if rel in forbidden_rel_paths:
                errors.append(
                    f"crates/: transitional reference product directory {rel!r} exists on disk"
                )

    return errors


def validate_disposition(repo_root: Path) -> tuple[dict[str, Any], list[str]]:
    errors: list[str] = []
    stats: dict[str, Any] = {}

    inventory_path = repo_root / "docs" / "reference" / "scope-inventory.json"
    disposition_path = repo_root / "docs" / "m5" / "disposition.json"
    ledger_path = repo_root / "specs" / "ledger.toml"

    for label, p in (
        ("scope-inventory.json", inventory_path),
        ("disposition.json", disposition_path),
        ("ledger.toml", ledger_path),
    ):
        if not p.is_file():
            return stats, [f"Missing required file: {p.relative_to(repo_root).as_posix()} ({label})"]

    try:
        inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return stats, [f"Invalid JSON in scope-inventory.json: {exc}"]

    try:
        disposition = json.loads(disposition_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return stats, [f"Invalid JSON in docs/m5/disposition.json: {exc}"]

    try:
        ledger = tomllib.loads(ledger_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        return stats, [f"Invalid TOML in specs/ledger.toml: {exc}"]

    if disposition.get("schema_version") != 1:
        errors.append(f"disposition.json: expected schema_version = 1, got {disposition.get('schema_version')!r}")

    revision = disposition.get("revision", "")
    if not isinstance(revision, str) or not revision.strip():
        errors.append("disposition.json: top-level revision must be a non-empty string")

    inv_members = inventory.get("workspace_members", [])
    disp_items = disposition.get("items", [])
    if not isinstance(inv_members, list) or not isinstance(disp_items, list):
        return stats, ["scope-inventory.json workspace_members and disposition.json items must be lists"]

    expected_by_key: dict[str, tuple[str, dict[str, Any]]] = {}
    for idx, member in enumerate(inv_members, start=1):
        pkg = member.get("package")
        expected_id = f"INV-{idx:02d}"
        expected_by_key[pkg] = (expected_id, member)

    ledger_reqs = ledger.get("requirements", [])
    ledger_req_ids: set[str] = set()
    unknown_cells: list[str] = []
    passed_cells = 0
    blocked_cells = 0
    for req in ledger_reqs:
        rid = req.get("id")
        if isinstance(rid, str):
            ledger_req_ids.add(rid)
        coverage = req.get("coverage", {})
        if isinstance(coverage, dict):
            for plat, entry in coverage.items():
                st = entry.get("state") if isinstance(entry, dict) else None
                if st == "unknown":
                    unknown_cells.append(f"{rid}[{plat}]")
                elif st == "passed":
                    passed_cells += 1
                elif st == "blocked":
                    blocked_cells += 1

    if unknown_cells:
        errors.append(
            f"specs/ledger.toml: {len(unknown_cells)} coverage cell(s) still 'unknown': "
            + ", ".join(unknown_cells[:8])
        )

    seen_ids: set[str] = set()
    seen_keys: set[str] = set()
    covered_req_ids: set[str] = set()
    class_counts: collections.Counter[str] = collections.Counter()
    status_counts: collections.Counter[str] = collections.Counter()

    for idx, item in enumerate(disp_items):
        if not isinstance(item, dict):
            errors.append(f"items[{idx}]: must be an object")
            continue

        item_id = item.get("id", "")
        key = item.get("inventory_key", "")
        prefix = f"items[{idx}] ({item_id or key})"

        if not isinstance(item_id, str) or not INV_ID_RE.match(item_id):
            errors.append(f"{prefix}: invalid id {item_id!r} (expected INV-01..INV-43)")
        elif item_id in seen_ids:
            errors.append(f"{prefix}: duplicate id {item_id!r}")
        else:
            seen_ids.add(item_id)

        if not isinstance(key, str) or not key.strip():
            errors.append(f"{prefix}: inventory_key must be a non-empty string")
            continue
        if key in seen_keys:
            errors.append(f"{prefix}: duplicate inventory_key {key!r}")
            continue
        seen_keys.add(key)

        if key not in expected_by_key:
            errors.append(f"{prefix}: extra inventory_key {key!r} not in scope-inventory.json")
            continue

        expected_id, inv_member = expected_by_key[key]
        if item_id != expected_id:
            errors.append(
                f"{prefix}: id mismatch for {key!r} (expected {expected_id!r}, got {item_id!r})"
            )

        classification = item.get("classification")
        expected_class = inv_member.get("classification")
        if classification not in VALID_CLASSIFICATIONS:
            errors.append(f"{prefix}: invalid classification {classification!r}")
        elif classification != expected_class:
            errors.append(
                f"{prefix}: classification mismatch for {key!r} "
                f"(expected {expected_class!r}, got {classification!r})"
            )
        else:
            class_counts[classification] += 1

        zone = item.get("zone")
        if zone != inv_member.get("zone"):
            errors.append(
                f"{prefix}: zone mismatch for {key!r} "
                f"(expected {inv_member.get('zone')!r}, got {zone!r})"
            )

        item_rev = item.get("revision", "")
        if not isinstance(item_rev, str) or not item_rev.strip():
            errors.append(f"{prefix}: revision must be a non-empty string")

        ret_status = item.get("retirement_status")
        if ret_status not in VALID_RETIREMENT_STATUSES:
            errors.append(f"{prefix}: invalid retirement_status {ret_status!r}")
        else:
            status_counts[ret_status] += 1

        req_ids = item.get("requirement_ids")
        if not isinstance(req_ids, list) or not all(isinstance(r, str) for r in req_ids):
            errors.append(f"{prefix}: requirement_ids must be a list of strings")
            req_ids = []
        else:
            for rid in req_ids:
                if rid not in ledger_req_ids:
                    errors.append(f"{prefix}: unknown requirement_id {rid!r} not in specs/ledger.toml")
                else:
                    covered_req_ids.add(rid)

        if classification == "replace":
            if ret_status not in REPLACE_STATUSES:
                errors.append(
                    f"{prefix}: 'replace' item must use retirement_status in "
                    f"{sorted(REPLACE_STATUSES)}, got {ret_status!r}"
                )
            if not req_ids:
                errors.append(f"{prefix}: 'replace' item must reference >= 1 requirement_id")
            errors.extend(
                _validate_existing_paths(
                    prefix, "replacement_targets", item.get("replacement_targets"), repo_root, True
                )
            )
            errors.extend(
                _validate_existing_paths(
                    prefix, "evidence_paths", item.get("evidence_paths"), repo_root, True
                )
            )
        elif classification == "retain-as-tooling":
            if ret_status != "retained-as-tooling":
                errors.append(
                    f"{prefix}: 'retain-as-tooling' item must have retirement_status "
                    f"'retained-as-tooling', got {ret_status!r}"
                )
            errors.extend(
                _validate_existing_paths(
                    prefix, "replacement_targets", item.get("replacement_targets"), repo_root, False
                )
            )
            errors.extend(
                _validate_existing_paths(
                    prefix, "evidence_paths", item.get("evidence_paths"), repo_root, True
                )
            )
        elif classification == "out-of-scope":
            if ret_status != "out-of-scope":
                errors.append(
                    f"{prefix}: 'out-of-scope' item must have retirement_status "
                    f"'out-of-scope', got {ret_status!r}"
                )
            targets = item.get("replacement_targets")
            if targets != []:
                errors.append(f"{prefix}: 'out-of-scope' item must have empty replacement_targets")
            errors.extend(
                _validate_existing_paths(
                    prefix, "evidence_paths", item.get("evidence_paths"), repo_root, True
                )
            )

    missing_keys = sorted(set(expected_by_key.keys()) - seen_keys)
    if missing_keys:
        errors.append(
            f"disposition.json: missing {len(missing_keys)} inventory item(s): "
            + ", ".join(missing_keys[:10])
        )

    uncovered_reqs = sorted(ledger_req_ids - covered_req_ids)
    if uncovered_reqs:
        errors.append(
            f"disposition.json: {len(uncovered_reqs)} ledger requirement(s) not mapped by any item: "
            + ", ".join(uncovered_reqs)
        )

    # Validate summary block matches computed counts.
    summary = disposition.get("summary")
    if not isinstance(summary, dict):
        errors.append("disposition.json: missing summary object")
    else:
        if summary.get("items_total") != len(inv_members):
            errors.append(
                f"disposition.json: summary.items_total={summary.get('items_total')} "
                f"!= {len(inv_members)}"
            )
        if summary.get("by_classification") != dict(class_counts):
            errors.append(
                f"disposition.json: summary.by_classification={summary.get('by_classification')} "
                f"!= {dict(class_counts)}"
            )
        if summary.get("by_retirement_status") != dict(status_counts):
            errors.append(
                f"disposition.json: summary.by_retirement_status="
                f"{summary.get('by_retirement_status')} != {dict(status_counts)}"
            )
        if summary.get("requirements_total") != len(ledger_req_ids):
            errors.append(
                f"disposition.json: summary.requirements_total="
                f"{summary.get('requirements_total')} != {len(ledger_req_ids)}"
            )
        if summary.get("requirements_covered") != len(covered_req_ids):
            errors.append(
                f"disposition.json: summary.requirements_covered="
                f"{summary.get('requirements_covered')} != {len(covered_req_ids)}"
            )

    # Validate approved_narrow_adapters exist on disk.
    adapters = disposition.get("approved_narrow_adapters")
    if not isinstance(adapters, list) or not adapters:
        errors.append("disposition.json: approved_narrow_adapters must be a non-empty list")
    else:
        for idx, adapter in enumerate(adapters):
            if not isinstance(adapter, dict):
                errors.append(f"approved_narrow_adapters[{idx}]: must be an object")
                continue
            errors.extend(
                _validate_existing_paths(
                    f"approved_narrow_adapters[{idx}]",
                    "paths",
                    adapter.get("paths"),
                    repo_root,
                    True,
                )
            )

    errors.extend(_validate_zero_transitional_paths(repo_root, inv_members))
    errors.extend(_validate_m5_wall_cleanliness(repo_root))

    stats = {
        "items_total": len(disp_items),
        "expected_items": len(inv_members),
        "by_classification": dict(class_counts),
        "by_retirement_status": dict(status_counts),
        "requirements_total": len(ledger_req_ids),
        "requirements_covered": len(covered_req_ids),
        "ledger_passed_cells": passed_cells,
        "ledger_blocked_cells": blocked_cells,
        "ledger_unknown_cells": len(unknown_cells),
        "error_count": len(errors),
    }
    return stats, errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Validate the M5 Daybreak 43-item replacement disposition ledger."
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="Repository root directory (default: parent of scripts/).",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit machine-readable JSON output.",
    )
    args = parser.parse_args(argv)

    stats, errors = validate_disposition(args.repo_root)

    if args.json:
        print(json.dumps({"stats": stats, "errors": errors}, indent=2))
        return 1 if errors else 0

    if errors:
        print(
            f"Disposition validation FAILED with {len(errors)} error(s):",
            file=sys.stderr,
        )
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    cls = stats["by_classification"]
    print(
        f"Disposition validation OK: {stats['items_total']}/{stats['expected_items']} inventory items "
        f"({cls.get('replace', 0)} replace, {cls.get('retain-as-tooling', 0)} retain-as-tooling, "
        f"{cls.get('out-of-scope', 0)} out-of-scope), "
        f"{stats['requirements_covered']}/{stats['requirements_total']} ledger requirements covered "
        f"({stats['ledger_passed_cells']} passed, {stats['ledger_blocked_cells']} blocked, "
        f"{stats['ledger_unknown_cells']} unknown)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
