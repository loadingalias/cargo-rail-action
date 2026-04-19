#!/usr/bin/env python3
"""Fail fast on unsupported cargo-rail planner contracts."""

from __future__ import annotations

import argparse
import json
import sys

SUPPORTED_PLAN_CONTRACT_VERSION = 3
SUPPORTED_SCOPE_CONTRACT_VERSION = 1


def parse_args() -> argparse.Namespace:
  parser = argparse.ArgumentParser()
  plan_group = parser.add_mutually_exclusive_group(required=True)
  plan_group.add_argument("--plan-json")
  plan_group.add_argument("--plan-json-file")
  scope_group = parser.add_mutually_exclusive_group(required=True)
  scope_group.add_argument("--scope-json")
  scope_group.add_argument("--scope-json-file")
  return parser.parse_args()


def load_json_value(raw: str | None, path: str | None, label: str) -> object:
  try:
    if path is not None:
      with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)
    if raw is None:
      raise ValueError(f"{label} input missing")
    return json.loads(raw)
  except FileNotFoundError:
    raise SystemExit(f"::error::{label} file not found: {path}")
  except json.JSONDecodeError as exc:
    raise SystemExit(f"::error::{label} is not valid JSON: {exc}")
  except OSError as exc:
    raise SystemExit(f"::error::failed to read {label}: {exc}")
  except ValueError as exc:
    raise SystemExit(f"::error::{exc}")


def classify_version(actual: object, expected: int, field: str) -> str:
  if not isinstance(actual, int):
    raise SystemExit(
      f"::error::{field} missing or invalid in planner output; this cargo-rail build is too old for cargo-rail-action"
    )
  if actual == expected:
    return ""
  direction = "too old" if actual < expected else "too new"
  return f"::error::{field} {direction}: got {actual}, expected {expected}"


def main() -> int:
  args = parse_args()
  plan = load_json_value(args.plan_json, args.plan_json_file, "plan_json")
  scope = load_json_value(args.scope_json, args.scope_json_file, "scope_json")

  messages = []
  messages.append(
    classify_version(plan.get("plan_contract_version"), SUPPORTED_PLAN_CONTRACT_VERSION, "plan_contract_version")
  )
  messages.append(
    classify_version(scope.get("scope_contract_version"), SUPPORTED_SCOPE_CONTRACT_VERSION, "scope_contract_version")
  )

  messages = [message for message in messages if message]
  if not messages:
    return 0

  version = plan.get("reproducibility", {}).get("cargo_rail_version")
  if version:
    messages.append(f"::error::planner reported cargo-rail version {version}")
  messages.append("::error::install a supported cargo-rail release or upgrade cargo-rail-action to a compatible version")
  print("\n".join(messages), file=sys.stderr)
  return 1


if __name__ == "__main__":
  raise SystemExit(main())
