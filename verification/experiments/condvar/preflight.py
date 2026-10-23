"""Record the original Reaper Condvar translation surface, never a semantic negative."""
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


def sources():
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
    before = sources()
    start = time.monotonic()
    llbc = output / 'original.llbc'
    command = [str(tools / 'aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
               '--error-on-warnings', '--sysroot', 'default']
    for boundary in ('std::sync', 'std::process', 'std::thread', 'std::time', 'tokio', 'rustix'):
        command += ['--opaque', boundary]
    command += ['--include', 'sandbox_runtime', '--start-from',
                'crate::child_process::reaper::_::run', '--dest-file', str(llbc),
                '--', '--manifest-path', 'sandbox-runtime/Cargo.toml', '--lib', '--locked', '--offline']
    with (output / 'charon.log').open('xb') as log:
        code = subprocess.run(command, cwd=ROOT, stdout=log,
                              stderr=subprocess.STDOUT, timeout=900).returncode
    record = dict(kind='original-translation-preflight-not-proof-or-semantic-counterexample',
                  charon_command=command, charon_exit=code, sources=before)
    if code == 0 and llbc.is_file():
        tree = json.loads(llbc.read_text())
        bodies = [fun for fun in tree['translated']['fun_decls']
                  if fun and fun['item_meta']['is_local'] and isinstance(fun['body'], dict)
                  and 'Structured' in fun['body']]
        if tree.get('has_errors') is not False or not bodies:
            raise ValueError('Original transparent Reaper body was not extracted')
        record.update(llbc_sha256=hashlib.sha256(llbc.read_bytes()).hexdigest(),
                      local_bodies=[fun['def_id'] for fun in bodies])
        generated = output / 'generated'
        generated.mkdir()
        backend = [str(tools / 'aeneas/bin/aeneas'), '-backend', 'lean', '-abort-on-error',
                   '-warnings-as-errors', '-split-files', '-emit-json', '-namespace',
                   'CondvarPreflightRust', '-subdir', 'CondvarPreflight',
                   '-dest', str(generated), str(llbc)]
        with (output / 'aeneas.log').open('xb') as log:
            failed = subprocess.run(backend, cwd=ROOT, stdout=log,
                                    stderr=subprocess.STDOUT, timeout=900).returncode
        record.update(aeneas_command=backend, aeneas_exit=failed)
    record.update(sources_unchanged=sources() == before,
                  seconds=round(time.monotonic() - start, 3))
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    if not record['sources_unchanged']:
        raise ValueError('Production inputs changed during original Condvar preflight')
    print(json.dumps({key: value for key, value in record.items() if key != 'sources'}))
    # This fixed-tool limitation is admission evidence, not a semantic pass.
    if record.get('charon_exit') != 0 or record.get('aeneas_exit') != 2:
        raise ValueError('Original Condvar translation differs; inspect the preserved evidence')
    diagnostic = (output / 'aeneas.log').read_text()
    if 'interp/InterpReduceCollapse.ml, line 1137' not in diagnostic:
        raise ValueError('Original Condvar backend limitation changed; review before reuse')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    preflight(parser.parse_args().output.resolve())
