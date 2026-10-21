"""Private ordinary-host component, not an official full-suite native seal."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    names = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'],
        cwd=ROOT,
    ).decode().split('\0')
    return {n: sha(ROOT / n) for n in sorted(set(names))
            if n and (ROOT / n).is_file() and not (ROOT / n).is_symlink()}


def native(output, target):
    output.mkdir(mode=0o700)
    target.mkdir(mode=0o700)
    started = time.monotonic()
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE='true',
               CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='4', RUST_TEST_THREADS='1',
               PYTHONDONTWRITEBYTECODE='1')
    for name in ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'LD_PRELOAD', 'LD_LIBRARY_PATH']:
        env.pop(name, None)
    record = dict(kind='stage1-protocol-component-not-full-native-seal', steps=[])

    def run(name, argv):
        t = time.monotonic()
        log = output / (name + '.log')
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=subprocess.STDOUT, timeout=1200)
        record['steps'].append(dict(name=name, argv=argv, exit_code=result.returncode,
                                    seconds=time.monotonic()-t, log_sha256=sha(log)))
        (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if result.returncode:
            raise RuntimeError('ordinary component failed: ' + name)
        return log

    try:
        record['environment'] = {n: env.get(n) for n in [
            'PATH', 'CARGO_HOME', 'CARGO_TARGET_DIR', 'CARGO_NET_OFFLINE', 'CARGO_BUILD_JOBS',
            'CARGO_INCREMENTAL', 'RUST_TEST_THREADS', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS']}
        record['versions'] = {n: subprocess.check_output(argv, cwd=ROOT, env=env, text=True)
                              for n, argv in [('rustc', ['rustc', '-vV']),
                                              ('cargo', ['cargo', '-vV']),
                                              ('clippy', ['cargo', 'clippy', '--version'])]}
        run('format', ['cargo', 'fmt', '-p', 'sandproof-protocol', '-p', 'sandproof-internal'])
        run('extract-format', ['cargo', 'fmt', '--manifest-path',
                              'verification/experiments/stage1-protocol/Cargo.toml'])
        before = sources()
        record['source_sha256'] = before
        packages = ['-p', 'sandproof-protocol', '-p', 'sandproof-internal']
        log = run('build', ['cargo', 'test', *packages, '--locked', '--offline',
                            '--no-run', '--message-format=json'])
        own = {'sandproof_protocol': 'sandbox-protocol/src/lib.rs',
               'sandproof_internal': 'sandbox-internal/src/lib.rs'}
        libraries, tests = {}, []
        compiled = output / 'compiled'
        compiled.mkdir()
        for line in log.read_text().splitlines():
            if not line.startswith('{'):
                continue
            item = json.loads(line)
            if item.get('reason') != 'compiler-artifact' or item['target']['name'] not in own:
                continue
            if Path(item['target']['src_path']).resolve() != ROOT / own[item['target']['name']]:
                raise ValueError('actual own library/test comes from a different source')
            if item['fresh'] is not False:
                raise ValueError('expected actual fresh own library/test compilation')
            if item['profile']['test']:
                tests.append(item)
            elif 'lib' in item['target']['kind']:
                libraries[item['target']['name']] = item
            for name in item['filenames']:
                path = Path(name)
                if path.is_file():
                    shutil.copy2(path, compiled / path.name)
        if set(libraries) != set(own) or len(tests) != 2:
            raise ValueError('missing actual two libraries and lib test programs')
        record.update(actual_libraries=libraries, actual_lib_tests=tests)
        run('test', ['cargo', 'test', *packages, '--locked', '--offline'])
        run('clippy', ['cargo', 'clippy', *packages, '--all-targets', '--locked', '--offline',
                       '--', '-D', 'warnings'])
        run('docs', ['bash', 'scripts/python-entry.sh', 'tools/check_docs.py'])
        run('fmt-check', ['cargo', 'fmt', '-p', 'sandproof-protocol', '-p',
                          'sandproof-internal', '--check'])
        if sources() != before:
            raise ValueError('component source changed during verification')
        record['compiled_sha256'] = {p.name: sha(p) for p in compiled.iterdir() if p.is_file()}
        record['success'] = True
    except BaseException as error:
        record.update(success=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--target', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    result = native(args.output.absolute(), args.target.absolute())
    print(json.dumps({k: result.get(k) for k in ['kind', 'success', 'seconds', 'error']}))
    raise SystemExit(0 if result['success'] else 1)
