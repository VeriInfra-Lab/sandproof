"""Fresh fixed-Charon source admission, without a Serde/Lean acceptance claim."""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import tarfile
import time

from source_binding import HERE, ROOT, check, inspect


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check_production_inputs(root, names, expected):
    actual = {name for name in names if not name.startswith('verification/')
              and name.endswith(('.rs', '.toml', '.lock'))}
    expected_paths = set(expected)
    if actual != expected_paths:
        extra = sorted(actual - expected_paths)
        missing = sorted(expected_paths - actual)
        raise ValueError(f'production input inventory changed: extra={extra}, missing={missing}')
    for name, digest in expected.items():
        path = root / name
        if path.is_symlink() or not path.is_file() or sha(path) != digest:
            raise ValueError('reviewed production input changed: ' + name)


def verify_archived_inputs(source, profile):
    names = (path.relative_to(source).as_posix() for path in source.rglob('*')
             if path.is_file() or path.is_symlink())
    check_production_inputs(source, names, profile['production_inputs'])


def inputs(profile, head):
    names = subprocess.check_output(
        ['git', 'ls-tree', '-r', '--name-only', '-z', head], cwd=ROOT,
    ).decode().split('\0')
    check_production_inputs(ROOT, names, profile['production_inputs'])
    spec = importlib.util.spec_from_file_location('json_call_fixed_library', HERE.parent / 'serde-binding/library.py')
    library = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(library)
    if library.capture(ROOT) != profile['library_inputs']:
        raise ValueError('fixed serializer features/sources changed')


def apply_variant(source, profile, variant):
    name = profile['source']
    path = source / name
    if sha(path) != profile['production_inputs'][name]:
        raise ValueError('original JSON caller source changed')
    original = sha(path)
    if variant != 'correct':
        mutation = profile['mutations'][variant]
        before, after = mutation['before'].encode(), mutation['after'].encode()
        text = path.read_bytes()
        if text.count(before) != 1:
            raise ValueError('mutation does not occur exactly once')
        path.write_bytes(text.replace(before, after))
    return dict(path=name, variant=variant, before_sha256=original, after_sha256=sha(path))


def run(output, lock, variant):
    os.umask(0o077)
    output.mkdir(mode=0o700)
    started = time.monotonic()
    record = dict(kind='unload-json-source-admission', complete=False, commands=[], variant=variant,
                  development_only=True, native_acceptance=False, public_acceptance=False,
                  library_refinement_proved=False, actual_wrapper_lean_proved=False)
    try:
        profile = json.loads((HERE / 'profile.json').read_text())
        record['head'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        subprocess.run(['git', 'diff', '--quiet', record['head']], cwd=ROOT, check=True)
        inputs(profile, record['head'])
        record['profile_sha256'] = sha(HERE / 'profile.json')
        cpus = sorted(os.sched_getaffinity(0))[:4]
        os.sched_setaffinity(0, cpus)
        record['cpus'] = cpus
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', record['head']], cwd=ROOT, stdout=stream, check=True)
        record['archive_sha256'] = sha(archive)
        source = output / 'source'
        source.mkdir(mode=0o700)
        with tarfile.open(archive) as tar:
            tar.extractall(source, filter='data')
        verify_archived_inputs(source, profile)
        record['mutation'] = apply_variant(source, profile, variant)
        expected = sha(source / profile['source'])
        fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
        charon = (ROOT / '.tools/aeneas/charon/bin/charon').resolve(strict=True)
        if sha(charon) != fixed['charon']['binary_sha256']:
            raise ValueError('fixed Charon binary changed')
        record['charon'] = fixed['charon']
        config = profile['configuration']
        selectors = [part for flag, key in [('--start-from', 'roots'), ('--include', 'include'), ('--opaque', 'opaque')]
                     for value in config[key] for part in [flag, value]]
        ir = output / 'actual.llbc'
        argv = [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                '--lib', '--locked', '--offline', '--target', fixed['target']]
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        log = output / 'charon.log'
        row = dict(argv=list(map(str, argv)))
        record['commands'].append(row)
        begin = time.monotonic()
        with lock.open('r+') as guard, log.open('xb') as stream:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            child = subprocess.Popen(argv, cwd=source, env=env, stdout=stream,
                                     stderr=subprocess.STDOUT, start_new_session=True)
            try:
                child.wait(timeout=240)
            finally:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGTERM)
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(child.pid, signal.SIGKILL)
                        child.wait(timeout=5)
                row.update(exit_code=child.returncode, seconds=time.monotonic()-begin, log_sha256=sha(log))
        if row['exit_code'] != 0:
            raise ValueError('real Rust/Charon failed; not a property rejection')
        raw = json.loads(ir.read_text())
        record['llbc_sha256'] = sha(ir)
        if variant == 'correct':
            report = check(raw, profile)
            (output / 'binding.json').write_text(json.dumps(report, indent=2)+'\n')
            record['binding_sha256'] = sha(output / 'binding.json')
        else:
            try:
                inspect(raw)
            except ValueError as error:
                record.update(source_contract_rejected=True, rejection=str(error),
                              lean_counterexample=False)
            else:
                raise ValueError('actual field substitution was admitted')
        if sha(source / profile['source']) != expected:
            raise ValueError('source changed during extraction')
        inputs(profile, record['head'])
        record['complete'] = True
    except BaseException as error:
        record['error'] = repr(error)
    finally:
        record['seconds'] = time.monotonic()-started
        (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    print(json.dumps({key: value for key, value in record.items() if key not in ('commands', 'charon')}, indent=2))
    return not record['complete']


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    parser.add_argument('--variant', choices=('correct', 'origin-request', 'options-empty'), required=True)
    args = parser.parse_args()
    raise SystemExit(run(args.output.resolve(), args.lock.resolve(strict=True), args.variant))
