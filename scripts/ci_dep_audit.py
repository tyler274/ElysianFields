#!/usr/bin/env python3
"""Audit Cargo build output to report workspace vs. third-party crate compilations.

Reads Cargo output from stdin, echoes it verbatim to stdout, and prints a concise
summary of how many workspace crates vs. third-party dependencies were compiled.
Optionally appends the audit row to GITHUB_STEP_SUMMARY so warm-cache hits (zero
third-party recompilations on source-only edits) are directly observable in CI.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import os
from pathlib import Path
import re
import sys
from typing import Iterable

COMPILING_RE = re.compile(r"^\s*Compiling\s+([A-Za-z0-9_-]+)\s+v([^\s]+)(?:\s+\(([^)]+)\))?")


@dataclass(frozen=True)
class DepAudit:
    workspace_crates: tuple[str, ...]
    third_party_crates: tuple[str, ...]

    @property
    def workspace_count(self) -> int:
        return len(self.workspace_crates)

    @property
    def third_party_count(self) -> int:
        return len(self.third_party_crates)

    @property
    def third_party_recompiled(self) -> bool:
        return self.third_party_count > 0


def parse_cargo_output(lines: Iterable[str], workspace_root: Path | None = None) -> DepAudit:
    root_prefix: str | None = None
    if workspace_root is not None:
        root_prefix = str(workspace_root.resolve()).replace("\\", "/").rstrip("/")

    workspace: list[str] = []
    third_party: list[str] = []
    seen_workspace: set[str] = set()
    seen_third_party: set[str] = set()

    for line in lines:
        match = COMPILING_RE.match(line)
        if not match:
            continue
        crate_name = match.group(1)
        path_str = match.group(3)
        is_workspace = False
        if path_str:
            normalized = path_str.strip().replace("\\", "/")
            if root_prefix is not None:
                is_workspace = normalized == root_prefix or normalized.startswith(root_prefix + "/")
            else:
                is_workspace = True
        if is_workspace:
            if crate_name not in seen_workspace:
                seen_workspace.add(crate_name)
                workspace.append(crate_name)
        else:
            if crate_name not in seen_third_party:
                seen_third_party.add(crate_name)
                third_party.append(crate_name)

    return DepAudit(
        workspace_crates=tuple(workspace),
        third_party_crates=tuple(third_party),
    )


def format_report(label: str, audit: DepAudit) -> str:
    cache_state = (
        "CACHE HIT (0 third-party crates compiled)"
        if not audit.third_party_recompiled
        else f"REBUILT {audit.third_party_count} third-party crate(s)"
    )
    return (
        f"[dep-audit] {label}: {cache_state} | "
        f"workspace={audit.workspace_count}, third_party={audit.third_party_count}"
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--label", required=True, help="Human-readable CI step label.")
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path.cwd(),
        help="Repository root used to distinguish workspace crates from third-party crates.",
    )
    parser.add_argument(
        "--summary",
        type=Path,
        default=(
            Path(os.environ["GITHUB_STEP_SUMMARY"])
            if os.environ.get("GITHUB_STEP_SUMMARY")
            else None
        ),
        help="Optional Markdown file to append the dependency compilation audit line.",
    )
    args = parser.parse_args(argv)

    captured: list[str] = []
    for line in sys.stdin:
        sys.stdout.write(line)
        captured.append(line)
    sys.stdout.flush()

    audit = parse_cargo_output(captured, workspace_root=args.workspace_root)
    report = format_report(args.label, audit)
    print(report)

    if args.summary is not None:
        args.summary.parent.mkdir(parents=True, exist_ok=True)
        with args.summary.open("a", encoding="utf-8") as handle:
            handle.write(f"- {report}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
