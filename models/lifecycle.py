#!/usr/bin/env python3
"""Finite protocol sketch, NOT a proof or an E2B implementation model.

Persistent intent and a separately installed worker fence are distinct steps.
Messages remain deliverable to represent arbitrary delay and duplicate delivery.
Actual VM state below is ghost/environment state, not a perfect controller read.
"""
from collections import deque
from dataclasses import asdict, dataclass, replace
from pathlib import Path
import argparse
import json


@dataclass(frozen=True)
class State:
    phase: str = "Empty"
    revision: int = 0
    generation: int = 0
    tombstone: bool = False
    worker_revision: int = 0
    worker_dead: bool = False
    vms: tuple = ("Gone", "Gone")
    route: int = 0
    local_snapshot: int = 0
    durable_snapshot: int = 0
    messages: tuple = ()
    replies: tuple = ()
    up: bool = True
    uncertain: bool = False


def add(items, item):
    return tuple(sorted(set(items) | {item}))


def vm_set(s, g, value):
    result = list(s.vms)
    result[g - 1] = value
    return tuple(result)


def successors(s, mutant, max_revision):
    def emit(label, **changes):
        return label, replace(s, **changes)

    if s.up:
        yield emit("controller crashes; durable intent survives", up=False)
    else:
        yield emit("controller recovers from journal", up=True)

    if s.up and s.phase in ("Starting", "Pausing", "Killing") and not s.uncertain:
        changes = {"uncertain": True}
        if (mutant == "timeout_means_rollback" and s.phase == "Starting"
                and s.generation > 1 and s.vms[s.generation - 1] == "Running"):
            changes["phase"] = "Paused"
        yield emit("RPC timeout; outcome unknown", **changes)
    if s.up and s.uncertain:
        yield emit("reconcile operation identity; no inverse operation", uncertain=False)

    if s.up and s.revision < max_revision:
        if not s.tombstone and s.phase in ("Empty", "Paused") and s.generation < len(s.vms):
            q, g = s.revision + 1, s.generation + 1
            yield emit("persist create/resume intent", phase="Starting", revision=q,
                       generation=g, uncertain=False,
                       messages=add(s.messages, ("load", q, g)))
        if not s.tombstone and s.phase == "Running":
            q = s.revision + 1
            yield emit("persist pause intent", phase="Pausing", revision=q,
                       uncertain=False, local_snapshot=0,
                       messages=add(s.messages, ("freeze", q, s.generation)))
        if not s.tombstone and s.phase != "Empty":
            yield emit("persist terminal kill intent", phase="Killing",
                       revision=s.revision + 1, tombstone=True, uncertain=False)

    # Worker learns the journal revision in a separate event. Old work can
    # still take effect before this barrier, but never after the barrier.
    if s.worker_revision < s.revision:
        yield emit("install worker fence and close admission", worker_revision=s.revision,
                   worker_dead=s.tombstone, route=0)

    for kind, q, g in s.messages:
        valid = q == s.worker_revision and not s.worker_dead
        if mutant == "no_effect_fence":
            valid = True
        if not valid:
            continue
        if kind == "load" and s.vms[g - 1] == "Gone":
            yield emit(f"worker loads paused VM g={g}, q={q}", vms=vm_set(s, g, "Prepared"),
                       replies=add(s.replies, ("loaded", q, g)))
        if kind == "activate" and s.vms[g - 1] == "Prepared" and "Running" not in s.vms:
            yield emit(f"worker activates VM g={g}, q={q}", vms=vm_set(s, g, "Running"),
                       replies=add(s.replies, ("activated", q, g)))
        if kind == "freeze" and s.vms[g - 1] == "Running":
            yield emit(f"worker freezes VM g={g}, q={q}", vms=vm_set(s, g, "Frozen"),
                       route=0, replies=add(s.replies, ("frozen", q, g)))

    if s.up:
        for kind, q, g in s.replies:
            valid = (q == s.revision and g == s.generation and not s.tombstone
                     and s.phase == "Starting")
            if kind == "loaded" and valid:
                yield emit(f"consume loaded reply g={g}, q={q}",
                           messages=add(s.messages, ("activate", q, g)))
            if kind == "activated" and (valid or mutant == "no_callback_guard"):
                yield emit(f"consume activated reply g={g}, q={q}", phase="Running", route=g)

    if s.phase == "Pausing" and s.worker_revision == s.revision:
        g = s.generation
        if s.vms[g - 1] == "Frozen" and s.local_snapshot != g:
            yield emit("capture sealed local snapshot", local_snapshot=g)
        if s.local_snapshot == g and s.durable_snapshot != g:
            yield emit("fsync snapshot bundle and commit manifest", durable_snapshot=g)
        if s.vms[g - 1] == "Frozen" and s.local_snapshot == g:
            yield emit("old VM exits", vms=vm_set(s, g, "Gone"))
        ready = s.local_snapshot == g and all(v == "Gone" for v in s.vms)
        durable = s.durable_snapshot == g or mutant == "pause_before_durable"
        if s.up and ready and durable:
            yield emit("publish PauseCompleted", phase="Paused", uncertain=False)

    if s.tombstone and s.worker_dead and any(v != "Gone" for v in s.vms):
        yield emit("worker terminates and reaps all VM processes", vms=("Gone",) * len(s.vms), route=0)
    if s.up and s.phase == "Killing":
        ready = (s.worker_dead and s.worker_revision == s.revision
                 and all(v == "Gone" for v in s.vms) and s.route == 0)
        if ready or mutant == "kill_before_quiescent":
            yield emit("publish DestroyCompleted", phase="Destroyed", uncertain=False)


