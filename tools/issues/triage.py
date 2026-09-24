#!/usr/bin/env python3
"""First-pass triage of boundary candidates into the evaluation verdict classes.

This is deliberately conservative: it auto-decides only clearly out-of-scope
classes (SDK, build, envd/guest, feature/perf) and security/ownership issues
that an existing property covers; everything else stays `needs-review` so a
human confirms it. Output: references/issues/ledger-triaged.jsonl.

Usage: python3 tools/issues/triage.py
"""
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ISSUES = os.path.join(ROOT, "references", "issues")

SDK_LABELS = {"sdk", "python", "javascript", "cli", "Code Interpreter", "Build System"}
GUEST_LABELS = {"envd"}
GUEST_TERMS = ["envd", "stdout", "stderr", "websocket", " tcp ", "process",
               "pty", "watch_dir", "port scanner", "subscriber"]
FEATURE_PREFIXES = ("feat:", "perf:", "obs:", "rfc", "proposal", "feature",
                    "improvement", "docs:", "self-hosting", "question")
BUILD_TERMS = ["template build", "build template", "rootfs", "pnpm", "dockerfile"]

# (keyword set, verdict, property/class, rationale)
RULES = [
    (["cross-team", "getlastsnapshot", "teamid filtering", "team_id", "ownership check",
      "permission security risk", "missing teamid"],
     "violates", "ResourceOwnership/SnapshotOwnership", "cross-team access is excluded by owner/ownership invariants"),
    (["orchestratorip", "routing fails", "sandbox routing", "routing fail"],
     "violates", "ActiveSlotValid/ReclaimedResourcesNotCurrent", "route/catalog identity is covered"),
    (["stale running", "state transition not broadcast", "removing state"],
     "violates", "TerminalConsistent/StableStatesQuiescent", "a removed sandbox must not be observed running"),
    (["reattach", "re-pause", "persist", "data loss", "memfile", "corrupt", "incomplete filesystem"],
     "adapter", "adapter storage fidelity", "content/metadata fidelity is an adapter contract, not modeled"),
    (["orphan", "leak", "leftover", "zombie", "not killed", "never cleaned", "indefinitely", "stuck", "hang"],
     "liveness", "liveness/completion", "resource eventually freed is a liveness property, not proved"),
    (["timeout validation", "non-positive timeout", "instant termination"],
     "out-of-scope", "API input validation", "input validation is outside the safety model"),
]


def labels(issue):
    return {x["name"] for x in issue.get("labels", [])}


def classify(issue):
    text = f"{issue['title']} {issue.get('body') or ''}".lower()
    title = issue["title"].lower()
    lab = labels(issue)
    if lab & SDK_LABELS and "sdk" not in issue["title"].lower():
        return "out-of-scope", "sdk/tooling", "issue belongs to SDK/CLI/build tooling, not the control plane"
    if lab & GUEST_LABELS or any(t in text for t in GUEST_TERMS):
        return "out-of-scope", "guest data plane", "guest/envd process or stream behavior is outside the boundary"
    if any(t in title for t in BUILD_TERMS):
        return "out-of-scope", "template build", "template build path is outside the lifecycle boundary"
    if title.startswith(FEATURE_PREFIXES):
        return "out-of-scope", "feature/perf/docs", "feature, performance, observability, or docs"
    for keys, verdict, prop, why in RULES:
        if any(k in text for k in keys):
            return verdict, prop, why
    return "needs-review", "", "no conservative rule applies; needs human classification"


def main():
    ledger_path = os.path.join(ISSUES, "ledger.jsonl")
    entries = [json.loads(l) for l in open(ledger_path)]
    out = []
    for e in entries:
        if not e["in_scope"]:
            e["proposed_verdict"] = "out-of-scope"
            e["proposed_property"] = ""
            e["rationale"] = "outside boundary filter"
            out.append(e)
            continue
        raw_path = os.path.join(ISSUES, e["repo"].replace("/", "__") + ".jsonl")
        raw = next((json.loads(l) for l in open(raw_path) if json.loads(l)["number"] == e["number"]), None)
        verdict, prop, why = classify(raw) if raw else ("needs-review", "", "raw issue missing")
        e["proposed_verdict"] = verdict
        e["proposed_property"] = prop
        e["rationale"] = why
        out.append(e)
    path = os.path.join(ISSUES, "ledger-triaged.jsonl")
    with open(path, "w") as handle:
        for e in out:
            handle.write(json.dumps(e) + "\n")
    from collections import Counter
    ins = [e for e in out if e["in_scope"]]
    print(f"wrote {path}")
    print("in-scope:", len(ins))
    print("proposed verdicts:", dict(Counter(e["proposed_verdict"] for e in ins)))
    print("violates by property:")
    for k, v in Counter(e["proposed_property"] for e in ins if e["proposed_verdict"] == "violates").most_common():
        print(f"  {k}: {v}")


if __name__ == "__main__":
    sys.exit(main())
