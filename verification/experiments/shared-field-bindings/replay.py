"""Lightweight ordinary-host reference checks; no Rust/Lean/tool installation."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.check_shared_field_bindings import check, check_frozen


def run(root, output):
    if output.exists() or output.is_symlink():
        raise ValueError('fresh private ordinary-host evidence directory required')
    output.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    record = dict(kind='shared-field-reference-catalogue-checks-not-program-proof', steps=[])
    try:
        before = check_frozen(root)
        record['catalogue'] = check(root)
        env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1')
        for name, command in [
            ('regressions', ['bash', 'scripts/python-entry.sh', '-m', 'unittest',
                             'tools.tests.test_shared_field_bindings', '-v']),
            ('documentation', ['bash', 'scripts/python-entry.sh', 'tools/check_docs.py']),
        ]:
            began = time.monotonic()
            log = output / (name + '.log')
            with log.open('xb') as stream:
                code = subprocess.run(command, cwd=root, env=env, stdout=stream,
                                      stderr=subprocess.STDOUT, timeout=180).returncode
            record['steps'].append(dict(name=name, command=command, exit_code=code,
                seconds=time.monotonic() - began, log_sha256=hashlib.sha256(log.read_bytes()).hexdigest()))
            if code:
                raise ValueError(name + ' failed; preserve original log')
        if check_frozen(root) != before:
            raise ValueError('reference source changed during ordinary checks')
        record.update(success=True, input_sha256=before)
    except BaseException as error:
        record.update(success=False, error=repr(error))
    record['seconds'] = time.monotonic() - start
    (output / 'result.json').write_text(json.dumps(record, ensure_ascii=False, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=ROOT)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = run(args.root.resolve(strict=True), args.output.absolute())
    print(json.dumps({k: v for k, v in result.items() if k != 'input_sha256'}, ensure_ascii=False))
    raise SystemExit(0 if result['success'] else 1)
