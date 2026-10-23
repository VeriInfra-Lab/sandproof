"""Extract real protocol roots with the fixed original tools; not a Lean proof."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import subprocess
import shutil
import re
import sys
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
PROFILES = {
    'request': ['request_validate', 'replay_matches'],
    'manifest': ['manifest_validate'],
    'carriers': ['capture_from_u64', 'capture_value', 'capture_next', 'digest_roundtrip'],
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inputs():
    paths = list((ROOT / 'sandbox-protocol').rglob('*.rs'))
    paths += [ROOT / n for n in [
        'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml',
        'sandbox-protocol/Cargo.toml', 'verification/foundations-environment.json',
        'verification/experiments/stage1-protocol/Cargo.toml',
        'verification/experiments/stage1-protocol/Cargo.lock',
        'verification/experiments/stage1-protocol/src/lib.rs',
        'verification/experiments/stage1-protocol/extract.py',
    ]]
    return {str(p.relative_to(ROOT)): sha(p) for p in sorted(paths)}


def extract(output, profiles, tool_root):
    output.mkdir(mode=0o700, parents=True)
    record = dict(kind='stage1-protocol-original-generation-not-Lean-proof',
                  profiles={}, source_sha256=inputs())
    started = time.monotonic()

    def run(folder, name, argv, cwd=ROOT):
        t = time.monotonic()
        log = folder / (name + '.log')
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=cwd, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=subprocess.STDOUT, timeout=900)
        return dict(name=name, argv=argv, cwd=str(cwd), exit_code=result.returncode,
                    seconds=time.monotonic()-t, log_sha256=sha(log))

    try:
        expected = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
        for name, relative in [('charon', 'aeneas/charon/bin/charon'),
                               ('aeneas', 'aeneas/bin/aeneas')]:
            if sha(tool_root / relative) != expected[name]['binary_sha256']:
                raise ValueError('fixed tool binary mismatch: ' + name)
        record['tools'] = {n: expected[n] for n in ['charon', 'aeneas']}
        record['environment'] = {k: os.environ.get(k) for k in [
            'CARGO_HOME', 'CARGO_TARGET_DIR', 'CARGO_NET_OFFLINE', 'CARGO_BUILD_JOBS',
            'RUSTUP_HOME', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'TMPDIR']}
        for name in profiles:
            folder = output / name
            folder.mkdir()
            row = dict(roots=PROFILES[name], steps=[])
            record['profiles'][name] = row
            llbc = folder / 'original.llbc'
            crate = ROOT / 'verification/experiments/stage1-protocol'
            raw = ROOT / 'artifacts/stage1-protocol/raw' / (name + '.llbc')
            raw.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            if raw.is_symlink():
                raise ValueError('raw destination must not be a symlink')
            if raw.exists():
                shutil.copy2(raw, folder / 'previous.llbc')
                raw.unlink()
            destination = '../../../artifacts/stage1-protocol/raw/' + name + '.llbc'
            argv = [str(tool_root / 'aeneas/charon/bin/charon'), 'cargo',
                    '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                    '--include', 'sandproof_protocol']
            for root in PROFILES[name]:
                argv += ['--start-from', 'crate::' + root]
            argv += ['--dest-file', destination, '--', '--manifest-path',
                     'Cargo.toml', '--lib',
                     '--locked', '--offline', '--target', 'x86_64-unknown-linux-gnu']
            step = run(folder, 'charon', argv, cwd=crate)
            row['steps'].append(step)
            if step['exit_code']:
                raise RuntimeError('Charon failed: ' + name)
            if not raw.is_file():
                raise ValueError('Charon did not create the new fixed raw destination')
            shutil.copy2(raw, llbc)
            ir = json.loads(llbc.read_text())
            if ir['translated']['options']['dest_file'] != destination:
                raise ValueError('raw artifact destination does not match actual driver')
            if {t['key'] for t in ir['translated']['target_information']} != {expected['target']}:
                raise ValueError('raw target does not match the fixed environment')
            sys.path.insert(0, str(ROOT / 'tools'))
            from check_foundations_extraction import llbc_digest
            row['llbc_canonical_sha256'] = llbc_digest(ir)
            row['llbc_sha256'] = sha(llbc)
            row['has_errors'] = ir.get('has_errors')
            if row['has_errors'] is not False:
                raise ValueError('Charon did not produce a complete error-free crate')
            generated = folder / 'generated'
            generated.mkdir()
            module = 'SandproofStage1' + name.title()
            argv = [str(tool_root / 'aeneas/bin/aeneas'), '-backend', 'lean',
                    '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
                    '-namespace', module + 'Rust', '-subdir', module,
                    '-dest', str(generated), str(llbc)]
            step = run(folder, 'aeneas', argv)
            row['steps'].append(step)
            if step['exit_code']:
                raise RuntimeError('Aeneas failed: ' + name)
            external = []
            for template in generated.rglob('*External_Template.lean'):
                external += re.findall(r'^axiom\s+([A-Za-z0-9_.]+)', template.read_text(), re.M)
            allowed = {
                'request': {'alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'alloc.vec.Vec.is_empty'},
                'manifest': {'core.str.Str.len', 'core.str.Str.is_empty', 'core.str.Str.as_bytes',
                             'Str.Insts.AllocBorrowToOwnedString.to_owned',
                             'alloc.string.String.Insts.CoreCmpPartialEqString.eq',
                             'alloc.string.String.Insts.CoreOpsDerefDerefStr.deref', 'alloc.vec.Vec.is_empty'},
                'carriers': set(),
            }
            # The key-only Rust mutant no longer reaches operation equality.
            # Register only this existing A6 primitive when it is emitted.
            bool_ne = 'Bool.Insts.CoreCmpPartialEqBool.ne'
            if name == 'request' and bool_ne in external:
                allowed[name].add(bool_ne)
            if set(external) != allowed[name] or len(external) != len(set(external)):
                raise ValueError('unregistered opaque primitive or project function: ' + repr(external))
            row['external_functions'] = sorted(external)
            row['generated_sha256'] = {str(p.relative_to(generated)): sha(p)
                                       for p in sorted(generated.rglob('*')) if p.is_file()}
            row['generation_success'] = True
            (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if inputs() != record['source_sha256']:
            raise ValueError('actual extraction inputs changed during generation')
        record.update(sources_unchanged=True, success=True)
    except BaseException as error:
        record.update(success=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--profile', choices=['all', *PROFILES], default='all')
    parser.add_argument('--tool-root', type=Path, default=ROOT / '.tools')
    args = parser.parse_args()
    os.umask(0o077)
    selected = list(PROFILES) if args.profile == 'all' else [args.profile]
    result = extract(args.output.absolute(), selected, args.tool_root.absolute())
    print(json.dumps({k: result.get(k) for k in ['kind', 'success', 'seconds', 'error']}))
    raise SystemExit(0 if result['success'] else 1)
