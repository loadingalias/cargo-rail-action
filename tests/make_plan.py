#!/usr/bin/env python3
"""Create deterministic signed v8 plans for Action contract tests."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
from typing import Any


def canonical(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()


def evidence(code: str, subject: str, description: str, input_value: str | None, complete: bool) -> tuple[str, dict[str, Any]]:
    record = {"code": code, "subject": subject, "description": description, "input": input_value, "complete": complete}
    portable = {key: record[key] for key in ("code", "subject", "input", "complete")}
    return f"evidence:sha256:{hashlib.sha256(canonical(portable)).hexdigest()}", record


def cargo_scope() -> dict[str, Any]:
    return {
        "kind": "cargo",
        "selection": {
            "kind": "packages",
            "packages": [
                {
                    "key": "demo@0.1.0#path:demo",
                    "name": "demo",
                    "cargo_spec": "demo;echo-not-a-shell",
                }
            ],
            "cargo_args": ["-p", "demo;echo-not-a-shell"],
            "targets": [{"package": "demo@0.1.0#path:demo", "name": "contract", "kind": ["test"]}],
        },
    }


def create(scenario: str, head: str) -> dict[str, Any]:
    records: dict[str, dict[str, Any]] = {}
    work: dict[str, dict[str, Any]] = {}

    def add(work_id: str, state: str, *, cause: str | None = None, scope: dict[str, Any] | None = None, complete: bool = True) -> None:
        code = "changed_input" if state == "required" else "declared_inputs_disjoint"
        reference, record = evidence(code, work_id, f"fixture evidence for {work_id}", None, complete)
        records[reference] = record
        decision: dict[str, Any] = {"state": state, "evidence": [reference]}
        if state == "required":
            decision.update({"cause": cause, "scope": scope})
        work[work_id] = decision

    if scenario == "rust":
        add("cargo.build", "required", cause="changed_input", scope=cargo_scope())
        add("cargo.test", "required", cause="incomplete_evidence", scope=cargo_scope(), complete=False)
        add("docs", "skipped")
        add("miri", "required", cause="changed_input", scope=cargo_scope())
        files = [{"path": "demo/src/lib.rs", "kind": "modified", "provenance": ["committed"]}]
    else:
        add("cargo.build", "skipped")
        add("cargo.test", "skipped")
        add("docs", "required", cause="changed_input", scope={"kind": "repository"})
        add("miri", "skipped")
        files = [{"path": "README.md", "kind": "modified", "provenance": ["committed"]}]

    required = sorted(work_id for work_id, decision in work.items() if decision["state"] == "required")
    plan: dict[str, Any] = {
        "plan_contract_version": 8,
        "identity": f"plan-v8:sha256:{'0' * 64}",
        "inputs": {
            "base": "HEAD~1",
            "head": "WORKTREE",
            "head_commit": head,
            "capture": None,
            "cargo": f"resolution-universe-v1:sha256:{'1' * 64}",
            "configuration": f"cargo-configuration-v1:sha256:{'2' * 64}",
            "toolchain": "stable-test-toolchain",
            "target": f"planning-target-v1:sha256:{'3' * 64}",
            "platform": "fixture-platform",
            "catalog": f"work-catalog-v1:sha256:{'4' * 64}",
            "evidence": [],
            "override": "none",
        },
        "changes": {"files": files, "cargo": [], "config": []},
        "work": dict(sorted(work.items())),
        "required": required,
        "evidence": dict(sorted(records.items())),
    }
    portable = {
        "plan_contract_version": plan["plan_contract_version"],
        "inputs": plan["inputs"],
        "changes": plan["changes"],
        "work": plan["work"],
        "required": plan["required"],
        "evidence": sorted(plan["evidence"]),
    }
    plan["identity"] = f"plan-v8:sha256:{hashlib.sha256(canonical(portable)).hexdigest()}"
    return plan


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", choices=("rust", "docs"))
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--head", default="0" * 40)
    arguments = parser.parse_args()
    arguments.output.write_text(json.dumps(create(arguments.scenario, arguments.head), indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
