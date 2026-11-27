"""Run the ordinary native suite and retire only this task's private builds."""
import argparse
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools import ci_retention
from tools.system_acceptance import digest, json_file, private_path, write_once


OWNER = 'disposable-native-task-v1'
SIGNALS = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)


def directory_identity(path):
    s = path.stat()
    return [s.st_dev, s.st_ino]


def preserve(path, value):
    if os.path.lexists(path):
        if json_file(private_path(path), limit=16 * 1024**2) != value:
            raise ValueError('existing retirement evidence differs: ' + str(path))
    else:
        write_once(path, value)


@contextmanager
def task_lock(output):
    with open(output / 'native-task.lock', 'a', opener=lambda p, f: os.open(
            p, f | os.O_NOFOLLOW, 0o600)) as stream:
        fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def enroll(output, candidate, scope, aliases, observer):
    output.mkdir(mode=0o700)
    row = dict(kind=OWNER, candidate=candidate, identity=directory_identity(output),
               scope=scope, aliases=[str(p) for p in aliases],
               observer=str(observer), observer_sha256=digest(observer))
    write_once(output / 'native-task.json', row)
    ci_retention.fsync_directory(output.parent)
    return row


def admission(output, observer):
    row = json_file(private_path(output / 'native-task.json'))
    if row['kind'] != OWNER or row['identity'] != directory_identity(output):
        raise ValueError('original disposable native task enrollment required')
    if row['observer'] != str(observer) or row['observer_sha256'] != digest(observer):
        raise ValueError('same installed host observer required')
    for alias in map(Path, row['aliases']):
        if os.path.lexists(alias) and (alias.is_symlink() or not alias.samefile(output)):
            raise ValueError('native task alias no longer names the enrolled directory')
    return row


def launch(output, argv, *, pass_fds=()):
    """Record the actually waited-for child status, including cancellation."""
    child = None
    pending = None

    def cancel(signum, _frame):
        nonlocal pending
        pending = signum
        if child is not None and child.returncode is None:
            try:
                os.killpg(child.pid, signum)
            except ProcessLookupError:
                pass

    old = {sig: signal.signal(sig, cancel) for sig in SIGNALS}
    try:
        with (output / 'native-task.log').open('xb') as log:
            with subprocess.Popen(argv, stdout=log, stderr=subprocess.STDOUT,
                                  pass_fds=pass_fds, start_new_session=True) as child:
                if pending is not None:
                    cancel(pending, None)
                status = child.wait()
            os.fsync(log.fileno())
        observed = dict(returncode=status, process_group=child.pid, cancel_signal=pending)
        write_once(output / 'native-task-exit.json', observed)
        return observed
    finally:
        for sig, handler in old.items():
            signal.signal(sig, handler)


def all_workers_exited(terminal):
    try:
        os.killpg(terminal['process_group'], 0)
    except ProcessLookupError:
        return
    raise ValueError('the original process group still exists')


def observe(observer, output, aliases, paths):
    roots = [str(prefix / p.relative_to(output)) for prefix in (output, *aliases) for p in paths]
    # The host supplies its trusted observer. An unavailable observer, an
    # incomplete scan and a live reference all leave the original data intact.
    result = subprocess.run([observer, *roots], capture_output=True, check=False, timeout=120)
    report = json.loads(result.stdout)
    if (result.returncode != 0 or not isinstance(report, dict)
            or report.get('complete_no_references_or_mounts') is not True
            or report.get('matching_references') != [] or report.get('observation_errors') != []
            or type(report.get('examined_processes')) is not int or report['examined_processes'] < 1):
        raise ValueError('trusted observer did not confirm reference and mount absence')
    return report


