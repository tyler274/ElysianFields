"""Unit tests for the offline mdBook integrity verifier (`scripts/check_book.py`)."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_book  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent


def _write_minimal_book(root: Path) -> Path:
    book_src = root / "book" / "src"
    book_src.mkdir(parents=True, exist_ok=True)
    (root / "book" / "book.toml").write_text(
        '[book]\ntitle = "Test Book"\nsrc = "src"\n',
        encoding="utf-8",
    )
    (book_src / "SUMMARY.md").write_text(
        "# Summary\n\n- [Intro](./intro.md)\n- [Details](./details.md)\n",
        encoding="utf-8",
    )
    (book_src / "intro.md").write_text(
        "# Intro\n\nSee [Details](./details.md#architecture).\n",
        encoding="utf-8",
    )
    (book_src / "details.md").write_text(
        "# Details\n\n## Architecture\n\nCurrent truth description.\n",
        encoding="utf-8",
    )
    return book_src


class CheckBookTests(unittest.TestCase):
    def test_real_repo_book_passes_cli_and_json(self) -> None:
        scanned, violations = check_book.check_book(REPO_ROOT)
        self.assertGreaterEqual(scanned, 9)
        self.assertEqual(violations, [], f"unexpected book violations: {violations}")

        proc = subprocess.run(
            [sys.executable, str(REPO_ROOT / "scripts" / "check_book.py"), "--json"],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        payload = json.loads(proc.stdout)
        self.assertEqual(payload["violation_count"], 0)
        self.assertGreaterEqual(payload["scanned_markdown_files"], 9)

    def test_minimal_valid_book_with_anchored_link_and_fenced_examples_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            src = _write_minimal_book(root)
            (src / "details.md").write_text(
                "# Details\n\n```sh\n# 2026-01-01 example shell comment\n# [not a link](./missing.md)\n```\n\n"
                "Inline `[also ignored](./missing.md)` and [external](https://example.org/docs).\n",
                encoding="utf-8",
            )
            _, violations = check_book.check_book(root)
            self.assertEqual(violations, [])

    def test_detects_missing_and_empty_chapters_in_summary(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            src = _write_minimal_book(root)
            (src / "details.md").unlink()
            (src / "intro.md").write_text("   \n", encoding="utf-8")

            _, violations = check_book.check_book(root)
            codes = {v.code for v in violations}
            self.assertIn("BOOK-CHAPTER-MISSING", codes)
            self.assertIn("BOOK-CHAPTER-EMPTY", codes)

    def test_detects_broken_relative_links_with_and_without_anchors(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            src = _write_minimal_book(root)
            (src / "intro.md").write_text(
                "# Intro\n\nBroken [a](./nope.md) and [b](./nope.md#sec).\n",
                encoding="utf-8",
            )
            _, violations = check_book.check_book(root)
            broken = [v for v in violations if v.code == "BOOK-BROKEN-LINK"]
            self.assertEqual(len(broken), 2)

    def test_detects_ticket_ids_and_dated_changelog_headings(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            src = _write_minimal_book(root)
            (src / "details.md").write_text(
                "# Details\n\n## 2026-05-01 Changelog\n\nTracked under TYL-123.\n",
                encoding="utf-8",
            )
            _, violations = check_book.check_book(root)
            codes = {v.code for v in violations}
            self.assertIn("BOOK-CHANGELOG-HEADING", codes)
            self.assertIn("BOOK-TICKET-ID", codes)

    def test_detects_invalid_book_toml(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_minimal_book(root)
            (root / "book" / "book.toml").write_text("[book\nbroken", encoding="utf-8")
            _, violations = check_book.check_book(root)
            self.assertEqual([v.code for v in violations], ["BOOK-TOML-INVALID"])


if __name__ == "__main__":
    unittest.main()
