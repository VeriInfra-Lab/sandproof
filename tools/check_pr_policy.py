#!/usr/bin/env python3
"""Validate the normal workflow and the owner-approved dot-ning batch."""

import argparse
import json
from pathlib import Path
import re
import sys


TASK_BRANCH = re.compile(
    r"(?:codex/)?(?:feat|fix|refactor|test|docs|ci|chore|proof)/"
    r"[1-9][0-9]*-[a-z0-9]+(?:-[a-z0-9]+)*"
)


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
    if base not in {"main", "test", "dot-ning"}:
        return "Pull requests must target test, main, or the temporary dot-ning batch."
    if head == base:
        return "Source and target branches must differ."

    # Temporary, exact-name exception for the owner's current integration batch.
    # Handle it before main/test promotion so those branches cannot enter it.
    if base == "dot-ning":
        if len(head) > 100 or not TASK_BRANCH.fullmatch(head):
            return "The dot-ning batch accepts only normally named task branches."
        try:
            head_repo = pr["head"]["repo"]["id"]
            base_repo = pr["base"]["repo"]["id"]
        except (KeyError, TypeError):
            return "The dot-ning batch requires repository identity metadata."
        if (
            type(head_repo) is not int
            or type(base_repo) is not int
            or head_repo <= 0
            or head_repo != base_repo
        ):
            return "The dot-ning batch accepts only this repository's task branches."
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
        return "Task branches must target test; only test may promote to main."
    if len(head) > 100 or not TASK_BRANCH.fullmatch(head):
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
