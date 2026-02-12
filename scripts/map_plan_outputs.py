#!/usr/bin/env python3
"""Map planner JSON contract into action output key/value projections."""

from __future__ import annotations

import argparse
import json
import os
import sys
from typing import Any

OUTPUT_KEYS = [
  "files",
  "changed_files_count",
  "surfaces",
  "trace",
  "crates",
  "count",
  "cargo_args",
  "matrix",
  "active_surfaces",
]


def compact_json(value: Any, *, sort_keys: bool = False) -> str:
  return json.dumps(value, separators=(",", ":"), sort_keys=sort_keys)


def build_projection(plan: dict[str, Any]) -> dict[str, str]:
  files = [f.get("path", "") for f in plan.get("files", []) if f.get("path")]
  impact = plan.get("impact", {})
  direct = impact.get("direct_crates", [])
  transitive = impact.get("transitive_crates", [])
  surfaces = plan.get("surfaces", {})
  trace = plan.get("trace", [])

  crate_union = sorted(set(direct + transitive))
  cargo_args = " ".join(f"-p {name}" for name in crate_union)
  active_surfaces = sorted(
    name
    for name, value in surfaces.items()
    if isinstance(value, dict) and value.get("enabled") is True
  )

  return {
    "files": compact_json(files),
    "changed_files_count": str(len(files)),
    "surfaces": compact_json(surfaces, sort_keys=True),
    "trace": compact_json(trace),
    "crates": " ".join(crate_union),
    "count": str(len(crate_union)),
    "cargo_args": cargo_args,
    "matrix": compact_json(crate_union),
    "active_surfaces": compact_json(active_surfaces),
  }


def read_plan_json(args: argparse.Namespace) -> dict[str, Any]:
  if args.plan_json_file:
    with open(args.plan_json_file, "r", encoding="utf-8") as f:
      return json.load(f)

  if args.plan_json:
    return json.loads(args.plan_json)

  if "PLAN_JSON" in os.environ:
    return json.loads(os.environ["PLAN_JSON"])

  raise ValueError("plan json not provided")


def write_kv_lines(mapping: dict[str, str], output_path: str) -> None:
  with open(output_path, "a", encoding="utf-8") as f:
    for key in OUTPUT_KEYS:
      f.write(f"{key}={mapping[key]}\\n")


def main() -> int:
  parser = argparse.ArgumentParser()
  parser.add_argument("--plan-json", default="")
  parser.add_argument("--plan-json-file", default="")
  parser.add_argument("--output", default="")
  parser.add_argument("--stdout", action="store_true")
  args = parser.parse_args()

  try:
    plan = read_plan_json(args)
  except Exception as e:
    print(f"error reading plan json: {e}", file=sys.stderr)
    return 1

  mapping = build_projection(plan)

  output_path = args.output or os.environ.get("GITHUB_OUTPUT", "")
  if output_path:
    write_kv_lines(mapping, output_path)

  if args.stdout or not output_path:
    for key in OUTPUT_KEYS:
      print(f"{key}={mapping[key]}")

  return 0


if __name__ == "__main__":
  raise SystemExit(main())
