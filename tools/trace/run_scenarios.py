#!/usr/bin/env python3
"""Run every collected scenario through the checker and summarise.

Usage: tools/trace/run_scenarios.py
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from tools.trace import collect
from tools.trace.checker import Checker
from tools.trace.trace_model import Record

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "references", "trace", "scenarios")


def main():
    os.makedirs(OUT, exist_ok=True)
    summary = []
    for name in collect.SCENARIOS:
        raw = collect.run(name)
        records = [Record(**r) for r in raw]
        path = os.path.join(OUT, name + ".jsonl")
        with open(path, "w") as handle:
            for r in raw:
                handle.write(json.dumps(r, sort_keys=True) + "\n")
        checker = Checker()
        verdict = checker.run(records)
        applied = {k: v for k, v in checker.coverage.items() if v}
        summary.append({
            "scenario": name, "verdict": verdict.status,
            "findings": [
                {"rule": f.rule, "detail": f.detail, "seq": f.seq,
                 "properties": f.properties, "inconclusive": f.inconclusive}
                for f in verdict.findings
            ],
            "rules_applied": sorted(applied),
        })
        print(f"{name:16s} {verdict.status}")
        for f in verdict.findings:
            print(f"    {f.rule}: {f.detail}")
    with open(os.path.join(OUT, "summary.json"), "w") as handle:
        json.dump(summary, handle, indent=2, sort_keys=True)
    print(f"\nwrote {OUT}")


if __name__ == "__main__":
    sys.exit(main())
