#!/usr/bin/env python3
"""Validate the Elysian Fields requirement ledger and its on-disk package references."""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path
from typing import Any

VALID_DOMAINS = frozenset({
    "CLINIC",
    "MOBILE",
    "CONN",
    "HARNESS",
    "PLATFORM",
    "EXT",
    "PUBLIC",
})

VALID_PRODUCTS = frozenset({
    "curie-clinic",
    "curie-mobile",
    "connected-care",
    "shared-platform",
    "browser-extension",
    "public-web",
})

VALID_PLATFORMS = frozenset({
    "browser-wasm-desktop",
    "browser-wasm-mobile",
    "desktop-linux",
    "desktop-macos",
    "desktop-windows",
    "android-native",
    "ios-native",
    "chrome-extension-popup",
    "hosted-services",
    "static-public-web",
})

VALID_STATUSES = frozenset({
    "observed-and-required",
    "intended-but-unimplemented",
    "accepted-difference",
    "unresolved",
})

VALID_COVERAGE_STATES = frozenset({
    "unknown",
    "blocked",
    "waived",
    "passed",
})

REQ_ID_RE = re.compile(
    r"^EF-(CLINIC|MOBILE|CONN|HARNESS|PLATFORM|EXT|PUBLIC)-([0-9]{4})$"
)
SCN_ID_RE = re.compile(
    r"^SCN-(CLINIC|MOBILE|CONN|HARNESS|PLATFORM|EXT|PUBLIC|M[0-9]+)-([0-9]{4})$"
)

REQUIRED_REQ_FIELDS = (
    "id",
    "title",
    "domain",
    "product",
    "platforms",
    "status",
    "package",
    "scenario_id",
    "coverage",
)


def _is_within_root(repo_root: Path, candidate: Path) -> bool:
    try:
        candidate.resolve().relative_to(repo_root.resolve())
        return True
    except ValueError:
        return False


def _validate_coverage_entry(
    req_id: str,
    platform: str,
    entry: Any,
    repo_root: Path,
) -> list[str]:
    errors: list[str] = []
    prefix = f"{req_id} coverage[{platform}]"
    if not isinstance(entry, dict):
        return [f"{prefix}: must be a table"]

    state = entry.get("state")
    revision = entry.get("revision")
    evidence = entry.get("evidence")
    reason = entry.get("reason", "")

    if not isinstance(state, str) or state not in VALID_COVERAGE_STATES:
        errors.append(
            f"{prefix}: invalid state {state!r}; expected one of {sorted(VALID_COVERAGE_STATES)}"
        )
        return errors

    if not isinstance(revision, str):
        errors.append(f"{prefix}: revision must be a string")
    if not isinstance(evidence, str):
        errors.append(f"{prefix}: evidence must be a string")
    if not isinstance(reason, str):
        errors.append(f"{prefix}: reason must be a string")

    if errors:
        return errors

    if state == "passed":
        if not revision.strip():
            errors.append(f"{prefix}: state 'passed' requires a non-empty revision")
        if not evidence.strip():
            errors.append(f"{prefix}: state 'passed' requires a non-empty evidence path")
        else:
            evidence_path = repo_root / evidence
            if not _is_within_root(repo_root, evidence_path):
                errors.append(f"{prefix}: evidence path {evidence!r} escapes repository root")
            elif not evidence_path.exists():
                errors.append(f"{prefix}: evidence path {evidence!r} does not exist on disk")
    elif state in {"blocked", "waived"}:
        if not reason.strip():
            errors.append(f"{prefix}: state {state!r} requires a non-empty reason")
    elif state == "unknown":
        if revision.strip() or evidence.strip():
            errors.append(
                f"{prefix}: state 'unknown' must have empty revision and evidence fields"
            )

    return errors


