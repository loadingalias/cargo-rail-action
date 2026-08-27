#!/usr/bin/env python3
"""Check whether the release-train Cargo-Rail assets are available."""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys

SEMVER = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+")
REQUIRED_ASSETS = {
    "SHA256SUMS",
    "cargo-rail-x86_64-pc-windows-msvc.zip",
    "cargo-rail-x86_64-unknown-linux-gnu.tar.gz",
}


class ReleaseError(RuntimeError):
    """The release train or GitHub release state is invalid."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ReleaseError(message)


def release_version(path: pathlib.Path) -> str:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ReleaseError(f"cannot read release train: {error}") from error
    require(
        isinstance(document, dict)
        and set(document) == {"schema_version", "cargo_rail_version"}
        and document["schema_version"] == 1,
        "release-train.json has unknown or missing fields",
    )
    version = document["cargo_rail_version"]
    require(isinstance(version, str) and SEMVER.fullmatch(version) is not None, "Cargo-Rail version is invalid")
    return version


def parse_required(value: str) -> bool:
    require(value in {"", "false", "true"}, "require-release must be true or false")
    return value == "true"


def release_assets(version: str) -> set[str] | None:
    result = subprocess.run(
        [
            "gh",
            "api",
            f"repos/loadingalias/cargo-rail/releases/tags/v{version}",
            "--jq",
            ".assets[].name",
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode == 0:
        return {line for line in result.stdout.splitlines() if line}
    if "HTTP 404" in result.stderr:
        return None
    detail = result.stderr.strip()
    suffix = f": {detail}" if detail else ""
    raise ReleaseError(f"cannot inspect Cargo-Rail v{version} release assets{suffix}")


def append_output(path: pathlib.Path, available: bool) -> None:
    try:
        with path.open("a", encoding="utf-8", newline="\n") as handle:
            handle.write(f"available={str(available).lower()}\n")
    except OSError as error:
        raise ReleaseError(f"cannot publish release availability: {error}") from error


def run(arguments: argparse.Namespace) -> None:
    version = release_version(arguments.release_train)
    required = parse_required(arguments.require)
    assets = release_assets(version)
    missing = REQUIRED_ASSETS if assets is None else REQUIRED_ASSETS - assets
    available = not missing
    append_output(arguments.github_output, available)
    if available:
        print(f"Cargo-Rail v{version} release integration is available.")
        return

    detail = "release is absent" if assets is None else "missing " + ", ".join(sorted(missing))
    print(
        f"::notice::Cargo-Rail v{version} {detail}; release-dependent Action jobs are deferred.",
        file=sys.stderr,
    )
    if required:
        raise ReleaseError(f"Cargo-Rail v{version} with every required native asset must exist before Action release")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-train", required=True, type=pathlib.Path)
    parser.add_argument("--require", default="")
    parser.add_argument("--github-output", required=True, type=pathlib.Path)
    return parser.parse_args()


if __name__ == "__main__":
    try:
        run(parse_args())
    except (OSError, ReleaseError) as error:
        print(f"::error::{error}", file=sys.stderr)
        raise SystemExit(1) from error
