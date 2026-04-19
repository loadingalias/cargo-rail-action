#!/usr/bin/env python3
"""Render a deterministic CI summary for planner results."""

from __future__ import annotations

import argparse
import json
import os
from collections import Counter

INSTALL_MAP = {
  "binary": "Binary download",
  "binstall": "cargo-binstall",
  "cargo-install": "cargo install (compiled)",
  "cached": "Already installed",
}

LIST_PREVIEW_LIMIT = 12
TRACE_PREVIEW_LIMIT = 20
REASON_COUNT_PREVIEW_LIMIT = 8

# Fallback descriptions for older planner contracts that don't ship descriptions inline.
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
    desc = next(
      (
        entry.get("description")
        for rid, entry in lookup.items()
        if rid in reasons and entry.get("code") == code and entry.get("description")
      ),
      REASON_DESCRIPTIONS.get(code, code),
    )
    if count > 1:
      parts.append(f"{desc} ({count}x)")
    else:
      parts.append(desc)

  return "; ".join(parts[:3])  # Limit to top 3 reasons


def collect_active_reason_ids(surfaces: dict) -> list[int]:
  """Collect unique reason ids from enabled surfaces only."""
  reason_ids: set[int] = set()
  for value in surfaces.values():
    if isinstance(value, dict) and value.get("enabled") is True:
      for reason_id in value.get("reasons", []):
        if isinstance(reason_id, int):
          reason_ids.add(reason_id)
  return sorted(reason_ids)


def preview_items(items: list[str], limit: int = LIST_PREVIEW_LIMIT) -> str:
  """Render a stable preview for a potentially large list."""
  if not items:
    return "none"

  preview = items[: max(limit, 1)]
  rendered = ", ".join(preview)
  if len(items) <= limit:
    return rendered
  return f"{rendered}, ... +{len(items) - limit} more"


def summarize_reason_counts(trace: list[dict]) -> list[tuple[str, int]]:
  """Count reason codes across the full trace."""
  counts = Counter(item.get("code", "UNKNOWN") for item in trace)
  return sorted(counts.items(), key=lambda item: (-item[1], item[0]))


def reason_description(code: str, trace: list[dict]) -> str:
  for item in trace:
    if item.get("code") == code and item.get("description"):
      return item["description"]
  return REASON_DESCRIPTIONS.get(code, code)


def render_trace_entry(item: dict) -> str:
  """Render a single raw trace entry."""
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
  return f"- {' '.join(parts)}"


def render(args: argparse.Namespace, plan: dict) -> str:
  files = [f.get("path", "") for f in plan.get("files", []) if f.get("path")]
  impact = plan.get("impact", {})
  direct = list(impact.get("direct_crates", []))
  surfaces = plan.get("surfaces", {})
  scope = plan.get("scope", {})
  trace = plan.get("trace", [])

  # Build reason lookup
  reason_lookup = build_reason_lookup(trace)
  scope_surfaces = scope.get("surfaces", {})
  scope_mode = scope.get("mode", "empty")
  scope_crates = list(scope.get("crates", []))

  active_builtin_surfaces = sorted(
    name for name, enabled in scope_surfaces.items() if enabled is True and not name.startswith("custom:")
  )
  active_custom_surfaces = sorted(
    name for name, enabled in scope_surfaces.items() if enabled is True and name.startswith("custom:")
  )
  top_reasons = summarize_surface_reasons(collect_active_reason_ids(surfaces), reason_lookup)

  lines: list[str] = []
  lines.append("## cargo-rail plan")
  lines.append("")
  lines.append("| | |")
  lines.append("|---|---|")
  lines.append(f"| **Version** | `{args.install_version}` |")
  lines.append(f"| **Install** | {INSTALL_MAP.get(args.install_method, 'Unknown')} |")
  lines.append(f"| **Base** | `{args.base_ref}` |")
  lines.append(f"| **Changed files** | {len(files)} |")
  lines.append(f"| **Scope mode** | `{scope_mode}` |")
  lines.append(f"| **Direct crates** | {len(direct)} |")
  lines.append(
    f"| **Active surfaces** | {', '.join(active_builtin_surfaces) if active_builtin_surfaces else 'none'} |"
  )
  lines.append("")

  if direct:
    lines.append(f"**Changed direct crates ({len(direct)}):** `{preview_items(direct)}`")
  if scope_mode == "crates" and scope_crates:
    lines.append(f"**Execution crates ({len(scope_crates)}):** `{preview_items(scope_crates)}`")
  elif scope_mode == "workspace":
    lines.append("**Execution scope:** full workspace")
  if active_custom_surfaces:
    lines.append(f"**Active custom surfaces:** `{preview_items(active_custom_surfaces)}`")
  lines.append(f"**Top reasons:** {top_reasons}")

  lines.append("")
  lines.append(f"<details><summary>Trace summary ({len(trace)} reasons)</summary>")
  lines.append("")

  reason_counts = summarize_reason_counts(trace)
  if reason_counts:
    lines.append("**Reason counts**")
    for code, count in reason_counts[:REASON_COUNT_PREVIEW_LIMIT]:
      desc = reason_description(code, trace)
      lines.append(f"- {desc}: {count}")
    if len(reason_counts) > REASON_COUNT_PREVIEW_LIMIT:
      lines.append(f"- ... +{len(reason_counts) - REASON_COUNT_PREVIEW_LIMIT} more reason types")
    lines.append("")

  if trace:
    lines.append(f"**Sample trace entries ({min(len(trace), TRACE_PREVIEW_LIMIT)} of {len(trace)})**")
    for item in trace[:TRACE_PREVIEW_LIMIT]:
      lines.append(render_trace_entry(item))
    if len(trace) > TRACE_PREVIEW_LIMIT:
      lines.append(f"- ... +{len(trace) - TRACE_PREVIEW_LIMIT} more trace entries")
  else:
    lines.append("No trace reasons.")

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
