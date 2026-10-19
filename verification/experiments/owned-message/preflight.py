"""Original Charon preflight of actual CommandSubscription::next on a6000.

An unsupported coroutine is a translator limitation, NOT a semantic negative.
No production source, tool or failed LLBC is rewritten to obtain success.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]


def source_inputs():
    names = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).decode().split('\0')
    names = [n for n in names if n and (n in ('Cargo.lock', 'Cargo.toml', 'rust-toolchain.toml')
             or n.split('/')[0].startswith(('sandbox-', 'controller-')))]
    return {n: hashlib.sha256((ROOT/n).read_bytes()).hexdigest() for n in names}


def preflight(output):
    output.mkdir(parents=True, exist_ok=False)
    tools = Path(os.environ.get('SANDPROOF_PROOF_TOOL_ROOT', str(ROOT/'.tools')))
    for directory, expected in [('aeneas', '505b6ca35217e7be5c96c3e2f8045edfbdf47291'),
                                ('aeneas/charon', 'b104e24fea7d721b71e6c39fd70f26ff20bc0980')]:
        actual = subprocess.check_output(['git', '-C', str(tools/directory), 'rev-parse', 'HEAD'], text=True).strip()
        if actual != expected:
            raise ValueError('Unreviewed original tool version: ' + directory)
    if 'CARGO_TARGET_DIR' not in os.environ:
        raise ValueError('Reuse the registered formal compiler target, not a new CI target')
    before = source_inputs()
    path = output/'command-next.llbc'
    command = [str(tools/'aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas', '--error-on-warnings',
        '--sysroot', 'default', '--opaque', 'tokio', '--opaque', 'bytes', '--opaque', 'hyper',
        '--opaque', 'axum', '--opaque', 'sandproof_guest_runtime', '--include', 'sandproof_guest_service',
        '--start-from', 'crate::command::streams::_::next', '--dest-file', str(path),
        '--', '--manifest-path', 'sandbox-guest-service/Cargo.toml', '--lib', '--locked', '--offline']
    start = time.monotonic()
    with (output/'charon.log').open('xb') as log:
        result = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=False, timeout=900)
    partial = json.loads(path.read_text()) if path.exists() else None
    log = (output/'charon.log').read_text()
    actual_unsupported = (result.returncode != 0 and partial is not None
                          and partial.get('has_errors') is True and 'Coroutines are not supported' in log)
    receipt = dict(kind='translation-preflight-not-semantic-counterexample', command=command,
        source=before, sources_unchanged=source_inputs()==before, exit_code=result.returncode,
        seconds=round(time.monotonic()-start,3), coroutine_unsupported=actual_unsupported,
        partial_llbc_sha256=hashlib.sha256(path.read_bytes()).hexdigest() if path.exists() else None)
    (output/'result.json').write_text(json.dumps(receipt, indent=2)+'\n')
    if not receipt['sources_unchanged'] or not actual_unsupported:
        raise ValueError('Preflight differs from reviewed async limitation; inspect original evidence: '+str(output))
    print(json.dumps({k:v for k,v in receipt.items() if k != 'source'},indent=2))


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    preflight(parser.parse_args().output.resolve())
