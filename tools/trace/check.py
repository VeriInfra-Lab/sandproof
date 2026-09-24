#!/usr/bin/env python3
"""CLI: check a JSONL trace against the safety-property rules.

Usage: tools/trace/check.py <trace.jsonl>
Exit codes: 0 Conforms, 1 Violates, 2 Inconclusive.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from tools.trace import mapper
from tools.trace.checker import Checker, render


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 3
    verdict = Checker().run(mapper.load(sys.argv[1]))
    print(render(verdict))
    return {"Conforms": 0, "Violates": 1, "Inconclusive": 2}[verdict.status]


if __name__ == "__main__":
    sys.exit(main())