def _validate_package_and_scenario(
    req_id: str,
    scenario_id: str,
    package_rel: str,
    platforms: list[str],
    repo_root: Path,
) -> list[str]:
    errors: list[str] = []
    if not package_rel.startswith("specs/packages/"):
        return [f"{req_id}: package {package_rel!r} must live under 'specs/packages/'"]

    package_dir = repo_root / package_rel
    if not _is_within_root(repo_root, package_dir):
        return [f"{req_id}: package {package_rel!r} escapes repository root"]
    if not package_dir.is_dir():
        return [f"{req_id}: package directory {package_rel!r} does not exist"]

    spec_file = package_dir / "spec.md"
    if not spec_file.is_file():
        errors.append(f"{req_id}: missing specification file '{package_rel}/spec.md'")
    else:
        spec_text = spec_file.read_text(encoding="utf-8")
        if not re.search(rf"\b{re.escape(req_id)}\b", spec_text):
            errors.append(
                f"{req_id}: '{package_rel}/spec.md' does not reference requirement ID {req_id}"
            )

    scenarios_file = package_dir / "scenarios.toml"
    if not scenarios_file.is_file():
        errors.append(f"{req_id}: missing scenarios file '{package_rel}/scenarios.toml'")
        return errors

    try:
        scenarios_data = tomllib.loads(scenarios_file.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        errors.append(f"{req_id}: invalid TOML in '{package_rel}/scenarios.toml': {exc}")
        return errors

    scenarios = scenarios_data.get("scenarios")
    if not isinstance(scenarios, list):
        errors.append(
            f"{req_id}: '{package_rel}/scenarios.toml' must define a [[scenarios]] list"
        )
        return errors

    matches = [
        item for item in scenarios
        if isinstance(item, dict) and item.get("id") == scenario_id
    ]
    if not matches:
        errors.append(
            f"{req_id}: scenario_id {scenario_id!r} not found in '{package_rel}/scenarios.toml'"
        )
        return errors
    if len(matches) > 1:
        errors.append(
            f"{req_id}: duplicate scenario_id {scenario_id!r} in '{package_rel}/scenarios.toml'"
        )
        return errors

    scenario = matches[0]
    scn_req_id = scenario.get("requirement_id")
    if scn_req_id != req_id:
        errors.append(
            f"{req_id}: scenario {scenario_id!r} in '{package_rel}/scenarios.toml' "
            f"declares requirement_id={scn_req_id!r}, expected {req_id!r}"
        )

    scn_platforms = scenario.get("platforms")
    if isinstance(scn_platforms, list):
        missing = sorted(set(platforms) - set(scn_platforms))
        if missing:
            errors.append(
                f"{req_id}: scenario {scenario_id!r} does not cover declared platforms {missing}"
            )

    steps = scenario.get("steps")
    if not isinstance(steps, list) or not steps:
        errors.append(
            f"{req_id}: scenario {scenario_id!r} in '{package_rel}/scenarios.toml' "
            f"must define at least one executable step"
        )

    return errors


def validate_ledger(
    repo_root: Path,
    ledger_rel: Path = Path("specs/ledger.toml"),
) -> list[str]:
    """Return a list of validation errors for the requirement ledger (empty if valid)."""
    errors: list[str] = []
    ledger_path = repo_root / ledger_rel
    if not ledger_path.is_file():
        return [f"ledger file does not exist: {ledger_rel}"]

    try:
        data = tomllib.loads(ledger_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        return [f"invalid TOML in {ledger_rel}: {exc}"]

    if data.get("schema_version") != 1:
        errors.append(f"schema_version must be 1, got {data.get('schema_version')!r}")

    requirements = data.get("requirements")
    if not isinstance(requirements, list) or not requirements:
        errors.append("requirements must be a non-empty list of [[requirements]] entries")
        return errors

    seen_ids: set[str] = set()

    for idx, req in enumerate(requirements):
        label = f"requirements[{idx}]"
        if not isinstance(req, dict):
            errors.append(f"{label}: entry must be a table")
            continue

        missing_fields = [f for f in REQUIRED_REQ_FIELDS if f not in req]
        if missing_fields:
            errors.append(f"{label}: missing required fields {missing_fields}")
            continue

        req_id = req["id"]
        if not isinstance(req_id, str):
            errors.append(f"{label}: id must be a string")
            continue

        req_match = REQ_ID_RE.match(req_id)
        if not req_match:
            errors.append(
                f"{label}: id {req_id!r} does not match EF-<DOMAIN>-<NNNN>"
            )
            continue

        if req_id in seen_ids:
            errors.append(f"{req_id}: duplicate requirement ID")
        seen_ids.add(req_id)

        title = req["title"]
        if not isinstance(title, str) or not title.strip():
            errors.append(f"{req_id}: title must be a non-empty string")

        domain = req["domain"]
        if not isinstance(domain, str) or domain not in VALID_DOMAINS:
            errors.append(
                f"{req_id}: invalid domain {domain!r}; expected one of {sorted(VALID_DOMAINS)}"
            )
        elif req_match.group(1) != domain:
            errors.append(
                f"{req_id}: domain {domain!r} does not match ID domain {req_match.group(1)!r}"
            )

        product = req["product"]
        if not isinstance(product, str) or product not in VALID_PRODUCTS:
            errors.append(
                f"{req_id}: invalid product {product!r}; expected one of {sorted(VALID_PRODUCTS)}"
            )

        status = req["status"]
        if not isinstance(status, str) or status not in VALID_STATUSES:
            errors.append(
                f"{req_id}: invalid status {status!r}; expected one of {sorted(VALID_STATUSES)}"
            )

        platforms = req["platforms"]
        if not isinstance(platforms, list) or not platforms:
            errors.append(f"{req_id}: platforms must be a non-empty list")
            continue
        if len(set(platforms)) != len(platforms):
            errors.append(f"{req_id}: platforms contains duplicate entries")
        bad_platforms = [
            p for p in platforms if not isinstance(p, str) or p not in VALID_PLATFORMS
        ]
        if bad_platforms:
            errors.append(
                f"{req_id}: invalid platforms {bad_platforms!r}; "
                f"expected subset of {sorted(VALID_PLATFORMS)}"
            )

        scenario_id = req["scenario_id"]
        if not isinstance(scenario_id, str):
            errors.append(f"{req_id}: scenario_id must be a string")
        else:
            scn_match = SCN_ID_RE.match(scenario_id)
            if not scn_match:
                errors.append(
                    f"{req_id}: scenario_id {scenario_id!r} does not match "
                    f"SCN-<DOMAIN|M#>-<NNNN>"
                )
            elif (
                isinstance(domain, str)
                and scn_match.group(1) in VALID_DOMAINS
                and scn_match.group(1) != domain
            ):
                errors.append(
                    f"{req_id}: scenario_id {scenario_id!r} domain {scn_match.group(1)!r} "
                    f"does not match requirement domain {domain!r}"
                )

        coverage = req["coverage"]
        if not isinstance(coverage, dict):
            errors.append(f"{req_id}: coverage must be a table keyed by platform")
        else:
            declared_set = set(p for p in platforms if isinstance(p, str))
            coverage_set = set(coverage.keys())
            missing_cov = sorted(declared_set - coverage_set)
            extra_cov = sorted(coverage_set - declared_set)
            if missing_cov:
                errors.append(
                    f"{req_id}: missing coverage entries for platforms {missing_cov}"
                )
            if extra_cov:
                errors.append(
                    f"{req_id}: coverage contains undeclared platforms {extra_cov}"
                )
            for platform_name, entry in sorted(coverage.items()):
                if platform_name in declared_set:
                    errors.extend(
                        _validate_coverage_entry(req_id, platform_name, entry, repo_root)
                    )

        package_rel = req["package"]
        if not isinstance(package_rel, str) or not package_rel.strip():
            errors.append(f"{req_id}: package must be a non-empty path string")
        elif isinstance(scenario_id, str) and not bad_platforms:
            errors.extend(
                _validate_package_and_scenario(
                    req_id, scenario_id, package_rel, platforms, repo_root
                )
            )

    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Validate specs/ledger.toml and all referenced packages/scenarios."
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="Repository root directory (default: parent of scripts/).",
    )
    parser.add_argument(
        "--ledger",
        type=Path,
        default=Path("specs/ledger.toml"),
        help="Path to ledger TOML relative to --repo-root.",
    )
    args = parser.parse_args(argv)

    errors = validate_ledger(args.repo_root, args.ledger)
    if errors:
        print(f"Ledger validation FAILED ({len(errors)} error(s)):", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    data = tomllib.loads((args.repo_root / args.ledger).read_text(encoding="utf-8"))
    count = len(data["requirements"])
    print(f"Ledger validation OK: {count} requirement(s) verified in {args.ledger}.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
