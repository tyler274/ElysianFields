#!/usr/bin/env python3
"""Pure-Python 3-OS desktop packager and checksum verifier for Elysian Fields.

Packages compiled native Bevy desktop binaries (`ef-clinic-desktop` and
`ef-mobile-desktop`) into platform-idiomatic release bundles with zero
third-party Python dependencies:
  - linux-x86_64:   .tar.gz with bin/, .desktop entries, SVG icons, and install.sh
  - macos-arm64:    .tar.gz with Curie Clinic.app and Curie Mobile.app + Info.plist
  - windows-x86_64: .zip with .exe binaries, .cmd launchers, and README-Windows.txt
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import plistlib
import re
import sys
import tarfile
import zipfile
from pathlib import Path
from typing import Any

SUPPORTED_PLATFORMS: tuple[str, ...] = (
    "linux-x86_64",
    "macos-arm64",
    "windows-x86_64",
)

VERSION_RE = re.compile(r"^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$")

CLINIC_DESKTOP_ENTRY = """\
[Desktop Entry]
Type=Application
Name=Curie Clinic
Comment=Elysian Fields Curie Clinic native desktop workspace
Exec=ef-clinic-desktop
Icon=curie-clinic
Terminal=false
Categories=MedicalSoftware;Office;
StartupNotify=true
"""

MOBILE_DESKTOP_ENTRY = """\
[Desktop Entry]
Type=Application
Name=Curie Mobile
Comment=Elysian Fields Curie Mobile personal health companion
Exec=ef-mobile-desktop
Icon=curie-mobile
Terminal=false
Categories=MedicalSoftware;Utility;
StartupNotify=true
"""

LINUX_INSTALL_SH = """\
#!/usr/bin/env bash
# User-local installer for Elysian Fields desktop applications (no root required).
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
prefix="${PREFIX:-$HOME/.local}"

mkdir -p "${prefix}/bin" "${prefix}/share/applications" "${prefix}/share/icons/hicolor/scalable/apps"

install -m 0755 "${script_dir}/bin/ef-clinic-desktop" "${prefix}/bin/ef-clinic-desktop"
install -m 0755 "${script_dir}/bin/ef-mobile-desktop" "${prefix}/bin/ef-mobile-desktop"
install -m 0644 "${script_dir}/share/applications/elysian-curie-clinic.desktop" "${prefix}/share/applications/elysian-curie-clinic.desktop"
install -m 0644 "${script_dir}/share/applications/elysian-curie-mobile.desktop" "${prefix}/share/applications/elysian-curie-mobile.desktop"
install -m 0644 "${script_dir}/share/icons/hicolor/scalable/apps/curie-clinic.svg" "${prefix}/share/icons/hicolor/scalable/apps/curie-clinic.svg"
install -m 0644 "${script_dir}/share/icons/hicolor/scalable/apps/curie-mobile.svg" "${prefix}/share/icons/hicolor/scalable/apps/curie-mobile.svg"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "${prefix}/share/applications" >/dev/null 2>&1 || true
fi

echo "Installed Curie Clinic and Curie Mobile into ${prefix}."
echo "Ensure ${prefix}/bin is on your PATH, then run: ef-clinic-desktop --demo"
"""

WINDOWS_LAUNCH_CLINIC_CMD = (
    "@echo off\r\n"
    "setlocal\r\n"
    "\"%~dp0ef-clinic-desktop.exe\" %*\r\n"
)

WINDOWS_LAUNCH_MOBILE_CMD = (
    "@echo off\r\n"
    "setlocal\r\n"
    "\"%~dp0ef-mobile-desktop.exe\" %*\r\n"
)

WINDOWS_README_TEMPLATE = """\
Elysian Fields Desktop ({version} — Windows x86_64)
===================================================

Contents:
  - ef-clinic-desktop.exe   Curie Clinic native desktop workspace
  - ef-mobile-desktop.exe   Curie Mobile personal health companion
  - Launch-Curie-Clinic.cmd Double-click launcher for Curie Clinic
  - Launch-Curie-Mobile.cmd Double-click launcher for Curie Mobile

