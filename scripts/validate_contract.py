#!/usr/bin/env python3
"""Fail fast on unsupported cargo-rail planner contracts."""

from __future__ import annotations

import argparse
import json
import sys

SUPPORTED_PLAN_CONTRACT_VERSION = 5
SUPPORTED_SCOPE_CONTRACT_VERSION = 3


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


def require_object(value: object, label: str) -> dict[str, object]:
  if not isinstance(value, dict):
    raise SystemExit(f"::error::{label} must be a JSON object")
  return value


def string_list(value: object) -> list[str] | None:
  if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
    return None
  return value


def validate_scope_cargo_args(scope: dict[str, object], label: str = "scope") -> list[str]:
  cargo_args = string_list(scope.get("cargo_args"))
  crates = string_list(scope.get("crates"))
  mode = scope.get("mode")

  messages: list[str] = []
  if cargo_args is None:
    messages.append(f"::error::{label}.cargo_args missing or invalid in planner output")
    return messages
  if crates is None:
    messages.append(f"::error::{label}.crates missing or invalid in planner output")
    return messages

  if mode == "empty":
    expected: list[str] = []
  elif mode == "workspace":
    expected = ["--workspace"]
  elif mode == "crates":
    expected = []
    for crate_name in crates:
      expected.extend(["-p", crate_name])
  else:
    messages.append(f"::error::{label}.mode invalid in planner output: {mode!r}")
    return messages

  if cargo_args != expected:
    messages.append(
      f"::error::{label}.cargo_args does not match scope mode/crates: "
      f"got {json.dumps(cargo_args)}, expected {json.dumps(expected)}"
    )
  return messages


def validate_plan(plan: dict[str, object], scope: dict[str, object]) -> list[str]:
  messages: list[str] = []

  inputs = plan.get("inputs")
  if not isinstance(inputs, dict) or "snapshot_id" not in inputs:
    messages.append("::error::inputs.snapshot_id missing in planner output")

  plan_scope = plan.get("scope")
  if plan_scope != scope:
    messages.append("::error::plan.scope does not match scope_json")

  surfaces = plan.get("surfaces")
  if not isinstance(surfaces, dict):
    messages.append("::error::plan.surfaces missing or invalid in planner output")
    return messages

  required_surfaces = {"bench", "build", "docs", "infra", "test"}
  for name in sorted(required_surfaces - surfaces.keys()):
    messages.append(f"::error::plan.surfaces.{name} missing or invalid in planner output")

  for name, decision in sorted(surfaces.items()):
    if not isinstance(decision, dict):
      messages.append(f"::error::plan.surfaces.{name} missing or invalid in planner output")
      continue
    if not isinstance(decision.get("enabled"), bool):
      messages.append(f"::error::plan.surfaces.{name}.enabled missing or invalid in planner output")
    reasons = decision.get("reasons")
    if not isinstance(reasons, list) or not all(isinstance(reason, int) for reason in reasons):
      messages.append(f"::error::plan.surfaces.{name}.reasons missing or invalid in planner output")
    surface_scope = decision.get("scope")
    if not isinstance(surface_scope, dict):
      messages.append(f"::error::plan.surfaces.{name}.scope missing or invalid in planner output")
    else:
      messages.extend(validate_scope_cargo_args(surface_scope, f"plan.surfaces.{name}.scope"))

  return messages


def main() -> int:
  args = parse_args()
  plan = require_object(load_json_value(args.plan_json, args.plan_json_file, "plan_json"), "plan_json")
  scope = require_object(load_json_value(args.scope_json, args.scope_json_file, "scope_json"), "scope_json")

  messages = []
  messages.append(
    classify_version(plan.get("plan_contract_version"), SUPPORTED_PLAN_CONTRACT_VERSION, "plan_contract_version")
  )
  messages.append(
    classify_version(scope.get("scope_contract_version"), SUPPORTED_SCOPE_CONTRACT_VERSION, "scope_contract_version")
  )
  messages.extend(validate_scope_cargo_args(scope))
  messages.extend(validate_plan(plan, scope))

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
