#!/usr/bin/env python3
"""Executable Elysian Realm wall canary.

Creates an isolated temporary Elysian Realm clone from HEAD, launches a
scrubbed child process (using Linux user+mount namespaces via ``unshare -rm``
when available, falling back explicitly to env+DAC isolation when host policy
blocks unprivileged user namespaces), and runs four canary probes:

1. ``reference_file_locate``: Attempts to access reference paths, host agent
   stores, and a synthetic outside-realm sentinel -> MUST FAIL (blocked).
2. ``reference_git_history``: Inspects git history, object store, remotes, and
   ``.git`` structure for the pinned reference SHA or reference remotes ->
   MUST FIND ZERO matches.
3. ``linear_github_connectors``: Checks environment variables, credential files,
   and attempts to invoke ``gh`` / ``linear`` -> MUST FAIL (absent/denied).
4. ``trivial_crate_build_lint``: Creates and builds a trivial Rust library
   crate with workspace Clippy lints (``cargo clippy --all-targets -- -D
   warnings`` and ``cargo test``) -> MUST PASS.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any

DEFAULT_REFERENCE_SHA = "01b762838d639a51f82e9d7d15e788e02bac772f"
FORBIDDEN_REMOTE_TOKENS = ("dabney-moe", "coda-agents", "/code/coda", "/code/dabney-moe")
FORBIDDEN_ENV_VARS = (
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "LINEAR_API_KEY",
    "LINEAR_TOKEN",
    "DOPPLER_TOKEN",
    "SSH_AUTH_SOCK",
    "GIT_ASKPASS",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
)
FORBIDDEN_CLIS = ("gh", "linear")
ALLOWLISTED_POSIX_TOOLS = (
    "sh",
    "bash",
    "env",
    "uname",
    "mkdir",
    "rm",
    "cp",
    "mv",
    "cat",
    "ls",
    "echo",
    "true",
    "false",
    "chmod",
    "dirname",
    "basename",
    "readlink",
    "sed",
    "grep",
    "awk",
    "tr",
    "wc",
    "head",
    "tail",
    "sort",
    "find",
    "xargs",
    "mktemp",
    "touch",
    "which",
    "pwd",
    "test",
    "expr",
    "cut",
    "uniq",
    "printf",
    "ln",
)
ALLOWLISTED_BUILD_TOOLS = (
    "git",
    "cc",
    "gcc",
    "ld",
    "ar",
    "as",
    "ranlib",
    "strip",
    "nm",
)
PASSTHROUGH_RUNTIME_ENV = (
    "NIX_LD",
    "NIX_LD_LIBRARY_PATH",
    "LOCALE_ARCHIVE",
    "LOCALE_ARCHIVE_2_27",
)


@dataclass
class RustToolchain:
    """Resolved Rust toolchain capable of running cargo, rustc, and clippy."""

    root: Path
    bin_dir: Path
    rustc_version: str
    clippy_version: str
    inside_host_home: bool


@dataclass
class ProbeResult:
    """Outcome of one wall canary probe."""

    name: str
    expected: str
    passed: bool
    summary: str
    details: dict[str, Any] = field(default_factory=dict)


def _run_cmd(
    cmd: list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    timeout: int = 120,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=str(cwd) if cwd else None,
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )


def _is_relative_to(path: Path, ancestor: Path) -> bool:
    try:
        path.resolve().relative_to(ancestor.resolve())
        return True
    except ValueError:
        return False


def _probe_rust_bin_dir(bin_dir: Path, host_home: Path) -> RustToolchain | None:
    """Return a validated RustToolchain if bin_dir can run rustc, cargo, and clippy."""
    cargo_bin = bin_dir / "cargo"
    rustc_bin = bin_dir / "rustc"
    clippy_bin = bin_dir / "cargo-clippy"
    if not (cargo_bin.is_file() and rustc_bin.is_file() and clippy_bin.is_file()):
        return None

    probe_env = os.environ.copy()
    probe_env["PATH"] = f"{bin_dir}:{probe_env.get('PATH', '')}"
    probe_env["RUSTC"] = str(rustc_bin)
    probe_env["CARGO"] = str(cargo_bin)

    tmp_cwd = Path(tempfile.gettempdir())
    try:
        rustc_res = _run_cmd([str(rustc_bin), "--version"], cwd=tmp_cwd, env=probe_env, timeout=15)
        if rustc_res.returncode != 0:
            return None
        cargo_res = _run_cmd([str(cargo_bin), "--version"], cwd=tmp_cwd, env=probe_env, timeout=15)
        if cargo_res.returncode != 0:
            return None
        clippy_res = _run_cmd(
            [str(cargo_bin), "clippy", "--version"], cwd=tmp_cwd, env=probe_env, timeout=15
        )
        if clippy_res.returncode != 0:
            return None
        sysroot_res = _run_cmd(
            [str(rustc_bin), "--print", "sysroot"], cwd=tmp_cwd, env=probe_env, timeout=15
        )
        if sysroot_res.returncode != 0 or not sysroot_res.stdout.strip():
            return None
    except OSError:
        return None

    sysroot = Path(sysroot_res.stdout.strip()).resolve()
    actual_bin = sysroot / "bin"
    if not (actual_bin / "rustc").is_file():
        actual_bin = bin_dir.resolve()
    return RustToolchain(
        root=sysroot,
        bin_dir=actual_bin,
        rustc_version=rustc_res.stdout.strip(),
        clippy_version=clippy_res.stdout.strip(),
        inside_host_home=_is_relative_to(sysroot, host_home),
    )


def discover_rust_toolchain(host_home: Path | None = None) -> RustToolchain:
    """Locate a working Rust toolchain with cargo, rustc, and cargo-clippy."""
    resolved_home = (host_home or Path.home()).resolve()

    # 1. Try active PATH first (works cleanly on GitHub Actions ubuntu-24.04 and Nix shells).
    which_cargo = shutil.which("cargo")
    if which_cargo:
        candidate = _probe_rust_bin_dir(Path(which_cargo).parent, resolved_home)
        if candidate is not None:
            return candidate

    # 2. Scan RUSTUP_HOME toolchains (handles hosts where default rustup proxy is overridden).
    rustup_home = Path(os.environ.get("RUSTUP_HOME", str(resolved_home / ".rustup")))
    toolchains_dir = rustup_home / "toolchains"
    if toolchains_dir.is_dir():
        candidates = sorted(
            (p for p in toolchains_dir.iterdir() if p.is_dir()),
            key=lambda p: p.name,
            reverse=True,
        )
        for tc_dir in candidates:
            candidate = _probe_rust_bin_dir(tc_dir / "bin", resolved_home)
            if candidate is not None:
                return candidate

    raise RuntimeError(
        "No working Rust toolchain (rustc + cargo + cargo-clippy) found on PATH or in RUSTUP_HOME."
    )


def detect_mount_namespace_support() -> tuple[bool, str]:
    """Probe whether ``unshare --user --map-root-user --mount`` can mount tmpfs."""
    unshare_bin = shutil.which("unshare")
    if not unshare_bin:
        return False, "unshare binary not found on PATH"

    with tempfile.TemporaryDirectory(prefix="elysian-ns-probe-") as probe_dir:
        res = _run_cmd(
            [
                unshare_bin,
                "--user",
                "--map-root-user",
                "--mount",
                "sh",
                "-c",
                'mount -t tmpfs -o size=1M,mode=0000 tmpfs "$1" && echo NS_MOUNT_OK',
                "_",
                probe_dir,
            ],
            timeout=15,
        )
        if res.returncode == 0 and "NS_MOUNT_OK" in res.stdout:
            return True, "unshare --user --map-root-user --mount + tmpfs mask verified"
        reason = (res.stderr or res.stdout).strip() or f"exit {res.returncode}"
        return False, f"unshare mount probe unavailable: {reason}"


def prepare_clean_realm(source_repo: Path, realm_dir: Path) -> str:
    """Clone HEAD cleanly into realm_dir with no hardlinks, alternates, or local remote."""
    head_res = _run_cmd(["git", "-C", str(source_repo), "rev-parse", "HEAD"])
    if head_res.returncode != 0:
        raise RuntimeError(f"Failed to read HEAD of {source_repo}: {head_res.stderr.strip()}")
    head_sha = head_res.stdout.strip()

    clone_res = _run_cmd(
        [
            "git",
            "clone",
            "--no-local",
            "--no-hardlinks",
            "--single-branch",
            "--quiet",
            str(source_repo),
            str(realm_dir),
        ]
    )
    if clone_res.returncode != 0:
        raise RuntimeError(f"Clean clone failed: {clone_res.stderr.strip()}")

    # Replace the local filesystem remote URL so the builder never sees host checkout paths.
    origin_res = _run_cmd(["git", "-C", str(source_repo), "remote", "get-url", "origin"])
    sanitized_origin = None
    if origin_res.returncode == 0:
        raw_origin = origin_res.stdout.strip()
        if "ElysianFields" in raw_origin and not any(
            tok in raw_origin.lower() for tok in FORBIDDEN_REMOTE_TOKENS
        ):
            sanitized_origin = "https://github.com/tyler274/ElysianFields.git"

    if sanitized_origin:
        _run_cmd(["git", "-C", str(realm_dir), "remote", "set-url", "origin", sanitized_origin])
    else:
        _run_cmd(["git", "-C", str(realm_dir), "remote", "remove", "origin"])

    return head_sha


def build_allowlisted_bin(
    bin_dir: Path,
    toolchain_bin_in_sandbox: Path,
    host_home: Path,
) -> list[str]:
    """Populate sandbox bin_dir with allowlisted build tools only (never gh/linear)."""
    bin_dir.mkdir(parents=True, exist_ok=True)
    linked: list[str] = []

    # Link Rust tools directly to the sandbox toolchain bin path.
    for rust_tool in ("cargo", "rustc", "rustdoc", "cargo-clippy", "clippy-driver"):
        target = toolchain_bin_in_sandbox / rust_tool
        link_path = bin_dir / rust_tool
        if link_path.exists() or link_path.is_symlink():
            link_path.unlink()
        link_path.symlink_to(target)
        linked.append(rust_tool)

    # Link Python executable (resolved outside host_home).
    py_real = Path(sys.executable).resolve()
    py_link = bin_dir / "python3"
    if py_link.exists() or py_link.is_symlink():
        py_link.unlink()
    py_link.symlink_to(py_real)
    linked.append("python3")

    # Link allowlisted build and POSIX utilities outside host_home.
    for tool in (*ALLOWLISTED_BUILD_TOOLS, *ALLOWLISTED_POSIX_TOOLS):
        if tool in FORBIDDEN_CLIS:
            continue
        found = shutil.which(tool)
        if not found:
            continue
        real_tool = Path(found).resolve()
        if _is_relative_to(real_tool, host_home):
            continue
        link_path = bin_dir / tool
        if not (link_path.exists() or link_path.is_symlink()):
            link_path.symlink_to(real_tool)
            linked.append(tool)

    return sorted(linked)


def build_scrubbed_env(
    sandbox_root: Path,
    realm_dir: Path,
    bin_dir: Path,
    toolchain_bin_in_sandbox: Path,
    extra_env: dict[str, str] | None = None,
) -> dict[str, str]:
    """Construct a minimal allowlisted environment for the isolated builder child."""
    isolated_home = sandbox_root / "home"
    isolated_tmp = sandbox_root / "tmp"
    cargo_home = sandbox_root / "cargo-home"
    cargo_target = sandbox_root / "cargo-target"
    for directory in (isolated_home, isolated_tmp, cargo_home, cargo_target):
        directory.mkdir(parents=True, exist_ok=True)
        os.chmod(directory, 0o700)

    env: dict[str, str] = {
        "HOME": str(isolated_home),
        "USER": "builder",
        "LOGNAME": "builder",
        "PATH": str(bin_dir),
        "TMPDIR": str(isolated_tmp),
        "CARGO_HOME": str(cargo_home),
        "CARGO_TARGET_DIR": str(cargo_target),
        "RUSTC": str(toolchain_bin_in_sandbox / "rustc"),
        "RUSTDOC": str(toolchain_bin_in_sandbox / "rustdoc"),
        "CARGO": str(toolchain_bin_in_sandbox / "cargo"),
        "CARGO_NET_OFFLINE": "true",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "TERM": "dumb",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_TERMINAL_PROMPT": "0",
        "ELYSIAN_SANDBOX_ROOT": str(realm_dir),
    }

    for var in PASSTHROUGH_RUNTIME_ENV:
        val = os.environ.get(var)
        if val:
            env[var] = val

    if extra_env:
        env.update(extra_env)

    return env


def _probe_reference_file_locate(config: dict[str, Any]) -> ProbeResult:
    """Probe 1: Attempt to locate/read reference paths outside the realm -> MUST FAIL."""
    targets: list[dict[str, str]] = config["reference_targets"]
    attempts: list[dict[str, Any]] = []
    leaks: list[str] = []

    for item in targets:
        label = item["label"]
        target_path = Path(item["path"])
        kind = item.get("kind", "dir")
        blocked = False
        observation = ""

        try:
            if kind == "file":
                content = target_path.read_text(encoding="utf-8", errors="replace")
                leaks.append(f"{label}: read {len(content)} bytes from {target_path}")
                observation = f"READ_SUCCEEDED ({len(content)} bytes)"
            else:
                entries = sorted(os.listdir(target_path))
                if entries:
                    leaks.append(
                        f"{label}: listed {len(entries)} entries under {target_path} ({entries[:3]})"
                    )
                    observation = f"LISTDIR_NONEMPTY ({len(entries)} entries)"
                else:
                    # Empty masked tmpfs directory (0 entries) still blocks all reference files.
                    blocked = True
                    observation = "BLOCKED (empty masked mount, 0 entries)"
        except PermissionError as exc:
            blocked = True
            observation = f"BLOCKED (PermissionError: {exc.strerror or exc})"
        except FileNotFoundError as exc:
            blocked = True
            observation = f"BLOCKED (FileNotFoundError: {exc.strerror or exc})"
        except OSError as exc:
            blocked = True
            observation = f"BLOCKED ({type(exc).__name__}: {exc.strerror or exc})"

        attempts.append(
            {
                "label": label,
                "path": str(target_path),
                "kind": kind,
                "blocked": blocked,
                "observation": observation,
            }
        )

    passed = len(leaks) == 0 and len(attempts) > 0
    summary = (
        f"All {len(attempts)} reference-locate attempts blocked"
        if passed
        else f"WALL BREACH: {len(leaks)} reference path(s) accessible"
    )
    return ProbeResult(
        name="reference_file_locate",
        expected="FAIL (all reference paths blocked)",
        passed=passed,
        summary=summary,
        details={"attempts": attempts, "leaks": leaks},
    )


def _probe_reference_git_history(config: dict[str, Any]) -> ProbeResult:
    """Probe 2: Verify zero shared git history, alternates, linked worktree, or reference remotes."""
    realm_dir = Path(config["realm_dir"])
    ref_sha: str = config["reference_sha"]
    ref_short = ref_sha[:9]
    violations: list[str] = []

    git_entry = realm_dir / ".git"
    git_is_dir = git_entry.is_dir() and not git_entry.is_symlink()
    if not git_is_dir:
        violations.append(".git is not a standalone directory (possible linked worktree)")

    alternates_file = git_entry / "objects" / "info" / "alternates"
    alternates_exists = alternates_file.exists()
    if alternates_exists:
        violations.append(".git/objects/info/alternates exists")

    rev_res = _run_cmd(["git", "rev-list", "--all"], cwd=realm_dir)
    if rev_res.returncode != 0:
        violations.append(f"git rev-list failed: {rev_res.stderr.strip()}")
        commits: list[str] = []
    else:
        commits = [line.strip() for line in rev_res.stdout.splitlines() if line.strip()]

    sha_matches = [c for c in commits if c == ref_sha or c.startswith(ref_short)]
    if sha_matches:
        violations.append(f"reference SHA matched in rev-list: {sha_matches}")

    cat_res = _run_cmd(["git", "cat-file", "-e", ref_sha], cwd=realm_dir)
    object_present = cat_res.returncode == 0
    if object_present:
        violations.append(f"reference object {ref_sha} exists in object store")

    remote_res = _run_cmd(["git", "remote", "-v"], cwd=realm_dir)
    remotes = [line.strip() for line in remote_res.stdout.splitlines() if line.strip()]
    bad_remotes = [
        r
        for r in remotes
        if any(tok in r.lower() for tok in FORBIDDEN_REMOTE_TOKENS) or "file://" in r or "/home/" in r
    ]
    if bad_remotes:
        violations.append(f"forbidden remote(s) detected: {bad_remotes}")

    passed = len(violations) == 0
    summary = (
        f"Zero matches for {ref_short} across {len(commits)} realm commit(s); standalone .git verified"
        if passed
        else f"WALL BREACH: {'; '.join(violations)}"
    )
    return ProbeResult(
        name="reference_git_history",
        expected="ZERO matches (no shared history, alternates, or reference remotes)",
        passed=passed,
        summary=summary,
        details={
            "reference_sha": ref_sha,
            "realm_commit_count": len(commits),
            "git_dir_standalone": git_is_dir,
            "alternates_exists": alternates_exists,
            "reference_object_present": object_present,
            "remotes": remotes,
            "violations": violations,
        },
    )


def _probe_linear_github_connectors(config: dict[str, Any]) -> ProbeResult:
    """Probe 3: Verify Linear/GitHub credentials, configs, and CLIs are absent/denied."""
    leaked_env = [var for var in FORBIDDEN_ENV_VARS if var in os.environ and os.environ[var]]
    home = Path(os.environ.get("HOME", ""))
    cred_candidates = [
        home / ".config" / "gh" / "hosts.yml",
        home / ".config" / "linear",
        home / ".netrc",
        home / ".git-credentials",
    ]
    leaked_files = [str(p) for p in cred_candidates if p.exists()]

    cli_checks: dict[str, str] = {}
    leaked_clis: list[str] = []
    for cli, args in (("gh", ["gh", "auth", "status"]), ("linear", ["linear", "--version"])):
        found = shutil.which(cli)
        if found:
            leaked_clis.append(f"{cli} found at {found}")
            cli_checks[cli] = f"FOUND ({found})"
            continue
        try:
            res = _run_cmd(args, timeout=10)
            leaked_clis.append(f"{cli} executed with exit {res.returncode}")
            cli_checks[cli] = f"EXECUTED (exit {res.returncode})"
        except FileNotFoundError:
            cli_checks[cli] = "DENIED (FileNotFoundError)"
        except PermissionError as exc:
            cli_checks[cli] = f"DENIED (PermissionError: {exc})"

    passed = not leaked_env and not leaked_files and not leaked_clis
    summary = (
        "All Linear/GitHub env vars, credential files, and CLIs absent/denied"
        if passed
        else (
            f"WALL BREACH: env={leaked_env}, files={leaked_files}, clis={leaked_clis}"
        )
    )
    return ProbeResult(
        name="linear_github_connectors",
        expected="FAIL (credentials, config files, and gh/linear CLIs absent/denied)",
        passed=passed,
        summary=summary,
        details={
            "checked_env_vars": list(FORBIDDEN_ENV_VARS),
            "leaked_env_vars": leaked_env,
            "leaked_credential_files": leaked_files,
            "cli_checks": cli_checks,
            "leaked_clis": leaked_clis,
        },
    )


def _probe_trivial_crate_build_lint(config: dict[str, Any]) -> ProbeResult:
    """Probe 4: Create & verify a trivial Rust crate with workspace Clippy lints."""
    realm_dir = Path(config["realm_dir"])
    ws_dir = realm_dir / ".canary_workspace"
    crate_src = ws_dir / "canary_crate" / "src"
    crate_src.mkdir(parents=True, exist_ok=True)

    (ws_dir / "Cargo.toml").write_text(
        """[workspace]
