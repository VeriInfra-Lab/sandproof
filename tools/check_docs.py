#!/usr/bin/env python3
"""Check local inline Markdown file links and the docs topic index (offline)."""

import re
from pathlib import Path
import subprocess
import sys
from urllib.parse import unquote, urlsplit


ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"!?\[[^\]\n]*\]\(([^)\n]+)\)")


def targets(markdown):
    """Yield inline link targets outside fenced code; external links are ignored."""
    fence = None
    for line in markdown.splitlines():
        stripped = line.lstrip()
        if stripped.startswith(("```", "~~~")):
            marker = stripped[0]
            if fence is None:
                fence = marker
            elif fence == marker:
                fence = None
            continue
        if fence is not None:
            continue
        for match in LINK.finditer(line):
            value = match.group(1).strip()
            if value.startswith("<"):
                value = value[1:].split(">", 1)[0]
            else:
                value = value.split()[0] if value else ""
            parsed = urlsplit(value)
            if parsed.scheme or parsed.netloc or not parsed.path:
                continue
            yield unquote(parsed.path)


def validate(root, files):
    root = root.resolve()
    errors = []
    index = root / "docs/README.md"
    indexed = set()
    for file in files:
        path = root / file
        for target in targets(path.read_text(encoding="utf-8")):
            resolved = (path.parent / target).resolve()
            if not resolved.is_relative_to(root):
                errors.append(f"{file}: link escapes repository: {target}")
            elif not resolved.exists():
                errors.append(f"{file}: missing link target: {target}")
            if path == index:
                indexed.add(resolved)
    if not index.is_file():
        errors.append("Missing docs/README.md topic index.")
    for file in files:
        path = root / file
        if file.startswith("docs/") and path != index and path.resolve() not in indexed:
            errors.append(f"{file}: not listed in docs/README.md.")
    return errors


def main():
    listed = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "*.md"],
        cwd=ROOT,
    ).decode("utf-8").split("\0")
    # Deleted tracked files remain in the index until staged; they are not inputs.
    files = sorted({file for file in listed if file and (ROOT / file).is_file()})
    errors = validate(ROOT, files)
    for error in errors:
        print(error, file=sys.stderr)
    if errors:
        return 1
    print(f"Documentation checks passed: {len(files)} Markdown files, local links and topic index.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
