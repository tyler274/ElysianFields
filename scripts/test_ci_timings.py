#!/usr/bin/env python3
"""Unit tests for scripts/ci_timings.py including hard budget enforcement."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import ci_timings

REPO_ROOT = Path(__file__).resolve().parent.parent
BUDGETS_PATH = REPO_ROOT / "docs" / "ci" / "budgets.json"


def _fake_api(event: str = "pull_request", native_secs: int = 40, total_end_sec: int = 85):
    """Build a mock GitHub Actions API callable returning a deterministic run."""

    def _ts(offset_secs: int) -> str:
        mins, secs = divmod(offset_secs, 60)
        hours, mins = divmod(mins, 60)
        return f"2026-11-27T{hours:02d}:{mins:02d}:{secs:02d}Z"

    run_payload = {
        "id": 1001,
        "run_attempt": 1,
        "display_title": "Test PR CI",
        "head_sha": "0123456789abcdef0123456789abcdef01234567",
        "event": event,
        "status": "in_progress",
        "conclusion": None,
        "run_started_at": _ts(0),
    }
    jobs_payload = {
        "total_count": 5,
        "jobs": [
            {
                "name": "Select CI scope",
                "status": "completed",
                "conclusion": "success",
                "started_at": _ts(0),
                "completed_at": _ts(8),
                "steps": [
                    {
                        "name": "Classify change scope",
                        "status": "completed",
                        "conclusion": "success",
                        "started_at": _ts(5),
                        "completed_at": _ts(7),
                    }
                ],
            },
            {
                "name": "Lightweight checks",
                "status": "completed",
                "conclusion": "success",
                "started_at": _ts(10),
                "completed_at": _ts(28),
                "steps": [],
            },
            {
                "name": "Rust native",
                "status": "completed",
                "conclusion": "success",
                "started_at": _ts(10),
                "completed_at": _ts(10 + native_secs),
                "steps": [],
            },
            {
                "name": "Rust Wasm",
                "status": "completed",
                "conclusion": "success",
                "started_at": _ts(10),
                "completed_at": _ts(total_end_sec),
                "steps": [],
            },
            {
                "name": "CI result",
                "status": "in_progress",
                "conclusion": None,
                "started_at": _ts(total_end_sec),
                "completed_at": None,
                "steps": [],
            },
        ],
    }

    def api(endpoint: str) -> dict:
        if endpoint.endswith("/runs/1001") or endpoint.endswith("/attempts/1"):
            return run_payload
        if "/jobs?" in endpoint:
            return jobs_payload
        raise KeyError(f"unexpected endpoint: {endpoint}")

    return api


class CiTimingsTests(unittest.TestCase):
    def test_budgets_json_loads_required_keys(self) -> None:
        budgets = ci_timings.load_budgets(BUDGETS_PATH)
        self.assertGreater(budgets["pr_feedback_secs"], 0)
        self.assertGreater(budgets["merge_to_live_secs"], 0)
        self.assertGreater(budgets["client_only_merge_secs"], 0)
        for job in ("scope", "lightweight", "rust-native", "rust-wasm", "cargo-deny", "deploy-client"):
            self.assertIn(job, budgets["per_job_budgets"])

    def test_within_budget_passes_and_writes_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out_html = Path(tmp) / "report.html"
            out_md = Path(tmp) / "summary.md"
            rc = ci_timings.main(
                [
                    "run",
                    "--repo",
                    "tyler274/ElysianFields",
                    "--run-id",
                    "1001",
                    "--attempt",
                    "1",
                    "--budgets",
                    str(BUDGETS_PATH),
                    "--check-budgets",
                    "--output",
                    str(out_html),
                    "--summary",
                    str(out_md),
                ],
                api=_fake_api(event="pull_request", native_secs=40, total_end_sec=85),
            )
            self.assertEqual(rc, 0)
            self.assertTrue(out_html.is_file())
            self.assertTrue(out_html.with_suffix(".json").is_file())
            self.assertTrue(out_md.is_file())
            self.assertIn("BUDGET PASS", out_md.read_text(encoding="utf-8"))
            self.assertIn("<svg", out_html.read_text(encoding="utf-8"))

    def test_job_budget_breach_exits_nonzero(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out_html = Path(tmp) / "report.html"
            out_md = Path(tmp) / "summary.md"
            rc = ci_timings.main(
                [
                    "run",
                    "--repo",
                    "tyler274/ElysianFields",
                    "--run-id",
                    "1001",
                    "--attempt",
                    "1",
                    "--budgets",
                    str(BUDGETS_PATH),
                    "--check-budgets",
                    "--output",
                    str(out_html),
                    "--summary",
                    str(out_md),
                ],
                api=_fake_api(event="pull_request", native_secs=300, total_end_sec=310),
            )
            self.assertEqual(rc, 1)
            md = out_md.read_text(encoding="utf-8")
            self.assertIn("BUDGET BREACH", md)
            self.assertIn("Rust native", md)

    def test_synthetic_extra_secs_triggers_budget_breach(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out_html = Path(tmp) / "report.html"
            out_md = Path(tmp) / "summary.md"
            rc = ci_timings.main(
                [
                    "run",
                    "--repo",
                    "tyler274/ElysianFields",
                    "--run-id",
                    "1001",
                    "--attempt",
                    "1",
                    "--budgets",
                    str(BUDGETS_PATH),
                    "--check-budgets",
                    "--test-extra-secs",
                    "500",
                    "--output",
                    str(out_html),
                    "--summary",
                    str(out_md),
                ],
                api=_fake_api(event="pull_request", native_secs=40, total_end_sec=85),
            )
            self.assertEqual(rc, 1)
            self.assertIn("BUDGET BREACH", out_md.read_text(encoding="utf-8"))

    def test_client_only_push_uses_client_only_budget(self) -> None:
        budgets = json.loads(BUDGETS_PATH.read_text(encoding="utf-8"))
        collector = ci_timings.Collector(
            "tyler274/ElysianFields",
            api=_fake_api(event="push", native_secs=30, total_end_sec=260),
        )
        run = collector.run(1001, 1)
        ev = ci_timings.evaluate_budgets(run, budgets, client_only=True)
        self.assertEqual(ev.total_metric, "client_only_merge_secs")
        self.assertFalse(ev.passed)


if __name__ == "__main__":
    unittest.main()
