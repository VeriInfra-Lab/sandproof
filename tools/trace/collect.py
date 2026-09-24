#!/usr/bin/env python3
"""Trace collector for the local E2B harness.

Emits a JSONL trace for a named scenario:
  normal          create -> pause -> resume -> kill
  race            F-1: kill during an in-flight resume
  dup_kill        kill twice
  dup_pause       pause twice
  resume_resume   two concurrent resumes
  kill_resume     resume a killed sandbox
  kill_pause      pause a killed sandbox
  multi           several sandboxes with interleaved lifecycle ops
  all             run every scenario (separate traces; not valid JSONL here)

Usage: tools/trace/collect.py <scenario> [> out.jsonl]
"""
import json
import os
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from tools.trace import harness
from tools.trace.observe import Observer

_observer = Observer()
_lock = threading.Lock()
_seq = 0
_records = []


def now_ns():
    return time.monotonic_ns()


def _emit(record):
    global _seq
    with _lock:
        _seq += 1
        record["seq"] = _seq
        record.setdefault("ts", now_ns())
        _records.append(record)


def event(name, sid, status, body="", method=None, path=None, t=None):
    rec = {
        "source": "client", "kind": "event", "event": name, "sandbox": sid or "?",
        "attrs": {"method": method, "path": path, "http": status,
                  "result": _classify(status), "raw_body": body},
    }
    if t is not None:
        rec["ts"] = t
    _emit(rec)
    _emit({"source": "observer", "kind": "observe", "sandbox": sid or "?",
           "obs": _observer.sample(sid) if sid else {}})


def _classify(status):
    if status is None:
        return "unknown"
    return "success" if 200 <= status < 300 else "reject"


def create():
    status, body = harness.http("POST", "/sandboxes",
                                {"templateID": harness.TPL, "timeout": 600})
    sid = None
    if status and 200 <= status < 300:
        try:
            sid = json.loads(body)["sandboxID"]
        except (json.JSONDecodeError, KeyError):
            sid = None
    return sid, status, body


def _life(sid, op, http):
    status, body = http()
    event(op, sid, status, body)


def normal():
    sid, status, body = create()
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    _life(sid, "acceptPause", lambda: harness.http("POST", f"/sandboxes/{sid}/pause"))
    _life(sid, "acceptResume", lambda: harness.http("POST", f"/sandboxes/{sid}/resume", {}))
    _life(sid, "acceptKill", lambda: harness.http("DELETE", f"/sandboxes/{sid}"))
    return sid


def race():
    sid, status, body = create()
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    _life(sid, "acceptPause", lambda: harness.http("POST", f"/sandboxes/{sid}/pause"))
    result = {}

    def resume_worker():
        result["r"] = harness.http("POST", f"/sandboxes/{sid}/resume", {})

    thread = threading.Thread(target=resume_worker)
    thread.start()
    time.sleep(2)
    _life(sid, "acceptKill", lambda: harness.http("DELETE", f"/sandboxes/{sid}"))
    thread.join()
    status, body = result["r"]
    event("acceptResume", sid, status, body, "POST", f"/sandboxes/{sid}/resume")
    return sid


def dup_kill():
    sid, status, body = create()
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    _life(sid, "acceptKill", lambda: harness.http("DELETE", f"/sandboxes/{sid}"))
    _life(sid, "acceptKill", lambda: harness.http("DELETE", f"/sandboxes/{sid}"))
    return sid


def dup_pause():
    sid, status, body = create()
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    _life(sid, "acceptPause", lambda: harness.http("POST", f"/sandboxes/{sid}/pause"))
    _life(sid, "acceptPause", lambda: harness.http("POST", f"/sandboxes/{sid}/pause"))
    return sid


def resume_resume():
    sid, status, body = create()
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    _life(sid, "acceptPause", lambda: harness.http("POST", f"/sandboxes/{sid}/pause"))
    out = {}

    def worker(key):
        out[key] = harness.http("POST", f"/sandboxes/{sid}/resume", {})

    threads = [threading.Thread(target=worker, args=(k,)) for k in ("a", "b")]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    for key in ("a", "b"):
        status, body = out.get(key, (None, ""))
        event("acceptResume", sid, status, body, "POST", f"/sandboxes/{sid}/resume")
    return sid


