#!/usr/bin/env python3
"""Collect GitHub Actions timings, render HTML/Markdown reports, and enforce CI budgets.

Only `gh api` accesses GitHub. Pass a custom `api` callable to `Collector` or `main`
for deterministic offline testing.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from datetime import datetime, timezone
from html import escape
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
from urllib.parse import quote

DEFAULT_BUDGETS_PATH = Path("docs/ci/budgets.json")

# Map GitHub Actions display job names to canonical `per_job_budgets` keys.
JOB_KEY_ALIASES: dict[str, str] = {
    "scope": "scope",
    "select ci scope": "scope",
    "lightweight": "lightweight",
    "lightweight checks": "lightweight",
    "rust-native": "rust-native",
    "rust native": "rust-native",
    "rust-wasm": "rust-wasm",
    "rust wasm": "rust-wasm",
    "cargo-deny": "cargo-deny",
    "deploy-client": "deploy-client",
    "deploy client (/elysianfields)": "deploy-client",
}

RESULT_JOB_NAMES: frozenset[str] = frozenset({"result", "ci result"})


def gh_api(endpoint: str) -> dict:
    """Authenticated GitHub API call; surface errors instead of returning empty data."""
    result = subprocess.run(
        ["gh", "api", endpoint],
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


class Collector:
    def __init__(self, repo: str, api=gh_api):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
            raise ValueError("repo must be OWNER/REPO")
        self.repo = repo
        self.root = f"repos/{repo}/actions"
        self.raw: dict[str, dict] = {}

        def record(endpoint: str) -> dict:
            payload = api(endpoint)
            self.raw[endpoint] = payload
            return payload

        self.api = record

    def pages(self, endpoint: str, key: str, limit: int | None = None) -> list[dict]:
        items: list[dict] = []
        page = 1
        size = min(100, limit) if limit else 100
        while limit is None or len(items) < limit:
            separator = "&" if "?" in endpoint else "?"
            payload = self.api(f"{endpoint}{separator}per_page={size}&page={page}")
            batch = payload[key]
            items.extend(batch)
            if len(batch) < size or len(items) >= payload.get("total_count", float("inf")):
                break
            page += 1
        return items[:limit] if limit else items

    def run(self, run_id: int, attempt: int | None = None) -> ReportRun:
        if run_id < 1 or (attempt is not None and attempt < 1):
            raise ValueError("run ID and attempt must be positive")
        base = f"{self.root}/runs/{run_id}"
        if attempt is None:
            attempt = self.api(base)["run_attempt"]
        run = self.api(f"{base}/attempts/{attempt}")
        jobs = self.pages(f"{base}/attempts/{attempt}/jobs", "jobs")
        return analyze(self.repo, run, jobs)

    def history(self, limit: int = 30, workflow: str = "ci.yml") -> list[ReportRun]:
        if limit < 1:
            raise ValueError("limit must be positive")
        runs = self.pages(
            f"{self.root}/workflows/{quote(workflow, safe='')}/runs",
            "workflow_runs",
            limit,
        )
        return [
            self.run(run["id"], attempt)
            for run in runs
            for attempt in range(1, run.get("run_attempt", 1) + 1)
        ]


def timestamp(value: str | None) -> datetime | None:
    if not value:
        return None
    try:
        result = datetime.fromisoformat(value.replace("Z", "+00:00"))
        if result.year < 2000 or result.tzinfo is None:
            return None
        return result
    except (ValueError, TypeError):
        return None


def interval(item: dict) -> tuple[datetime, datetime] | None:
    if item.get("status") != "completed" or item.get("conclusion") == "skipped":
        return None
    start = timestamp(item.get("started_at"))
    end = timestamp(item.get("completed_at"))
    return (start, end) if start and end and end >= start else None


def normalize_job_key(name: str) -> str:
    lowered = name.strip().lower()
    if lowered in JOB_KEY_ALIASES:
        return JOB_KEY_ALIASES[lowered]
    return lowered


def is_result_job(name: str) -> bool:
    return name.strip().lower() in RESULT_JOB_NAMES


@dataclass
class Timing:
    label: str
    budget_key: str
    start: datetime
    end: datetime
    conclusion: str

    @property
    def seconds(self) -> float:
        return (self.end - self.start).total_seconds()


@dataclass
class BudgetBreach:
    target: str
    actual_secs: float
    budget_secs: float

    @property
    def overage_secs(self) -> float:
        return self.actual_secs - self.budget_secs

    def format_line(self) -> str:
        return (
            f"{self.target}: {self.actual_secs:.1f}s exceeded budget "
            f"{self.budget_secs:.1f}s (+{self.overage_secs:.1f}s)"
        )


@dataclass
class BudgetEvaluation:
    total_metric: str
    total_actual_secs: float | None
    total_budget_secs: float
    job_rows: list[tuple[str, float, float | None, bool]]
    breaches: list[BudgetBreach]

    @property
    def passed(self) -> bool:
        return not self.breaches


@dataclass
class ReportRun:
    run_id: int
    attempt: int
    title: str
    url: str
    start: datetime | None
    sha: str
    event: str
    status: str
    conclusion: str
    jobs: list[Timing]
    steps: list[Timing]
    omitted: list[str]
    elapsed: float | None
    complete: bool

    @property
    def label(self) -> str:
        return f"run {self.run_id} (attempt {self.attempt})"


def analyze(repo: str, run: dict, jobs: list[dict]) -> ReportRun:
    completed_jobs: list[Timing] = []
    steps: list[Timing] = []
    omitted: list[str] = []
    complete = True

    for job in jobs:
        name = job.get("name", "unknown")
        times = interval(job)
        if times is None:
            reason = job.get("conclusion") or job.get("status") or "unknown"
            if job.get("status") == "completed" and reason != "skipped":
                reason += "; missing/invalid timestamps"
            omitted.append(f"{name}: {reason}")
            if not is_result_job(name) and job.get("conclusion") != "skipped":
                complete = False
            continue
        row = Timing(
            label=name,
            budget_key=normalize_job_key(name),
            start=times[0],
            end=times[1],
            conclusion=job.get("conclusion") or "unknown",
        )
        completed_jobs.append(row)
        for step in job.get("steps", []):
            step_times = interval(step)
            if step_times:
                steps.append(
                    Timing(
                        label=f"{name} / {step['name']}",
                        budget_key=row.budget_key,
                        start=step_times[0],
                        end=step_times[1],
                        conclusion=step.get("conclusion") or "unknown",
                    )
                )

    run_start = timestamp(run.get("run_started_at")) or min(
        (j.start for j in completed_jobs), default=None
    )
    end = max((j.end for j in completed_jobs), default=None)
    elapsed = (end - run_start).total_seconds() if run_start and end and end >= run_start else None
    attempt = int(run.get("run_attempt", 1))
    return ReportRun(
        run_id=int(run["id"]),
        attempt=attempt,
        title=run.get("display_title", "CI"),
        url=f"https://github.com/{repo}/actions/runs/{run['id']}/attempts/{attempt}",
        start=run_start,
        sha=run.get("head_sha", "unknown"),
        event=run.get("event", "unknown"),
        status=run.get("status", "unknown"),
        conclusion=run.get("conclusion") or "pending",
        jobs=completed_jobs,
        steps=steps,
        omitted=omitted,
        elapsed=elapsed,
        complete=complete,
    )


def load_budgets(path: Path) -> dict:
    data = json.loads(path.read_text(encoding="utf-8"))
    for required in (
        "pr_feedback_secs",
        "merge_to_live_secs",
        "client_only_merge_secs",
        "per_job_budgets",
    ):
        if required not in data:
            raise ValueError(f"budgets file {path} missing required key {required!r}")
    return data


def select_total_budget_key(event: str, *, client_only: bool = False) -> str:
    if event in {"pull_request", "merge_group"}:
        return "pr_feedback_secs"
    if client_only:
        return "client_only_merge_secs"
    return "merge_to_live_secs"


def evaluate_budgets(
    run: ReportRun,
    budgets: dict,
    *,
    client_only: bool = False,
    extra_secs: float = 0.0,
) -> BudgetEvaluation:
    """Evaluate a single run against total and per-job time budgets."""
    total_metric = select_total_budget_key(run.event, client_only=client_only)
    total_budget = float(budgets[total_metric])
    per_job: dict[str, float] = {
        k: float(v) for k, v in budgets.get("per_job_budgets", {}).items()
    }

    breaches: list[BudgetBreach] = []
    job_rows: list[tuple[str, float, float | None, bool]] = []

    for job in run.jobs:
        actual = job.seconds + extra_secs
        limit = per_job.get(job.budget_key)
        breached = limit is not None and actual > limit
        job_rows.append((job.label, actual, limit, breached))
        if breached and limit is not None:
            breaches.append(
                BudgetBreach(
                    target=f"job `{job.label}` ({job.budget_key})",
                    actual_secs=actual,
                    budget_secs=limit,
                )
            )

    total_actual = (run.elapsed + extra_secs) if run.elapsed is not None else None
    if total_actual is not None and total_actual > total_budget:
        breaches.append(
            BudgetBreach(
                target=f"total wall clock ({total_metric})",
                actual_secs=total_actual,
                budget_secs=total_budget,
            )
        )

    return BudgetEvaluation(
        total_metric=total_metric,
        total_actual_secs=total_actual,
        total_budget_secs=total_budget,
        job_rows=job_rows,
        breaches=breaches,
    )


def fmt_secs(seconds: float | None) -> str:
    if seconds is None:
        return "unavailable"
    if seconds >= 60:
        mins = int(seconds // 60)
        rem = seconds - mins * 60
        return f"{seconds:.1f}s ({mins}m {rem:04.1f}s)"
    return f"{seconds:.1f}s"


def chart(title: str, rows: list[tuple[str, float, float, str, bool]]) -> str:
    """Inline accessible SVG chart. Rows: (label, offset_s, dur_s, url, breached)."""
    if not rows:
        return f"<h3>{escape(title)}</h3><p>No completed timings.</p>"
    width, left, plot, row_height = 1100, 440, 560, 28
    extent = max((offset + dur for _, offset, dur, _, _ in rows), default=1.0) or 1.0
    height = 44 + len(rows) * row_height
    parts = [
        f'<h3>{escape(title)}</h3><svg role="img" aria-label="{escape(title)}" '
        f'viewBox="0 0 {width} {height}" xmlns="http://www.w3.org/2000/svg">',
        f"<title>{escape(title)} — seconds</title>",
    ]
    for i in range(6):
        x = left + plot * i / 5
        parts.append(
            f'<path d="M{x:.1f} 24V{height}" stroke="#d9e1e8"/>'
            f'<text x="{x:.1f}" y="17" text-anchor="middle">{extent * i / 5:.0f}s</text>'
        )
    for index, (label, offset, dur, url, breached) in enumerate(rows):
        y = 32 + index * row_height
        color = "#dc2626" if breached else "#2563eb"
        short = escape(label if len(label) <= 58 else label[:55] + "…")
        full = escape(f"{label}: {dur:.1f}s (start +{offset:.1f}s)")
        parts.append(
            f'<a href="{escape(url, quote=True)}"><title>{full}</title>'
            f'<text x="0" y="{y + 14}">{short}</text>'
            f'<rect x="{left + offset / extent * plot:.2f}" y="{y}" '
            f'width="{max(dur / extent * plot, 2.0):.2f}" height="19" rx="3" fill="{color}"/>'
            f'<text x="{left + (offset + dur) / extent * plot + 6:.2f}" y="{y + 14}">'
            f"{dur:.1f}s</text></a>"
        )
    parts.append("</svg>")
    return "".join(parts)


def render_markdown(
    runs: list[ReportRun],
    evaluations: dict[tuple[int, int], BudgetEvaluation] | None = None,
) -> str:
    evaluations = evaluations or {}
    lines = ["## CI timing & budget report", ""]
    elapsed_vals = [r.elapsed for r in runs if r.elapsed is not None]
    if len(runs) > 1 and elapsed_vals:
        lines.append(
            f"Collected {len(runs)} attempt(s); wall-clock median {fmt_secs(statistics.median(elapsed_vals))}, "
            f"max {fmt_secs(max(elapsed_vals))}."
        )
        lines.append("")

    for run in runs:
        ev = evaluations.get((run.run_id, run.attempt))
        status_tag = ""
        if ev is not None:
            status_tag = " — **BUDGET PASS**" if ev.passed else " — **BUDGET BREACH**"
        lines.append(
            f"### [{run.label}]({run.url}) (`{run.event}`, `{run.sha[:12]}`){status_tag}"
        )
        lines.append("")
        if ev is not None:
            lines.append(
                f"- Wall clock: **{fmt_secs(ev.total_actual_secs)}** "
                f"(budget `{ev.total_metric}`: {fmt_secs(ev.total_budget_secs)})"
            )
        else:
            lines.append(f"- Wall clock: **{fmt_secs(run.elapsed)}**")
        lines.append("")
        lines.append("| Job | Duration | Budget | Status |")
        lines.append("| --- | ---: | ---: | --- |")
        if ev is not None:
            for label, actual, limit, breached in ev.job_rows:
                budget_col = fmt_secs(limit) if limit is not None else "—"
                state_col = "BREACH" if breached else "PASS"
                lines.append(f"| `{label}` | {fmt_secs(actual)} | {budget_col} | {state_col} |")
        else:
            for job in run.jobs:
                lines.append(f"| `{job.label}` | {fmt_secs(job.seconds)} | — | {job.conclusion} |")
        lines.append("")
        if ev is not None and ev.breaches:
            lines.append("**Budget breaches:**")
            for breach in ev.breaches:
                lines.append(f"- {breach.format_line()}")
            lines.append("")
        if run.omitted:
            lines.append(f"- Omitted / in-progress: `{'; '.join(run.omitted)}`")
            lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def render_html(
    runs: list[ReportRun],
    repo: str,
    evaluations: dict[tuple[int, int], BudgetEvaluation] | None = None,
) -> str:
    evaluations = evaluations or {}
    generated = datetime.now(timezone.utc).isoformat(timespec="seconds")
    parts = [
        '<!doctype html><html lang="en"><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width,initial-scale=1">'
        "<title>Elysian Fields CI timing report</title><style>"
        "body{font:16px system-ui;color:#17243b;background:#f6f8fb;margin:2rem auto;max-width:1200px;padding:0 1rem}"
        "section{background:white;padding:1rem;margin:1rem 0;border:1px solid #dae1ea;border-radius:8px}"
        "svg{width:100%;min-width:760px}svg text{font:12px system-ui;fill:#17243b}"
        ".chart{overflow-x:auto}.breach{color:#b91c1c;font-weight:600}.pass{color:#15803d;font-weight:600}"
        "</style><body>",
        f"<h1>CI timing report — {escape(repo)}</h1><p>Generated {escape(generated)}</p>",
    ]
    for run in runs:
        ev = evaluations.get((run.run_id, run.attempt))
        breached_labels = (
            {label for label, _, _, breached in ev.job_rows if breached} if ev else set()
        )
        parts.append("<section>")
        parts.append(
            f'<h2><a href="{escape(run.url, quote=True)}">{escape(run.label)}</a> — {escape(run.title)}</h2>'
        )
        if ev is not None:
            cls = "pass" if ev.passed else "breach"
            label_text = "PASS" if ev.passed else f"BREACH ({len(ev.breaches)})"
            parts.append(
                f'<p>Wall clock: <strong>{escape(fmt_secs(ev.total_actual_secs))}</strong> '
                f'(budget <code>{escape(ev.total_metric)}</code>: {escape(fmt_secs(ev.total_budget_secs))}) — '
                f'<span class="{cls}">{label_text}</span></p>'
            )
        origin = run.start or min((j.start for j in run.jobs), default=None)
        job_rows = [
            (
                j.label,
                max(0.0, (j.start - origin).total_seconds()) if origin else 0.0,
                j.seconds,
                run.url,
                j.label in breached_labels,
            )
            for j in run.jobs
        ]
        parts.append('<div class="chart">' + chart("Jobs (timeline from run start)", job_rows) + "</div>")
        step_rows = [
            (s.label, 0.0, s.seconds, run.url, False)
            for s in sorted(run.steps, key=lambda x: x.seconds, reverse=True)[:25]
        ]
        parts.append('<div class="chart">' + chart("Longest steps", step_rows) + "</div>")
        parts.append("</section>")
    parts.append("</body></html>")
    return "\n".join(parts)


def positive(value: str) -> int:
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def main(argv: list[str] | None = None, api=gh_api) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for mode in ("history", "run"):
        sub = commands.add_parser(mode)
        sub.add_argument(
            "--repo",
            default=os.environ.get("GITHUB_REPOSITORY", "tyler274/ElysianFields"),
        )
        sub.add_argument("--output", type=Path, default=Path(f"ci-timings/{mode}.html"))
        sub.add_argument(
            "--summary",
            type=Path,
            help="Markdown destination (appended when matching GITHUB_STEP_SUMMARY).",
        )
        sub.add_argument(
            "--budgets",
            type=Path,
            default=DEFAULT_BUDGETS_PATH,
            help="Path to CI budgets JSON file.",
        )
        sub.add_argument(
            "--check-budgets",
            action="store_true",
            help="Exit non-zero if any job or total wall-clock duration exceeds budget.",
        )
        sub.add_argument(
            "--client-only",
            action="store_true",
            default=os.environ.get("CI_SCOPE_CLIENT_ONLY", "").lower() == "true",
            help="Evaluate push runs against client_only_merge_secs instead of merge_to_live_secs.",
        )
        sub.add_argument(
            "--test-extra-secs",
            type=float,
            default=float(os.environ.get("CI_BUDGET_TEST_EXTRA_SECS", "0") or "0"),
            help="Synthetic extra seconds added to timings for budget-breach verification.",
        )
        if mode == "history":
            sub.add_argument("--limit", type=positive, default=20)
            sub.add_argument("--workflow", default="ci.yml")
        else:
            run_id_env = os.environ.get("GITHUB_RUN_ID")
            attempt_env = os.environ.get("GITHUB_RUN_ATTEMPT")
            sub.add_argument(
                "--run-id",
                type=positive,
                default=int(run_id_env) if run_id_env else None,
            )
            sub.add_argument(
                "--attempt",
                type=positive,
                default=int(attempt_env) if attempt_env else None,
            )

    args = parser.parse_args(argv)
    if args.command == "run" and args.run_id is None:
        parser.error("run requires --run-id or GITHUB_RUN_ID")

    summary = args.summary or args.output.with_suffix(".md")
    try:
        budgets = load_budgets(args.budgets) if args.budgets.is_file() else None
        if args.check_budgets and budgets is None:
            raise FileNotFoundError(f"budgets file not found: {args.budgets}")

        collector = Collector(args.repo, api=api)
        runs = (
            collector.history(args.limit, args.workflow)
            if args.command == "history"
            else [collector.run(args.run_id, args.attempt)]
        )

        evaluations: dict[tuple[int, int], BudgetEvaluation] = {}
        all_breaches: list[BudgetBreach] = []
        if budgets is not None:
            for run in runs:
                ev = evaluate_budgets(
                    run,
                    budgets,
                    client_only=args.client_only,
                    extra_secs=args.test_extra_secs,
                )
                evaluations[(run.run_id, run.attempt)] = ev
                all_breaches.extend(ev.breaches)

        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(render_html(runs, args.repo, evaluations), encoding="utf-8")
        args.output.with_suffix(".json").write_text(
            json.dumps(collector.raw, indent=2) + "\n",
            encoding="utf-8",
        )
        summary.parent.mkdir(parents=True, exist_ok=True)
        write_mode = "a" if str(summary) == os.environ.get("GITHUB_STEP_SUMMARY") else "w"
        with summary.open(write_mode, encoding="utf-8") as handle:
            handle.write(render_markdown(runs, evaluations))
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f"ci_timings: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1

    print(f"HTML timing report: {args.output}\nMarkdown summary: {summary}")
    if args.check_budgets and all_breaches:
        print("CI budget check FAILED:", file=sys.stderr)
        for breach in all_breaches:
            print(f"  - {breach.format_line()}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