def violations(s):
    failures = []
    if sum(v == "Running" for v in s.vms) > 1:
        failures.append("I1: at most one physically executing incarnation")
    if s.route and (s.phase not in ("Running", "Pausing", "Killing") or s.route != s.generation
                    or s.vms[s.route - 1] != "Running" or s.worker_dead):
        failures.append("I2: published admission references the current running VM")
    if s.tombstone and s.phase not in ("Killing", "Destroyed"):
        failures.append("I3: terminal intent never returns to a live phase")
    if s.phase == "Paused" and (s.durable_snapshot != s.generation
                               or any(v != "Gone" for v in s.vms) or s.route):
        failures.append("I4: PauseCompleted has durable snapshot and no old VM")
    if s.phase == "Destroyed" and (not s.tombstone or not s.worker_dead
                                  or s.worker_revision != s.revision
                                  or any(v != "Gone" for v in s.vms) or s.route):
        failures.append("I5: DestroyCompleted has a fence, no VM and no admission")
    return failures


def explore(mutant="none", max_revision=4):
    start = State()
    queue = deque([start])
    parents = {start: None}
    transitions = 0
    covered = set()
    while queue:
        current = queue.popleft()
        covered.add(current.phase)
        failures = violations(current)
        if failures:
            trace = []
            node = current
            while parents[node] is not None:
                previous, label = parents[node]
                trace.append({"event": label, "state": asdict(node)})
                node = previous
            trace.reverse()
            return {"variant": mutant, "result": "counterexample", "states_seen": len(parents),
                    "transitions_examined": transitions, "violations": failures, "trace": trace}
        for label, next_state in successors(current, mutant, max_revision):
            transitions += 1
            if next_state not in parents:
                parents[next_state] = current, label
                queue.append(next_state)
    return {"variant": mutant, "result": "no_counterexample_in_finite_model",
            "states_seen": len(parents), "transitions_examined": transitions,
            "phases_reached": sorted(covered)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path)
    parser.add_argument("--max-revision", type=int, default=4)
    args = parser.parse_args()
    if args.max_revision < 4:
        parser.error("max-revision must be >= 4 to cover resume and terminal kill")
    variants = ["none", "no_effect_fence", "no_callback_guard", "pause_before_durable",
                "timeout_means_rollback", "kill_before_quiescent"]
    results = [explore(v, args.max_revision) for v in variants]
    report = {"scope": "finite design sketch; NOT E2B execution or an unbounded proof",
              "bounds": {"sandboxes": 1, "incarnations": 2, "max_revision": args.max_revision,
                         "controller_crashes": "unbounded repetitions with finite state quotient",
                         "messages": "arbitrary delay and duplicate delivery; no network FIFO"},
              "results": results}
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    for r in results:
        print(f"{r['variant']}: {r['result']}; {r['states_seen']} states; "
              f"{r['transitions_examined']} transitions; {len(r.get('trace', []))} witness steps")
    assert results[0]["result"] == "no_counterexample_in_finite_model", results[0]
    assert set(results[0]["phases_reached"]) == {"Empty", "Starting", "Running", "Pausing", "Paused", "Killing", "Destroyed"}
    assert all(r["result"] == "counterexample" for r in results[1:]), results


if __name__ == "__main__":
    main()
