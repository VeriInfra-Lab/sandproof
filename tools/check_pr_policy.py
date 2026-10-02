#!/usr/bin/env python3
"""Validate task stacks, the normal workflow and the temporary dot-ning batch."""

import argparse
import json
from pathlib import Path
import re
import sys


TASK_BRANCH = re.compile(
    r"(?:codex/)?(?:feat|fix|refactor|test|docs|ci|chore|proof)/"
    r"[1-9][0-9]*-[a-z0-9]+(?:-[a-z0-9]+)*"
)


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
    task_target = is_task_branch(base)
    if base not in {"main", "test", "dot-ning"} and not task_target:
        return (
            "Pull requests must target test, main, a normally named task branch, "
            "or the temporary dot-ning batch."
        )
    if head == base:
        return "Source and target branches must differ."

    # Stacks and the exact-name batch exception are same-repository task flows.
    # Handle them before promotion/sync so main/test cannot enter either flow.
    if task_target or base == "dot-ning":
        target_kind = "A task stack" if task_target else "The dot-ning batch"
        if not is_task_branch(head):
            return f"{target_kind} accepts only normally named task branches."
        try:
            head_repo = pr["head"]["repo"]["id"]
            base_repo = pr["base"]["repo"]["id"]
        except (KeyError, TypeError):
            return f"{target_kind} requires repository identity metadata."
        if (
            type(head_repo) is not int
            or type(base_repo) is not int
            or head_repo <= 0
            or head_repo != base_repo
        ):
            return f"{target_kind} accepts only this repository's task branches."
        return None

    if head in {"main", "test"}:
        try:
            head_repo = pr["head"]["repo"]["id"]
            base_repo = pr["base"]["repo"]["id"]
        except (KeyError, TypeError):
            return "Promotion/sync requires repository identity metadata."
        if (
            type(head_repo) is not int
            or type(base_repo) is not int
            or head_repo <= 0
            or head_repo != base_repo
        ):
            return "Promotion/sync must use this repository's main/test branches."
        return None

    if base != "test":
        return "Only this repository's test branch may promote to main."
    if not is_task_branch(head):
        return "Expected [codex/]<type>/<issue>-<lowercase-slug>, at most 100 characters."
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
