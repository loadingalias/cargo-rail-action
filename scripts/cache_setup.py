#!/usr/bin/env python3
"""Configure Cargo-Rail remote cache policy and publish a redacted local status."""

from __future__ import annotations

import argparse
import html
import json
import pathlib
import re
import subprocess
import sys
from typing import Any

PROJECTION_VERSION = 1
AUTHORITY = re.compile(r"remote-authority-v1-sha256-[0-9a-f]{64}")
SEMVER = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-.][0-9A-Za-z.-]+)?")
INSTALL_METHODS = {"binary", "binstall", "cargo-install", "cached"}
PROVIDERS = {"aws-s3", "azure-blob", "cloudflare-r2", "s3-compatible"}
MAX_STATUS_BYTES = 1024 * 1024
MAX_URL_BYTES = 4 * 1024
MAX_PATH_BYTES = 4 * 1024


class SetupError(RuntimeError):
    """The requested cache policy or resulting installation is invalid."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SetupError(message)


def safe_inline(value: Any) -> str:
    return html.escape(str(value), quote=True).replace("|", "&#124;").replace("\r", " ").replace("\n", " ")


def append_output(path: pathlib.Path, name: str, value: str) -> None:
    require("\n" not in value and "\r" not in value, f"output {name} is not single-line")
    with path.open("a", encoding="utf-8", newline="\n") as handle:
        handle.write(f"{name}={value}\n")


def object_field(value: Any, field: str, subject: str) -> dict[str, Any]:
    selected = value.get(field) if isinstance(value, dict) else None
    require(isinstance(selected, dict), f"{subject}.{field} is missing or invalid")
    return selected


def string_field(value: dict[str, Any], field: str, subject: str) -> str:
    selected = value.get(field)
    require(isinstance(selected, str) and selected, f"{subject}.{field} is missing or invalid")
    return selected


def project_status(document: Any, install_method: str, install_version: str, requested_mode: str) -> dict[str, Any]:
    status = object_field(document, "status", "cache status")
    require(isinstance(status.get("schema_version"), int), "cache status schema_version is missing or invalid")
    installation = object_field(status, "installation", "cache status")
    state = string_field(installation, "state", "cache status.installation")
    healthy = installation.get("healthy")
    max_bytes = installation.get("max_bytes")
    require(isinstance(healthy, bool), "cache status.installation.healthy is missing or invalid")
    require(
        isinstance(max_bytes, int) and not isinstance(max_bytes, bool) and 0 < max_bytes <= 2**64 - 1,
        "cache status.installation.max_bytes is missing or invalid",
    )
    require(healthy, "Cargo-Rail cache installation is unhealthy after setup")
    require(state == "installed", "Cargo-Rail cache installation is not installed after setup")

    remote = object_field(status, "remote", "cache status")
    provider = string_field(remote, "provider", "cache status.remote")
    authority = string_field(remote, "authority", "cache status.remote")
    mode = string_field(remote, "mode", "cache status.remote")
    activation = string_field(remote, "activation", "cache status.remote")
    require(provider in PROVIDERS, "cache status.remote.provider is unsupported")
    require(AUTHORITY.fullmatch(authority) is not None, "cache status.remote.authority is not a redacted authority identity")
    require(mode == requested_mode, f"cache status remote mode is {mode!r}, expected {requested_mode!r}")
    require(activation == "direct_transport_selected", "Cargo-Rail remote cache transport is not active after setup")

    return {
        "cache_action_status_version": PROJECTION_VERSION,
        "cargo_rail_version": install_version,
        "install_method": install_method,
        "installation": {"state": state, "healthy": healthy, "max_bytes": max_bytes},
        "remote": {"provider": provider, "authority": authority, "mode": mode, "activation": activation},
    }


def human_bytes(value: int) -> str:
    units = ["B", "KiB", "MiB", "GiB", "TiB"]
    selected = float(value)
    for unit in units:
        if selected < 1024 or unit == units[-1]:
            return f"{selected:.0f} {unit}" if unit == "B" else f"{selected:.1f} {unit}"
        selected /= 1024
    raise AssertionError("unreachable")


def render_summary(projection: dict[str, Any]) -> str:
    installation = projection["installation"]
    remote = projection["remote"]
    return "\n".join(
        [
            "## Cargo-Rail cache",
            "",
            "| | |",
            "|---|---|",
            f"| Cargo-Rail | `{safe_inline(projection['cargo_rail_version'])}` via `{safe_inline(projection['install_method'])}` |",
            f"| Installation | `{safe_inline(installation['state'])}`; healthy: `{str(installation['healthy']).lower()}` |",
            f"| Local L1 bound | {human_bytes(installation['max_bytes'])} |",
            f"| Remote provider | `{safe_inline(remote['provider'])}` |",
            f"| Remote authority | `{safe_inline(remote['authority'])}` |",
            f"| Remote mode | `{safe_inline(remote['mode'])}` |",
            f"| Activation | `{safe_inline(remote['activation'])}` |",
            "",
            "Status inspection was local and did not contact the remote provider.",
            "",
        ]
    )


def run(arguments: argparse.Namespace) -> None:
    require(arguments.mode in {"read", "read-write"}, "mode must be explicitly set to read or read-write")
    require(arguments.install_method in INSTALL_METHODS, "install-method is invalid")
    require(SEMVER.fullmatch(arguments.install_version) is not None, "install-version must be an exact semantic version")
    require(arguments.url and len(arguments.url.encode("utf-8")) <= MAX_URL_BYTES, "url must be at most 4 KiB")
    require("\n" not in arguments.url and "\r" not in arguments.url, "url must be single-line")
    require(
        arguments.max_size
        and len(arguments.max_size.encode("utf-8")) <= 64
        and "\n" not in arguments.max_size
        and "\r" not in arguments.max_size,
        "max-size must be one bounded non-empty value",
    )

    setup = [
        "cargo",
        "rail",
        "cache",
        "setup",
        "--remote",
        arguments.url,
        "--remote-mode",
        arguments.mode,
        "--max-size",
        arguments.max_size,
    ]
    if arguments.local_dir:
        require(
            len(arguments.local_dir.encode("utf-8")) <= MAX_PATH_BYTES
            and "\n" not in arguments.local_dir
            and "\r" not in arguments.local_dir,
            "local-dir must be one path no longer than 4 KiB",
        )
        setup.extend(["--local-dir", arguments.local_dir])
    completed = subprocess.run(setup, check=False)
    require(completed.returncode == 0, f"cargo rail cache setup failed with exit code {completed.returncode}")

    completed = subprocess.run(
        ["cargo", "rail", "cache", "status", "--scope", "local", "-f", "json"],
        check=False,
        stdout=subprocess.PIPE,
    )
    require(completed.returncode == 0, f"cargo rail cache status failed with exit code {completed.returncode}")
    require(len(completed.stdout) <= MAX_STATUS_BYTES, "cargo rail cache status exceeded the 1 MiB input bound")
    try:
        document = json.loads(completed.stdout.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise SetupError(f"cargo rail cache status returned invalid JSON: {error}") from error
    projection = project_status(document, arguments.install_method, arguments.install_version, arguments.mode)
    compact = json.dumps(projection, separators=(",", ":"), sort_keys=True)
    outputs = {
        "status_json": compact,
        "healthy": str(projection["installation"]["healthy"]).lower(),
        "provider": projection["remote"]["provider"],
        "mode": projection["remote"]["mode"],
        "activation": projection["remote"]["activation"],
        "max_bytes": str(projection["installation"]["max_bytes"]),
    }
    for name, value in outputs.items():
        append_output(arguments.github_output, name, value)
    with arguments.summary.open("a", encoding="utf-8", newline="\n") as handle:
        handle.write(render_summary(projection))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True)
    parser.add_argument("--mode", required=True)
    parser.add_argument("--max-size", required=True)
    parser.add_argument("--local-dir", default="")
    parser.add_argument("--install-method", required=True)
    parser.add_argument("--install-version", required=True)
    parser.add_argument("--github-output", required=True, type=pathlib.Path)
    parser.add_argument("--summary", required=True, type=pathlib.Path)
    return parser.parse_args()


if __name__ == "__main__":
    try:
        run(parse_args())
    except (OSError, SetupError) as error:
        print(f"::error::{error}", file=sys.stderr)
        raise SystemExit(1) from error
