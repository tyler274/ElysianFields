#!/usr/bin/env python3
"""Per-deployable, per-target CI scope classifier and result gate validator."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Iterable

SCOPE_KEYS: tuple[str, ...] = (
    "docs_only",
    "native",
    "wasm",
    "desktop",
    "mobile",
    "server",
    "client_only",
    "full",
)

ZERO_SHA = "0" * 40

DOCS_EXACT: frozenset[str] = frozenset(
    {
        "README.md",
        "AGENTS.md",
        "LICENSE",
        ".gitignore",
    }
)

DOCS_PREFIXES: tuple[str, ...] = (
    "docs/",
    "book/",
    "specs/",
    ".agents/",
)

# Files inside docs/ or root that directly configure CI enforcement or workspace builds.
INFRA_EXACT: frozenset[str] = frozenset(
    {
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "deny.toml",
        "flake.nix",
        "flake.lock",
        "docs/ci/budgets.json",
    }
)

INFRA_PREFIXES: tuple[str, ...] = (
    ".github/workflows/",
    ".github/actions/",
    "nix/",
)

CI_SCRIPT_EXACT: frozenset[str] = frozenset(
    {
        "scripts/ci_scope.py",
        "scripts/ci_timings.py",
        "scripts/ci_dep_audit.py",
        "scripts/dev-check.sh",
    }
)

CLIENT_WEB_PREFIXES: tuple[str, ...] = (
    "web/",
    "assets/web/",
    "crates/client-web/",
    "crates/elysian-web/",
)

CLIENT_UI_CRATES: frozenset[str] = frozenset(
    {
        "ef-ui-catalog",
        "ef-clinic-client",
        "ef-mobile-client",
        "elysian-bevy-probe",
        "m1-accessibility",
        "m1-streaming-perf",
        "m1-text-i18n",
        "m1-web-delivery",
    }
)

DESKTOP_PREFIXES: tuple[str, ...] = (
    "platform/desktop/",
    "crates/client-desktop/",
    "crates/elysian-desktop/",
    "crates/ef-mobile-desktop/",
)

MOBILE_PREFIXES: tuple[str, ...] = (
    "platform/android/",
    "platform/ios/",
    "crates/client-mobile/",
    "crates/elysian-mobile/",
)

MOBILE_CRATES: frozenset[str] = frozenset(
    {
        "m1-mobile-lifecycle",
    }
)

SERVER_PREFIXES: tuple[str, ...] = (
    "infra/",
    "crates/server/",
    "crates/elysian-server/",
)

HEADLESS_SERVER_CRATES: frozenset[str] = frozenset(
    {
        "ef-clinic-server",
        "ef-mobile-server",
        "ef-harness",
        "ef-boundaries",
        "ef-clinic-workflows",
        "ef-mobile-domain",
        "ef-connected-care",
        "ef-clinical-lib",
        "ef-encoder",
        "ef-patient-bank",
        "ef-extension",
        "ef-public-site",
        "ef-migration-rehearsal",
        "ef-pgwire",
    }
)

# Shared foundational crates whose edits must trigger full `--workspace` validation
# on PRs so every downstream consumer is checked.
SHARED_FANOUT_CRATES: frozenset[str] = frozenset(
    {
        "ef-core",
        "ef-ui-catalog",
        "ef-clinic-domain",
        "ef-mobile-domain",
        "elysian-smoke",
    }
)


def _crate_dir(path: str) -> str | None:
    if not path.startswith("crates/"):
        return None
    parts = path.split("/")
    return parts[1] if len(parts) >= 2 and parts[1] else None


def is_infra_path(path: str) -> bool:
    return (
        path in INFRA_EXACT
        or path in CI_SCRIPT_EXACT
        or path.startswith(INFRA_PREFIXES)
    )


def is_docs_only_path(path: str) -> bool:
    if is_infra_path(path):
        return False
    if path in DOCS_EXACT:
        return True
    if path.startswith(DOCS_PREFIXES):
        return True
    if "/" not in path and path.endswith(".md"):
        return True
    return False


def is_client_web_path(path: str) -> bool:
    if path.startswith(CLIENT_WEB_PREFIXES):
        return True
    crate = _crate_dir(path)
    if not crate:
        return False
    if crate in CLIENT_UI_CRATES:
        return True
    return (
        crate.endswith("-web")
        or crate.endswith("-wasm")
        or crate.endswith("-client")
        or crate.endswith("-ui")
        or crate.endswith("-ui-catalog")
    )


def is_desktop_path(path: str) -> bool:
    if path.startswith(DESKTOP_PREFIXES):
        return True
    crate = _crate_dir(path)
    return bool(crate and crate.endswith("-desktop"))


def is_mobile_path(path: str) -> bool:
    if path.startswith(MOBILE_PREFIXES):
        return True
    crate = _crate_dir(path)
    if not crate:
        return False
    return crate in MOBILE_CRATES or crate.endswith("-mobile")


def is_server_path(path: str) -> bool:
    if path.startswith(SERVER_PREFIXES):
        return True
    crate = _crate_dir(path)
    if not crate:
        return False
    if crate in HEADLESS_SERVER_CRATES:
        return True
    return (
        crate.endswith("-server")
        or crate.endswith("-backend")
        or crate.endswith("-api")
        or crate.endswith("-harness")
        or crate.endswith("-boundaries")
    )


def _full_scope() -> dict[str, bool]:
    return {
        "docs_only": False,
        "native": True,
        "wasm": True,
        "desktop": True,
        "mobile": True,
        "server": True,
        "client_only": False,
        "full": True,
    }


def classify_paths(paths: Iterable[str]) -> dict[str, bool]:
    """Classify changed repository paths into per-deployable, per-target CI flags."""
    path_list = [p.strip() for p in paths if p and p.strip()]
    if not path_list:
        return {
            "docs_only": True,
            "native": False,
            "wasm": False,
            "desktop": False,
            "mobile": False,
            "server": False,
            "client_only": False,
            "full": False,
        }

    native = False
    wasm = False
    desktop = False
    mobile = False
    server = False
    full = False
    all_docs = True

    for path in path_list:
        if is_docs_only_path(path):
            continue

        all_docs = False

        if is_infra_path(path):
            full = True
            native = True
            wasm = True
            desktop = True
            mobile = True
            server = True
            continue

        # Pure script unit tests (e.g. scripts/test_*.py) only require lightweight.
        if path.startswith("scripts/test_") and path.endswith(".py"):
            continue

        if is_server_path(path):
            server = True
            native = True
            continue

        if is_desktop_path(path):
            desktop = True
            native = True
            continue

        if is_mobile_path(path):
            mobile = True
            native = True
            continue

        if is_client_web_path(path):
            wasm = True
            continue

        if path.startswith("crates/"):
            # Shared workspace crates (e.g. elysian-smoke, ef-core, ef-clinic-domain)
            # must build and lint cleanly on both host native and wasm32 targets.
            native = True
            wasm = True
            continue

        # Any other unclassified code/config path fails closed to full validation.
        full = True
        native = True
        wasm = True
        desktop = True
        mobile = True
        server = True

    client_only = (
        wasm
        and not full
        and not native
        and not desktop
        and not mobile
        and not server
    )

    return {
        "docs_only": all_docs,
        "native": native,
        "wasm": wasm,
        "desktop": desktop,
        "mobile": mobile,
        "server": server,
        "client_only": client_only,
        "full": full,
    }


def wasm_build_crates(workspace_root: Path | None = None) -> list[str]:
    """Return workspace package names that target browser/Wasm builds.

    Excludes headless server, harness, boundary, desktop-only, and native-mobile-only
    crates so `cargo build --target wasm32-unknown-unknown` compiles only crates that
    actually participate in Wasm client delivery (while `cargo clippy --workspace
    --target wasm32-unknown-unknown` still checks cross-compilation across the tree).
    """
    root = workspace_root or Path(__file__).resolve().parent.parent
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    members = manifest.get("workspace", {}).get("members", [])

    selected: list[str] = []
    for member in members:
        probe_path = member.rstrip("/") + "/Cargo.toml"
        if is_server_path(probe_path) or is_desktop_path(probe_path) or is_mobile_path(probe_path):
            continue
        crate_toml = root / member / "Cargo.toml"
        if not crate_toml.is_file():
            continue
        pkg = tomllib.loads(crate_toml.read_text(encoding="utf-8")).get("package", {})
        name = pkg.get("name")
        if isinstance(name, str) and name:
            selected.append(name)
    return selected


def native_release_crates(workspace_root: Path | None = None) -> list[str]:
    """Return workspace package names that ship native server binaries.

    Limits `main`-branch native release builds to headless binary crates rather
    than ThinLTO-compiling Bevy client/prototype libraries that are shipped via
    `rust-wasm` or desktop/mobile client packaging.
    """
    root = workspace_root or Path(__file__).resolve().parent.parent
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    members = manifest.get("workspace", {}).get("members", [])

    selected: list[str] = []
    for member in members:
        probe_path = member.rstrip("/") + "/Cargo.toml"
        if not is_server_path(probe_path):
            continue
        crate_dir = root / member
        crate_toml = crate_dir / "Cargo.toml"
        if not crate_toml.is_file():
            continue
        parsed = tomllib.loads(crate_toml.read_text(encoding="utf-8"))
        has_bin = (crate_dir / "src" / "main.rs").is_file() or bool(parsed.get("bin"))
        if not has_bin:
            continue
        pkg = parsed.get("package", {})
        name = pkg.get("name")
        if isinstance(name, str) and name:
            selected.append(name)
    return selected


def desktop_workspace_crates(workspace_root: Path | None = None) -> list[str]:
    """Return workspace package names classified as native desktop GUI crates.

    Splitting native desktop GUI crates from `--workspace` Clippy/test invocations
    prevents Cargo feature unification from pulling desktop GPU/render Bevy
    features (`wgpu`, `naga`, `bevy_render`) into headless workspace crates.
    """
    root = workspace_root or Path(__file__).resolve().parent.parent
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    members = manifest.get("workspace", {}).get("members", [])

    selected: list[str] = []
    for member in members:
        probe_path = member.rstrip("/") + "/Cargo.toml"
        if not is_desktop_path(probe_path):
            continue
        crate_toml = root / member / "Cargo.toml"
        if not crate_toml.is_file():
            continue
        pkg = tomllib.loads(crate_toml.read_text(encoding="utf-8")).get("package", {})
        name = pkg.get("name")
        if isinstance(name, str) and name:
            selected.append(name)
    return selected


def parse_name_status_z(raw: bytes) -> list[str]:
    """Parse `git diff --name-status -z -M`, preserving old and new paths on renames."""
    if not raw:
        return []
    tokens = raw.split(b"\0")
    if tokens and tokens[-1] == b"":
        tokens.pop()

    paths: list[str] = []
    idx = 0
    while idx < len(tokens):
        status = tokens[idx].decode("utf-8", errors="surrogateescape")
        idx += 1
        if not status:
            continue
        code = status[0]
        if code in ("R", "C"):
            if idx + 1 >= len(tokens):
                raise ValueError(f"truncated rename/copy record: {status!r}")
            paths.append(tokens[idx].decode("utf-8", errors="surrogateescape"))
            paths.append(tokens[idx + 1].decode("utf-8", errors="surrogateescape"))
            idx += 2
        else:
            if idx >= len(tokens):
                raise ValueError(f"truncated path record: {status!r}")
            paths.append(tokens[idx].decode("utf-8", errors="surrogateescape"))
            idx += 1
    return paths


def _git(args: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", *args],
        cwd=str(cwd) if cwd else None,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def _git_diff_paths(base: str, head: str, cwd: Path | None = None) -> list[str] | None:
    if not base or not head or base == ZERO_SHA or head == ZERO_SHA:
        return None
    proc = _git(["diff", "--name-status", "-z", "-M", base, head], cwd=cwd)
    if proc.returncode != 0:
        return None
    try:
        return parse_name_status_z(proc.stdout)
    except ValueError:
        return None


def _git_merge_base(base: str, head: str, cwd: Path | None = None) -> str | None:
    if not base or not head or base == ZERO_SHA or head == ZERO_SHA:
        return None
    proc = _git(["merge-base", base, head], cwd=cwd)
    if proc.returncode != 0:
        return None
    result = proc.stdout.decode("utf-8", errors="replace").strip()
    return result or None


def _resolve_crate_package_name(
    crate_dir: str,
    workspace_root: Path | None,
) -> str | None:
    if workspace_root is None:
        return crate_dir
    crate_toml = workspace_root / "crates" / crate_dir / "Cargo.toml"
    if not crate_toml.is_file():
        return None
    pkg = tomllib.loads(crate_toml.read_text(encoding="utf-8")).get("package", {})
    name = pkg.get("name")
    return name if isinstance(name, str) and name else None


def compute_touched_crates(
    paths: Iterable[str] | None,
    *,
    event_name: str,
    workspace_root: Path | None = None,
) -> str:
    """Return Cargo `-p <crate>` flags for PRs touching isolated leaf crates.

    Returns an empty string (signaling full `--workspace` validation) on non-PR
    events (`main` pushes, `merge_group`, `workflow_dispatch`, `schedule`), root
    manifest/infra changes, shared-crate fanout changes, or deleted crates.
    Always includes `-p ef-boundaries` when any crate is targeted.
    """
    if event_name != "pull_request" or paths is None:
        return ""

    touched: set[str] = set()
    for raw_path in paths:
        path = raw_path.strip()
        if not path:
            continue
        if is_docs_only_path(path):
            continue
        if path.startswith("scripts/test_") and path.endswith(".py"):
            continue
        if is_infra_path(path):
            return ""
        crate_dir = _crate_dir(path)
        if crate_dir is None:
            return ""
        if crate_dir in SHARED_FANOUT_CRATES:
            return ""
        pkg_name = _resolve_crate_package_name(crate_dir, workspace_root)
        if pkg_name is None:
            return ""
        touched.add(pkg_name)

    if not touched:
        return ""

    if workspace_root is None or (workspace_root / "crates" / "ef-boundaries" / "Cargo.toml").is_file():
        touched.add("ef-boundaries")

    return " ".join(f"-p {name}" for name in sorted(touched))


def _event_diff_paths(
    event_name: str,
    event_payload: dict | None,
    *,
    github_sha: str = "",
    cwd: Path | None = None,
) -> tuple[list[str] | None, str]:
    payload = event_payload or {}
    if event_name == "pull_request":
        pr = payload.get("pull_request") or {}
        base_sha = ((pr.get("base") or {}).get("sha") or "").strip()
        head_sha = ((pr.get("head") or {}).get("sha") or github_sha or "").strip()
        merge_base = _git_merge_base(base_sha, head_sha, cwd=cwd)
        if merge_base:
            return (
                _git_diff_paths(merge_base, head_sha, cwd=cwd),
                f"pull_request diff {merge_base[:12]}..{head_sha[:12]}",
            )
    elif event_name == "merge_group":
        mg = payload.get("merge_group") or {}
        base_sha = (mg.get("base_sha") or "").strip()
        head_sha = (mg.get("head_sha") or github_sha or "").strip()
        return (
            _git_diff_paths(base_sha, head_sha, cwd=cwd),
            f"merge_group diff {base_sha[:12]}..{head_sha[:12]}",
        )
    elif event_name == "push":
        before = (payload.get("before") or "").strip()
        after = (payload.get("after") or github_sha or "").strip()
        if before == ZERO_SHA or not before:
            return None, "push has initial/empty base SHA; failing closed to full validation"
        return (
            _git_diff_paths(before, after, cwd=cwd),
            f"push diff {before[:12]}..{after[:12]}",
        )
    return None, f"uncertain comparison for event {event_name!r}; failing closed to full validation"


def select_scope(
    event_name: str,
    event_payload: dict | None,
    *,
    github_sha: str = "",
    cwd: Path | None = None,
) -> tuple[dict[str, bool], str]:
    """Select CI scope flags from GitHub event metadata and Git history."""
    if event_name == "workflow_dispatch":
        return _full_scope(), "manual workflow_dispatch runs full validation"

    if event_name == "schedule":
        return _full_scope(), "scheduled run runs full validation"

    paths, reason = _event_diff_paths(
        event_name,
        event_payload,
        github_sha=github_sha,
        cwd=cwd,
    )
    if paths is None:
        return _full_scope(), reason

    scope = classify_paths(paths)
    return scope, f"{reason} ({len(paths)} changed path entries)"


def _bool_str(value: bool) -> str:
    return "true" if value else "false"


def cmd_select(_args: argparse.Namespace) -> int:
    event_name = os.environ.get("GITHUB_EVENT_NAME", "").strip()
    event_path = os.environ.get("GITHUB_EVENT_PATH", "").strip()
    github_sha = os.environ.get("GITHUB_SHA", "").strip()

    payload: dict | None = None
    if event_path:
        path = Path(event_path)
        if path.is_file():
            try:
                payload = json.loads(path.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError):
                payload = None

    scope, reason = select_scope(event_name, payload, github_sha=github_sha)
    diff_paths, _ = _event_diff_paths(event_name, payload, github_sha=github_sha)
    repo_root = Path(__file__).resolve().parent.parent
    touched_crates = compute_touched_crates(
        diff_paths,
        event_name=event_name,
        workspace_root=repo_root,
    )

    lines = [f"{key}={_bool_str(scope[key])}" for key in SCOPE_KEYS]
    lines.append(f"touched_crates={touched_crates}")
    output_path = os.environ.get("GITHUB_OUTPUT", "").strip()
    if output_path:
        with Path(output_path).open("a", encoding="utf-8") as handle:
            handle.write("\n".join(lines) + "\n")

    summary_path = os.environ.get("GITHUB_STEP_SUMMARY", "").strip()
    if summary_path:
        with Path(summary_path).open("a", encoding="utf-8") as handle:
            handle.write(
                f"## CI scope\n\n- Reason: {reason}\n"
                + "".join(f"- `{k}`: `{_bool_str(scope[k])}`\n" for k in SCOPE_KEYS)
                + f"- `touched_crates`: `{touched_crates or '--workspace'}`\n"
            )

    for line in lines:
        print(line)
    return 0


def verify_result(
    needs: dict,
    *,
    event_name: str = "",
    ref_name: str = "",
) -> list[str]:
    """Return failure descriptions for any required job that did not succeed."""
    failures: list[str] = []

    def require_success(job: str) -> None:
        entry = needs.get(job)
        actual = entry.get("result") if isinstance(entry, dict) else None
        if actual != "success":
            failures.append(f"{job}: expected success, got {actual or 'missing'}")

    require_success("scope")
    require_success("lightweight")

    scope_entry = needs.get("scope") if isinstance(needs.get("scope"), dict) else {}
    outputs = (
        scope_entry.get("outputs")
        if isinstance(scope_entry.get("outputs"), dict)
        else {}
    )

    is_full = outputs.get("full") == "true"
    is_native = outputs.get("native") == "true"
    is_wasm = outputs.get("wasm") == "true"
    is_client_only = outputs.get("client_only") == "true"

    # PRs editing client/UI crates still run native unit tests; `main` client-only
    # merges skip `rust-native` to deploy static `/elysianfields/` assets without
    # rebuilding server binaries.
    if is_full or is_native or (event_name == "pull_request" and is_client_only):
        require_success("rust-native")

    if is_full or is_wasm or is_client_only:
        require_success("rust-wasm")

    if is_full or is_native or is_wasm or is_client_only:
        require_success("cargo-deny")

    if (
        event_name == "push"
        and ref_name == "refs/heads/main"
        and (is_full or is_wasm or is_client_only)
    ):
        require_success("deploy-client")

    return failures


def cmd_result(_args: argparse.Namespace) -> int:
    raw = os.environ.get("CI_NEEDS", "")
    if not raw.strip():
        print("CI result failed: CI_NEEDS is empty or unset", file=sys.stderr)
        return 1
    try:
        needs = json.loads(raw)
    except json.JSONDecodeError as exc:
        print(f"CI result failed: invalid CI_NEEDS JSON: {exc}", file=sys.stderr)
        return 1
    if not isinstance(needs, dict):
        print("CI result failed: CI_NEEDS must be a JSON object", file=sys.stderr)
        return 1

    event_name = os.environ.get("GITHUB_EVENT_NAME", "").strip()
    ref_name = os.environ.get("GITHUB_REF", "").strip()
    failures = verify_result(needs, event_name=event_name, ref_name=ref_name)
    if failures:
        for failure in failures:
            print(f"CI result failed: {failure}", file=sys.stderr)
        return 1

    print("All checks selected for this change passed.")
    return 0


def cmd_wasm_crates(_args: argparse.Namespace) -> int:
    crates = wasm_build_crates()
    if not crates:
        print("No wasm-targeted workspace crates found.", file=sys.stderr)
        return 1
    print(" ".join(f"-p {name}" for name in crates))
    return 0


def cmd_native_release_crates(_args: argparse.Namespace) -> int:
    crates = native_release_crates()
    if not crates:
        print("No native release binary crates found.", file=sys.stderr)
        return 1
    print(" ".join(f"-p {name}" for name in crates))
    return 0


def cmd_headless_workspace_excludes(_args: argparse.Namespace) -> int:
    crates = desktop_workspace_crates()
    if crates:
        print(" ".join(f"--exclude {name}" for name in crates))
    return 0


def cmd_desktop_crates(_args: argparse.Namespace) -> int:
    crates = desktop_workspace_crates()
    if crates:
        print(" ".join(f"-p {name}" for name in crates))
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="ci_scope.py",
        description="Classify changed files into CI scope flags and verify selected job results.",
    )
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("select", help="Classify changed files and write CI scope outputs.")
    sub.add_parser("result", help="Verify every selected CI job succeeded from CI_NEEDS.")
    sub.add_parser(
        "wasm-crates",
        help="Emit Cargo `-p <crate>` arguments for Wasm-targeted workspace crates.",
    )
    sub.add_parser(
        "native-release-crates",
        help="Emit Cargo `-p <crate>` arguments for headless native release binaries.",
    )
    sub.add_parser(
        "headless-workspace-excludes",
        help="Emit Cargo `--exclude <crate>` arguments for native desktop GUI crates.",
    )
    sub.add_parser(
        "desktop-crates",
        help="Emit Cargo `-p <crate>` arguments for native desktop GUI crates.",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.command == "select":
        return cmd_select(args)
    if args.command == "result":
        return cmd_result(args)
    if args.command == "wasm-crates":
        return cmd_wasm_crates(args)
    if args.command == "native-release-crates":
        return cmd_native_release_crates(args)
    if args.command == "headless-workspace-excludes":
        return cmd_headless_workspace_excludes(args)
    if args.command == "desktop-crates":
        return cmd_desktop_crates(args)
    parser.error(f"unknown command: {args.command}")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
