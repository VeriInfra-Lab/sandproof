"""Observe the local E2B harness and project it onto abstract World fields.

The projection is driven by the abstraction-clause registry
(`tools/mapping/clauses.json`); only clauses marked `observable` are probed.
Unobservable/partial fields are deliberately omitted so the checker does not
constrain them.
"""
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from tools.mapping import probes, registry, probes as _probes  # noqa: F401


class Observer:
    def sample(self, sid):
        facts = probes.gather(sid)
        obs = {}
        for clause in registry.observable():
            value = probes.project(facts, clause)
            if value is probes.MISSING:
                continue
            obs[clause["abstract_field"]] = value
        return obs