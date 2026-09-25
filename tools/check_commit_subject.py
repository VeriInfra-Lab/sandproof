#!/usr/bin/env python3
"""Check this project's Conventional Commit subject or a pull-request title."""

import argparse
import json
import re
import sys
from pathlib import Path


SUBJECT = re.compile(
    r"(?:feat|fix|refactor|test|docs|build|ci|chore|perf|revert)"
    r"\((?:core|service|runtime|api|contract|test|bench|docs|ci|deps|repo|proof)\)"
    r"!?: \S[^\r\n]*"
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("subject", nargs="?", help="Commit subject or PR title")
    parser.add_argument("--event", type=Path, help="GitHub pull_request event JSON")
    args = parser.parse_args()
    if (args.subject is None) == (args.event is None):
        parser.error("provide exactly one subject or --event")

    subject = args.subject
    if args.event is not None:
        try:
            event = json.loads(args.event.read_text(encoding="utf-8"))
            subject = event["pull_request"]["title"]
        except (OSError, ValueError, KeyError, TypeError) as error:
            print(f"Cannot read pull-request title: {error}", file=sys.stderr)
            return 1

    if (
        not isinstance(subject, str)
        or not SUBJECT.fullmatch(subject)
        or len(subject) > 100
        or any(ord(character) < 32 or ord(character) == 127 for character in subject)
    ):
        print(
            "Expected type(scope): description, at most 100 characters. "
            "See CONTRIBUTING.md for allowed types and scopes.",
            file=sys.stderr,
        )
        return 1

    print("Commit/PR subject format passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
