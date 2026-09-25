"""obs/evt mapping: concrete JSONL records -> abstract events and observations.

The format is documented in `docs/trace-tools.md`. The mapping is deliberately
small: the abstract event names are already written by the collector; the
mapper validates and normalises them so that rules depend on a stable
vocabulary rather than raw record shapes.
"""
import json
from typing import Iterator, Optional

from tools.trace.trace_model import Record

# Abstract events produced by the collector for the controller lifecycle.
LIFECYCLE_EVENTS = {
    "acceptCreate",
    "acceptPause",
    "acceptResume",
    "acceptKill",
    "rpcTimeout",
}

OBS_KEYS = {
    "running",
    "snapshot_present",
    "snapshot_deleted",
    "phase_hint",
}


def load(path: str) -> list[Record]:
    records: list[Record] = []
    with open(path) as handle:
        for line in handle:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            records.append(Record(**json.loads(line)))
    records.sort(key=lambda r: r.seq)
    return records


def result_of(record: Record) -> Optional[str]:
    return record.attrs.get("result")


def sandbox_of(record: Record) -> Optional[str]:
    return record.sandbox


def is_event(record: Record, name: str) -> bool:
    return record.kind == "event" and record.event == name


def is_observation(record: Record) -> bool:
    return record.kind == "observe"


def running(record: Record) -> Optional[bool]:
    value = record.obs.get("running")
    return value if isinstance(value, bool) else None


def phase_hint(record: Record) -> Optional[str]:
    return record.obs.get("phase_hint")


def snapshot_present(record: Record) -> Optional[bool]:
    value = record.obs.get("snapshot_present")
    return value if isinstance(value, bool) else None


def snapshot_deleted(record: Record) -> Optional[bool]:
    value = record.obs.get("snapshot_deleted")
    return value if isinstance(value, bool) else None