def retire(output, observer):
    enrolled = admission(output, observer)
    terminal = json_file(private_path(output / 'native-task-exit.json'))
    status = terminal['returncode']
    if type(status) is not int or type(terminal['process_group']) is not int or terminal['process_group'] < 1:
        raise ValueError('original terminal execution record required')
    result = output / 'native-task-retired.json'
    value = dict(candidate=enrolled['candidate'], scope=enrolled['scope'], returncode=status)
    if os.path.lexists(result):
        previous = json_file(private_path(result))
        if any(previous.get(key) != original for key, original in value.items()):
            raise ValueError('native retirement result belongs to another execution')
        return previous
    all_workers_exited(terminal)
    aliases = tuple(map(Path, enrolled['aliases']))
    works = sorted(p for p in output.glob('run-*') if p.is_dir())
    if not works and status != 0:
        value.update(status='no-build-allocated', target_count=0, allocated_bytes=0,
                     remaining_build_bytes=0)
        preserve(result, value)
        return value
    work, = works
    private_path(work, directory=True)
    candidate = enrolled['candidate']
    if status == 0:
        manifest = work / 'build-targets.json'
        kind = 'passed'
    else:
        kind = 'cancelled' if terminal['cancel_signal'] or -status in SIGNALS else 'failed'
        manifest = work / 'build-terminal-targets.json'
        if not os.path.lexists(manifest):
            observe(observer, output, aliases, [work])
            exit_record = work / 'executor-exit.json'
            preserve(exit_record, dict(kind='native-process-exit-v1', candidate=candidate,
                identity=directory_identity(work), status=kind, exit_code=status,
                processes_exited=True, observation_complete=True, observation_error=None))
            ci_retention.record_native_terminal(work, exit_record)
    invocation = json_file(private_path(manifest), limit=16 * 1024**2)
    if invocation['candidate'] != candidate or invocation['identity'] != directory_identity(work):
        raise ValueError('retirement manifest differs from enrolled native run')
    targets = [Path(item['path']) for item in invocation['targets']]
    if any(p.name != 'target' or not p.is_relative_to(work) for p in targets):
        raise ValueError('retirement targets must be private builds in this task')
    managed = output / 'native-disposable-store'
    value.update(status=kind, target_count=len(targets), remaining_build_bytes=0,
                 allocated_bytes=sum(row['allocated_bytes'] for row in invocation['targets']))
    if targets:
        managed.mkdir(mode=0o700, exist_ok=True)
        store = ci_retention.Store(managed)
        with store.lock():
            # A separate store is enrolled for this task only. There is no
            # retain-last policy for these explicitly disposable bytes.
            observed = output / 'native-task-observation.json'
            receipt = output / 'native-task-cleanup-receipt.json'
            observe(observer, output, aliases, [*targets, managed / 'payloads'])
            preserve(observed, dict(manifest_sha256=digest(manifest), references_absent=True,
                mounts_absent=True, observation_complete=True, observation_error=None))
            preserve(receipt, dict(candidate=candidate, status=kind, resource_cleanup_confirmed=True,
                evidence_sha256=digest(observed)))
            payload = ci_retention.handoff_native_builds(store, manifest, receipt, observed, now=int(time.time()))
            _, record, row = store.read(payload.name)
            observe(observer, output, aliases, [payload])
            store.remove_retired(payload, record, row)
    preserve(result, value)
    return value


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('run', 'resume'))
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--observer', required=True, type=Path)
    parser.add_argument('--alias-parent', type=Path, action='append', default=[])
    parser.add_argument('--scope', default='stage1-protocol')
    parser.add_argument('--candidate', required=True)
    args = parser.parse_args(argv)
    from tools import proof_native_suite
    proof_native_suite.check_native_scope(args.scope)
    cwd = Path.cwd()
    proof_native_suite.trusted_checkout(cwd)
    if (cwd != proof_native_suite.ROOT
            or subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip() != args.candidate):
        raise ValueError('the exact native task checkout is required')
    if args.output.is_symlink():
        raise ValueError('native task output cannot be an alias')
    output = args.output.resolve()
    observer = args.observer.resolve(strict=True)
    if observer.is_relative_to(output) or observer.is_relative_to(cwd) or not observer.is_file():
        raise ValueError('installed host observer must be outside the checkout and task')
    os.umask(0o077)
    if args.action == 'run':
        aliases = []
        for parent in args.alias_parent:
            if not parent.is_absolute() or not parent.samefile(output.parent):
                raise ValueError('each alias parent must refer to the enrolled output parent')
            aliases.append(parent / output.name)
        enroll(output, args.candidate, args.scope, aliases, observer)
    private_path(output, directory=True)
    with task_lock(output):
        original = admission(output, observer)
        if (original['candidate'], original['scope']) != (args.candidate, args.scope):
            raise ValueError('requested candidate or scope differs from enrolled task')
        if args.action == 'run':
            fds = {int(os.environ[key]) for key in (
                'SANDPROOF_LOCK_FD', 'SANDPROOF_PROOF_HEAVY_LOCK_FD') if key in os.environ}
            outcome = launch(output, [sys.executable, '-B', 'tools/proof_native_suite.py', 'run',
                                      '--scope', args.scope, '--output', str(output)], pass_fds=tuple(sorted(fds)))
            try:
                result = retire(output, observer)
            except (ValueError, OSError, subprocess.SubprocessError) as exc:
                print(f'Native build retirement is pending: {exc}', file=sys.stderr)
                if outcome['returncode'] == 0:
                    return 75
            else:
                print(json.dumps(result, sort_keys=True))
            status = outcome['returncode']
        else:
            result = retire(output, observer)
            print(json.dumps(result, sort_keys=True))
            status = result['returncode']
        return status if status >= 0 else 128 - status


if __name__ == '__main__':
    sys.exit(main())