def kill_resume():
    sid, status, body = create()
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    _life(sid, "acceptKill", lambda: harness.http("DELETE", f"/sandboxes/{sid}"))
    _life(sid, "acceptResume", lambda: harness.http("POST", f"/sandboxes/{sid}/resume", {}))
    return sid


def kill_pause():
    sid, status, body = create()
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    _life(sid, "acceptKill", lambda: harness.http("DELETE", f"/sandboxes/{sid}"))
    _life(sid, "acceptPause", lambda: harness.http("POST", f"/sandboxes/{sid}/pause"))
    return sid


def multi():
    sids = []
    for _ in range(3):
        sid, status, body = create()
        event("acceptCreate", sid, status, body, "POST", "/sandboxes")
        sids.append(sid)
    for sid in sids[:2]:
        _life(sid, "acceptPause", lambda sid=sid: harness.http("POST", f"/sandboxes/{sid}/pause"))
    for sid in sids[:1]:
        _life(sid, "acceptResume", lambda sid=sid: harness.http("POST", f"/sandboxes/{sid}/resume", {}))
    for sid in sids:
        _life(sid, "acceptKill", lambda sid=sid: harness.http("DELETE", f"/sandboxes/{sid}"))
    return sids[0]


def observe_only(sid):
    _emit({"source": "observer", "kind": "observe", "sandbox": sid,
           "obs": _observer.sample(sid)})


def observe_global():
    """Sample the shared index/store consistency (no single sandbox)."""
    _emit({"source": "observer", "kind": "observe", "sandbox": "__global__",
           "obs": _observer.sample("__global__")})


def bad_create():
    """A rejected create must not leave an orphan/registration behind."""
    status, body = harness.http("POST", "/sandboxes",
                                {"templateID": "00000000-0000-0000-0000-000000000000",
                                 "timeout": 60})
    event("acceptCreate", None, status, body, "POST", "/sandboxes")
    time.sleep(2)
    observe_global()
    return None


def stale_index_control():
    """Positive control: inject a stale index member and check the rule fires."""
    sid, status, body = create()
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    subprocess.run(["docker", "exec", "redis", "redis-cli", "SADD",
                    harness.INDEX_KEY, "phantom-sandbox"], capture_output=True)
    observe_only(sid)
    subprocess.run(["docker", "exec", "redis", "redis-cli", "SREM",
                    harness.INDEX_KEY, "phantom-sandbox"], capture_output=True)
    observe_only(sid)
    _life(sid, "acceptKill", lambda: harness.http("DELETE", f"/sandboxes/{sid}"))
    return sid


def expire():
    """Create a short-lived sandbox and wait for eviction, then sample."""
    status, body = harness.http("POST", "/sandboxes",
                                {"templateID": harness.TPL, "timeout": 3})
    sid = None
    if status and 200 <= status < 300:
        sid = json.loads(body)["sandboxID"]
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    time.sleep(20)
    observe_only(sid)
    return sid


