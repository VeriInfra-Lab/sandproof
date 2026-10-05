#!/usr/bin/env python3
"""Validate integration, release promotion, sync and same-repository task stacks."""

import argparse
import json
from pathlib import Path
import re
import sys


TASK_BRANCH = re.compile(
    r"(?:codex/)?(?:feat|fix|refactor|test|docs|ci|chore|proof)/"
    r"[1-9][0-9]*-[a-z0-9]+(?:-[a-z0-9]+)*"
)
LONG_LIVED = {"integration", "test", "main"}
RELEASE_FLOWS = {
    ("integration", "test"), ("test", "main"),
    ("main", "test"), ("test", "integration"),
}


def is_task_branch(branch):
    return len(branch) <= 100 and TASK_BRANCH.fullmatch(branch) is not None


def validate(event):
    """Return None on success, otherwise a user-facing error; no API calls."""
    try:
        pr = event["pull_request"]
        head = pr["head"]["ref"]
        base = pr["base"]["ref"]
    except (KeyError, TypeError):
        return "Missing pull-request head/base metadata."
    if not isinstance(head, str) or not isinstance(base, str):
        return "Branch names must be strings."
    if head == base:
        return "Source and target branches must differ."
    task_target = is_task_branch(base)
    if base not in LONG_LIVED and not task_target:
        return "Target integration, test, main, or a normally named task branch."

    if head in LONG_LIVED:
        if (head, base) not in RELEASE_FLOWS:
            return "Use integration -> test -> main; sync in the reverse direction."
    elif not is_task_branch(head):
        return "Expected [codex/]<type>/<issue>-<lowercase-slug>, at most 100 characters."
    elif base != "integration" and not task_target:
        return "Task branches must enter integration before release promotion."

    try:
        head_repo = pr["head"]["repo"]["id"]
        base_repo = pr["base"]["repo"]["id"]
    except (KeyError, TypeError):
        return "Pull requests require repository identity metadata."
    if (
        type(head_repo) is not int
        or type(base_repo) is not int
        or head_repo <= 0
        or head_repo != base_repo
    ):
        return "Integration, promotion and task stacks require the same repository."
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--event", required=True, type=Path)
    args = parser.parse_args()
    try:
        event = json.loads(args.event.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        print(f"Cannot read PR event: {error}", file=sys.stderr)
        return 1
    error = validate(event)
    if error:
        print(f"{error} See CONTRIBUTING.md.", file=sys.stderr)
        return 1
    print("PR branch name and target policy passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
