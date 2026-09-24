"""Load the abstraction-clause registry (alpha clauses)."""
import json
import os
from typing import Optional

_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "clauses.json")


def load() -> dict:
    with open(_PATH) as handle:
        return json.load(handle)


def clauses() -> list[dict]:
    return load()["clauses"]


def observable() -> list[dict]:
    return [c for c in clauses() if c["observability"] == "observable"]


def non_observable() -> list[dict]:
    return [c for c in clauses() if c["observability"] != "observable"]


def by_field() -> dict[str, dict]:
    return {c["abstract_field"]: c for c in clauses()}