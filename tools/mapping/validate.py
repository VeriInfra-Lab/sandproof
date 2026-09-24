#!/usr/bin/env python3
"""Validate the abstraction mapping (alpha) clauses.

Usage:
  tools/mapping/validate.py                 # coverage + citation check (offline)
  tools/mapping/validate.py --witness       # also run a live lifecycle witness
  tools/mapping/validate.py --mutation      # prove witnesses catch a mutated probe

The witness runs against the local harness; mutation reuses the facts captured
by the witness and perturbs each probe to confirm the witness would fail.
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from tools.mapping import probes, registry
from tools.trace import harness

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SOURCES = os.path.join(ROOT, "research-sources", "e2b-infra")


def coverage() -> int:
    total = len(registry.clauses())
    obs = registry.observable()
    non = registry.non_observable()
    print(f"clauses: {total}  observable: {len(obs)}  non-observable: {len(non)}")
    print("\nunobservable / partial (trust boundary):")
    for clause in non:
        blocked = ", ".join(clause.get("blocked_properties", [])) or "-"
        print(f"  [{clause['observability']:11s}] {clause['abstract_field']:20s} blocks: {blocked}")
    return 0


def citations() -> int:
    missing = 0
    for clause in registry.observable():
        path = clause.get("citation", "")
        if not path:
            continue
        full = os.path.join(SOURCES, path)
        if not os.path.exists(full):
            print(f"  MISSING citation: {clause['id']} -> {path}")
            missing += 1
    print(f"citations checked: {len(registry.observable())}, missing: {missing}")
    return 1 if missing else 0


def _expect(facts, clause, expected, stage, failures):
    if expected is None:
        return
    value = probes.project(facts, clause)
    if value is probes.MISSING:
        failures.append(f"{stage}: {clause['abstract_field']} unexpectedly unobservable")
    elif value != expected:
        failures.append(f"{stage}: {clause['abstract_field']} = {value!r}, expected {expected!r}")


def witness() -> tuple[int, dict]:
    """Run create -> pause and assert the clause expectations. Returns the
    captured facts per stage (for mutation)."""
    by_field = registry.by_field()
    stages: dict[str, probes.Facts] = {}
    failures: list[str] = []

    status, body = harness.http("POST", "/sandboxes", {"templateID": harness.TPL, "timeout": 600})
    if not status or not 200 <= status < 300:
        print(f"witness: create failed ({status})")
        return 1, {}
    sid = json.loads(body)["sandboxID"]
    stages["create"] = probes.gather(sid)
    for field in ("running", "phase_hint", "catalog_present"):
        clause = by_field[field]
        _expect(stages["create"], clause, clause.get("expected_after_create"), "create", failures)
    if probes.project(stages["create"], by_field["workspace"]) is probes.MISSING:
        failures.append("create: workspace unobservable")
    index = set(stages["create"].index_members)
    existing = set(stages["create"].existing_sandboxes)
    if not existing <= index:
        failures.append(f"create: running-store ids not in index: {sorted(existing - index)}")

    harness.http("POST", f"/sandboxes/{sid}/pause")
    stages["pause"] = probes.gather(sid)
    for field in ("running", "snapshot_present", "snapshot_deleted", "phase_hint"):
        clause = by_field[field]
        _expect(stages["pause"], clause, clause.get("expected_after_pause"), "pause", failures)

    harness.http("DELETE", f"/sandboxes/{sid}")

    if failures:
        print("witness: FAIL")
        for f in failures:
            print("  " + f)
        return 1, stages
    print("witness: PASS (create + pause observations match the clauses)")
    return 0, stages


def mutation(stages: dict) -> int:
    if not stages:
        print("mutation: no witness facts; run --witness first")
        return 1
    by_field = registry.by_field()
    checks = [
        ("create", "running", by_field["running"].get("expected_after_create")),
        ("create", "phase_hint", by_field["phase_hint"].get("expected_after_create")),
        ("pause", "running", by_field["running"].get("expected_after_pause")),
        ("pause", "snapshot_present", by_field["snapshot_present"].get("expected_after_pause")),
        ("pause", "phase_hint", by_field["phase_hint"].get("expected_after_pause")),
    ]
    escapes = 0
    for stage, field, expected in checks:
        clause = by_field[field]
        original = probes.PROBES[clause["probe"]]
        probes.PROBES[clause["probe"]] = (lambda wrong: (lambda f: wrong))(object())
        mutated = probes.project(stages[stage], clause)
        probes.PROBES[clause["probe"]] = original
        if mutated == expected:
            print(f"  ESCAPED: mutated {field} still matched expected {expected!r}")
            escapes += 1
    print(f"mutation: {len(checks)} checks, escapes: {escapes}")
    return 1 if escapes else 0


def main() -> int:
    code = coverage()
    print()
    code |= citations()
    if "--witness" in sys.argv or "--mutation" in sys.argv:
        print()
        wcode, stages = witness()
        code |= wcode
        if "--mutation" in sys.argv:
            print()
            code |= mutation(stages)
    return code


if __name__ == "__main__":
    sys.exit(main())