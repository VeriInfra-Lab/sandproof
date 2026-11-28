"""Fixed-tool extraction of the actual codec or isolated accepted-route candidate."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import time
import tomllib
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/json-wire')
PROFILES = {
    'accepted': ['accepted_decode', 'request_validate', 'replay_matches', 'accepted_validate', 'recorded_matches'],
    'decode_request': ['decode_request'],
    'encode_request': ['encode_request'],
    'decode_manifest': ['decode_manifest'],
    'encode_manifest': ['encode_manifest'],
}
EXTERNALS = {
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq',
    'alloc.vec.Vec.is_empty', 'Bool.Insts.CoreCmpPartialEqBool.ne',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def extract(output, profile):
    output.mkdir(mode=0o700, parents=True)
    record = dict(kind='fixed-json-wire-extraction-not-proof', profile=profile, steps=[], success=False)
    begin = time.monotonic()
    crate = ROOT / HERE

    def run(name, argv):
        started = time.monotonic()
        log = output / (name + '.log')
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=crate, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=900)
        record['steps'].append(dict(name=name, argv=argv, cwd=str(crate),
            exit_code=result.returncode, seconds=time.monotonic()-started, log_sha256=sha(log)))
        if result.returncode:
            raise ValueError('extraction command failed: ' + name)

    try:
        fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
        for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
            if sha(ROOT / '.tools/aeneas' / suffix) != fixed[name]['binary_sha256']:
                raise ValueError('fixed tool binary differs: ' + name)
        record['tools'] = {n: fixed[n] for n in ['charon', 'aeneas']}
        target = Path(os.environ.get('CARGO_TARGET_DIR', ''))
        if not target.is_absolute() or not target.is_dir():
            raise ValueError('allocated absolute private CARGO_TARGET_DIR required')
        record['environment'] = {n: os.environ.get(n) for n in ['CARGO_HOME', 'CARGO_TARGET_DIR',
            'CARGO_NET_OFFLINE', 'CARGO_BUILD_JOBS', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS']}
        packages = ['sandproof-protocol', 'sandproof-internal', 'sandproof-stage1-json-wire-extract']
        refresh = ['rustup', 'run', fixed['extraction_frontend'], 'cargo', 'clean', '--locked',
            '--offline', '--target', fixed['target']]
        for name in packages:
            refresh += ['-p', name]
        run('refresh-own', refresh)
        raw = ROOT / 'artifacts/json-wire/raw' / (profile + '.llbc')
        raw.parent.mkdir(parents=True, exist_ok=True)
        if raw.is_symlink():
            raise ValueError('raw output must not be a symlink')
        if raw.exists():
            shutil.copy2(raw, output / 'previous.llbc')
            raw.unlink()
        destination = '../../../../artifacts/json-wire/raw/' + profile + '.llbc'
        argv = [str(ROOT / '.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default', '--include', 'sandproof_protocol',
            '--include', 'sandproof_internal']
        for name in PROFILES[profile]:
            argv += ['--start-from', 'crate::' + name]
        argv += ['--dest-file', destination, '--', '--manifest-path', 'Cargo.toml', '--lib',
            '--locked', '--offline', '--target', fixed['target']]
        run('charon', argv)
        observed = {re.sub(r'\x1b\[[0-9;]*m', '', line).strip()
                    for line in (output / 'charon.log').read_text().splitlines()}
        for manifest in [ROOT / 'sandbox-protocol/Cargo.toml', ROOT / 'sandbox-internal/Cargo.toml', crate / 'Cargo.toml']:
            package = tomllib.loads(manifest.read_text())['package']
            if f"Compiling {package['name']} v{package['version']} ({manifest.parent})" not in observed:
                raise ValueError('actual own package not freshly compiled: ' + package['name'])
        record['own_compilation_observed'] = packages
        ir = json.loads(raw.read_text())
        if ir.get('has_errors') is not False or ir['translated']['options']['dest_file'] != destination:
            raise ValueError('incomplete or misdirected original LLBC')
        if {t['key'] for t in ir['translated']['target_information']} != {fixed['target']}:
            raise ValueError('wrong extraction target')
        llbc = output / 'original.llbc'
        shutil.copy2(raw, llbc)
        record['llbc_sha256'] = sha(llbc)
        generated = output / 'generated'
        generated.mkdir()
        run('aeneas', [str(ROOT / '.tools/aeneas/bin/aeneas'), '-backend', 'lean',
            '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
            '-namespace', 'SandproofStage1RequestRust', '-subdir', 'SandproofStage1Request',
            '-dest', str(generated), str(llbc)])
        external = [name for p in generated.rglob('*External_Template.lean')
                    for name in re.findall(r'^axiom\s+([A-Za-z0-9_.]+)', p.read_text(), re.M)]
        if profile == 'accepted' and (set(external) != EXTERNALS or len(external) != len(EXTERNALS)):
            raise ValueError('unregistered external or hidden business function: ' + repr(external))
        record.update(success=True, external_functions=sorted(external),
            generated_sha256={str(p.relative_to(generated)): sha(p) for p in sorted(generated.rglob('*')) if p.is_file()})
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-begin
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--profile', required=True, choices=PROFILES)
    args = parser.parse_args()
    os.umask(0o077)
    result = extract(args.output.absolute(), args.profile)
    print(json.dumps({n: result.get(n) for n in ['kind', 'profile', 'success', 'seconds', 'error']}))
    raise SystemExit(0 if result['success'] else 1)
