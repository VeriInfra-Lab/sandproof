"""Re-extract all four Rust views and check every actual Lean file separately.

This component result is not a shared formal/native acceptance receipt. The
coordinator must register it and enforce the original current native admission.
"""
import argparse
import contextlib
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import traceback

import capture
import prepare
import support


def run(root, output, lock_path, lock_fd=None):
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    result = dict(kind='source-live-four-view-proof-component', complete=False, commands=[],
                  formal_batch_acceptance=False, ning_acceptance=False, full_v3=False)

    def command(project, name, args):
        log = project / 'artifacts' / (name + '.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            code = subprocess.run(args, cwd=project, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800).returncode
        result['commands'].append(dict(argv=args, cwd=str(project), exit_code=code,
            seconds=time.monotonic() - begin, log=str(log), log_sha256=support.sha(log)))
        support.require(code == 0, 'actual proof component failed: ' + name)

    try:
        support.require(os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') == 'formal',
                        'source-live proof needs the existing proof-host role')
        support.require(Path.cwd().resolve() == root, 'proof root differs from admitted source')
        support.require(1 <= len(os.sched_getaffinity(0)) <= 4, 'proof CPU allocation differs')
        prepare.inventory(root)
        with (lock_path.open('r+') if lock_fd is None else os.fdopen(os.dup(lock_fd), 'r+')) as lock:
            actual, wanted = os.fstat(lock.fileno()), lock_path.stat()
            support.require((actual.st_dev, actual.st_ino) == (wanted.st_dev, wanted.st_ino),
                            'proof inherited lock identifies another file')
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            captured = capture.run(root, output / 'capture', lock_path, lock.fileno())
            result['capture_result'] = str(output / 'capture/result.json')
            support.require(captured['capture_complete'], 'current actual four-view capture failed')
            data = prepare.inventory(root)
            for view, files in data['generated'].items():
                for name, wanted in files.items():
                    path = output / 'capture' / view / 'generated/SandproofSourceLive' / name
                    support.require(support.sha(path) == wanted, 'fresh generated body differs: ' + view)
            projects = prepare.prepare(root, output / 'projects')
            result['views'] = projects
            services_path = root / 'tools/proof_services.py'
            for view, entry in projects.items():
                support.require(shutil.disk_usage(root).free >= capture.FLOOR, 'proof capacity floor reached')
                project = Path(entry['project'])
                (project / 'artifacts').mkdir()
                command(project, 'kernel', ['lake', 'build', *entry['targets']])
                services = support.module('source_live_actual_services_' + view, services_path)
                services.ROOT, services.FILES = project, tuple(entry['files'])
                stream = project / 'artifacts/actual-services.stdout'
                with stream.open('x') as log, contextlib.redirect_stdout(log):
                    services.main()
                for message in stream.read_text().splitlines():
                    if message.startswith('Checked '):
                        print(message, flush=True)
                entry['service_stdout_sha256'] = support.sha(stream)
                support.require(all(support.sha(project / p) == h for p, h in entry['inputs'].items()),
                                'actual proof files changed during checking')
            base = support.module('source_live_postguard_snapshot', root /
                'verification/experiments/stage1-protocol/schema-leaves/formal.py')
            support.require(base.snapshot(root) == captured['inputs'] and
                all(support.mode(root / p) == m for p, m in captured['input_modes'].items()),
                'component source inputs or modes changed')
            batch = support.module('source_live_postguard_runtime', root / 'tools/proof_batch.py')
            batch.ROOT, batch.MODULES = root, ('stage1-protocol',)
            registry = support.module('source_live_postguard_registry', root /
                'verification/experiments/stage1-protocol/envelope-writer/native.py')
            registry.HERE = Path('.')
            support.require(batch.runtime() == captured['runtime'] and
                registry.registry() == captured['library_registry'], 'component tools or libraries changed')
            support.require(all(support.sha(Path(c['log'])) == c['log_sha256']
                                for c in result['commands']), 'kernel log changed')
            support.require(shutil.disk_usage(root).free >= capture.FLOOR, 'proof capacity floor reached')
            result.update(complete=True, actual_generated_bodies_matched=True,
                all_actual_files_and_imports_checked=True, actual_same_contract_counterexamples=3)
    except Exception as error:
        result.update(error=repr(error), traceback=traceback.format_exc())
    finally:
        result['seconds'] = time.monotonic() - started
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    parser.add_argument('--lock-fd', type=int)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(support.ROOT, args.output.resolve(), args.lock.resolve(), args.lock_fd)
    print(json.dumps({k: result.get(k) for k in ('complete', 'error', 'formal_batch_acceptance')}))
    raise SystemExit(0 if result['complete'] else 1)
