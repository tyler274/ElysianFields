"""Unit tests for scripts/m6_toolchain.sh config-only opt-in and switch-back."""

from __future__ import annotations

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = REPO_ROOT / "scripts" / "m6_toolchain.sh"


class M6ToolchainTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        tmp_path = Path(self._tmp.name)
        self.fake_lib = tmp_path / "libmimalloc.so"
        self.fake_lib.write_bytes(b"fake-so")
        self.fake_elyld = tmp_path / "elyld"
        self.fake_elyld.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        self.fake_elyld.chmod(self.fake_elyld.stat().st_mode | stat.S_IXUSR)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def _run(self, *args: str, env_overrides: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env.pop("LD_PRELOAD", None)
        env.pop("RUSTFLAGS", None)
        env.pop("ELYSIAN_ELYMALLOC_LIB", None)
        env.pop("ELYSIAN_ELYLD_BIN", None)
        if env_overrides:
            env.update(env_overrides)
        return subprocess.run(
            ["bash", str(SCRIPT), *args],
            capture_output=True,
            text=True,
            env=env,
            check=False,
        )

    def test_help_prints_pinned_revisions(self) -> None:
        res = self._run("--help")
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("cce11466569646e4e1f7afe9b44c6c80dd613b9f", res.stdout)
        self.assertIn("1b1e263ff40d5eea10966f6d2651f50ce43831ab", res.stdout)

    def test_default_mode_print_env_unsets_overrides(self) -> None:
        res = self._run("--mode", "default", "--print-env")
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("export ELYSIAN_M6_MODE='default'", res.stdout)
        self.assertIn("unset LD_PRELOAD", res.stdout)
        self.assertIn("unset RUSTFLAGS", res.stdout)

    def test_elymalloc_mode_sets_ld_preload_only(self) -> None:
        res = self._run(
            "--mode",
            "elymalloc",
            "--elymalloc-lib",
            str(self.fake_lib),
            "--print-env",
        )
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn(f"export LD_PRELOAD='{self.fake_lib}'", res.stdout)
        self.assertIn("unset RUSTFLAGS", res.stdout)

    def test_elyld_mode_gc_sections_default(self) -> None:
        res = self._run(
            "--mode",
            "elyld",
            "--elyld-bin",
            str(self.fake_elyld),
            "--print-env",
        )
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("unset LD_PRELOAD", res.stdout)
        self.assertIn(f"--ld-path={self.fake_elyld}", res.stdout)
        self.assertNotIn("--incremental", res.stdout)
        self.assertNotIn("--no-gc-sections", res.stdout)

    def test_elyld_mode_incremental_no_gc_opt_in(self) -> None:
        res = self._run(
            "--mode",
            "elyld",
            "--gc-mode",
            "incremental-no-gc",
            "--elyld-bin",
            str(self.fake_elyld),
            "--print-env",
        )
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("-C link-arg=-Wl,--no-gc-sections", res.stdout)
        self.assertIn("-C link-arg=-Wl,--incremental", res.stdout)

    def test_both_mode_and_switch_back_execution(self) -> None:
        both = self._run(
            "--mode",
            "both",
            "--gc-mode",
            "gc-sections",
            "--elymalloc-lib",
            str(self.fake_lib),
            "--elyld-bin",
            str(self.fake_elyld),
            "--",
            "sh",
            "-c",
            "printf '%s|%s|%s' \"$ELYSIAN_M6_MODE\" \"${LD_PRELOAD:-}\" \"${RUSTFLAGS:-}\"",
        )
        self.assertEqual(both.returncode, 0, both.stderr)
        mode_val, preload_val, rustflags_val = both.stdout.split("|", 2)
        self.assertEqual(mode_val, "both")
        self.assertEqual(preload_val, str(self.fake_lib))
        self.assertIn(f"--ld-path={self.fake_elyld}", rustflags_val)

        # Switching back to default clears inherited LD_PRELOAD and RUSTFLAGS.
        reverted = self._run(
            "--mode",
            "default",
            "--",
            "sh",
            "-c",
            "printf '%s|%s|%s' \"$ELYSIAN_M6_MODE\" \"${LD_PRELOAD:-<unset>}\" \"${RUSTFLAGS:-<unset>}\"",
            env_overrides={
                "LD_PRELOAD": str(self.fake_lib),
                "RUSTFLAGS": f"-C linker=clang -C link-arg=--ld-path={self.fake_elyld}",
            },
        )
        self.assertEqual(reverted.returncode, 0, reverted.stderr)
        self.assertEqual(reverted.stdout, "default|<unset>|<unset>")

    def test_missing_artifacts_fail_closed(self) -> None:
        missing_lib = self._run("--mode", "elymalloc", "--print-env")
        self.assertEqual(missing_lib.returncode, 2)
        self.assertIn("ELYSIAN_ELYMALLOC_LIB", missing_lib.stderr)

        missing_ld = self._run("--mode", "elyld", "--elyld-bin", "/nonexistent/elyld", "--print-env")
        self.assertEqual(missing_ld.returncode, 2)
        self.assertIn("not found", missing_ld.stderr)

        bad_gc = self._run("--mode", "default", "--gc-mode", "bogus", "--print-env")
        self.assertEqual(bad_gc.returncode, 2)


if __name__ == "__main__":
    unittest.main()
