#!/usr/bin/env python3
"""Render a deterministic CI summary for planner results."""

from __future__ import annotations

import argparse
import json
import os

INSTALL_MAP = {
  "binary": "Binary download",
  "binstall": "cargo-binstall",
  "cargo-install": "cargo install (compiled)",
  "cached": "Already installed",
}


def parse_args() -> argparse.Namespace:
  p = argparse.ArgumentParser()
  p.add_argument("--plan-json", default="")
  p.add_argument("--plan-json-file", default="")
  p.add_argument("--install-method", default=os.environ.get("INSTALL_METHOD", ""))
  p.add_argument("--install-version", default=os.environ.get("INSTALL_VERSION", ""))
  p.add_argument("--base-ref", default=os.environ.get("BASE_REF", ""))
  return p.parse_args()


def load_plan(args: argparse.Namespace) -> dict:
  if args.plan_json_file:
    with open(args.plan_json_file, "r", encoding="utf-8") as f:
      return json.load(f)
  if args.plan_json:
    return json.loads(args.plan_json)
  env = os.environ.get("PLAN_JSON", "")
  if env:
    return json.loads(env)
  return {"files": [], "impact": {"direct_crates": [], "transitive_crates": []}, "surfaces": {}, "trace": []}


def render(args: argparse.Namespace, plan: dict) -> str:
  files = [f.get("path", "") for f in plan.get("files", []) if f.get("path")]
  impact = plan.get("impact", {})
  direct = list(impact.get("direct_crates", []))
  transitive = list(impact.get("transitive_crates", []))
  surfaces = plan.get("surfaces", {})
  trace = plan.get("trace", [])

  active_surfaces = sorted(
    name
    for name, value in surfaces.items()
    if isinstance(value, dict) and value.get("enabled") is True
  )

  lines: list[str] = []
  lines.append("## cargo-rail plan")
  lines.append("")
  lines.append("| | |")
  lines.append("|---|---|")
  lines.append(f"| **Version** | `{args.install_version}` |")
  lines.append(f"| **Install** | {INSTALL_MAP.get(args.install_method, 'Unknown')} |")
  lines.append(f"| **Base** | `{args.base_ref}` |")
  lines.append(f"| **Changed files** | {len(files)} |")
  lines.append(f"| **Direct crates** | {len(direct)} |")
  lines.append(f"| **Transitive crates** | {len(transitive)} |")
  lines.append(f"| **Active surfaces** | {', '.join(active_surfaces) if active_surfaces else 'none'} |")
  lines.append("")

  if direct:
    lines.append(f"**Direct crates:** `{ ' '.join(direct) }`")
  if transitive:
    lines.append(f"**Transitive crates:** `{ ' '.join(transitive) }`")

  if active_surfaces:
    lines.append("")
    lines.append("### Why Surfaces Are Active")
    for surface in active_surfaces:
      reasons = surfaces.get(surface, {}).get("reasons", [])
      lines.append(f"- `{surface}`: {len(reasons)} reason(s)")

  lines.append("")
  lines.append("<details><summary>Trace details (file -> crate -> surface)</summary>")
  lines.append("")

  for item in trace:
    rid = item.get("id")
    code = item.get("code", "")
    file_path = item.get("file")
    crate_name = item.get("crate")
    depends_on = item.get("depends_on")
    surface = item.get("surface")

    parts = [f"r{rid}", code]
    if file_path:
      parts.append(f"file={file_path}")
    if crate_name:
      parts.append(f"crate={crate_name}")
    if depends_on:
      parts.append(f"depends_on={depends_on}")
    if surface:
      parts.append(f"surface={surface}")
    lines.append(f"- {' '.join(parts)}")

  lines.append("")
  lines.append("</details>")
  return "\n".join(lines)


def main() -> int:
  args = parse_args()
  plan = load_plan(args)
  print(render(args, plan))
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
