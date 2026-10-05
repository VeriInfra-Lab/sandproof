"""Concrete fact gathering and probe functions for the abstraction clauses.

`gather(sid)` collects the raw concrete facts once; each clause's `probe`
projects one abstract field from them. See verification/README.zh-CN.md for mapping limits.
"""
import json
from dataclasses import dataclass, field
from typing import Any, Optional

from tools.trace import harness

MISSING = object()  # field not observable for this state

_RUNNING_PREFIX = harness.RUNNING_PREFIX
_INDEX_KEY = harness.INDEX_KEY


@dataclass
class Facts:
    sid: str
    record: Optional[dict] = None
    snapshot_present: bool = False
    snapshot_deleted: bool = False
    index_members: list = field(default_factory=list)
    existing_sandboxes: list = field(default_factory=list)
    node_sandboxes: list = field(default_factory=list)
    catalog_present: bool = False


def _record(sid: str) -> Optional[dict]:
    raw = harness.redis_get(_RUNNING_PREFIX + sid)
    if not raw:
        return None
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return None


def _snapshot(sid: str) -> tuple[bool, bool]:
    rows = harness.psql(f"SELECT env_id FROM snapshots WHERE sandbox_id='{sid}';")
    if not rows:
        return False, False
    env_id = rows.splitlines()[0].strip()
    deleted = harness.psql(f"SELECT deleted_at FROM envs WHERE id='{env_id}';")
    return True, bool(deleted)


def gather(sid: str) -> Facts:
    present, deleted = _snapshot(sid)
    existing = [k[len(_RUNNING_PREFIX):]
                for k in harness.redis_keys(_RUNNING_PREFIX + "*")]
    return Facts(
        sid=sid,
        record=_record(sid),
        snapshot_present=present,
        snapshot_deleted=deleted,
        index_members=harness.redis_smembers(_INDEX_KEY),
        existing_sandboxes=existing,
        node_sandboxes=harness.node_sandboxes(),
        catalog_present=bool(harness.redis_get(f"sandbox:catalog:{sid}")),
    )


def _phase_hint(f: Facts) -> str:
    if f.record is not None:
        return f.record.get("state", "running")
    if f.snapshot_present and not f.snapshot_deleted:
        return "paused"
    if f.snapshot_present and f.snapshot_deleted:
        return "deleted"
    return "absent"


def _from_record(key: str):
    return lambda f: f.record.get(key) if f.record is not None else MISSING


PROBES: dict[str, Any] = {
    "running": lambda f: f.record is not None,
    "phase_hint": _phase_hint,
    "snapshot_present": lambda f: f.snapshot_present,
    "snapshot_deleted": lambda f: f.snapshot_deleted,
    "workspace": _from_record("baseTemplateID"),
    "execution_id": _from_record("executionID"),
    "index_members": lambda f: f.index_members,
    "existing_sandboxes": lambda f: f.existing_sandboxes,
    "node_sandboxes": lambda f: f.node_sandboxes,
    "catalog_present": lambda f: f.catalog_present,
}


def project(f: Facts, clause: dict) -> Any:
    """Apply one clause's probe; returns MISSING when not observable here."""
    probe = PROBES.get(clause["probe"])
    if probe is None:
        return MISSING
    return probe(f)
