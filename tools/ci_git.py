#!/usr/bin/env python3
"""CI-only Git adapter: fetch locked dependency commits without their history.

Only the exact uv 0.11.31 fetch of a locked public dependency is changed. Other
commands, repositories and revisions are forwarded unchanged. No lock changes,
global Git settings, mirrors, TLS bypass, shared writable caches or host mounts.
"""
import os
from pathlib import Path
import re
import sys
import tomllib
from urllib.parse import parse_qs, urlsplit, urlunsplit

SOURCES = {'e2b': 'https://github.com/e2b-dev/E2B.git',
           'harbor': 'https://github.com/laude-institute/harbor.git'}


def locked_refs(lock):
    refs = set()
    for package in tomllib.loads(Path(lock).read_text())['package']:
        expected = SOURCES.get(package.get('name'))
        if expected is None:
            continue
        source = package.get('source', {}).get('git')
        if not isinstance(source, str):
            raise ValueError('CI dependency is not locked to Git')
        url = urlsplit(source)
        remote = urlunsplit((url.scheme, url.netloc, url.path, '', ''))
        if (remote != expected or not re.fullmatch(r'[0-9a-f]{40}', url.fragment)
                or parse_qs(url.query).get('rev') != [url.fragment]):
            raise ValueError('CI dependency requires an exact public Git revision')
        refs.add((remote, '+' + url.fragment + ':refs/commit/' + url.fragment))
    if not refs:
        raise ValueError('no locked CI Git dependencies')
    return refs


def git_arguments(arguments, refs):
    # Match the complete fetch suffix, not a substring in a path or ref name.
    # Local checkout/reset/version and any unfamiliar fetch remain unchanged.
    for index, argument in enumerate(arguments):
        suffix = arguments[index:]
        if (argument == 'fetch' and len(suffix) == 5
                and suffix[1:3] == ['--force', '--update-head-ok']
                and (suffix[3], suffix[4]) in refs):
            commit = suffix[4].split(':', 1)[0][1:]
            # Git disables --local cloning for shallow sources, and ordinary
            # clone does not advertise uv's refs/commit namespace. Keep uv's
            # original ref AND expose the same commit as a job-private branch.
            branch = '+' + commit + ':refs/heads/sandproof-ci-' + commit
            return arguments[:index + 1] + ['--depth=1'] + arguments[index + 1:] + [branch]
    return arguments


def main():
    try:
        refs = locked_refs(os.environ['CI_GIT_LOCK'])
    except (KeyError, OSError, ValueError, TypeError):
        print('CI Git preparation refused: invalid dependency lock', file=sys.stderr)
        return 2
    os.execv('/usr/bin/git', ['git'] + git_arguments(sys.argv[1:], refs))


if __name__ == '__main__':
    sys.exit(main())