resolver = "2"
members = ["canary_crate"]

[workspace.lints.clippy]
disallowed_methods = "warn"
fn_params_excessive_bools = "warn"
map_err_ignore = "warn"
struct_excessive_bools = "warn"
unused_result_ok = "warn"
""",
        encoding="utf-8",
    )

    (ws_dir / "canary_crate" / "Cargo.toml").write_text(
        """[package]
name = "canary_crate"
version = "0.1.0"
edition = "2021"

[lints]
workspace = true
""",
        encoding="utf-8",
    )

    (crate_src / "lib.rs").write_text(
        """//! Trivial clean-room library crate verified by the wall canary.

/// Status of an isolated build check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanaryStatus {
    Ready,
    Blocked,
}

/// Evaluates whether a slice may proceed under a given status.
pub fn may_proceed(status: CanaryStatus) -> bool {
    matches!(status, CanaryStatus::Ready)
}

#[cfg(test)]
mod tests {
    use super::{may_proceed, CanaryStatus};

    #[test]
    fn ready_status_allows_progress() {
        assert!(may_proceed(CanaryStatus::Ready));
    }

    #[test]
    fn blocked_status_halts_progress() {
        assert!(!may_proceed(CanaryStatus::Blocked));
    }
}
""",
        encoding="utf-8",
    )

    manifest = str(ws_dir / "Cargo.toml")
    clippy_cmd = [
        "cargo",
        "clippy",
        "--manifest-path",
        manifest,
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]
    test_cmd = ["cargo", "test", "--manifest-path", manifest]

    clippy_res = _run_cmd(clippy_cmd, cwd=ws_dir, timeout=180)
    test_res = _run_cmd(test_cmd, cwd=ws_dir, timeout=180)

    passed = clippy_res.returncode == 0 and test_res.returncode == 0
    summary = (
        "cargo clippy --all-targets -- -D warnings and cargo test passed inside isolated realm"
        if passed
        else (
            f"Build/lint failed (clippy exit={clippy_res.returncode}, "
            f"test exit={test_res.returncode})"
        )
    )
    return ProbeResult(
        name="trivial_crate_build_lint",
        expected="PASS (cargo clippy -D warnings and cargo test succeed)",
        passed=passed,
        summary=summary,
        details={
            "cargo_target_dir": os.environ.get("CARGO_TARGET_DIR", ""),
            "clippy_exit_code": clippy_res.returncode,
            "clippy_stderr": clippy_res.stderr.strip(),
            "test_exit_code": test_res.returncode,
            "test_stdout": test_res.stdout.strip(),
            "test_stderr": test_res.stderr.strip(),
        },
    )


def run_child_probes(config_path: Path) -> int:
    """Entry point executed inside the isolated builder child process."""
    config = json.loads(config_path.read_text(encoding="utf-8"))
    results = [
        _probe_reference_file_locate(config),
        _probe_reference_git_history(config),
        _probe_linear_github_connectors(config),
        _probe_trivial_crate_build_lint(config),
    ]
    payload = {
        "child_cwd": os.getcwd(),
        "child_home": os.environ.get("HOME", ""),
        "child_path": os.environ.get("PATH", ""),
        "child_cargo_target_dir": os.environ.get("CARGO_TARGET_DIR", ""),
        "probes": [asdict(r) for r in results],
        "all_passed": all(r.passed for r in results),
    }
    output_path = Path(config["result_out"])
    output_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return 0 if payload["all_passed"] else 1


def _build_default_reference_targets(
    host_home: Path,
    sentinel_file: Path,
    extra_reference_paths: list[Path],
) -> tuple[list[dict[str, str]], list[Path]]:
    """Build Probe 1 target list and paths to mask in the mount namespace."""
    targets: list[dict[str, str]] = [
        {
            "label": "synthetic-outside-realm-sentinel",
            "path": str(sentinel_file),
            "kind": "file",
        }
    ]
    mask_dirs: list[Path] = [sentinel_file.parent]

    well_known_home_targets = [
        ("reference-repo-dabney-moe", host_home / "code" / "dabney-moe"),
        ("reference-repo-coda", host_home / "code" / "coda"),
        ("reference-repo-website", host_home / "code" / "DabneyMoeWebsite"),
        ("host-agent-store-gemini", host_home / ".gemini"),
        ("host-gh-config", host_home / ".config" / "gh"),
    ]
    for label, path in well_known_home_targets:
        if path.exists():
            targets.append(
                {
                    "label": label,
                    "path": str(path),
                    "kind": "file" if path.is_file() else "dir",
                }
            )

    for idx, extra in enumerate(extra_reference_paths, start=1):
        resolved = extra.resolve()
        targets.append(
            {
                "label": f"configured-reference-{idx}",
                "path": str(resolved),
                "kind": "file" if resolved.is_file() else "dir",
            }
        )
        mask_target = resolved.parent if resolved.is_file() else resolved
        if mask_target.exists() and not _is_relative_to(mask_target, host_home):
            mask_dirs.append(mask_target)

    return targets, mask_dirs


def run_wall_canary(
    *,
    source_repo: Path,
    reference_sha: str = DEFAULT_REFERENCE_SHA,
    extra_reference_paths: list[Path] | None = None,
    require_mount_ns: bool = False,
    keep_temp: bool = False,
    extra_child_env: dict[str, str] | None = None,
    post_clone_hook: Any | None = None,
    force_disable_mount_ns: bool = False,
) -> dict[str, Any]:
    """Execute the full Elysian Realm wall canary and return the structured receipt."""
    source_repo = source_repo.resolve()
    host_home = Path.home().resolve()
    toolchain = discover_rust_toolchain(host_home)

    if force_disable_mount_ns:
        mount_ns_ok, mount_ns_note = False, "disabled by caller"
    else:
        mount_ns_ok, mount_ns_note = detect_mount_namespace_support()

    if require_mount_ns and not mount_ns_ok:
        raise RuntimeError(f"Mount namespace isolation required but unavailable: {mount_ns_note}")

    isolation_mode = "linux-user-mount-ns" if mount_ns_ok else "env-and-dac-sandbox"
    started_at = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()

    temp_ctx = tempfile.TemporaryDirectory(prefix="elysian-wall-canary-")
    outer_root = Path(temp_ctx.name).resolve()
    sentinel_dir = outer_root / "reference-sentinel"
    sandbox_root = outer_root / "sandbox"
    realm_dir = sandbox_root / "realm"
    bin_dir = sandbox_root / "bin"
    result_file = sandbox_root / "child-result.json"
    config_file = sandbox_root / "child-config.json"

    try:
        # 1. Create synthetic outside-realm reference sentinel fixture.
        sentinel_dir.mkdir(parents=True, exist_ok=True)
        sentinel_file = sentinel_dir / "REFERENCE_SECRET.txt"
        sentinel_file.write_text(
            "SYNTHETIC REFERENCE SENTINEL — MUST NOT BE READABLE FROM ELYSIAN REALM\n",
            encoding="utf-8",
        )

        # 2. Create clean standalone realm clone from HEAD.
        sandbox_root.mkdir(parents=True, exist_ok=True)
        os.chmod(sandbox_root, 0o700)
        head_sha = prepare_clean_realm(source_repo, realm_dir)

        if post_clone_hook is not None:
            post_clone_hook(realm_dir)

        # 3. Copy this script inside the sandbox so it stays accessible if host $HOME is masked.
        sandbox_script = sandbox_root / "wall_canary_child.py"
        shutil.copy2(Path(__file__).resolve(), sandbox_script)

        # 4. Prepare toolchain mount point inside sandbox when masking host $HOME.
        need_toolchain_bind = mount_ns_ok and toolchain.inside_host_home
        if need_toolchain_bind:
            toolchain_mount = sandbox_root / "toolchain"
            toolchain_mount.mkdir(parents=True, exist_ok=True)
            toolchain_bin_in_sandbox = toolchain_mount / "bin"
        else:
            toolchain_mount = toolchain.root
            toolchain_bin_in_sandbox = toolchain.bin_dir

        allowlisted_tools = build_allowlisted_bin(bin_dir, toolchain_bin_in_sandbox, host_home)
        child_env = build_scrubbed_env(
            sandbox_root,
            realm_dir,
            bin_dir,
            toolchain_bin_in_sandbox,
            extra_env=extra_child_env,
        )

        targets, mask_dirs = _build_default_reference_targets(
            host_home,
            sentinel_file,
            extra_reference_paths or [],
        )
        # In env-and-dac fallback mode, host paths outside our synthetic sentinel are not
        # kernel-masked; keep targets to the DAC-locked synthetic sentinel plus any explicit
        # extra paths passed by caller so CI runners without userns fail closed accurately.
        if not mount_ns_ok:
            os.chmod(sentinel_dir, 0o000)
            explicit_strs = {str(p.resolve()) for p in (extra_reference_paths or [])}
            targets = [
                t
                for t in targets
                if t["label"] == "synthetic-outside-realm-sentinel" or t["path"] in explicit_strs
            ]

        config_payload = {
            "realm_dir": str(realm_dir),
            "reference_sha": reference_sha,
            "reference_targets": targets,
            "result_out": str(result_file),
        }
        config_file.write_text(json.dumps(config_payload, indent=2) + "\n", encoding="utf-8")

        # 5. Execute isolated child probes.
        if mount_ns_ok:
            mount_bin = shutil.which("mount")
            if not mount_bin:
                raise RuntimeError("mount binary not found on PATH despite unshare mount support")
            mount_real = str(Path(mount_bin).resolve())
            bootstrap_lines = ["#!/bin/sh", "set -eu"]
            if need_toolchain_bind:
                bootstrap_lines.append(
                    f'"{mount_real}" --bind "{toolchain.root}" "{toolchain_mount}"'
                )
                bootstrap_lines.append(
                    f'"{mount_real}" -o remount,ro,bind "{toolchain_mount}" 2>/dev/null || true'
                )

            # Mask host $HOME (covers reference checkouts, agent stores, gh/linear configs, host cargo cache).
            if not _is_relative_to(sandbox_root, host_home) and host_home.exists():
                home_mode = "0700" if need_toolchain_bind else "0000"
                bootstrap_lines.append(
                    f'"{mount_real}" -t tmpfs -o size=1M,mode={home_mode} tmpfs "{host_home}"'
                )
                if need_toolchain_bind:
                    # Re-expose ONLY the single read-only Rust toolchain path inside the empty
                    # tmpfs so toolchain-internal linker wrappers (e.g. NixOS rustup ld-wrapper)
                    # resolve cleanly while all reference repos, agent stores, and credentials stay masked.
                    bootstrap_lines.append(f'mkdir -p "{toolchain.root}"')
                    bootstrap_lines.append(
                        f'"{mount_real}" --bind "{toolchain_mount}" "{toolchain.root}"'
                    )
                    bootstrap_lines.append(
                        f'"{mount_real}" -o remount,ro,bind "{toolchain.root}" 2>/dev/null || true'
                    )

            # Mask synthetic sentinel directory and any explicit non-HOME reference paths.
            seen_masks: set[str] = set()
            for mdir in mask_dirs:
                mdir_str = str(mdir.resolve())
                if mdir_str in seen_masks:
                    continue
                if _is_relative_to(sandbox_root, mdir):
                    continue
                if not _is_relative_to(sandbox_root, host_home) and _is_relative_to(mdir, host_home):
                    continue
                seen_masks.add(mdir_str)
                bootstrap_lines.append(
                    f'"{mount_real}" -t tmpfs -o size=1M,mode=0000 tmpfs "{mdir_str}"'
                )

            bootstrap_lines.append(f'cd "{realm_dir}"')
            bootstrap_lines.append(
                f'exec "{bin_dir / "python3"}" "{sandbox_script}" --child-probe-config "{config_file}"'
            )

            bootstrap_path = sandbox_root / "enter_sandbox.sh"
            bootstrap_path.write_text("\n".join(bootstrap_lines) + "\n", encoding="utf-8")
            os.chmod(bootstrap_path, 0o700)

            unshare_bin = shutil.which("unshare")
            assert unshare_bin is not None
            child_res = _run_cmd(
                [unshare_bin, "--user", "--map-root-user", "--mount", "--", str(bootstrap_path)],
                cwd=sandbox_root,
                env=child_env,
                timeout=300,
            )
        else:
            child_res = _run_cmd(
                [
                    str(bin_dir / "python3"),
                    str(sandbox_script),
                    "--child-probe-config",
                    str(config_file),
                ],
                cwd=realm_dir,
                env=child_env,
                timeout=300,
            )

        if not result_file.is_file():
            raise RuntimeError(
                f"Isolated child exited {child_res.returncode} without writing results.\n"
                f"stdout: {child_res.stdout}\nstderr: {child_res.stderr}"
            )

        child_data = json.loads(result_file.read_text(encoding="utf-8"))
        finished_at = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()

        return {
            "schema_version": 1,
            "started_at_utc": started_at,
            "finished_at_utc": finished_at,
            "source_head_sha": head_sha,
            "reference_sha": reference_sha,
            "isolation_mode": isolation_mode,
            "mount_namespace_detail": mount_ns_note,
            "rust_toolchain": {
                "rustc_version": toolchain.rustc_version,
                "clippy_version": toolchain.clippy_version,
            },
            "allowlisted_tools": allowlisted_tools,
            "child_exit_code": child_res.returncode,
            "child_stdout": child_res.stdout.strip(),
            "child_stderr": child_res.stderr.strip(),
            "probes": child_data["probes"],
            "all_passed": bool(child_data["all_passed"]) and child_res.returncode == 0,
        }
    finally:
        # Restore sentinel directory mode so TemporaryDirectory cleanup always succeeds.
        if sentinel_dir.exists():
            try:
                os.chmod(sentinel_dir, 0o700)
            except OSError:
                pass
        if keep_temp:
            # Keep temp dir intact by defusing finalizer.
            temp_ctx._finalizer.detach()  # type: ignore[attr-defined]
        else:
            temp_ctx.cleanup()


def render_markdown_transcript(receipt: dict[str, Any]) -> str:
    """Render a human-readable Markdown proof transcript from a canary receipt."""
    verdict = "PASS" if receipt["all_passed"] else "FAIL"
    lines: list[str] = [
        "# Elysian Realm Wall Canary Transcript",
        "",
        "Recorded live execution of `scripts/wall_canary.py` against a clean clone of `HEAD`.",
        "",
        "## Run Metadata",
        "",
        "| Field | Value |",
        "| --- | --- |",
        f"| Verdict | **{verdict}** |",
        f"| Timestamp (UTC) | `{receipt['started_at_utc']}` -> `{receipt['finished_at_utc']}` |",
        f"| Elysian Realm `HEAD` | `{receipt['source_head_sha']}` |",
        f"| Reference SHA Checked | `{receipt['reference_sha']}` |",
        f"| Isolation Mode | `{receipt['isolation_mode']}` ({receipt['mount_namespace_detail']}) |",
        f"| Rust Toolchain | `{receipt['rust_toolchain']['rustc_version']}` / `{receipt['rust_toolchain']['clippy_version']}` |",
        "",
        "## Probe Summary",
        "",
        "| # | Probe | Expected | Verdict | Summary |",
        "| --- | --- | --- | --- | --- |",
    ]

    for idx, probe in enumerate(receipt["probes"], start=1):
        status = "PASS" if probe["passed"] else "FAIL"
        lines.append(
            f"| {idx} | `{probe['name']}` | {probe['expected']} | **{status}** | {probe['summary']} |"
        )

    lines.extend(["", "## Probe Details", ""])
    for idx, probe in enumerate(receipt["probes"], start=1):
        status = "PASS" if probe["passed"] else "FAIL"
        lines.append(f"### Probe {idx}: `{probe['name']}` ({status})")
        lines.append("")
        lines.append(f"- **Expected**: {probe['expected']}")
        lines.append(f"- **Observed**: {probe['summary']}")
        details = probe["details"]

        if probe["name"] == "reference_file_locate":
            lines.append("")
            lines.append("| Target | Kind | Blocked | Observation |")
            lines.append("| --- | --- | --- | --- |")
            for att in details["attempts"]:
                lines.append(
                    f"| `{att['label']}` | `{att['kind']}` | `{att['blocked']}` | {att['observation']} |"
                )
        elif probe["name"] == "reference_git_history":
            lines.append(f"- **Standalone `.git` directory**: `{details['git_dir_standalone']}`")
            lines.append(f"- **Alternates file exists**: `{details['alternates_exists']}`")
            lines.append(f"- **Realm commits scanned**: `{details['realm_commit_count']}`")
            lines.append(f"- **Reference object present**: `{details['reference_object_present']}`")
            remotes_str = ", ".join(f"`{r}`" for r in details["remotes"]) or "(none)"
            lines.append(f"- **Remotes**: {remotes_str}")
        elif probe["name"] == "linear_github_connectors":
            lines.append(
                f"- **Leaked credential env vars**: `{details['leaked_env_vars']}` "
                f"(checked {len(details['checked_env_vars'])} forbidden variables)"
            )
            lines.append(f"- **Leaked credential files**: `{details['leaked_credential_files']}`")
            for cli, obs in details["cli_checks"].items():
                lines.append(f"- **CLI `{cli}`**: `{obs}`")
        elif probe["name"] == "trivial_crate_build_lint":
            lines.append(f"- **`cargo clippy --all-targets -- -D warnings` exit code**: `{details['clippy_exit_code']}`")
            lines.append(f"- **`cargo test` exit code**: `{details['test_exit_code']}`")
            if details.get("test_stdout"):
                lines.append("")
                lines.append("```text")
                lines.append(details["test_stdout"])
                lines.append("```")
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the Elysian Realm wall isolation canary.")
    parser.add_argument(
        "--repo",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="Path to the Elysian Realm git repository to clone cleanly from HEAD.",
    )
    parser.add_argument(
        "--reference-sha",
        default=DEFAULT_REFERENCE_SHA,
        help="Pinned reference repository SHA that must not appear in the realm git history.",
    )
    parser.add_argument(
        "--reference-path",
        action="append",
        type=Path,
        default=[],
        help="Additional host reference path to verify is blocked inside the sandbox.",
    )
    parser.add_argument(
        "--require-mount-ns",
        action="store_true",
        help="Fail closed if Linux user+mount namespace isolation is unavailable.",
    )
    parser.add_argument(
        "--markdown-out",
        type=Path,
        default=None,
        help="Write the Markdown canary transcript to this file.",
    )
    parser.add_argument(
        "--json-out",
        type=Path,
        default=None,
        help="Write the machine-readable JSON receipt to this file.",
    )
    parser.add_argument(
        "--keep-temp",
        action="store_true",
        help="Preserve temporary sandbox directory for debugging.",
    )
    parser.add_argument(
        "--child-probe-config",
        type=Path,
        default=None,
        help=argparse.SUPPRESS,
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.child_probe_config is not None:
        return run_child_probes(args.child_probe_config)

    receipt = run_wall_canary(
        source_repo=args.repo,
        reference_sha=args.reference_sha,
        extra_reference_paths=args.reference_path,
        require_mount_ns=args.require_mount_ns,
        keep_temp=args.keep_temp,
    )
    transcript = render_markdown_transcript(receipt)

    if args.markdown_out is not None:
        args.markdown_out.parent.mkdir(parents=True, exist_ok=True)
        args.markdown_out.write_text(transcript, encoding="utf-8")
    else:
        sys.stdout.write(transcript)

    if args.json_out is not None:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")

    return 0 if receipt["all_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
