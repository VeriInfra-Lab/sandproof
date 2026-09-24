#!/usr/bin/env python3
"""Issue-corpus study for boundary-scoped relative completeness.

Subcommands:
  fetch   pull all issues (open+closed) from the tracked repos into JSONL
  filter  boundary-filter + map to a candidate safety property -> ledger
  report  coverage metrics over the ledger

The boundary filter and property mapping are heuristics; `overrides.json`
lets a human correct individual entries, and the ledger always keeps the raw
suggestion so the decision is auditable.

Usage:
  python3 tools/issues/study.py fetch
  python3 tools/issues/study.py filter
  python3 tools/issues/study.py report
"""
import json
import os
import subprocess
import sys
from dataclasses import asdict, dataclass, field

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "references", "issues")
REPOS = ["e2b-dev/runtime", "e2b-dev/E2B"]

# Lifecycle-authority vocabulary: an issue must mention at least one.
IN_SCOPE_TERMS = [
    "sandbox", "pause", "paused", "resume", "resumed", "kill", "snapshot",
    "lifecycle", "restore", "workspace", "cleanup", "race", "concurrent",
    "stale", "generation", "execution", "route", "routing", "catalog",
    "orchestrator", "firecracker", "vm ", "microvm", "evict", "orphan",
    "duplicate", "idempot", "transition", "removing", "revoke", "lock",
    "crash", "restart", "reconcile", "filesystem", "restoring", "reservation",
]

# Clearly outside the controller/worker lifecycle authority boundary.
OUT_OF_SCOPE_TERMS = [
    "template build", "dockerfile", "npm", "pip ", "sdk", "cli", "docs",
    "documentation", "dashboard", "billing", "gpu", "auth token", "api key",
    "typescript", "javascript", "python sdk", "ui ", "frontend", "changelog",
    "release", "helm", "terraform", "clickhouse", "grafana", "quota",
]

# (any keyword present) -> (candidate property/class, candidate verdict)
# The heuristic only proposes candidates; a human confirms via overrides.json.
MAPPINGS = [
    (["kill", "resume"], "TerminalConsistent", "violates"),
    (["resume", "restore"], "RestoreSourceValid", "violates"),
    (["pause", "snapshot"], "PausedSnapshotValid", "violates"),
    (["retry", "duplicate", "idempot", "ack", "timeout"],
     "operationResultStable/effectAdmission", "violates"),
    (["cleanup", "orphan", "removal", "removing", "reclaim"],
     "cleanupAdmission", "violates"),
    (["route", "routing", "catalog"], "ReclaimedResourcesNotCurrent", "violates"),
    (["expectexecution", "stale", "evict"], "generation/epoch admission", "violates"),
    (["file content", "corrupt", "lost", "missing file", "data loss"],
     "adapter storage fidelity", "adapter"),
    (["stdout", "tcp", "envd", "websocket"],
     "guest data plane", "out-of-scope"),
    (["pending", "stuck", "hang", "forever", "never completes"],
     "liveness", "liveness"),
]


@dataclass
class LedgerEntry:
    repo: str
    number: int
    title: str
    state: str
    url: str
    in_scope: bool
    score: int
    matched: list[str] = field(default_factory=list)
    suggested_property: str = ""
    verdict: str = "needs-review"
    note: str = ""


def _slug(repo: str) -> str:
    return repo.replace("/", "__")


def fetch() -> None:
    os.makedirs(OUT, exist_ok=True)
    for repo in REPOS:
        fields = "number,title,body,state,url,createdAt,labels"
        raw = subprocess.run(
            ["gh", "issue", "list", "--repo", repo, "--state", "all",
             "--limit", "2000", "--json", fields],
            capture_output=True, text=True, check=True).stdout
        path = os.path.join(OUT, _slug(repo) + ".jsonl")
        with open(path, "w") as handle:
            for issue in json.loads(raw):
                handle.write(json.dumps(issue) + "\n")
        print(f"{repo}: {len(json.loads(raw))} issues -> {path}")


def _load_overrides() -> dict:
    path = os.path.join(OUT, "overrides.json")
    if os.path.exists(path):
        with open(path) as handle:
            return json.load(handle)
    return {}


def _classify(repo: str, issue: dict) -> LedgerEntry:
    text = f"{issue.get('title', '')} {issue.get('body') or ''}".lower()
    matched = [t for t in IN_SCOPE_TERMS if t in text]
    out_hits = [t for t in OUT_OF_SCOPE_TERMS if t in text]
    score = len(matched) - len(out_hits)
    in_scope = bool(matched) and score > 0
    prop, verdict = "", "needs-review"
    if in_scope:
        for keys, suggested, suggested_verdict in MAPPINGS:
            if any(k in text for k in keys):
                prop, verdict = suggested, suggested_verdict
                break
        else:
            prop, verdict = "unmapped", "needs-review"
    entry = LedgerEntry(
        repo=repo, number=issue["number"], title=issue["title"],
        state=issue["state"], url=issue["url"], in_scope=in_scope,
        score=score, matched=matched, suggested_property=prop, verdict=verdict,
    )
    override = _load_overrides().get(f"{repo}#{issue['number']}")
    if override:
        for key, value in override.items():
            setattr(entry, key, value)
    return entry


def filter_() -> None:
    entries: list[LedgerEntry] = []
    for repo in REPOS:
        path = os.path.join(OUT, _slug(repo) + ".jsonl")
        if not os.path.exists(path):
            print(f"missing {path}; run fetch first", file=sys.stderr)
            continue
        with open(path) as handle:
            for line in handle:
                entries.append(_classify(repo, json.loads(line)))
    path = os.path.join(OUT, "ledger.jsonl")
    with open(path, "w") as handle:
        for entry in entries:
            handle.write(json.dumps(asdict(entry)) + "\n")
    scope = sum(1 for e in entries if e.in_scope)
    print(f"{len(entries)} issues, {scope} in-scope -> {path}")


def report() -> None:
    path = os.path.join(OUT, "ledger.jsonl")
    if not os.path.exists(path):
        print("no ledger; run fetch && filter", file=sys.stderr)
        return
    entries = [json.loads(l) for l in open(path)]
    in_scope = [e for e in entries if e["in_scope"]]
    print(f"total: {len(entries)}")
    print(f"in-scope: {len(in_scope)}")
    by_verdict: dict[str, int] = {}
    by_prop: dict[str, int] = {}
    for e in in_scope:
        by_verdict[e["verdict"]] = by_verdict.get(e["verdict"], 0) + 1
        by_prop[e["suggested_property"]] = by_prop.get(e["suggested_property"], 0) + 1
    print("by suggested verdict:")
    for key, count in sorted(by_verdict.items(), key=lambda x: -x[1]):
        print(f"  {key}: {count}")
    print("by suggested property:")
    for key, count in sorted(by_prop.items(), key=lambda x: -x[1]):
        print(f"  {key}: {count}")


def main() -> int:
    command = sys.argv[1] if len(sys.argv) > 1 else ""
    if command == "fetch":
        fetch()
    elif command == "filter":
        filter_()
    elif command == "report":
        report()
    else:
        print(__doc__)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
