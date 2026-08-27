#!/usr/bin/env python3
"""Project the Cargo-Rail release-train manifest into action-owned defaults."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

DEFAULT_ROOT = Path(__file__).resolve().parent.parent
SEMVER = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$")
SEMVER_TEXT = r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?"


def replace_owned(text: str, pattern: str, replacement: str, expected_count: int, path: Path) -> str:
    text, count = re.subn(pattern, replacement, text)
    if count != expected_count:
        raise ValueError(f"{path}: expected {expected_count} release-train projections, found {count}")
    return text


def projected(path: Path, version: str) -> str:
    text = path.read_text()
    relative = path.relative_to(DEFAULT_ROOT if path.is_relative_to(DEFAULT_ROOT) else path.parent)
    if path.name == "action.yaml":
        return replace_owned(
            text,
            r'(?m)^(  version:\n(?:    .*\n)*?    default: ")[^"]+("$)',
            rf"\g<1>{version}\g<2>",
            1,
            path,
        )
    if path.name == "README.md":
        text = replace_owned(text, rf"(?m)^(\s+version: ){SEMVER_TEXT}$", rf"\g<1>{version}", 2, path)
        return replace_owned(
            text,
            rf"(?m)^(\| `version` \| `){SEMVER_TEXT}(` \|)",
            rf"\g<1>{version}\g<2>",
            2,
            path,
        )
    if path.suffix == ".json":
        return replace_owned(
            text,
            rf'(?m)^(\s*"cargo_rail_version": "){SEMVER_TEXT}("[, ]*$)',
            rf"\g<1>{version}\g<2>",
            1,
            path,
        )
    if path.suffix == ".md":
        return replace_owned(
            text,
            rf"(?m)^(\| \*\*Version\*\* \| `){SEMVER_TEXT}(` \|)$",
            rf"\g<1>{version}\g<2>",
            1,
            path,
        )
    raise ValueError(f"{relative}: unsupported release-train projection")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--cargo-rail-version")
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    args = parser.parse_args()
    root = args.root.resolve()
    manifest_path = root / "release-train.json"

    manifest = json.loads(manifest_path.read_text())
    if manifest != {
        "schema_version": 1,
        "cargo_rail_version": manifest.get("cargo_rail_version"),
    }:
        raise ValueError("release-train.json has unknown or missing fields")
    version = args.cargo_rail_version or manifest["cargo_rail_version"]
    if not isinstance(version, str) or not SEMVER.fullmatch(version):
        raise ValueError("Cargo-Rail release-train version must be semantic")

    if args.cargo_rail_version:
        manifest["cargo_rail_version"] = version
        rendered_manifest = json.dumps(manifest, indent=2) + "\n"
        if args.check and manifest_path.read_text() != rendered_manifest:
            print(f"out of date: {manifest_path.relative_to(root)}", file=sys.stderr)
            return 1
        if not args.check:
            manifest_path.write_text(rendered_manifest)

    stale = []
    for relative in (
        "action.yaml",
        "cache/action.yaml",
        "README.md",
    ):
        path = root / relative
        expected = projected(path, version)
        if path.read_text() != expected:
            stale.append(relative)
            if not args.check:
                path.write_text(expected)
    if stale and args.check:
        print("release-train projections are stale: " + ", ".join(stale), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
