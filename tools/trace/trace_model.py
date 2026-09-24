"""Trace data types shared by the collector, mapper, rules, and checker."""
from dataclasses import dataclass, field
from typing import Any, Optional


@dataclass
class Record:
    """One JSONL line: an observed event or state sample."""
    seq: int
    ts: int
    source: str
    kind: str  # "event" | "observe"
    sandbox: Optional[str] = None
    event: Optional[str] = None
    attrs: dict[str, Any] = field(default_factory=dict)
    obs: dict[str, Any] = field(default_factory=dict)


@dataclass
class Finding:
    """A rule violation (or an undecidable suspicion)."""
    rule: str
    sandbox: str
    seq: int
    detail: str
    minimal_subsequence: list[int]
    properties: list[str]
    inconclusive: bool = False


@dataclass
class Verdict:
    status: str  # Conforms | Violates | Inconclusive
    findings: list[Finding]
