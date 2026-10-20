"""Kernel, original physical-file service and exact audits in one private view."""
from __future__ import annotations

import argparse
import importlib.util
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.check_bounded_writer import ASSETS, LIBRARY, MODELS, NEGATIVE_PROOF, check_frozen, check_pair, check_sources, check_tools, expected_audits
# Reuse the existing complete/non-stale physical-file protocol checker.
_spec = importlib.util.spec_from_file_location('bounded_writer_service_protocol',
    ROOT / 'verification/experiments/schema-domains/replay.py')
_service = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_service)
diagnostic = _service.diagnostic


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prove(root, variant, output, tool_root):
    if str(root) != '/tmp/bounded-writer/source' or Path.cwd() != root:
        raise ValueError('registered private source mapping required')
    if output.exists():
        raise ValueError('fresh formal evidence directory required')
    check_frozen(root, variant)
    check_sources(root, variant)
    check_pair(root)
    check_tools(root, tool_root)
    output.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    record = dict(kind='bounded-writer-physical-proof-component-not-full-suite', variant=variant)
    env = dict(os.environ, LEAN_NUM_THREADS='2', LEAN_STACK_SIZE_KB='65536', PYTHONDONTWRITEBYTECODE='1')
    negative = variant == 'no-limit'
    try:
        common = [MODELS / name for name in ('Model.lean', 'Contract.lean', 'Library.lean')]
        if negative:
            copied = MODELS / 'Negative.lean'
            if (root / copied).exists():
                raise ValueError('private negative view already used')
            shutil.copyfile(root / NEGATIVE_PROOF, root / copied)
            target = 'SandboxModel.Boundaries.BoundedWriter.Negative'
            files = common + [copied]
        else:
            target = 'SandboxModel.Boundaries.BoundedWriter.Audit'
            files = common + [MODELS / name for name in ('Refinement.lean', 'Examples.lean', 'Audit.lean')]
            files += [Path('SandboxModel/Boundaries/BoundedWriter.lean')]
        command = ['lake', 'build', target]
        with (output / 'build.log').open('xb') as log:
            code = subprocess.run(command, cwd=root, env=env, stdout=log,
                                  stderr=subprocess.STDOUT, timeout=600).returncode
        record['build'] = dict(command=command, exit_code=code, log_sha256=sha(output / 'build.log'))
        if code:
            raise ValueError('actual module build failed')
        files += [LIBRARY / 'SandproofBoundedWriter' / name for name in ('TypesExternal.lean', 'Types.lean', 'Funs.lean')]
        expected = expected_audits(root, negative)
        record['files'] = [diagnostic(root, str(name), output / ('file-' + str(i)), env)
                           for i, name in enumerate(files)]
        observed = sum(item['audits'] for item in record['files'])
        if observed != len(expected):
            raise ValueError('exact audit total changed')
        if negative:
            for name in ('Model.lean', 'Contract.lean', 'Library.lean'):
                if sha(root / MODELS / name) != check_pair(root)['same_property_files'][name]:
                    raise ValueError('negative changed the correct specification')
        record.update(success=True, audits=len(expected))
    except BaseException as error:
        record.update(success=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic() - start
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--variant', choices=['correct', 'no-limit'], required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--tool-root', type=Path, required=True)
    args = parser.parse_args()
    result = prove(ROOT, args.variant, args.output.resolve(), args.tool_root.resolve(strict=True))
    print(json.dumps(result))
    raise SystemExit(0 if result['success'] else 1)
