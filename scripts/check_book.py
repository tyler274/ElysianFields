#!/usr/bin/env python3
"""Offline integrity verifier for the Elysian Realm mdBook (`book/`).

Validates:
- `book/book.toml` parses as valid TOML and declares a non-empty title and source directory.
- `book/src/SUMMARY.md` exists, lists chapters, and every referenced chapter file exists,
  stays inside `book/src/`, and is non-empty.
- Every relative markdown link across `book/src/**/*.md` (outside fenced code blocks and
  inline code spans) resolves to an existing file on disk (handling `#anchors`).
- Zero Linear ticket IDs (`TYL-\\d+`) and zero dated changelog/history headings appear
  anywhere inside `book/src/`.

Usage:
    python3 scripts/check_book.py [--repo-root <dir>] [--json]
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from dataclasses import asdict, dataclass
from pathlib import Path
from urllib.parse import unquote

EXTERNAL_SCHEMES = ("http://", "https://", "mailto:", "tel:")
TICKET_ID_RE = re.compile(r"\bTYL-[0-9]+\b")
HEADING_RE = re.compile(r"^\s{0,3}#{1,6}\s+(.+?)\s*$")
DATED_HEADING_RE = re.compile(r"\b20[0-9]{2}-[0-9]{2}-[0-9]{2}\b")
CHANGELOG_HEADING_RE = re.compile(
    r"\b(?:changelog|change\s+log|revision\s+history|audit\s+trail|decision\s+log)\b",
    re.IGNORECASE,
)
FENCE_RE = re.compile(r"^\s{0,3}(`{3,}|~{3,})")
INLINE_CODE_RE = re.compile(r"`[^`]*`")
MD_LINK_RE = re.compile(r"!?\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")


@dataclass(frozen=True)
class BookViolation:
    file: str
    line: int
    code: str
    message: str


def _iter_non_fenced_lines(text: str) -> list[tuple[int, str]]:
    result: list[tuple[int, str]] = []
    in_fence = False
    fence_char = ""
    fence_len = 0

    for line_no, line in enumerate(text.splitlines(), start=1):
        match = FENCE_RE.match(line)
        if match:
            marker = match.group(1)
            if not in_fence:
                in_fence = True
                fence_char = marker[0]
                fence_len = len(marker)
            elif marker[0] == fence_char and len(marker) >= fence_len:
                in_fence = False
                fence_char = ""
                fence_len = 0
            continue
        if not in_fence:
            result.append((line_no, line))
    return result


def _strip_inline_code(line: str) -> str:
    return INLINE_CODE_RE.sub("", line)


def _is_external_target(target: str) -> bool:
    lower = target.lower()
    return any(lower.startswith(scheme) for scheme in EXTERNAL_SCHEMES)


def _extract_links(line: str) -> list[str]:
    cleaned = _strip_inline_code(line)
    links: list[str] = []
    for match in MD_LINK_RE.finditer(cleaned):
        raw = match.group(1).strip()
        if raw.startswith("<") and raw.endswith(">"):
            raw = raw[1:-1].strip()
        if raw:
            links.append(raw)
    return links


def check_book(repo_root: Path) -> tuple[int, list[BookViolation]]:
    repo_root = repo_root.resolve()
    violations: list[BookViolation] = []

    book_dir = repo_root / "book"
    book_toml = book_dir / "book.toml"
    if not book_toml.is_file():
        return 0, [
            BookViolation(
                file="book/book.toml",
                line=1,
                code="BOOK-TOML-MISSING",
                message="book/book.toml does not exist",
            )
        ]

    try:
        config = tomllib.loads(book_toml.read_text(encoding="utf-8"))
    except (tomllib.TOMLDecodeError, UnicodeDecodeError) as exc:
        return 0, [
            BookViolation(
                file="book/book.toml",
                line=1,
                code="BOOK-TOML-INVALID",
                message=f"failed to parse book/book.toml: {exc}",
            )
        ]

    book_table = config.get("book")
    if not isinstance(book_table, dict):
        violations.append(
            BookViolation(
                file="book/book.toml",
                line=1,
                code="BOOK-TOML-SCHEMA",
                message="missing [book] section in book/book.toml",
            )
        )
        src_rel = "src"
    else:
        title = book_table.get("title")
        if not isinstance(title, str) or not title.strip():
            violations.append(
                BookViolation(
                    file="book/book.toml",
                    line=1,
                    code="BOOK-TOML-TITLE",
                    message="[book].title must be a non-empty string",
                )
            )
        src_rel = str(book_table.get("src", "src"))

    src_dir = (book_dir / src_rel).resolve()
    if not src_dir.is_dir():
        violations.append(
            BookViolation(
                file="book/book.toml",
                line=1,
                code="BOOK-SRC-MISSING",
                message=f"configured book source directory does not exist: {src_rel}",
            )
        )
        return 0, violations

    summary_path = src_dir / "SUMMARY.md"
    if not summary_path.is_file():
        violations.append(
            BookViolation(
                file=summary_path.relative_to(repo_root).as_posix(),
                line=1,
                code="BOOK-SUMMARY-MISSING",
                message="book SUMMARY.md does not exist",
            )
        )
    else:
        summary_text = summary_path.read_text(encoding="utf-8")
        summary_rel = summary_path.relative_to(repo_root).as_posix()
        seen_chapters: set[Path] = set()
        chapter_count = 0

        for line_no, line in _iter_non_fenced_lines(summary_text):
            for target in _extract_links(line):
                if _is_external_target(target) or target.startswith("#"):
                    continue
                path_part = unquote(target.split("#", 1)[0].split("?", 1)[0])
                if not path_part:
                    continue
                chapter_count += 1
                resolved = (summary_path.parent / path_part).resolve()
                try:
                    resolved.relative_to(src_dir)
                except ValueError:
                    violations.append(
                        BookViolation(
                            file=summary_rel,
                            line=line_no,
                            code="BOOK-SUMMARY-ESCAPE",
                            message=f"SUMMARY.md chapter escapes book src dir: {target}",
                        )
                    )
                    continue

                if resolved in seen_chapters:
                    violations.append(
                        BookViolation(
                            file=summary_rel,
                            line=line_no,
                            code="BOOK-SUMMARY-DUPLICATE",
                            message=f"duplicate chapter entry in SUMMARY.md: {target}",
                        )
                    )
                    continue
                seen_chapters.add(resolved)

                if not resolved.is_file():
                    violations.append(
                        BookViolation(
                            file=summary_rel,
                            line=line_no,
                            code="BOOK-CHAPTER-MISSING",
                            message=f"SUMMARY.md references missing chapter: {target}",
                        )
                    )
                elif not resolved.read_text(encoding="utf-8").strip():
                    violations.append(
                        BookViolation(
                            file=summary_rel,
                            line=line_no,
                            code="BOOK-CHAPTER-EMPTY",
                            message=f"SUMMARY.md references empty chapter: {target}",
                        )
                    )

        if chapter_count == 0:
            violations.append(
                BookViolation(
                    file=summary_rel,
                    line=1,
                    code="BOOK-SUMMARY-EMPTY",
                    message="SUMMARY.md must reference at least one chapter",
                )
            )

    md_files = sorted(p for p in src_dir.rglob("*.md") if p.is_file())
    for md_path in md_files:
        rel_posix = md_path.relative_to(repo_root).as_posix()
        text = md_path.read_text(encoding="utf-8")

        for line_no, line in enumerate(text.splitlines(), start=1):
            ticket_match = TICKET_ID_RE.search(line)
            if ticket_match is not None:
                violations.append(
                    BookViolation(
                        file=rel_posix,
                        line=line_no,
                        code="BOOK-TICKET-ID",
                        message=f"forbidden Linear ticket ID {ticket_match.group(0)!r} in book",
                    )
                )

        for line_no, line in _iter_non_fenced_lines(text):
            heading_match = HEADING_RE.match(line)
            if heading_match is not None:
                heading_text = heading_match.group(1)
                if DATED_HEADING_RE.search(heading_text) or CHANGELOG_HEADING_RE.search(
                    heading_text
                ):
                    violations.append(
                        BookViolation(
                            file=rel_posix,
                            line=line_no,
                            code="BOOK-CHANGELOG-HEADING",
                            message=f"dated/changelog heading forbidden in current-truth book: {heading_text!r}",
                        )
                    )

            for target in _extract_links(line):
                if _is_external_target(target):
                    continue
                if target.startswith("#"):
                    if len(target) <= 1:
                        violations.append(
                            BookViolation(
                                file=rel_posix,
                                line=line_no,
                                code="BOOK-EMPTY-ANCHOR",
                                message="empty anchor link '#' in markdown",
                            )
                        )
                    continue

                path_part, _, anchor_part = target.partition("#")
                path_clean = unquote(path_part.split("?", 1)[0])
                if "#" in target and not anchor_part.strip():
                    violations.append(
                        BookViolation(
                            file=rel_posix,
                            line=line_no,
                            code="BOOK-EMPTY-ANCHOR",
                            message=f"empty anchor fragment in link: {target!r}",
                        )
                    )
                    continue

                if not path_clean:
                    continue

                resolved_target = (md_path.parent / path_clean).resolve()
                if not resolved_target.exists():
                    violations.append(
                        BookViolation(
                            file=rel_posix,
                            line=line_no,
                            code="BOOK-BROKEN-LINK",
                            message=f"broken relative link {target!r} (resolved to {resolved_target})",
                        )
                    )

    return len(md_files), violations


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Verify book/book.toml, SUMMARY.md, relative links, and current-truth rules."
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

    scanned_files, violations = check_book(args.repo_root)
    if args.json:
        payload = {
            "scanned_markdown_files": scanned_files,
            "violation_count": len(violations),
            "violations": [asdict(v) for v in violations],
        }
        print(json.dumps(payload, indent=2))
        return 1 if violations else 0

    if violations:
        print(
            f"Book integrity check FAILED: {len(violations)} violation(s) across "
            f"{scanned_files} markdown file(s):",
            file=sys.stderr,
        )
        for v in violations:
            print(f"  - {v.file}:{v.line} [{v.code}] {v.message}", file=sys.stderr)
        return 1

    print(f"Book integrity check OK: {scanned_files} markdown file(s) verified, 0 violations.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