Quick Start:
  1. Double-click Launch-Curie-Clinic.cmd or Launch-Curie-Mobile.cmd
     to launch in offline demo mode.
  2. Optional command-line flags:
       ef-clinic-desktop.exe --server http://127.0.0.1:8088 --locale en --theme dark
       ef-mobile-desktop.exe --server http://127.0.0.1:8089 --locale en --theme light
       ef-clinic-desktop.exe --headless-smoke
       ef-mobile-desktop.exe --headless-smoke

Integrity Verification:
  Verify this archive against SHA256SUMS.txt using PowerShell:
    Get-FileHash elysian-fields-desktop-{version}-windows-x86_64.zip -Algorithm SHA256
"""


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_version(version: str) -> str:
    cleaned = version.strip()
    if not cleaned or ".." in cleaned or not VERSION_RE.match(cleaned):
        raise ValueError(
            f"invalid version {version!r}; expected alphanumeric/dot/dash/underscore token"
        )
    return cleaned


def build_macos_info_plist(
    *,
    bundle_name: str,
    bundle_identifier: str,
    executable: str,
    icon_file: str,
    version: str,
) -> bytes:
    payload = {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleDisplayName": bundle_name,
        "CFBundleExecutable": executable,
        "CFBundleIconFile": icon_file,
        "CFBundleIdentifier": bundle_identifier,
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": bundle_name,
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "LSMinimumSystemVersion": "13.0",
        "NSHighResolutionCapable": True,
    }
    return plistlib.dumps(payload, fmt=plistlib.FMT_XML, sort_keys=True)


def _add_tar_bytes(
    tar: tarfile.TarFile,
    arcname: str,
    data: bytes,
    *,
    mode: int,
) -> None:
    info = tarfile.TarInfo(name=arcname)
    info.size = len(data)
    info.mode = mode
    info.mtime = 0
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    tar.addfile(info, io.BytesIO(data))


def _add_zip_bytes(
    zf: zipfile.ZipFile,
    arcname: str,
    data: bytes,
    *,
    mode: int = 0o644,
) -> None:
    info = zipfile.ZipInfo(filename=arcname, date_time=(1980, 1, 1, 0, 0, 0))
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = (mode & 0xFFFF) << 16
    zf.writestr(info, data)


def package_desktop(
    *,
    platform: str,
    version: str,
    clinic_bin: Path,
    mobile_bin: Path,
    out_dir: Path,
    icons_dir: Path | None = None,
) -> dict[str, Any]:
    if platform not in SUPPORTED_PLATFORMS:
        raise ValueError(f"unsupported platform {platform!r}; expected one of {SUPPORTED_PLATFORMS}")
    ver = validate_version(version)

    for label, bin_path in (("clinic-bin", clinic_bin), ("mobile-bin", mobile_bin)):
        if not bin_path.is_file():
            raise FileNotFoundError(f"missing {label}: {bin_path}")

    resolved_icons = icons_dir or (repo_root() / "assets" / "desktop")
    clinic_icon = resolved_icons / "curie-clinic.svg"
    mobile_icon = resolved_icons / "curie-mobile.svg"
    for icon in (clinic_icon, mobile_icon):
        if not icon.is_file():
            raise FileNotFoundError(f"missing desktop icon: {icon}")

    clinic_bytes = clinic_bin.read_bytes()
    mobile_bytes = mobile_bin.read_bytes()
    clinic_icon_bytes = clinic_icon.read_bytes()
    mobile_icon_bytes = mobile_icon.read_bytes()

    out_dir.mkdir(parents=True, exist_ok=True)
    entries: list[str] = []

    if platform == "linux-x86_64":
        archive_name = f"elysian-fields-desktop-{ver}-linux-x86_64.tar.gz"
        archive_path = out_dir / archive_name
        files: list[tuple[str, bytes, int]] = [
            ("bin/ef-clinic-desktop", clinic_bytes, 0o755),
            ("bin/ef-mobile-desktop", mobile_bytes, 0o755),
            ("install.sh", LINUX_INSTALL_SH.encode("utf-8"), 0o755),
            (
                "share/applications/elysian-curie-clinic.desktop",
                CLINIC_DESKTOP_ENTRY.encode("utf-8"),
                0o644,
            ),
            (
                "share/applications/elysian-curie-mobile.desktop",
                MOBILE_DESKTOP_ENTRY.encode("utf-8"),
                0o644,
            ),
            ("share/icons/hicolor/scalable/apps/curie-clinic.svg", clinic_icon_bytes, 0o644),
            ("share/icons/hicolor/scalable/apps/curie-mobile.svg", mobile_icon_bytes, 0o644),
        ]
        with tarfile.open(archive_path, mode="w:gz", format=tarfile.PAX_FORMAT) as tar:
            for arcname, payload, mode in sorted(files, key=lambda item: item[0]):
                _add_tar_bytes(tar, arcname, payload, mode=mode)
                entries.append(arcname)

    elif platform == "macos-arm64":
        archive_name = f"elysian-fields-desktop-{ver}-macos-arm64.tar.gz"
        archive_path = out_dir / archive_name
        clinic_plist = build_macos_info_plist(
            bundle_name="Curie Clinic",
            bundle_identifier="fields.elysian.curie.clinic",
            executable="ef-clinic-desktop",
            icon_file="curie-clinic.svg",
            version=ver,
        )
        mobile_plist = build_macos_info_plist(
            bundle_name="Curie Mobile",
            bundle_identifier="fields.elysian.curie.mobile",
            executable="ef-mobile-desktop",
            icon_file="curie-mobile.svg",
            version=ver,
        )
        files = [
            ("Curie Clinic.app/Contents/Info.plist", clinic_plist, 0o644),
            ("Curie Clinic.app/Contents/MacOS/ef-clinic-desktop", clinic_bytes, 0o755),
            ("Curie Clinic.app/Contents/Resources/curie-clinic.svg", clinic_icon_bytes, 0o644),
            ("Curie Mobile.app/Contents/Info.plist", mobile_plist, 0o644),
            ("Curie Mobile.app/Contents/MacOS/ef-mobile-desktop", mobile_bytes, 0o755),
            ("Curie Mobile.app/Contents/Resources/curie-mobile.svg", mobile_icon_bytes, 0o644),
        ]
        with tarfile.open(archive_path, mode="w:gz", format=tarfile.PAX_FORMAT) as tar:
            for arcname, payload, mode in sorted(files, key=lambda item: item[0]):
                _add_tar_bytes(tar, arcname, payload, mode=mode)
                entries.append(arcname)

    else:  # windows-x86_64
        archive_name = f"elysian-fields-desktop-{ver}-windows-x86_64.zip"
        archive_path = out_dir / archive_name
        readme_bytes = WINDOWS_README_TEMPLATE.format(version=ver).replace("\n", "\r\n").encode("utf-8")
        zip_files: list[tuple[str, bytes, int]] = [
            ("Launch-Curie-Clinic.cmd", WINDOWS_LAUNCH_CLINIC_CMD.encode("utf-8"), 0o755),
            ("Launch-Curie-Mobile.cmd", WINDOWS_LAUNCH_MOBILE_CMD.encode("utf-8"), 0o755),
            ("README-Windows.txt", readme_bytes, 0o644),
            ("ef-clinic-desktop.exe", clinic_bytes, 0o755),
            ("ef-mobile-desktop.exe", mobile_bytes, 0o755),
        ]
        with zipfile.ZipFile(archive_path, mode="w", compression=zipfile.ZIP_DEFLATED) as zf:
            for arcname, payload, mode in sorted(zip_files, key=lambda item: item[0]):
                _add_zip_bytes(zf, arcname, payload, mode=mode)
                entries.append(arcname)

    archive_sha256 = sha256_file(archive_path)
    archive_bytes = archive_path.stat().st_size

    checksums_path = out_dir / "SHA256SUMS.txt"
    checksums_path.write_text(f"{archive_sha256}  {archive_name}\n", encoding="utf-8")

    manifest: dict[str, Any] = {
        "platform": platform,
        "version": ver,
        "archive": archive_name,
        "bytes": archive_bytes,
        "sha256": archive_sha256,
        "apps": ["ef-clinic-desktop", "ef-mobile-desktop"],
        "entries": entries,
    }
    manifest_path = out_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest


def verify_out_dir(out_dir: Path) -> dict[str, Any]:
    checksums_path = out_dir / "SHA256SUMS.txt"
    manifest_path = out_dir / "manifest.json"
    if not checksums_path.is_file():
        raise FileNotFoundError(f"missing checksum file: {checksums_path}")
    if not manifest_path.is_file():
        raise FileNotFoundError(f"missing manifest file: {manifest_path}")

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    lines = [
        line.strip()
        for line in checksums_path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    if not lines:
        raise ValueError(f"empty checksum file: {checksums_path}")

    verified: dict[str, str] = {}
    for line in lines:
        parts = line.split(None, 1)
        if len(parts) != 2:
            raise ValueError(f"malformed SHA256SUMS.txt line: {line!r}")
        expected_hash, rel_name = parts[0].lower(), parts[1].lstrip("*").strip()
        target = out_dir / rel_name
        if not target.is_file():
            raise FileNotFoundError(f"archive listed in SHA256SUMS.txt not found: {target}")
        actual_hash = sha256_file(target)
        if actual_hash != expected_hash:
            raise ValueError(
                f"SHA-256 mismatch for {rel_name}: expected {expected_hash}, got {actual_hash}"
            )
        verified[rel_name] = actual_hash

    archive_name = manifest.get("archive")
    if archive_name not in verified:
        raise ValueError(f"manifest archive {archive_name!r} missing from SHA256SUMS.txt")
    if manifest.get("sha256") != verified[archive_name]:
        raise ValueError(
            f"manifest sha256 mismatch for {archive_name}: {manifest.get('sha256')} != {verified[archive_name]}"
        )
    actual_size = (out_dir / archive_name).stat().st_size
    if manifest.get("bytes") != actual_size:
        raise ValueError(
            f"manifest byte size mismatch for {archive_name}: {manifest.get('bytes')} != {actual_size}"
        )

    return {"status": "verified", "out_dir": str(out_dir), "verified_files": verified}


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="package_desktop.py",
        description="Package Elysian Fields native desktop binaries into OS-specific release bundles.",
    )
    parser.add_argument(
        "--verify",
        action="store_true",
        help="Verify SHA256SUMS.txt and manifest.json inside --out-dir instead of packaging.",
    )
    parser.add_argument(
        "--platform",
        choices=SUPPORTED_PLATFORMS,
        help="Target desktop platform (required unless --verify is passed).",
    )
    parser.add_argument(
        "--version",
        help="Release version tag without leading 'v' (e.g. 0.1.0).",
    )
    parser.add_argument("--clinic-bin", type=Path, help="Path to compiled ef-clinic-desktop binary.")
    parser.add_argument("--mobile-bin", type=Path, help="Path to compiled ef-mobile-desktop binary.")
    parser.add_argument(
        "--out-dir",
        type=Path,
        required=True,
        help="Output directory for the packaged archive, SHA256SUMS.txt, and manifest.json.",
    )
    parser.add_argument(
        "--icons-dir",
        type=Path,
        default=None,
        help="Optional override directory containing curie-clinic.svg and curie-mobile.svg.",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        if args.verify:
            result = verify_out_dir(args.out_dir)
            print(json.dumps(result, indent=2))
            return 0

        missing = [
            flag
            for flag, value in (
                ("--platform", args.platform),
                ("--version", args.version),
                ("--clinic-bin", args.clinic_bin),
                ("--mobile-bin", args.mobile_bin),
            )
            if value is None
        ]
        if missing:
            parser.error(f"missing required packaging argument(s): {', '.join(missing)}")

        manifest = package_desktop(
            platform=args.platform,
            version=args.version,
            clinic_bin=args.clinic_bin,
            mobile_bin=args.mobile_bin,
            out_dir=args.out_dir,
            icons_dir=args.icons_dir,
        )
        print(json.dumps(manifest, indent=2))
        return 0
    except Exception as exc:
        print(f"package_desktop.py error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
