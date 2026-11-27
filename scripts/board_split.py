#!/usr/bin/env python3
"""Zero-dependency, file-backed split-board swarm harness and isolation proof.

Enforces two physically and cryptographically separated message boards across
the Elysian Fields clean-room wall:

- Observer board (observer/reference environment):
  channels ``#observe``, ``#spec``, ``#verify``, ``#integrate``.
- Builder board (Elysian Realm):
  channels ``#slice-<slug>``, ``#questions``.

Every board root uses ``0700`` permissions, stores secrets and logs with
``0600`` permissions, issues board-scoped HMAC-SHA256 role-bound membership
tokens, rejects cross-wall reads/writes with ``WallViolationError``, enforces
the two-coordinator rule, and prevents any agent from joining both boards.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import hmac
import json
import os
import re
import secrets
import stat
import sys
import tempfile
from dataclasses import asdict, dataclass
from enum import Enum
from pathlib import Path
from typing import Any, Sequence


class WallViolationError(PermissionError):
    """Raised whenever an operation attempts to cross the clean-room board wall."""


class BoardSide(str, Enum):
    OBSERVER = "observer"
    BUILDER = "builder"


class AgentRole(str, Enum):
    SU_OBSERVER = "Su · observe"
    APONIA_GATEKEEPER = "Aponia · spec"
    MOBIUS_VERIFIER = "Mobius · verify"
    EDEN_INTEGRATOR = "Eden · integrate"
    OBSERVER_COORDINATOR = "coord-observer"
    VILL_V_BUILDER = "Vill-V · build"
    BUILDER_COORDINATOR = "coord-builder"


OBSERVER_ROLES = frozenset(
    {
        AgentRole.SU_OBSERVER,
        AgentRole.APONIA_GATEKEEPER,
        AgentRole.MOBIUS_VERIFIER,
        AgentRole.EDEN_INTEGRATOR,
        AgentRole.OBSERVER_COORDINATOR,
    }
)
BUILDER_ROLES = frozenset(
    {
        AgentRole.VILL_V_BUILDER,
        AgentRole.BUILDER_COORDINATOR,
    }
)
OBSERVER_CHANNELS = ("#observe", "#spec", "#verify", "#integrate")
BUILDER_SLICE_CHANNEL_RE = re.compile(r"^#slice-[a-z0-9][a-z0-9-]*$")
AGENT_ID_RE = re.compile(r"^[a-z0-9][a-z0-9_-]{0,47}$")


@dataclass(frozen=True)
class MemberToken:
    """Role-bound membership token scoped to a single board."""

    board_id: str
    board_side: BoardSide
    agent_id: str
    role: AgentRole
    signature: str


@dataclass(frozen=True)
class BoardMessage:
    """Single message appended to a board channel."""

    seq: int
    timestamp_utc: str
    channel: str
    agent_id: str
    role: str
    body: str


def role_side(role: AgentRole) -> BoardSide:
    if role in OBSERVER_ROLES:
        return BoardSide.OBSERVER
    if role in BUILDER_ROLES:
        return BoardSide.BUILDER
    raise WallViolationError(f"Unknown agent role: {role!r}")


def validate_channel_name(side: BoardSide, channel: str) -> None:
    if side is BoardSide.OBSERVER:
        if channel not in OBSERVER_CHANNELS:
            raise ValueError(
                f"Observer board allows only {OBSERVER_CHANNELS}; got {channel!r}"
            )
    elif side is BoardSide.BUILDER:
        if channel != "#questions" and not BUILDER_SLICE_CHANNEL_RE.match(channel):
            raise ValueError(
                f"Builder board allows only '#questions' or '#slice-<slug>'; got {channel!r}"
            )
    else:
        raise ValueError(f"Unsupported board side: {side!r}")


def _write_private_file(path: Path, content: str) -> None:
    flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC
    fd = os.open(str(path), flags, 0o600)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(content)
    finally:
        os.chmod(path, 0o600)


def _channel_filename(channel: str) -> str:
    return f"{channel.lstrip('#')}.jsonl"


class FileBackedBoard:
    """File-backed message board for one side of the clean-room wall."""

    def __init__(self, root: Path) -> None:
        self.root = root.resolve()
        meta_path = self.root / "board.json"
        if not meta_path.is_file():
            raise FileNotFoundError(f"Board metadata missing at {meta_path}")
        meta = json.loads(meta_path.read_text(encoding="utf-8"))
        self.board_id: str = meta["board_id"]
        self.side = BoardSide(meta["side"])
        self.channels: tuple[str, ...] = tuple(meta["channels"])

    @classmethod
    def initialize(
        cls,
        root: Path,
        side: BoardSide,
        channels: Sequence[str],
        *,
        board_id: str | None = None,
    ) -> "FileBackedBoard":
        """Create a new board directory with 0700 permissions and 0600 state files."""
        resolved = root.resolve()
        if not channels:
            raise ValueError("Board must define at least one channel")
        seen: list[str] = []
        for ch in channels:
            validate_channel_name(side, ch)
            if ch not in seen:
                seen.append(ch)

        resolved.mkdir(parents=True, exist_ok=True)
        os.chmod(resolved, 0o700)

        channels_dir = resolved / "channels"
        channels_dir.mkdir(parents=True, exist_ok=True)
        os.chmod(channels_dir, 0o700)

        assigned_id = board_id or f"{side.value}-{secrets.token_hex(6)}"
        secret_hex = secrets.token_hex(32)
        _write_private_file(resolved / ".board-secret", secret_hex + "\n")

        meta = {
            "schema_version": 1,
            "board_id": assigned_id,
            "side": side.value,
            "channels": seen,
            "created_at_utc": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat(),
        }
        _write_private_file(resolved / "board.json", json.dumps(meta, indent=2) + "\n")
        _write_private_file(resolved / "members.json", json.dumps({}, indent=2) + "\n")

        for ch in seen:
            _write_private_file(channels_dir / _channel_filename(ch), "")

        return cls(resolved)

    def _load_secret(self) -> bytes:
        raw = (self.root / ".board-secret").read_text(encoding="utf-8").strip()
        return bytes.fromhex(raw)

    def _sign_token(self, agent_id: str, role: AgentRole) -> str:
        msg = f"{self.board_id}|{self.side.value}|{agent_id}|{role.value}".encode("utf-8")
        return hmac.new(self._load_secret(), msg, hashlib.sha256).hexdigest()

    def _load_members(self) -> dict[str, dict[str, str]]:
        return json.loads((self.root / "members.json").read_text(encoding="utf-8"))

    def register_member(
        self,
        agent_id: str,
        role: AgentRole,
        *,
        opposite_boards: Sequence["FileBackedBoard"] = (),
    ) -> MemberToken:
        """Issue a role-bound token after verifying wall and dual-membership rules."""
        if not AGENT_ID_RE.match(agent_id):
            raise ValueError(f"Invalid agent_id: {agent_id!r}")

        expected_side = role_side(role)
        if expected_side is not self.side:
            raise WallViolationError(
                f"Role '{role.value}' belongs to the {expected_side.value} side and cannot "
                f"join {self.side.value} board '{self.board_id}'."
            )

        for other in opposite_boards:
            if other.side is self.side:
                continue
            other_members = other._load_members()
            if agent_id in other_members:
                raise WallViolationError(
                    f"Dual-board membership denied: agent '{agent_id}' is already registered "
                    f"on {other.side.value} board '{other.board_id}'."
                )

        members = self._load_members()
        existing = members.get(agent_id)
        if existing is not None and existing["role"] != role.value:
            raise WallViolationError(
                f"Agent '{agent_id}' already holds permanent role '{existing['role']}' on "
                f"board '{self.board_id}'; cannot reassign to '{role.value}'."
            )

        members[agent_id] = {
            "role": role.value,
            "registered_at_utc": dt.datetime.now(dt.timezone.utc)
            .replace(microsecond=0)
            .isoformat(),
        }
        _write_private_file(self.root / "members.json", json.dumps(members, indent=2) + "\n")

        return MemberToken(
            board_id=self.board_id,
            board_side=self.side,
            agent_id=agent_id,
            role=role,
            signature=self._sign_token(agent_id, role),
        )

    def _authorize(self, token: MemberToken, action: str) -> None:
        if token.board_side is not self.side:
            raise WallViolationError(
                f"Cross-wall {action} denied: {token.board_side.value}-board member "
                f"'{token.agent_id}' ({token.role.value}) attempted to {action} "
                f"{self.side.value} board '{self.board_id}'."
            )
        if role_side(token.role) is not self.side:
            raise WallViolationError(
                f"Role mismatch on {action}: '{token.role.value}' is forbidden on "
                f"{self.side.value} board '{self.board_id}'."
            )
        if token.board_id != self.board_id:
            raise WallViolationError(
                f"Foreign board token denied on {action}: token belongs to '{token.board_id}', "
                f"not '{self.board_id}'."
            )

        expected_sig = self._sign_token(token.agent_id, token.role)
        if not hmac.compare_digest(token.signature, expected_sig):
            raise WallViolationError(
                f"Invalid token signature for '{token.agent_id}' on board '{self.board_id}'."
            )

        members = self._load_members()
        record = members.get(token.agent_id)
        if record is None or record["role"] != token.role.value:
            raise WallViolationError(
                f"Agent '{token.agent_id}' is not an authorized member of board '{self.board_id}'."
            )

    def _channel_path(self, channel: str) -> Path:
        if channel not in self.channels:
            raise ValueError(
                f"Channel {channel!r} does not exist on {self.side.value} board '{self.board_id}'"
            )
        path = (self.root / "channels" / _channel_filename(channel)).resolve()
        # Prevent path traversal outside channels/.
        path.relative_to((self.root / "channels").resolve())
        return path

    def post(self, token: MemberToken, channel: str, body: str) -> BoardMessage:
        """Append a message to a channel after verifying membership and wall rules."""
        self._authorize(token, "post to")
        if not body.strip():
            raise ValueError("Board message body must not be empty")
        ch_path = self._channel_path(channel)

        existing_lines = [
            line for line in ch_path.read_text(encoding="utf-8").splitlines() if line.strip()
        ]
        next_seq = len(existing_lines) + 1
        message = BoardMessage(
            seq=next_seq,
            timestamp_utc=dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat(),
            channel=channel,
            agent_id=token.agent_id,
            role=token.role.value,
            body=body,
        )
        with ch_path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(asdict(message)) + "\n")
        os.chmod(ch_path, 0o600)
        return message

    def read(self, token: MemberToken, channel: str, *, since_seq: int = 0) -> list[BoardMessage]:
        """Read messages from a channel after verifying membership and wall rules."""
        self._authorize(token, "read from")
        ch_path = self._channel_path(channel)
        messages: list[BoardMessage] = []
        for line in ch_path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            raw = json.loads(line)
            msg = BoardMessage(**raw)
            if msg.seq > since_seq:
                messages.append(msg)
        return messages

    def audit_permissions(self) -> dict[str, Any]:
        """Verify 0700 directory and 0600 file permissions across board storage."""
        violations: list[str] = []
        checked: dict[str, str] = {}

        for dpath in (self.root, self.root / "channels"):
            mode = stat.S_IMODE(dpath.stat().st_mode)
            checked[str(dpath.relative_to(self.root) if dpath != self.root else ".")] = oct(mode)
            if mode != 0o700:
                violations.append(f"Directory {dpath} has mode {oct(mode)}, expected 0o700")

        file_paths = [
            self.root / ".board-secret",
            self.root / "board.json",
            self.root / "members.json",
            *(self.root / "channels" / _channel_filename(ch) for ch in self.channels),
        ]
        for fpath in file_paths:
            mode = stat.S_IMODE(fpath.stat().st_mode)
            checked[str(fpath.relative_to(self.root))] = oct(mode)
            if mode != 0o600:
                violations.append(f"File {fpath} has mode {oct(mode)}, expected 0o600")

        return {
            "board_id": self.board_id,
            "side": self.side.value,
            "ok": len(violations) == 0,
            "modes": checked,
            "violations": violations,
        }


def run_board_split_proof() -> dict[str, Any]:
    """Execute the end-to-end two-board isolation proof and return a receipt."""
    started_at = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()
    checks: list[dict[str, Any]] = []

    with tempfile.TemporaryDirectory(prefix="elysian-board-split-") as tmp:
        base = Path(tmp).resolve()
        observer_root = base / "observer-env" / "observer-board"
        builder_root = base / "elysian-realm" / "builder-board"

        # Check 1: Initialize observer board and verify worker -> coordinator round-trip.
        observer_board = FileBackedBoard.initialize(
            observer_root,
            BoardSide.OBSERVER,
            OBSERVER_CHANNELS,
            board_id="observer-main",
        )
        su_token = observer_board.register_member("su-observer", AgentRole.SU_OBSERVER)
        coord_obs_token = observer_board.register_member(
            "coord-observer", AgentRole.OBSERVER_COORDINATOR
        )
        obs_posted = observer_board.post(
            su_token,
            "#observe",
            "[done] Reference inventory captured for M0 review; ready for Gatekeeper sanitization.",
        )
        obs_received = observer_board.read(coord_obs_token, "#observe")
        c1_ok = (
            len(obs_received) == 1
            and obs_received[0].seq == obs_posted.seq
            and obs_received[0].body == obs_posted.body
            and list(observer_board.channels) == list(OBSERVER_CHANNELS)
        )
        checks.append(
            {
                "name": "observer_board_roundtrip",
                "expected": "PASS (observer worker posts to #observe and observer coordinator receives it)",
                "passed": c1_ok,
                "summary": (
                    f"Observer board initialized with {list(observer_board.channels)}; "
                    f"posted seq={obs_posted.seq} by {obs_posted.agent_id} and read by coord-observer"
                ),
                "details": {
                    "board_id": observer_board.board_id,
                    "channels": list(observer_board.channels),
                    "posted_message": asdict(obs_posted),
                    "received_count": len(obs_received),
                },
            }
        )

        # Check 2: Initialize builder board inside realm and verify worker -> coordinator round-trip.
        builder_board = FileBackedBoard.initialize(
            builder_root,
            BoardSide.BUILDER,
            ("#slice-m1", "#questions"),
            board_id="builder-realm",
        )
        villv_token = builder_board.register_member(
            "villv-m1",
            AgentRole.VILL_V_BUILDER,
            opposite_boards=(observer_board,),
        )
        coord_bld_token = builder_board.register_member(
            "coord-builder",
            AgentRole.BUILDER_COORDINATOR,
            opposite_boards=(observer_board,),
        )
        bld_posted = builder_board.post(
            villv_token,
            "#slice-m1",
            "[done] Prototype built against specs/packages/m1-feasibility; clippy and tests green.",
        )
        bld_received = builder_board.read(coord_bld_token, "#slice-m1")
        c2_ok = (
            len(bld_received) == 1
            and bld_received[0].seq == bld_posted.seq
            and bld_received[0].body == bld_posted.body
            and list(builder_board.channels) == ["#slice-m1", "#questions"]
        )
        checks.append(
            {
                "name": "builder_board_roundtrip",
                "expected": "PASS (builder worker posts to #slice-m1 and builder coordinator receives it)",
                "passed": c2_ok,
                "summary": (
                    f"Builder board initialized with {list(builder_board.channels)}; "
                    f"posted seq={bld_posted.seq} by {bld_posted.agent_id} and read by coord-builder"
                ),
                "details": {
                    "board_id": builder_board.board_id,
                    "channels": list(builder_board.channels),
                    "posted_message": asdict(bld_posted),
                    "received_count": len(bld_received),
                },
            }
        )

        # Check 3: Builder-board member attempts to read observer board -> MUST raise WallViolationError.
        c3_error = ""
        c3_ok = False
        try:
            observer_board.read(villv_token, "#observe")
        except WallViolationError as exc:
            c3_ok = True
            c3_error = str(exc)
        checks.append(
            {
                "name": "builder_read_observer_blocked",
                "expected": "FAIL with WallViolationError",
                "passed": c3_ok,
                "summary": (
                    f"Blocked with WallViolationError: {c3_error}"
                    if c3_ok
                    else "WALL BREACH: builder member read observer board!"
                ),
                "details": {"error": c3_error},
            }
        )

        # Check 4: Observer coordinator attempts to post directly to builder board -> WallViolationError.
        c4_error = ""
        c4_ok = False
        try:
            builder_board.post(coord_obs_token, "#slice-m1", "Direct brief from observer coordinator")
        except WallViolationError as exc:
            c4_ok = True
            c4_error = str(exc)
        checks.append(
            {
                "name": "two_coordinator_rule_enforced",
                "expected": "FAIL with WallViolationError (observer coordinator cannot post builder briefs)",
                "passed": c4_ok,
                "summary": (
                    f"Blocked with WallViolationError: {c4_error}"
                    if c4_ok
                    else "WALL BREACH: observer coordinator posted to builder board!"
                ),
                "details": {"error": c4_error},
            }
        )

        # Check 5: Dual-board membership attempt blocked with WallViolationError.
        c5_error = ""
        c5_ok = False
        try:
            observer_board.register_member(
                "villv-m1",
                AgentRole.SU_OBSERVER,
                opposite_boards=(builder_board,),
            )
        except WallViolationError as exc:
            c5_ok = True
            c5_error = str(exc)
        checks.append(
            {
                "name": "dual_board_membership_blocked",
                "expected": "FAIL with WallViolationError (no agent joins both boards)",
                "passed": c5_ok,
                "summary": (
                    f"Blocked with WallViolationError: {c5_error}"
                    if c5_ok
                    else "WALL BREACH: dual-board membership permitted!"
                ),
                "details": {"error": c5_error},
            }
        )

        # Check 6: Filesystem permission audit (0700 dirs, 0600 files).
        obs_perm = observer_board.audit_permissions()
        bld_perm = builder_board.audit_permissions()
        c6_ok = bool(obs_perm["ok"]) and bool(bld_perm["ok"])
        checks.append(
            {
                "name": "filesystem_permissions_audit",
                "expected": "PASS (0700 board directories, 0600 secrets/metadata/channel logs)",
                "passed": c6_ok,
                "summary": (
                    "Verified 0o700 directories and 0o600 files on both observer and builder boards"
                    if c6_ok
                    else f"Permission mismatch: obs={obs_perm['violations']} bld={bld_perm['violations']}"
                ),
                "details": {"observer_board": obs_perm, "builder_board": bld_perm},
            }
        )

    finished_at = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()
    return {
        "schema_version": 1,
        "started_at_utc": started_at,
        "finished_at_utc": finished_at,
        "checks": checks,
        "all_passed": all(c["passed"] for c in checks),
    }


def render_markdown_transcript(receipt: dict[str, Any]) -> str:
    """Render a human-readable Markdown proof transcript from a board-split receipt."""
    verdict = "PASS" if receipt["all_passed"] else "FAIL"
    lines: list[str] = [
        "# Split Two-Board Isolation Proof Transcript",
        "",
        "Recorded live execution of `scripts/board_split.py` verifying independent observer and builder boards and strict cross-wall rejection.",
        "",
        "## Run Metadata",
        "",
        "| Field | Value |",
        "| --- | --- |",
        f"| Verdict | **{verdict}** |",
        f"| Timestamp (UTC) | `{receipt['started_at_utc']}` -> `{receipt['finished_at_utc']}` |",
        f"| Total Checks | `{len(receipt['checks'])}` |",
        "",
        "## Verification Summary",
        "",
        "| # | Check | Expected | Verdict | Summary |",
        "| --- | --- | --- | --- | --- |",
    ]
    for idx, chk in enumerate(receipt["checks"], start=1):
        status = "PASS" if chk["passed"] else "FAIL"
        lines.append(
            f"| {idx} | `{chk['name']}` | {chk['expected']} | **{status}** | {chk['summary']} |"
        )

    lines.extend(["", "## Check Details", ""])
    for idx, chk in enumerate(receipt["checks"], start=1):
        status = "PASS" if chk["passed"] else "FAIL"
        lines.append(f"### Check {idx}: `{chk['name']}` ({status})")
        lines.append("")
        lines.append(f"- **Expected**: {chk['expected']}")
        lines.append(f"- **Observed**: {chk['summary']}")
        lines.append("")
        lines.append("```json")
        lines.append(json.dumps(chk["details"], indent=2))
        lines.append("```")
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run the split two-board swarm isolation proof."
    )
    parser.add_argument(
        "--markdown-out",
        type=Path,
        default=None,
        help="Write the Markdown transcript to this file.",
    )
    parser.add_argument(
        "--json-out",
        type=Path,
        default=None,
        help="Write the JSON receipt to this file.",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    receipt = run_board_split_proof()
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
