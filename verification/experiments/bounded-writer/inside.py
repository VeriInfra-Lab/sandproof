"""Run only this component from the existing fixed proof runtime.

The coordinator supplies an operator-owned read-only directory lease and the
measured bwrap alias. `extract-only` is explicitly not proof acceptance; `run`
always executes both extractions, all physical files and both audit sets.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess

from lease import admit, validate
from replay import ASSETS, component


def run(lease, archive, output, complete):
    executable = Path(os.environ['SANDPROOF_PROOF_BWRAP'])
    paths = admit(lease, executable)
    root, work, target = (paths[name] for name in ('source', 'work', 'target'))
    if Path(__file__).resolve().parents[3] != root:
        raise ValueError('execute inside.py from the exact leased source tree')
    archive = archive.resolve(strict=True)
    if not any(parent == archive.parent or parent in archive.parents for parent in (root, work)):
        raise ValueError('baseline archive must belong to a leased source/work directory')
    output = output.absolute()
    if output.resolve() != output or work not in output.parents:
        raise ValueError('new component output must be inside the exact leased work directory')
    document = json.loads(lease.read_text())
    lease_bytes = lease.read_bytes()

    def verify_mapping():
        if lease.read_bytes() != lease_bytes or validate(document) != paths:
            raise ValueError('operator mapping changed during component execution')

    def launch(base, command, log):
        verify_mapping()
        runtime = dict(os.environ, SANDPROOF_WRITER_TARGET=str(target),
                       PYTHONDONTWRITEBYTECODE='1')
        args = ['bash', str(root / ASSETS / 'inside-runtime.sh'), str(base)] + command
        with log.open('xb') as stream:
            code = subprocess.run(args, env=runtime, stdout=stream, stderr=subprocess.STDOUT,
                                  timeout=1200).returncode
        verify_mapping()
        return args, code

    result = component(root, archive, output, launch, complete,
                       {'runtime_kind': 'existing-runtime-narrow-nested-mapping',
                        'directory_lease': document}, verify_mapping)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    for name in ('extract-only', 'run'):
        entry = sub.add_parser(name)
        entry.add_argument('--lease', type=Path, required=True)
        entry.add_argument('--source-archive', type=Path, required=True)
        entry.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = run(args.lease, args.source_archive, args.output, args.command == 'run')
    print(json.dumps({k: v for k, v in result.items() if k != 'frozen_inputs'}))
    raise SystemExit(0 if result['success'] else 1)
