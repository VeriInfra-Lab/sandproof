"""Observe fixed-tool support for original Session::authorize, never a proof."""
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
from tools.check_first_chain_extraction import check_tools


def inputs():
    names = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).decode().split('\0')
    return {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
            for name in names if name and
            (name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml') or
             name.split('/')[0].startswith(('sandbox-', 'controller-')))}


def preflight(output):
    tools = Path(os.environ.get('SANDPROOF_PROOF_TOOL_ROOT', str(ROOT / '.tools')))
    check_tools(ROOT, tools)
    if 'CARGO_TARGET_DIR' not in os.environ:
        raise ValueError('Use the registered formal compiler target')
    output.mkdir(parents=True, exist_ok=False)
    before = inputs()
    llbc = output / 'original.llbc'
    command = [str(tools / 'aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
               '--error-on-warnings', '--sysroot', 'default', '--opaque', 'tokio',
               '--include', 'sandproof_guest_service', '--start-from',
               'crate::session::_::authorize', '--dest-file', str(llbc), '--',
               '--manifest-path', 'sandbox-guest-service/Cargo.toml', '--lib', '--locked', '--offline']
    start = time.monotonic()
    with (output / 'charon.log').open('xb') as log:
        code = subprocess.run(command, cwd=ROOT, stdout=log,
                              stderr=subprocess.STDOUT, timeout=900).returncode
    result = dict(kind='original-translation-observation-not-proof-or-semantic-counterexample',
                  command=command, sources=before, charon_exit=code,
                  sources_unchanged=inputs() == before,
                  llbc_sha256=hashlib.sha256(llbc.read_bytes()).hexdigest() if llbc.exists() else None,
                  seconds=round(time.monotonic() - start, 3))
    if llbc.exists():
        tree = json.loads(llbc.read_text())
        result['has_errors'] = tree.get('has_errors')
    (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    if not result['sources_unchanged']:
        raise ValueError('Production source changed during original preflight')
    print(json.dumps({k: v for k, v in result.items() if k != 'sources'}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    preflight(parser.parse_args().output.resolve())
