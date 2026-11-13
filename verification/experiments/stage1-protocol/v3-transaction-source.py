"""Run the two fixed V3 components in one admitted formal job."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT))
from tools.acceptance_receipts import sha, validate_native, write_json

SCOPE = 'stage1-v3-transaction-source'
COMPONENTS = (
    ('transaction', HERE.parent/'create-transaction-link/current/formal.py'),
    ('source-rows', HERE/'original-source-rows/formal.py'),
)


def run(output, scope, lock, lock_fd):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal' or len(os.sched_getaffinity(0)) != 4:
        raise ValueError('V3 requires the original four-CPU formal host allocation')
    validate_native(Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']), scope=scope)
    output.mkdir(parents=True, mode=0o700)
    if lock is None:
        if lock_fd is not None:
            raise ValueError('lock descriptor requires the original lock path')
        lock = output/'component.lock'
        lock.touch(exist_ok=False, mode=0o600)
    owned_lock = None
    if lock_fd is None:
        owned_lock = lock.open('r+')
        lock_fd = owned_lock.fileno()
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    env = dict(os.environ, SANDPROOF_LOCK_FILE=str(lock), SANDPROOF_LOCK_FD=str(lock_fd))
    fds = (lock_fd,)
    started = time.monotonic()
    result = dict(kind='v3-transaction-source-component', complete=False,
                  cases={}, commands=[], components={}, full_V3=False)
    spec = importlib.util.spec_from_file_location('v3_original_registered', HERE/'registered.py')
    registered = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(registered)
    helper = registered._execution_helper(ROOT)
    try:
        for case, driver in COMPONENTS:
            folder = output/case
            log = output/(case+'.log')
            argv = [sys.executable, str(driver), '--scope', scope,
                    '--output', str(folder), '--lock', str(lock)]
            argv += ['--lock-fd', str(lock_fd)]
            component_env = dict(env)
            if case == 'source-rows':
                target = output/'source-rows-target'
                target.mkdir(mode=0o700)
                component_env['CARGO_TARGET_DIR'] = str(target)
            with log.open('xb') as stream:
                subprocess.run(argv, cwd=ROOT, env=component_env, stdin=subprocess.DEVNULL, stdout=stream,
                               stderr=subprocess.STDOUT, check=True, pass_fds=fds)
            path = folder/'result.json'
            raw = json.loads(path.read_text())
            job = dict(name=case, args=[], cases=['actual'], actual_files=True)
            registered._check_actual(raw, job)
            evidence = registered._checkpoint_evidence(helper, job, raw, path, log)
            result['cases'][case] = raw['cases']['actual']
            result['commands'].extend(registered._kernel_commands(raw))
            result['components'][case] = dict(result=str(path), sha256=sha(path),
                                           log_sha256=sha(log), evidence_files=len(evidence))
            print(log.read_text(), end='', flush=True)
        validate_native(Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']), scope=scope)
        result['complete'] = True
    except Exception as error:
        result['error'] = repr(error)
        raise
    finally:
        if owned_lock is not None:
            owned_lock.close()
        result['seconds'] = time.monotonic()-started
        write_json(output/'result.json', result)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--scope', choices=(SCOPE, 'stage1-protocol'), default=SCOPE)
    parser.add_argument('--lock', type=Path)
    parser.add_argument('--lock-fd', type=int)
    args = parser.parse_args()
    run(args.output.resolve(), args.scope, args.lock.resolve() if args.lock else None, args.lock_fd)
