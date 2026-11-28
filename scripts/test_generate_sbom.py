"""Unit tests for deterministic CycloneDX 1.5 SBOM generator (`scripts/generate_sbom.py`)."""

from __future__ import annotations

import copy
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_sbom  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent


class GenerateSbomTests(unittest.TestCase):
    def test_real_repo_sbom_is_deterministic_and_passes_verify(self) -> None:
        sbom_a = generate_sbom.generate_sbom(REPO_ROOT, release_version="0.2.0")
        sbom_b = generate_sbom.generate_sbom(REPO_ROOT, release_version="0.2.0")
        self.assertEqual(
            generate_sbom.render_sbom_json(sbom_a),
            generate_sbom.render_sbom_json(sbom_b),
            "SBOM output must be byte-for-byte deterministic",
        )
        self.assertEqual(sbom_a["bomFormat"], "CycloneDX")
        self.assertEqual(sbom_a["specVersion"], "1.5")
        self.assertEqual(sbom_a["metadata"]["component"]["version"], "0.2.0")

        errors = generate_sbom.verify_sbom(sbom_a)
        self.assertEqual(errors, [], f"unexpected SBOM verification errors: {errors}")

        by_name = {c["name"]: c for c in sbom_a["components"]}
        for ws_crate in (
            "ef-core",
            "ef-clinic-server",
            "ef-mobile-server",
            "ef-clinic-desktop",
            "ef-mobile-desktop",
            "ef-ui-catalog",
            "ef-public-site",
        ):
            self.assertIn(ws_crate, by_name, f"missing workspace crate {ws_crate}")
            self.assertEqual(by_name[ws_crate]["purl"], f"pkg:cargo/{ws_crate}@0.1.0")
            self.assertIn(
                {"name": "elysian:origin", "value": "workspace"},
                by_name[ws_crate]["properties"],
            )

        for ext_crate in ("serde", "serde_json", "thiserror", "bevy"):
            self.assertIn(ext_crate, by_name, f"missing external crate {ext_crate}")
            self.assertIn("hashes", by_name[ext_crate])
            self.assertEqual(by_name[ext_crate]["hashes"][0]["alg"], "SHA-256")
            self.assertEqual(len(by_name[ext_crate]["hashes"][0]["content"]), 64)

    def test_cli_out_and_verify_roundtrip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out_path = Path(tmp) / "elysian-fields-1.0.0-sbom.cdx.json"
            gen = subprocess.run(
                [
                    sys.executable,
                    str(REPO_ROOT / "scripts" / "generate_sbom.py"),
                    "--version",
                    "1.0.0",
                    "--out",
                    str(out_path),
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(gen.returncode, 0, gen.stderr)
            self.assertTrue(out_path.is_file())

            ver = subprocess.run(
                [
                    sys.executable,
                    str(REPO_ROOT / "scripts" / "generate_sbom.py"),
                    "--verify",
                    "--out",
                    str(out_path),
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(ver.returncode, 0, ver.stderr)
            self.assertIn("verified OK", ver.stdout)

            data = json.loads(out_path.read_text(encoding="utf-8"))
            self.assertEqual(data["metadata"]["component"]["version"], "1.0.0")

    def test_verify_rejects_schema_purl_hash_order_and_secret_violations(self) -> None:
        base = generate_sbom.generate_sbom(REPO_ROOT, release_version="0.1.0")

        bad_spec = copy.deepcopy(base)
        bad_spec["specVersion"] = "1.4"
        self.assertTrue(any("specVersion" in e for e in generate_sbom.verify_sbom(bad_spec)))

        bad_purl = copy.deepcopy(base)
        bad_purl["components"][0]["purl"] = "pkg:npm/wrong@1.0.0"
        self.assertTrue(any("purl" in e for e in generate_sbom.verify_sbom(bad_purl)))

        bad_hash = copy.deepcopy(base)
        for comp in bad_hash["components"]:
            if comp.get("hashes"):
                comp["hashes"][0]["content"] = "not-a-valid-sha256"
                break
        self.assertTrue(any("SHA-256" in e for e in generate_sbom.verify_sbom(bad_hash)))

        unsorted = copy.deepcopy(base)
        unsorted["components"] = list(reversed(unsorted["components"]))
        self.assertTrue(any("sorted" in e for e in generate_sbom.verify_sbom(unsorted)))

        secret_leak = copy.deepcopy(base)
        secret_leak["metadata"]["properties"].append(
            {"name": "leak", "value": "ghp_1234567890abcdefghijklmnop"}
        )
        self.assertTrue(any("GitHub token" in e for e in generate_sbom.verify_sbom(secret_leak)))

        phi_leak = copy.deepcopy(base)
        phi_leak["metadata"]["properties"].append(
            {"name": "patient", "value": "patient.smith@clinic.org"}
        )
        self.assertTrue(any("email address" in e for e in generate_sbom.verify_sbom(phi_leak)))


if __name__ == "__main__":
    unittest.main()
