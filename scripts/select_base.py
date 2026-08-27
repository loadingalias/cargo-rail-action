#!/usr/bin/env python3
"""Select one safe Cargo-Rail comparison base from GitHub event state."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys

SHA = re.compile(r"[0-9a-fA-F]{40,64}")
MAX_EVENT_BYTES = 4 * 1024 * 1024


def fail(message: str) -> None:
    raise SystemExit(f"::error::{message}")


def append_output(path: pathlib.Path, name: str, value: str) -> None:
    if "\n" in value or "\r" in value:
        fail(f"selected {name} is not single-line")
    with path.open("a", encoding="utf-8", newline="\n") as handle:
        handle.write(f"{name}={value}\n")


def validate_ref(value: str, source: str) -> str:
    if not value or value != value.strip() or "\n" in value or "\r" in value:
        fail(f"{source} must be one non-empty Git ref")
    if value.startswith("-"):
        fail(f"{source} must not start with '-'")
    return value


def push_before() -> str | None:
    if os.environ.get("GITHUB_EVENT_NAME") != "push":
        return None
    raw_path = os.environ.get("GITHUB_EVENT_PATH")
    if not raw_path:
        fail("GITHUB_EVENT_PATH is missing for a push event")
    path = pathlib.Path(raw_path)
    try:
        if path.stat().st_size > MAX_EVENT_BYTES:
            fail("GitHub push event exceeds the 4 MiB input bound")
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read the GitHub push event: {error}")
    before = value.get("before") if isinstance(value, dict) else None
    if not isinstance(before, str) or SHA.fullmatch(before) is None:
        fail("GitHub push event has no valid previous commit SHA")
    return before


def git(*arguments: str) -> str | None:
    result = subprocess.run(
        ["git", *arguments],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def select(explicit: str, force_all: str) -> tuple[str, bool, bool, str]:
    if force_all not in {"true", "false"}:
        fail("all must be true or false")
    if force_all == "true":
        return "", True, False, "explicit all-work override"
    if explicit:
        selected = validate_ref(explicit, "since")
        zero_sha = SHA.fullmatch(selected) is not None and set(selected) == {"0"}
        return (
            ("", True, False, "explicit all-zero SHA")
            if zero_sha
            else (selected, False, False, "explicit input")
        )

    before = push_before()
    if before is not None:
        return (
            ("", True, False, "new or rewritten push")
            if set(before) == {"0"}
            else (before, False, False, "push event")
        )

    pull_request_base = os.environ.get("GITHUB_BASE_REF", "")
    if pull_request_base:
        return f"origin/{validate_ref(pull_request_base, 'pull-request base')}", False, True, "pull request merge base"

    remote_default = git("symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD")
    if remote_default:
        return validate_ref(remote_default, "origin default branch"), False, True, "origin default branch merge base"
    for candidate in ("origin/main", "origin/master"):
        if git("rev-parse", "--verify", f"{candidate}^{{commit}}"):
            return candidate, False, True, "available origin branch merge base"
    return "HEAD~1", False, False, "previous commit fallback"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--since", default="")
    parser.add_argument("--all", default="false")
    parser.add_argument("--github-output", required=True, type=pathlib.Path)
    arguments = parser.parse_args()
    ref, selected_all, merge_base, source = select(arguments.since, arguments.all)
    append_output(arguments.github_output, "ref", ref)
    append_output(arguments.github_output, "all", "true" if selected_all else "false")
    append_output(arguments.github_output, "merge_base", "true" if merge_base else "false")
    print(f"Cargo-Rail comparison: {'all work' if selected_all else ref} ({source})")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except OSError as error:
        print(f"::error::cannot publish Cargo-Rail comparison: {error}", file=sys.stderr)
        raise SystemExit(1) from error
