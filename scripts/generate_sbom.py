#!/usr/bin/env python3
"""Deterministic CycloneDX 1.5 SBOM generator and verifier for Elysian Fields.

Zero-dependency pure Python (3.11+) tool that parses `Cargo.lock` and workspace
`Cargo.toml` manifests to emit canonical, byte-for-byte deterministic CycloneDX 1.5
JSON and validate generated SBOMs against schema, purl, hash, and zero-secret/PHI
invariants.

Usage:
    python3 scripts/generate_sbom.py --version 0.1.0 --out dist/elysian-fields-0.1.0-sbom.cdx.json
    python3 scripts/generate_sbom.py --verify --out dist/elysian-fields-0.1.0-sbom.cdx.json
    python3 scripts/generate_sbom.py --verify
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tomllib
import uuid
from pathlib import Path
from typing import Any
from urllib.parse import quote

PURL_RE = re.compile(r"^pkg:cargo/[A-Za-z0-9._-]+@[A-Za-z0-9._+-]+$")
UUID_URN_RE = re.compile(
    r"^urn:uuid:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
)
SHA256_HEX_RE = re.compile(r"^[0-9a-f]{64}$")

FORBIDDEN_SBOM_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("private key material", re.compile(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----")),
    ("GitHub token", re.compile(r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}\b")),
    ("AWS access key", re.compile(r"\bAKIA[0-9A-Z]{16}\b")),
    ("Linear ticket ID", re.compile(r"\bTYL-[0-9]+\b")),
    ("SSN pattern", re.compile(r"\b[0-9]{3}-[0-9]{2}-[0-9]{4}\b")),
    (
        "email address",
        re.compile(r"(?<!pkg:cargo/)\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"),
    ),
)


def _load_workspace_metadata(repo_root: Path) -> tuple[str, str | None, dict[str, dict[str, Any]]]:
    root_manifest_path = repo_root / "Cargo.toml"
    if not root_manifest_path.is_file():
        raise FileNotFoundError(f"missing root Cargo.toml at {root_manifest_path}")

    root_data = tomllib.loads(root_manifest_path.read_text(encoding="utf-8"))
    workspace_table = root_data.get("workspace", {})
    ws_pkg = workspace_table.get("package", {})
    ws_version = str(ws_pkg.get("version", "0.1.0"))
    ws_license = ws_pkg.get("license")
    if isinstance(ws_license, str):
        ws_license = ws_license.strip() or None
    else:
        ws_license = None

    members: list[str] = list(workspace_table.get("members", []))
    crate_dirs: list[Path] = []
    seen_dirs: set[Path] = set()
    for member in members:
        candidate = (repo_root / member).resolve()
        if candidate.is_dir() and candidate not in seen_dirs:
            seen_dirs.add(candidate)
            crate_dirs.append(candidate)

    crates_root = repo_root / "crates"
    if crates_root.is_dir():
        for manifest in sorted(crates_root.glob("*/Cargo.toml")):
            parent = manifest.parent.resolve()
            if parent not in seen_dirs:
                seen_dirs.add(parent)
                crate_dirs.append(parent)

    workspace_crates: dict[str, dict[str, Any]] = {}
    for crate_dir in sorted(crate_dirs):
        manifest_path = crate_dir / "Cargo.toml"
        if not manifest_path.is_file():
            continue
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        pkg = manifest.get("package")
        if not isinstance(pkg, dict) or "name" not in pkg:
            continue
        name = str(pkg["name"])
        raw_ver = pkg.get("version")
        if isinstance(raw_ver, str):
            version = raw_ver
        elif isinstance(raw_ver, dict) and raw_ver.get("workspace") is True:
            version = ws_version
        else:
            version = ws_version

        raw_lic = pkg.get("license")
        if isinstance(raw_lic, str):
            license_id: str | None = raw_lic.strip() or None
        elif isinstance(raw_lic, dict) and raw_lic.get("workspace") is True:
            license_id = ws_license
        else:
            license_id = None

        has_bin = bool(manifest.get("bin")) or (crate_dir / "src" / "main.rs").is_file()
        has_lib = bool(manifest.get("lib")) or (crate_dir / "src" / "lib.rs").is_file()
        component_type = "application" if (has_bin and not has_lib) else "library"

        rel_manifest = manifest_path.relative_to(repo_root.resolve()).as_posix()
        workspace_crates[name] = {
            "version": version,
            "license": license_id,
            "manifest": rel_manifest,
            "type": component_type,
        }

    return ws_version, ws_license, workspace_crates


def _cargo_purl(name: str, version: str) -> str:
    return f"pkg:cargo/{quote(name, safe='._-')}@{quote(version, safe='._+-')}"


def _resolve_dep_ref(
    raw_dep: str,
    packages_by_name: dict[str, list[tuple[str, str]]],
) -> str | None:
    parts = raw_dep.strip().split()
    if not parts:
        return None
    dep_name = parts[0]
    candidates = packages_by_name.get(dep_name, [])
    if not candidates:
        return None
    if len(parts) >= 2:
        dep_ver = parts[1]
        for ver, bom_ref in candidates:
            if ver == dep_ver:
                return bom_ref
    return candidates[0][1]


def generate_sbom(repo_root: Path, release_version: str | None = None) -> dict[str, Any]:
    repo_root = repo_root.resolve()
    lock_path = repo_root / "Cargo.lock"
    if not lock_path.is_file():
        raise FileNotFoundError(f"missing Cargo.lock at {lock_path}")

    lock_bytes = lock_path.read_bytes()
    lock_sha256 = hashlib.sha256(lock_bytes).hexdigest()
    lock_data = tomllib.loads(lock_bytes.decode("utf-8"))
    packages = lock_data.get("package", [])
    if not isinstance(packages, list) or not packages:
        raise ValueError("Cargo.lock contains no [[package]] entries")

    ws_version, ws_license, workspace_crates = _load_workspace_metadata(repo_root)
    effective_version = (release_version or ws_version).strip()
    if not effective_version:
        effective_version = ws_version

    serial_uuid = uuid.uuid5(
        uuid.NAMESPACE_URL,
        f"https://elysianfields.local/cyclonedx/cargo-lock/{lock_sha256}",
    )
    serial_number = f"urn:uuid:{serial_uuid}"

    root_bom_ref = _cargo_purl("elysian-fields", effective_version)
    root_component: dict[str, Any] = {
        "type": "application",
        "bom-ref": root_bom_ref,
        "name": "elysian-fields",
        "version": effective_version,
        "purl": root_bom_ref,
    }
    if ws_license:
        root_component["licenses"] = [{"license": {"id": ws_license}}]

    components: list[dict[str, Any]] = []
    packages_by_name: dict[str, list[tuple[str, str]]] = {}
    raw_entries: list[tuple[str, dict[str, Any]]] = []
    workspace_refs: list[str] = []

    sorted_packages = sorted(
        packages,
        key=lambda pkg: (
            str(pkg.get("name", "")),
            str(pkg.get("version", "")),
            str(pkg.get("source", "")),
        ),
    )

    seen_refs: dict[str, int] = {}
    for pkg in sorted_packages:
        name = str(pkg["name"])
        version = str(pkg["version"])
        source = pkg.get("source")
        checksum = pkg.get("checksum")

        purl = _cargo_purl(name, version)
        count = seen_refs.get(purl, 0)
        seen_refs[purl] = count + 1
        bom_ref = purl if count == 0 else f"{purl}#{count}"

        is_workspace = source is None and name in workspace_crates
        ws_meta = workspace_crates.get(name) if is_workspace else None

        comp: dict[str, Any] = {
            "type": ws_meta["type"] if ws_meta else "library",
            "bom-ref": bom_ref,
            "name": name,
            "version": version,
            "scope": "required",
            "purl": purl,
        }

        if isinstance(checksum, str) and checksum.strip():
            comp["hashes"] = [{"alg": "SHA-256", "content": checksum.strip().lower()}]

        if ws_meta and ws_meta.get("license"):
            comp["licenses"] = [{"license": {"id": ws_meta["license"]}}]

        props: list[dict[str, str]] = [
            {"name": "elysian:origin", "value": "workspace" if is_workspace else "external"}
        ]
        if ws_meta:
            props.append({"name": "elysian:manifest", "value": ws_meta["manifest"]})
        elif isinstance(source, str) and source.strip():
            props.append({"name": "cargo:source", "value": source.strip()})
        props.sort(key=lambda item: (item["name"], item["value"]))
        comp["properties"] = props

        components.append(comp)
        packages_by_name.setdefault(name, []).append((version, bom_ref))
        raw_entries.append((bom_ref, pkg))
        if is_workspace:
            workspace_refs.append(bom_ref)

    components.sort(key=lambda item: (item["name"], item["version"], item["bom-ref"]))

    dependencies: list[dict[str, Any]] = [
        {
            "ref": root_bom_ref,
            "dependsOn": sorted(set(workspace_refs)),
        }
    ]
    for bom_ref, pkg in raw_entries:
        dep_refs: set[str] = set()
        for raw_dep in pkg.get("dependencies", []):
            if isinstance(raw_dep, str):
                resolved = _resolve_dep_ref(raw_dep, packages_by_name)
                if resolved:
                    dep_refs.add(resolved)
        entry: dict[str, Any] = {"ref": bom_ref}
        if dep_refs:
            entry["dependsOn"] = sorted(dep_refs)
        dependencies.append(entry)

    dependencies.sort(key=lambda item: item["ref"])

    return {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": serial_number,
        "version": 1,
        "metadata": {
            "component": root_component,
            "tools": {
                "components": [
                    {
                        "type": "application",
                        "name": "elysian-fields-sbom-generator",
                        "version": "1.0.0",
                    }
                ]
            },
            "properties": [
                {"name": "elysian:cargo-lock-sha256", "value": lock_sha256},
            ],
        },
        "components": components,
        "dependencies": dependencies,
    }


def render_sbom_json(sbom: dict[str, Any]) -> str:
    return json.dumps(sbom, indent=2, ensure_ascii=False) + "\n"


def verify_sbom(sbom: dict[str, Any]) -> list[str]:
    errors: list[str] = []

    if not isinstance(sbom, dict):
        return ["SBOM root must be a JSON object"]

    if sbom.get("bomFormat") != "CycloneDX":
        errors.append(f"expected bomFormat 'CycloneDX', got {sbom.get('bomFormat')!r}")
    if sbom.get("specVersion") != "1.5":
        errors.append(f"expected specVersion '1.5', got {sbom.get('specVersion')!r}")

    serial = sbom.get("serialNumber")
    if not isinstance(serial, str) or not UUID_URN_RE.match(serial):
        errors.append(f"invalid deterministic serialNumber: {serial!r}")

    if sbom.get("version") != 1:
        errors.append(f"expected integer version 1, got {sbom.get('version')!r}")

    metadata = sbom.get("metadata")
    if not isinstance(metadata, dict):
        errors.append("missing metadata object")
    else:
        root_comp = metadata.get("component")
        if not isinstance(root_comp, dict):
            errors.append("missing metadata.component object")
        else:
            root_purl = root_comp.get("purl")
            if not isinstance(root_purl, str) or not PURL_RE.match(root_purl):
                errors.append(f"invalid metadata.component.purl: {root_purl!r}")

    components = sbom.get("components")
    if not isinstance(components, list) or not components:
        errors.append("components list must be non-empty")
        components = []

    seen_bom_refs: set[str] = set()
    sort_keys: list[tuple[str, str, str]] = []
    workspace_count = 0
    external_count = 0

    for idx, comp in enumerate(components):
        if not isinstance(comp, dict):
            errors.append(f"component[{idx}] is not an object")
            continue
        name = comp.get("name")
        version = comp.get("version")
        purl = comp.get("purl")
        bom_ref = comp.get("bom-ref")

        if not isinstance(name, str) or not name:
            errors.append(f"component[{idx}] missing valid name")
            continue
        if not isinstance(version, str) or not version:
            errors.append(f"component[{idx}] ({name}) missing valid version")
            continue
        if not isinstance(purl, str) or not PURL_RE.match(purl):
            errors.append(f"component[{idx}] ({name}@{version}) invalid purl: {purl!r}")
        else:
            expected_purl = _cargo_purl(name, version)
            if purl != expected_purl:
                errors.append(
                    f"component[{idx}] purl mismatch: got {purl!r}, expected {expected_purl!r}"
                )

        if not isinstance(bom_ref, str) or not bom_ref:
            errors.append(f"component[{idx}] ({name}) missing bom-ref")
        elif bom_ref in seen_bom_refs:
            errors.append(f"duplicate component bom-ref: {bom_ref!r}")
        else:
            seen_bom_refs.add(bom_ref)
            sort_keys.append((name, version, bom_ref))

        props = {
            p.get("name"): p.get("value")
            for p in comp.get("properties", [])
            if isinstance(p, dict)
        }
        origin = props.get("elysian:origin")
        if origin == "workspace":
            workspace_count += 1
        elif origin == "external":
            external_count += 1
        else:
            errors.append(f"component {name}@{version} missing valid elysian:origin property")

        hashes = comp.get("hashes", [])
        if hashes:
            if not isinstance(hashes, list):
                errors.append(f"component {name}@{version} hashes must be a list")
            else:
                for h in hashes:
                    if (
                        not isinstance(h, dict)
                        or h.get("alg") != "SHA-256"
                        or not isinstance(h.get("content"), str)
                        or not SHA256_HEX_RE.match(h["content"])
                    ):
                        errors.append(f"component {name}@{version} has invalid SHA-256 hash: {h!r}")
        elif origin == "external" and str(props.get("cargo:source", "")).startswith("registry+"):
            errors.append(f"external registry component {name}@{version} missing SHA-256 checksum hash")

    if sort_keys != sorted(sort_keys):
        errors.append("components are not in canonical sorted order")
    if workspace_count == 0:
        errors.append("SBOM must include at least one workspace component")
    if external_count == 0:
        errors.append("SBOM must include at least one external dependency component")

    rendered = render_sbom_json(sbom)
    for label, pattern in FORBIDDEN_SBOM_PATTERNS:
        match = pattern.search(rendered)
        if match is not None:
            errors.append(f"forbidden {label} pattern detected in SBOM: {match.group(0)!r}")

    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Generate or verify a deterministic CycloneDX 1.5 SBOM from Cargo.lock."
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="Repository root directory (default: parent of scripts/).",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help="Path to write the SBOM JSON (or path to read when --verify is passed).",
    )
    parser.add_argument(
        "--version",
        dest="release_version",
        default=None,
        help="Release version string for the root application component.",
    )
    parser.add_argument(
        "--verify",
        action="store_true",
        help="Verify the SBOM schema, components, purls, hashes, and zero secrets/PHI.",
    )
    args = parser.parse_args(argv)

    if args.verify:
        if args.out is not None:
            if not args.out.is_file():
                print(f"SBOM verification FAILED: missing file {args.out}", file=sys.stderr)
                return 1
            try:
                sbom = json.loads(args.out.read_text(encoding="utf-8"))
            except json.JSONDecodeError as exc:
                print(f"SBOM verification FAILED: invalid JSON ({exc})", file=sys.stderr)
                return 1
        else:
            sbom = generate_sbom(args.repo_root, release_version=args.release_version)

        errors = verify_sbom(sbom)
        if errors:
            print(f"SBOM verification FAILED with {len(errors)} error(s):", file=sys.stderr)
            for err in errors:
                print(f"  - {err}", file=sys.stderr)
            return 1

        comps = sbom.get("components", [])
        ws_n = sum(
            1
            for c in comps
            if any(
                p.get("name") == "elysian:origin" and p.get("value") == "workspace"
                for p in c.get("properties", [])
            )
        )
        ext_n = len(comps) - ws_n
        print(
            f"CycloneDX 1.5 SBOM verified OK: {len(comps)} components "
            f"({ws_n} workspace, {ext_n} external), serialNumber={sbom['serialNumber']}"
        )
        return 0

    sbom = generate_sbom(args.repo_root, release_version=args.release_version)
    rendered = render_sbom_json(sbom)
    if args.out is not None:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(rendered, encoding="utf-8")
        print(
            f"Wrote CycloneDX 1.5 SBOM to {args.out} "
            f"({len(sbom['components'])} components, serialNumber={sbom['serialNumber']})"
        )
    else:
        sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
