#!/usr/bin/env python3
"""Unit and wall-violation tests for scripts/board_split.py."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import board_split  # noqa: E402
from board_split import (  # noqa: E402
    AgentRole,
    BoardSide,
    FileBackedBoard,
    MemberToken,
    OBSERVER_CHANNELS,
    WallViolationError,
)


class BoardSplitTests(unittest.TestCase):
    def test_observer_and_builder_boards_roundtrip_and_isolation(self) -> None:
        with tempfile.TemporaryDirectory(prefix="board-split-unit-") as tmp:
            root = Path(tmp)
            obs = FileBackedBoard.initialize(
                root / "observer-board", BoardSide.OBSERVER, OBSERVER_CHANNELS
            )
            bld = FileBackedBoard.initialize(
                root / "builder-board", BoardSide.BUILDER, ("#slice-m1", "#questions")
            )

            su = obs.register_member("su-1", AgentRole.SU_OBSERVER, opposite_boards=(bld,))
            coord_obs = obs.register_member(
                "coord-obs", AgentRole.OBSERVER_COORDINATOR, opposite_boards=(bld,)
            )
            villv = bld.register_member("villv-1", AgentRole.VILL_V_BUILDER, opposite_boards=(obs,))
            coord_bld = bld.register_member(
                "coord-bld", AgentRole.BUILDER_COORDINATOR, opposite_boards=(obs,)
            )

            posted_obs = obs.post(su, "#observe", "Captured reference baseline.")
            msgs_obs = obs.read(coord_obs, "#observe")
            self.assertEqual(len(msgs_obs), 1)
            self.assertEqual(msgs_obs[0].body, posted_obs.body)

            posted_bld = bld.post(villv, "#slice-m1", "Implemented crate from spec package.")
            msgs_bld = bld.read(coord_bld, "#slice-m1")
            self.assertEqual(len(msgs_bld), 1)
            self.assertEqual(msgs_bld[0].body, posted_bld.body)

            # AC3: Builder member attempting to read observer board MUST fail with WallViolationError.
            with self.assertRaises(WallViolationError):
                obs.read(villv, "#observe")

            # Builder member attempting to post to observer board MUST fail with WallViolationError.
            with self.assertRaises(WallViolationError):
                obs.post(villv, "#integrate", "Unauthorized post")

            # Two-coordinator rule: Observer coordinator cannot post directly to builder board.
            with self.assertRaises(WallViolationError):
                bld.post(coord_obs, "#slice-m1", "Observer coordinator brief leak")

            # Observer coordinator cannot read builder board directly either.
            with self.assertRaises(WallViolationError):
                bld.read(coord_obs, "#slice-m1")

            # Strict permissions verified.
            self.assertTrue(obs.audit_permissions()["ok"])
            self.assertTrue(bld.audit_permissions()["ok"])

    def test_forged_token_signature_rejected_with_wall_violation(self) -> None:
        with tempfile.TemporaryDirectory(prefix="board-split-forge-") as tmp:
            root = Path(tmp)
            obs = FileBackedBoard.initialize(
                root / "observer-board", BoardSide.OBSERVER, OBSERVER_CHANNELS
            )
            obs.register_member("su-1", AgentRole.SU_OBSERVER)

            forged = MemberToken(
                board_id=obs.board_id,
                board_side=BoardSide.OBSERVER,
                agent_id="su-1",
                role=AgentRole.SU_OBSERVER,
                signature="0" * 64,
            )
            with self.assertRaises(WallViolationError):
                obs.read(forged, "#observe")

    def test_wrong_role_registration_and_dual_membership_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="board-split-roles-") as tmp:
            root = Path(tmp)
            obs = FileBackedBoard.initialize(
                root / "observer-board", BoardSide.OBSERVER, OBSERVER_CHANNELS
            )
            bld = FileBackedBoard.initialize(
                root / "builder-board", BoardSide.BUILDER, ("#slice-m1", "#questions")
            )

            with self.assertRaises(WallViolationError):
                obs.register_member("villv-bad", AgentRole.VILL_V_BUILDER)

            with self.assertRaises(WallViolationError):
                bld.register_member("su-bad", AgentRole.SU_OBSERVER)

            bld.register_member("shared-agent", AgentRole.VILL_V_BUILDER, opposite_boards=(obs,))
            with self.assertRaises(WallViolationError):
                obs.register_member(
                    "shared-agent", AgentRole.SU_OBSERVER, opposite_boards=(bld,)
                )

    def test_channel_allowlists_enforced(self) -> None:
        with tempfile.TemporaryDirectory(prefix="board-split-channels-") as tmp:
            root = Path(tmp)
            with self.assertRaises(ValueError):
                FileBackedBoard.initialize(
                    root / "obs-bad", BoardSide.OBSERVER, ("#slice-m1",)
                )
            with self.assertRaises(ValueError):
                FileBackedBoard.initialize(
                    root / "bld-bad", BoardSide.BUILDER, ("#observe",)
                )

    def test_end_to_end_board_split_proof_passes(self) -> None:
        receipt = board_split.run_board_split_proof()
        self.assertTrue(receipt["all_passed"], json.dumps(receipt, indent=2))
        self.assertEqual(len(receipt["checks"]), 6)
        transcript = board_split.render_markdown_transcript(receipt)
        self.assertIn("**PASS**", transcript)
        self.assertIn("WallViolationError", transcript)


if __name__ == "__main__":
    unittest.main()
