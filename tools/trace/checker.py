"""Single-pass trace checker over a pluggable rule set."""
from typing import Iterable

from tools.trace.rules import REGISTRY, Rule
from tools.trace.trace_model import Finding, Record, Verdict


class Checker:
    def __init__(self, rules: Iterable[Rule] = REGISTRY):
        self._rules = list(rules)
        # rule name -> number of records it was actually applied to.
        self.coverage: dict[str, int] = {rule.name: 0 for rule in self._rules}

    def run(self, records: Iterable[Record]) -> Verdict:
        states: dict[tuple[str, str], dict] = {}
        findings: list[Finding] = []
        for record in records:
            sandbox = record.sandbox or "?"
            for rule in self._rules:
                if not rule.applies(record):
                    continue
                self.coverage[rule.name] += 1
                state = states.setdefault((rule.name, sandbox), {})
                if record.kind == "event":
                    finding = rule.on_event(state, record)
                else:
                    finding = rule.on_observe(state, record)
                if finding is not None:
                    findings.append(finding)
        # end-of-trace checks (e.g. completion)
        by_name = {rule.name: rule for rule in self._rules}
        for (rule_name, sandbox), state in states.items():
            finding = by_name[rule_name].on_finish(state, sandbox)
            if finding is not None:
                findings.append(finding)
        return Verdict(_status(findings), findings)


def _status(findings: list[Finding]) -> str:
    if any(not f.inconclusive for f in findings):
        return "Violates"
    if findings:
        return "Inconclusive"
    return "Conforms"


def render(verdict: Verdict) -> str:
    lines = [f"verdict: {verdict.status}"]
    for finding in verdict.findings:
        kind = " (inconclusive)" if finding.inconclusive else ""
        lines.append(f"  rule={finding.rule} sandbox={finding.sandbox} seq={finding.seq}{kind}")
        lines.append(f"    {finding.detail}")
        lines.append(f"    minimal_subsequence={finding.minimal_subsequence}")
        lines.append(f"    properties={finding.properties}")
    return "\n".join(lines)
