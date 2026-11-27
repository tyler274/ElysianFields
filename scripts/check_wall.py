#!/usr/bin/env python3
"""Automated clean-room wall linter for Elysian Fields.

Scans builder-side specifications (`specs/`), replacement crates (`crates/`),
and protocol documentation (`book/`) while excluding observer-side paths
(`docs/reference/`, `.observer-staging/`). Fails if any forbidden reference
path, crate/framework name, internal signature, DOM/CSS selector, or Linear
ticket ID crosses the wall.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import asdict, dataclass
from pathlib import Path

SCANNED_ROOTS: tuple[tuple[str, str], ...] = (
    ("specs", "specs"),
    ("crates", "crates"),
    ("book", "book"),
)

EXCLUDED_PATH_PREFIXES: tuple[str, ...] = (
    "docs/reference/",
    ".observer-staging/",
    "book/book/",
    "target/",
    ".git/",
)

SKIP_DIR_NAMES = frozenset({
    ".git",
    "target",
    "__pycache__",
    "node_modules",
})

BINARY_EXTENSIONS = frozenset({
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".webp",
    ".ico",
    ".woff",
    ".woff2",
    ".ttf",
    ".otf",
    ".wasm",
    ".pdf",
    ".zip",
    ".gz",
    ".tar",
})


@dataclass(frozen=True)
class WallRule:
    rule_id: str
    scopes: frozenset[str]
    pattern: re.Pattern[str]
    description: str


@dataclass(frozen=True)
class Violation:
    file: str
    line: int
    rule_id: str
    match: str
    description: str


RULES: tuple[WallRule, ...] = (
    WallRule(
        rule_id="WALL-001",
        scopes=frozenset({"specs", "crates", "book"}),
        pattern=re.compile(
            r"(?:/home/[^\s\"'`]+/dabney-moe\b|/home/[^\s\"'`]+/coda/worktrees\b|\belysian-ref-[0-9a-f]+\b|\bdabney-moe-rust\b)"
        ),
        description="Reference host checkout, worktree path, or reference repository leak",
    ),
    WallRule(
        rule_id="WALL-002",
        scopes=frozenset({"specs", "crates", "book"}),
        pattern=re.compile(
            r"(?:\bcrates/(?:physician-app|mobile-app|physician-sample|physician-worker|physician-bus|mobile-shell|clinical-library|duet-wire|extension-popup|front-door|dei-site|recovery-directory|ui-gallery|ancestry[a-z0-9_-]*)\b|\bsubmodules/coda-agents\b)"
        ),
        description="Reference workspace crate or submodule path leak",
    ),
    WallRule(
        rule_id="WALL-003",
        scopes=frozenset({"specs", "crates", "book"}),
        pattern=re.compile(r"\bdioxus\b", re.IGNORECASE),
        description="Reference UI framework name leak",
    ),
    WallRule(
        rule_id="WALL-004",
        scopes=frozenset({"specs", "crates", "book"}),
        pattern=re.compile(r"(?:\bsubmodules/coda-agents/eval\b|\beval/[a-zA-Z0-9._-]+)"),
        description="Sealed evaluation fixture path leak",
    ),
    WallRule(
        rule_id="WALL-005",
        scopes=frozenset({"specs", "crates"}),
        pattern=re.compile(
            r"(?:\b01b762838d639a51f82e9d7d15e788e02bac772f\b|\b01b762838\b|\bc0d217387\b)"
        ),
        description="Pinned reference git SHA leak behind the wall",
    ),
    WallRule(
        rule_id="WALL-006",
        scopes=frozenset({"book", "specs"}),
        pattern=re.compile(r"\bTYL-[0-9]+\b"),
        description="Linear issue-tracker ID forbidden in book/ and specs/",
    ),
    WallRule(
        rule_id="WALL-007",
        scopes=frozenset({"specs"}),
        pattern=re.compile(r"\bpub\s*\(\s*(?:crate|super|in\b)[^)]*\)"),
        description="Non-public Rust visibility signature forbidden in behavioral specs",
    ),
    WallRule(
        rule_id="WALL-008",
        scopes=frozenset({"specs"}),
        pattern=re.compile(
            r"(?:\bdata-(?:testid|test|cy|qa)\s*=|\b(?:querySelector|querySelectorAll|getElementById|getElementsByClassName)\s*\(|\b(?:class|className)\s*=\s*[\"'])",
            re.IGNORECASE,
        ),
        description="DOM test attribute, DOM API selector, or CSS class attribute forbidden in behavioral specs",
    ),
    WallRule(
        rule_id="WALL-009",
        scopes=frozenset({"specs"}),
        pattern=re.compile(
            r"(?:(?<![A-Za-z0-9_/.:])#[a-z][a-z0-9]*(?:[-_][a-z0-9]+)+(?![A-Za-z0-9_/-])|(?<=[\s\"'`(])\.[a-z][a-z0-9]*-[a-z0-9_-]+(?=[\"'`)\s,;]))"
        ),
        description="CSS ID or class selector pattern forbidden in behavioral specs",
    ),
)


def _is_excluded(rel_posix: str) -> bool:
    return any(rel_posix.startswith(prefix) for prefix in EXCLUDED_PATH_PREFIXES)


def _iter_target_files(repo_root: Path) -> list[tuple[str, Path]]:
    targets: list[tuple[str, Path]] = []
    for rel_dir, scope in SCANNED_ROOTS:
        root_dir = repo_root / rel_dir
        if not root_dir.is_dir():
            continue
        for path in sorted(root_dir.rglob("*")):
            if not path.is_file():
                continue
            if any(part in SKIP_DIR_NAMES for part in path.parts):
                continue
            rel_posix = path.relative_to(repo_root).as_posix()
            if _is_excluded(rel_posix):
                continue
            if path.suffix.lower() in BINARY_EXTENSIONS:
                continue
            targets.append((scope, path))
    return targets


def scan_file(repo_root: Path, scope: str, path: Path) -> list[Violation]:
    rel_posix = path.relative_to(repo_root).as_posix()
    try:
        content = path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        return []

    active_rules = [rule for rule in RULES if scope in rule.scopes]
    violations: list[Violation] = []

    for line_no, line in enumerate(content.splitlines(), start=1):
        for rule in active_rules:
            for match in rule.pattern.finditer(line):
                violations.append(
                    Violation(
                        file=rel_posix,
                        line=line_no,
                        rule_id=rule.rule_id,
                        match=match.group(0),
                        description=rule.description,
                    )
                )
    return violations


def scan_repo(repo_root: Path) -> tuple[int, list[Violation]]:
    targets = _iter_target_files(repo_root)
    violations: list[Violation] = []
    for scope, path in targets:
        violations.extend(scan_file(repo_root, scope, path))
    return len(targets), violations


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Run the Elysian Fields clean-room wall leakage linter."
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

    scanned_count, violations = scan_repo(args.repo_root)

    if args.json:
        payload = {
            "scanned_files": scanned_count,
            "violation_count": len(violations),
            "violations": [asdict(v) for v in violations],
        }
        print(json.dumps(payload, indent=2))
        return 1 if violations else 0

    if violations:
        print(
            f"Clean-room wall check FAILED: {len(violations)} violation(s) "
            f"across {scanned_count} scanned file(s):",
            file=sys.stderr,
        )
        for v in violations:
            print(
                f"  - {v.file}:{v.line} [{v.rule_id}] matched {v.match!r}: {v.description}",
                file=sys.stderr,
            )
        return 1

    print(f"Clean-room wall check OK: {scanned_count} file(s) scanned, 0 violations.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
