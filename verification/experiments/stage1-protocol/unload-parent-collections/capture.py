"""Recapture real collection roots in an isolated source archive.

Only the named field-omission variant mutates Rust, inside the new archive.
This records development evidence; it never issues native/public acceptance.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tarfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
GIB = 2**30


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def apply_variant(source, profile, variant):
    path = source / profile['source']
    before = path.read_bytes()
    if hashlib.sha256(before).hexdigest() != profile['source_sha256']:
        raise ValueError('actual collection source changed; review and recapture first')
    if variant == 'attempt-zero':
        old = profile['mutation']['before'].encode()
        new = profile['mutation']['after'].encode()
        if before.count(old) != 1:
            raise ValueError('field-omission mutation did not match exactly once')
        path.write_bytes(before.replace(old, new))
    return dict(path=profile['source'], variant=variant,
                original_sha256=hashlib.sha256(before).hexdigest(), source_sha256=sha(path))


def capture(output, lock, variant):
    os.umask(0o077)
    output.mkdir(mode=0o700)
    result = dict(development_only=True, native_acceptance=False,
                  public_acceptance=False, variant=variant, commands=[], complete=False)
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
               CARGO_TARGET_DIR=str(output / 'target'))
    for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(name, None)
    try:
        profile = json.loads((HERE / 'capture-profile.json').read_bytes())
        result['profile_sha256'] = sha(HERE / 'capture-profile.json')
        result['head'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        subprocess.run(['git', 'diff', '--quiet', 'HEAD'], cwd=ROOT, check=True)
        if shutil.disk_usage(output).free < 80*GIB:
            raise ValueError('insufficient bounded capture space')
        cpus = sorted(os.sched_getaffinity(0))[:4]
        os.sched_setaffinity(0, cpus)
        result['cpus'] = cpus
        fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
        tools = (ROOT / '.tools/aeneas').resolve(strict=True)
        charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
        for name, binary in [('charon', charon), ('aeneas', aeneas)]:
            if sha(binary) != fixed[name]['binary_sha256']:
                raise ValueError('fixed extractor binary changed: ' + name)
        result['binary_sha256'] = {name: sha(path) for name, path in [('charon', charon), ('aeneas', aeneas)]}
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', 'HEAD'], cwd=ROOT, stdout=stream, check=True)
        result['source_archive_sha256'] = sha(archive)
        source = output / 'source'
        source.mkdir(mode=0o700)
        with tarfile.open(archive) as tar:
            tar.extractall(source, filter='data')
        result['mutation'] = apply_variant(source, profile, variant)
        cfg = profile['configuration']

        def run(label, argv, seconds):
            started = time.monotonic()
            row = dict(label=label, argv=list(map(str, argv)))
            result['commands'].append(row)
            log = output / (label + '.log')
            with log.open('xb') as stream:
                child = subprocess.Popen(argv, cwd=source, env=env, stdout=stream,
                                         stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    while child.poll() is None:
                        target = output / 'target'
                        size = sum(p.lstat().st_blocks*512 for p in target.rglob('*')
                                   if not p.is_symlink()) if target.exists() else 0
                        result['peak_target_bytes'] = max(size, result.get('peak_target_bytes', 0))
                        if (time.monotonic()-started > seconds or size > 4*GIB
                                or shutil.disk_usage(output).free < 64*GIB):
                            row['bounded_stop'] = True
                            break
                        time.sleep(.5)
                finally:
                    if child.poll() is None:
                        os.killpg(child.pid, signal.SIGTERM)
                        try:
                            child.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            os.killpg(child.pid, signal.SIGKILL)
                            child.wait(timeout=5)
                    row.update(exit_code=child.returncode, seconds=time.monotonic()-started,
                               log_sha256=sha(log))
            if row['exit_code'] != 0 or row.get('bounded_stop'):
                raise ValueError('extractor failed: ' + label)

        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            selectors = [part for flag, key in [('--start-from', 'roots'), ('--include', 'include'), ('--opaque', 'opaque')]
                         for value in cfg[key] for part in [flag, value]]
            ir = output / 'actual.llbc'
            run('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                '--lib', '--locked', '--offline', '--target', fixed['target']], 240)
            run('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
                '-split-files', '-emit-json', '-namespace', cfg['namespace'], '-subdir', cfg['subdir'],
                '-dest', output / 'generated', ir], 120)
        result.update(complete=True, ir_sha256=sha(ir), generated={str(p.relative_to(output / 'generated')): sha(p)
            for p in (output / 'generated').rglob('*') if p.is_file()})
    except BaseException as error:
        result['error'] = repr(error)
    finally:
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: value for key, value in result.items() if key not in ('commands', 'generated')}, indent=2))
    return not result['complete']


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    parser.add_argument('--variant', choices=('correct', 'attempt-zero'), required=True)
    args = parser.parse_args()
    raise SystemExit(capture(args.output.resolve(), args.lock.resolve(strict=True), args.variant))
