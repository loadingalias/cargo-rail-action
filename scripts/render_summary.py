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

# Map reason codes to human-readable descriptions
REASON_DESCRIPTIONS = {
  "FILE_KIND_RUST_SRC": "Rust source file changed",
  "FILE_KIND_RUST_TEST": "Rust test file changed",
  "FILE_KIND_RUST_BENCH": "Rust benchmark file changed",
  "FILE_KIND_TOML_MANIFEST": "Cargo.toml changed",
  "FILE_KIND_TOML_WORKSPACE": "Workspace Cargo.toml changed",
  "FILE_KIND_TOML_TOOLING": "Tooling config changed",
  "FILE_KIND_CI": "CI/workflow file changed",
  "FILE_KIND_SCRIPT": "Script file changed",
  "FILE_KIND_DOCS": "Documentation changed",
  "FILE_KIND_CUSTOM": "Custom pattern matched",
  "FILE_KIND_UNCLASSIFIED": "Unclassified file changed",
  "FILE_OWNS_CRATE_DIRECT": "File directly owns crate",
  "TRANSITIVE_DEPENDS_ON_DIRECT": "Transitive dependency of changed crate",
  "OWNER_UNCERTAIN_FALLBACK": "Conservative fallback for uncertain ownership",
  "CONFIDENCE_PROFILE_STRICT": "Strict confidence profile active",
  "CONFIDENCE_PROFILE_BALANCED": "Balanced confidence profile active",
  "CONFIDENCE_PROFILE_FAST": "Fast confidence profile active",
  "CONFIDENCE_STRICT_OWNER_EXPANSION": "Strict mode expands owned crates",
  "CONFIDENCE_FAST_SKIP_TRANSITIVE": "Fast mode skips transitive expansion",
  "BOT_PR_CONFIDENCE_OVERRIDE": "Bot PR confidence override applied",
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


def build_reason_lookup(trace: list) -> dict[int, dict]:
  """Build a lookup from reason ID to trace entry."""
  return {item.get("id"): item for item in trace if item.get("id") is not None}


def summarize_surface_reasons(reasons: list[int], lookup: dict[int, dict]) -> str:
  """Generate a concise summary of why a surface is active."""
  if not reasons:
    return "No source changes"

  # Count unique reason codes
  code_counts: dict[str, int] = {}
  for rid in reasons:
    entry = lookup.get(rid, {})
    code = entry.get("code", "UNKNOWN")
    code_counts[code] = code_counts.get(code, 0) + 1

  # Generate summary
  parts = []
  for code, count in sorted(code_counts.items(), key=lambda x: -x[1]):
    desc = REASON_DESCRIPTIONS.get(code, code)
    if count > 1:
      parts.append(f"{desc} ({count}x)")
    else:
      parts.append(desc)

  return "; ".join(parts[:3])  # Limit to top 3 reasons


def render(args: argparse.Namespace, plan: dict) -> str:
  files = [f.get("path", "") for f in plan.get("files", []) if f.get("path")]
  impact = plan.get("impact", {})
  direct = list(impact.get("direct_crates", []))
  transitive = list(impact.get("transitive_crates", []))
  surfaces = plan.get("surfaces", {})
  trace = plan.get("trace", [])

  # Build reason lookup
  reason_lookup = build_reason_lookup(trace)

  # Separate built-in and custom surfaces
  builtin_surfaces = ["build", "test", "bench", "docs", "infra"]
  custom_surfaces = {k: v for k, v in surfaces.items() if k.startswith("custom:")}

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

  # Enhanced surface status table
  lines.append("")
  lines.append("### Surface Status")
  lines.append("")
  lines.append("| Surface | Status | Reason |")
  lines.append("|---------|--------|--------|")

  for surface_name in builtin_surfaces:
    surface_data = surfaces.get(surface_name, {})
    enabled = surface_data.get("enabled", False)
    reasons = surface_data.get("reasons", [])
    status = "on" if enabled else "off"
    reason_summary = summarize_surface_reasons(reasons, reason_lookup) if enabled else "No triggering changes"
    lines.append(f"| `{surface_name}` | **{status}** | {reason_summary} |")

  # Add custom surfaces if present
  if custom_surfaces:
    lines.append("")
    lines.append("**Custom surfaces:**")
    lines.append("")
    lines.append("| Surface | Status | Reason |")
    lines.append("|---------|--------|--------|")
    for surface_name in sorted(custom_surfaces.keys()):
      surface_data = custom_surfaces[surface_name]
      enabled = surface_data.get("enabled", False)
      reasons = surface_data.get("reasons", [])
      status = "on" if enabled else "off"
      reason_summary = summarize_surface_reasons(reasons, reason_lookup) if enabled else "No matching patterns"
      lines.append(f"| `{surface_name}` | **{status}** | {reason_summary} |")

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