def stress():
    """Several sandboxes with concurrent pause/resume/kill, then a global sample."""
    sids = []
    for _ in range(4):
        sid, status, body = create()
        event("acceptCreate", sid, status, body, "POST", "/sandboxes")
        sids.append(sid)
    for sid in sids:
        _life(sid, "acceptPause", lambda sid=sid: harness.http("POST", f"/sandboxes/{sid}/pause"))

    outcomes = {}

    def worker(sid):
        r = {}
        outcomes[sid] = r

        def resume():
            r["resume"] = harness.http("POST", f"/sandboxes/{sid}/resume", {})

        def kill():
            time.sleep(2)
            r["kill"] = harness.http("DELETE", f"/sandboxes/{sid}")

        threads = [threading.Thread(target=resume), threading.Thread(target=kill)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

    threads = [threading.Thread(target=worker, args=(sid,)) for sid in sids[:2]]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    for sid in sids[:2]:
        r = outcomes[sid]
        status, body = r.get("kill", (None, ""))
        event("acceptKill", sid, status, body, "DELETE", f"/sandboxes/{sid}")
        status, body = r.get("resume", (None, ""))
        event("acceptResume", sid, status, body, "POST", f"/sandboxes/{sid}/resume")
    for sid in sids[2:]:
        _life(sid, "acceptResume", lambda sid=sid: harness.http("POST", f"/sandboxes/{sid}/resume", {}))
        _life(sid, "acceptKill", lambda sid=sid: harness.http("DELETE", f"/sandboxes/{sid}"))
    observe_only(sids[0])
    return sids[0]


def race_pause_kill():
    """Kill a running sandbox while a pause is in flight (node pause delayed)."""
    sid, status, body = create()
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    out = {}

    def pause():
        out["pause"] = harness.http("POST", f"/sandboxes/{sid}/pause")

    thread = threading.Thread(target=pause)
    thread.start()
    time.sleep(1)
    _life(sid, "acceptKill", lambda: harness.http("DELETE", f"/sandboxes/{sid}"))
    thread.join()
    status, body = out.get("pause", (None, ""))
    event("acceptPause", sid, status, body, "POST", f"/sandboxes/{sid}/pause")
    return sid


def race_double_kill():
    """Two concurrent kills of a running sandbox."""
    sid, status, body = create()
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    out = {}

    def kill(key):
        out[key] = harness.http("DELETE", f"/sandboxes/{sid}")

    threads = [threading.Thread(target=kill, args=(k,)) for k in ("a", "b")]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    for key in ("a", "b"):
        status, body = out.get(key, (None, ""))
        event("acceptKill", sid, status, body, "DELETE", f"/sandboxes/{sid}")
    return sid


def kill_settle():
    """Kill and wait, then check the node no longer holds the sandbox."""
    sid, status, body = create()
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    _life(sid, "acceptKill", lambda: harness.http("DELETE", f"/sandboxes/{sid}"))
    time.sleep(8)
    observe_only(sid)
    observe_global()
    return sid


def autopause_resume():
    """Create with autoPause and a short TTL, then resume."""
    status, body = harness.http("POST", "/sandboxes",
                                {"templateID": harness.TPL, "timeout": 5, "autoPause": True})
    sid = None
    if status and 200 <= status < 300:
        sid = json.loads(body)["sandboxID"]
    event("acceptCreate", sid, status, body, "POST", "/sandboxes")
    time.sleep(30)
    observe_only(sid)
    if sid:
        _life(sid, "acceptResume", lambda: harness.http("POST", f"/sandboxes/{sid}/resume", {}))
    return sid


def client_timeout():
    """The client times out during a slow create; observe whether it landed."""
    observe_global()
    status, body = harness.http("POST", "/sandboxes",
                                {"templateID": harness.TPL, "timeout": 600}, timeout=1)
    event("acceptCreate", None, status, body, "POST", "/sandboxes")
    time.sleep(12)
    observe_global()
    return None


SCENARIOS = {
    "normal": normal, "race": race, "dup_kill": dup_kill, "dup_pause": dup_pause,
    "resume_resume": resume_resume, "kill_resume": kill_resume,
    "kill_pause": kill_pause, "multi": multi,
    "stale_index_control": stale_index_control, "expire": expire, "stress": stress,
    "bad_create": bad_create, "race_pause_kill": race_pause_kill,
    "race_double_kill": race_double_kill, "kill_settle": kill_settle,
    "autopause_resume": autopause_resume, "client_timeout": client_timeout,
}


def run(name):
    global _seq, _records
    _seq, _records = 0, []
    SCENARIOS[name]()
    return list(_records)


def main():
    name = sys.argv[1] if len(sys.argv) > 1 else "normal"
    if name == "all":
        for scenario in SCENARIOS:
            print(json.dumps({"scenario": scenario, "records": run(scenario)}))
        return
    if name not in SCENARIOS:
        print(__doc__)
        sys.exit(1)
    for record in run(name):
        print(json.dumps(record, sort_keys=True))


if __name__ == "__main__":
    main()
