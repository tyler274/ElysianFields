#!/usr/bin/env python3
"""Unit tests for scripts/ci_dep_audit.py."""

from __future__ import annotations

from pathlib import Path
import unittest

import ci_dep_audit


class CiDepAuditTests(unittest.TestCase):
    def test_distinguishes_workspace_and_third_party_compilations(self) -> None:
        lines = [
            "   Compiling proc-macro2 v1.0.95\n",
            "   Compiling bevy_ecs v0.20.1\n",
            "   Compiling elysian-smoke v0.1.0 (/repo/crates/elysian-smoke)\n",
            "   Compiling elysian-bevy-probe v0.1.0 (/repo/crates/elysian-bevy-probe)\n",
            "    Finished `dev` profile in 4.21s\n",
        ]
        audit = ci_dep_audit.parse_cargo_output(lines, workspace_root=Path("/repo"))
        self.assertEqual(audit.workspace_crates, ("elysian-smoke", "elysian-bevy-probe"))
        self.assertEqual(audit.third_party_crates, ("proc-macro2", "bevy_ecs"))
        self.assertTrue(audit.third_party_recompiled)
        self.assertIn("REBUILT 2 third-party", ci_dep_audit.format_report("native", audit))

    def test_warm_cache_hit_reports_zero_third_party_recompilations(self) -> None:
        lines = [
            "   Compiling elysian-bevy-probe v0.1.0 (/repo/crates/elysian-bevy-probe)\n",
            "    Finished `dev` profile in 0.85s\n",
        ]
        audit = ci_dep_audit.parse_cargo_output(lines, workspace_root=Path("/repo"))
        self.assertEqual(audit.workspace_count, 1)
        self.assertEqual(audit.third_party_count, 0)
        self.assertFalse(audit.third_party_recompiled)
        self.assertIn("CACHE HIT", ci_dep_audit.format_report("native", audit))


if __name__ == "__main__":
    unittest.main()
