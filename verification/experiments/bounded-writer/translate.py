"""Formal-only original extraction in the registered fixed virtual source layout."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.check_bounded_writer import check_frozen, check_generated, check_ir, check_sources, check_tools


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inputs(root):
    return {str(p.relative_to(root)): sha(p) for p in sorted(root.rglob('*'))
            if p.is_file() and not p.is_symlink()
            and not any(x in ('.git', '.tools', '.lake', 'target', 'artifacts') for x in p.relative_to(root).parts)
            and (p.suffix == '.rs' or p.name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'))}


def extract(root, variant, tool_root):
    if str(root) != '/tmp/bounded-writer/source' or Path.cwd() != root:
        raise ValueError('Enter the registered private runtime at /tmp/bounded-writer/source')
    for name, value in {'CARGO_HOME': '/tmp/bounded-writer/cargo-home',
                        'CARGO_TARGET_DIR': '/tmp/bounded-writer/target-formal',
                        'TMPDIR': '/tmp/bounded-writer/tmp'}.items():
        if os.environ.get(name) != value:
            raise ValueError('unregistered runtime mapping: ' + name)
    for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'LD_PRELOAD', 'LD_LIBRARY_PATH'):
        if os.environ.get(name):
            raise ValueError('unregistered compilation/runtime option: ' + name)
    check_frozen(root, variant)
    check_sources(root, variant)
    check_tools(root, tool_root)
    out = Path('/tmp/bounded-writer/evidence/preflight/policy')
    if out.exists():
        raise ValueError('Fresh physical variant directory required; never overwrite prior extraction')
    out.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    record = dict(kind='bounded-writer-original-tools-not-proof', variant=variant, steps=[])
    before = inputs(root)

    def run(args, name, timeout):
        began = time.monotonic()
        try:
            with (out / (name + '.log')).open('xb') as stream:
                result = subprocess.run(args, cwd=root, stdout=stream, stderr=subprocess.STDOUT,
                                        timeout=timeout).returncode
        except subprocess.TimeoutExpired:
            result = 124
        record['steps'].append(dict(name=name, command=args, exit_code=result,
            seconds=time.monotonic() - began, log_sha256=sha(out / (name + '.log'))))
        if result:
            raise ValueError(name + ' failed; this is not a semantic counterexample')

    try:
        llbc = out / 'actual.llbc'
        charon = tool_root / 'aeneas/charon/bin/charon'
        run([str(charon), 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
             '--targets', 'x86_64-unknown-linux-gnu', '--opaque', 'rusqlite::error::Error',
             '--include', 'sandproof_controller_service', '--include', 'sandproof_internal',
             '--include', 'sandproof_protocol', '--start-from', 'crate::candidate::write_entry',
             '--start-from', 'crate::candidate::finish_entry', '--start-from', 'crate::candidate::response_entry',
             '--dest-file', str(llbc), '--', '--manifest-path', 'verification/extract-bounded-writer/Cargo.toml',
             '--lib', '--features', 'candidate', '--locked', '--offline'], 'charon', 600)
        data = json.loads(llbc.read_text())
        check_ir(root, data, variant)
        generated = out / 'generated'
        generated.mkdir()
        run([str(tool_root / 'aeneas/bin/aeneas'), '-backend', 'lean', '-abort-on-error',
             '-warnings-as-errors', '-split-files', '-emit-json', '-namespace', 'SandproofBoundedWriterRust',
             '-subdir', 'SandproofBoundedWriter', '-dest', str(generated), str(llbc)], 'aeneas', 300)
        check_generated(root, generated, variant)
        if inputs(root) != before:
            raise ValueError('source changed during original extraction')
        record.update(success=True, sources_unchanged=True, source_inputs=before,
                      llbc_sha256=sha(llbc), has_errors=False)
    except BaseException as error:
        record.update(success=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic() - start
    (out / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--variant', choices=['correct', 'no-limit'], required=True)
    parser.add_argument('--tool-root', type=Path, required=True)
    args = parser.parse_args()
    result = extract(ROOT, args.variant, args.tool_root.resolve(strict=True))
    print(json.dumps({k: v for k, v in result.items() if k != 'source_inputs'}))
    raise SystemExit(0 if result['success'] else 1)
