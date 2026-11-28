"""Unit tests for scripts/package_desktop.py, assets/desktop icons, and release workflows."""

from __future__ import annotations

import json
import plistlib
import subprocess
import sys
import tarfile
import tempfile
import unittest
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "scripts"))
import package_desktop  # noqa: E402


class DesktopPackagingTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp_dir = Path(self._tmp.name)
        self.clinic_bin = self.tmp_dir / "ef-clinic-desktop"
        self.mobile_bin = self.tmp_dir / "ef-mobile-desktop"
        self.clinic_bin.write_bytes(b"\x7fELF-fake-clinic-binary-v1")
        self.mobile_bin.write_bytes(b"\x7fELF-fake-mobile-binary-v1")

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def test_clean_room_svg_icons_are_valid_xml_without_external_refs(self) -> None:
        icons_dir = REPO_ROOT / "assets" / "desktop"
        for name in ("curie-clinic.svg", "curie-mobile.svg"):
            icon_path = icons_dir / name
            self.assertTrue(icon_path.is_file(), f"missing {icon_path}")
            text = icon_path.read_text(encoding="utf-8")
            root = ET.fromstring(text)
            self.assertTrue(root.tag.endswith("svg"), f"unexpected root tag: {root.tag}")
            self.assertNotIn("<script", text.lower())
            self.assertNotIn("http://", text.lower().replace("http://www.w3.org/2000/svg", ""))
            self.assertNotIn("https://", text.lower())
            self.assertNotRegex(text, r"TYL-[0-9]+")

    def test_package_linux_x86_64_bundle_modes_desktop_entries_and_verify(self) -> None:
        out_dir = self.tmp_dir / "linux-out"
        manifest = package_desktop.package_desktop(
            platform="linux-x86_64",
            version="0.1.0",
            clinic_bin=self.clinic_bin,
            mobile_bin=self.mobile_bin,
            out_dir=out_dir,
        )
        archive_path = out_dir / "elysian-fields-desktop-0.1.0-linux-x86_64.tar.gz"
        self.assertTrue(archive_path.is_file())
        self.assertEqual(manifest["archive"], archive_path.name)

        expected_entries = {
            "bin/ef-clinic-desktop",
            "bin/ef-mobile-desktop",
            "install.sh",
            "share/applications/elysian-curie-clinic.desktop",
            "share/applications/elysian-curie-mobile.desktop",
            "share/icons/hicolor/scalable/apps/curie-clinic.svg",
            "share/icons/hicolor/scalable/apps/curie-mobile.svg",
        }
        with tarfile.open(archive_path, "r:gz") as tar:
            members = {m.name: m for m in tar.getmembers()}
            self.assertEqual(set(members.keys()), expected_entries)
            for exe in ("bin/ef-clinic-desktop", "bin/ef-mobile-desktop", "install.sh"):
                self.assertEqual(members[exe].mode & 0o777, 0o755, f"{exe} must be executable")
            clinic_entry = tar.extractfile("share/applications/elysian-curie-clinic.desktop").read().decode("utf-8")
            mobile_entry = tar.extractfile("share/applications/elysian-curie-mobile.desktop").read().decode("utf-8")
            for entry, exe_name, icon_name in (
                (clinic_entry, "Exec=ef-clinic-desktop", "Icon=curie-clinic"),
                (mobile_entry, "Exec=ef-mobile-desktop", "Icon=curie-mobile"),
            ):
                self.assertIn("[Desktop Entry]", entry)
                self.assertIn("Type=Application", entry)
                self.assertIn(exe_name, entry)
                self.assertIn(icon_name, entry)

            install_bytes = tar.extractfile("install.sh").read()
            install_script = self.tmp_dir / "extracted-install.sh"
            install_script.write_bytes(install_bytes)
            syntax = subprocess.run(["bash", "-n", str(install_script)], capture_output=True, text=True, check=False)
            self.assertEqual(syntax.returncode, 0, syntax.stderr)

        verified = package_desktop.verify_out_dir(out_dir)
        self.assertEqual(verified["status"], "verified")

    def test_package_macos_arm64_app_bundles_and_plist_validation(self) -> None:
        out_dir = self.tmp_dir / "macos-out"
        manifest = package_desktop.package_desktop(
            platform="macos-arm64",
            version="0.2.3",
            clinic_bin=self.clinic_bin,
            mobile_bin=self.mobile_bin,
            out_dir=out_dir,
        )
        archive_path = out_dir / "elysian-fields-desktop-0.2.3-macos-arm64.tar.gz"
        self.assertTrue(archive_path.is_file())
        self.assertEqual(manifest["platform"], "macos-arm64")

        with tarfile.open(archive_path, "r:gz") as tar:
            members = {m.name: m for m in tar.getmembers()}
            self.assertEqual(
                set(members.keys()),
                {
                    "Curie Clinic.app/Contents/Info.plist",
                    "Curie Clinic.app/Contents/MacOS/ef-clinic-desktop",
                    "Curie Clinic.app/Contents/Resources/curie-clinic.svg",
                    "Curie Mobile.app/Contents/Info.plist",
                    "Curie Mobile.app/Contents/MacOS/ef-mobile-desktop",
                    "Curie Mobile.app/Contents/Resources/curie-mobile.svg",
                },
            )
            self.assertEqual(members["Curie Clinic.app/Contents/MacOS/ef-clinic-desktop"].mode & 0o777, 0o755)
            self.assertEqual(members["Curie Mobile.app/Contents/MacOS/ef-mobile-desktop"].mode & 0o777, 0o755)

            clinic_plist = plistlib.loads(tar.extractfile("Curie Clinic.app/Contents/Info.plist").read())
            mobile_plist = plistlib.loads(tar.extractfile("Curie Mobile.app/Contents/Info.plist").read())
            self.assertEqual(clinic_plist["CFBundleIdentifier"], "fields.elysian.curie.clinic")
            self.assertEqual(clinic_plist["CFBundleVersion"], "0.2.3")
            self.assertEqual(clinic_plist["CFBundleExecutable"], "ef-clinic-desktop")
            self.assertIs(clinic_plist["NSHighResolutionCapable"], True)
            self.assertEqual(clinic_plist["LSMinimumSystemVersion"], "13.0")

            self.assertEqual(mobile_plist["CFBundleIdentifier"], "fields.elysian.curie.mobile")
            self.assertEqual(mobile_plist["CFBundleVersion"], "0.2.3")
            self.assertEqual(mobile_plist["CFBundleExecutable"], "ef-mobile-desktop")
            self.assertIs(mobile_plist["NSHighResolutionCapable"], True)
            self.assertEqual(mobile_plist["LSMinimumSystemVersion"], "13.0")

        verified = package_desktop.verify_out_dir(out_dir)
        self.assertEqual(verified["status"], "verified")

    def test_package_windows_x86_64_zip_and_checksum_tamper_detection(self) -> None:
        out_dir = self.tmp_dir / "windows-out"
        manifest = package_desktop.package_desktop(
            platform="windows-x86_64",
            version="1.0.0-rc1",
            clinic_bin=self.clinic_bin,
            mobile_bin=self.mobile_bin,
            out_dir=out_dir,
        )
        archive_path = out_dir / "elysian-fields-desktop-1.0.0-rc1-windows-x86_64.zip"
        self.assertTrue(archive_path.is_file())
        self.assertEqual(manifest["archive"], archive_path.name)

        with zipfile.ZipFile(archive_path, "r") as zf:
            names = set(zf.namelist())
            self.assertEqual(
                names,
                {
                    "ef-clinic-desktop.exe",
                    "ef-mobile-desktop.exe",
                    "Launch-Curie-Clinic.cmd",
                    "Launch-Curie-Mobile.cmd",
                    "README-Windows.txt",
                },
            )
            self.assertIn("ef-clinic-desktop.exe", zf.read("Launch-Curie-Clinic.cmd").decode("utf-8"))
            self.assertIn("ef-mobile-desktop.exe", zf.read("Launch-Curie-Mobile.cmd").decode("utf-8"))
            self.assertIn("SHA256SUMS.txt", zf.read("README-Windows.txt").decode("utf-8"))

        # CLI --verify passes on intact output.
        cli_ok = subprocess.run(
            [sys.executable, str(REPO_ROOT / "scripts" / "package_desktop.py"), "--verify", "--out-dir", str(out_dir)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(cli_ok.returncode, 0, cli_ok.stderr)
        self.assertEqual(json.loads(cli_ok.stdout)["status"], "verified")

        # Tampering with the archive causes --verify to fail closed.
        archive_path.write_bytes(archive_path.read_bytes() + b"tampered")
        cli_bad = subprocess.run(
            [sys.executable, str(REPO_ROOT / "scripts" / "package_desktop.py"), "--verify", "--out-dir", str(out_dir)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(cli_bad.returncode, 0)
        self.assertIn("SHA-256 mismatch", cli_bad.stderr)

    def test_cross_platform_and_release_workflows_wire_desktop_packaging(self) -> None:
        cp_wf = (REPO_ROOT / ".github" / "workflows" / "cross-platform.yml").read_text(encoding="utf-8")
        for needle in (
            "scripts/package_desktop.py",
            "assets/desktop/**",
            "--platform linux-x86_64",
            "--platform macos-arm64",
            "--platform windows-x86_64",
            "--verify --out-dir",
            "SHA256SUMS.txt",
        ):
            self.assertIn(needle, cp_wf)

        rel_wf_path = REPO_ROOT / ".github" / "workflows" / "release.yml"
        self.assertTrue(rel_wf_path.is_file())
        rel_wf = rel_wf_path.read_text(encoding="utf-8")
        for needle in (
            "v*.*.*",
            "workflow_dispatch",
            "package-desktop-linux:",
            "package-desktop-macos:",
            "package-desktop-windows:",
            "package-wasm-web:",
            "publish-release:",
            "scripts/package_desktop.py",
            "SHA256SUMS.txt",
            "gh release",
            "github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')",
        ):
            self.assertIn(needle, rel_wf)
        self.assertNotRegex(rel_wf, r"TYL-[0-9]+")


if __name__ == "__main__":
    unittest.main()
