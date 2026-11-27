"""Close out explicitly enrolled formal Cargo directories after original evidence readback."""

import os
from pathlib import Path
import re
import time

from tools import ci_retention, native_task
from tools.system_acceptance import digest, json_file, private_path, write_once


DIRECTORY = 'formal-build-closeout'
EXACT_TARGET_NAMES = {'cargo-target', 'formal-cargo-target', 'extract-target'}


def target_paths(run, names):
    paths = [Path(name) for name in names]
    if (not paths or len(set(paths)) != len(paths)
            or any(path.is_absolute() or '..' in path.parts or path.name not in EXACT_TARGET_NAMES
                   or {DIRECTORY, '.lake', '.git', '.tools'} & set(path.parts) for path in paths)
            or any(path != other and path.is_relative_to(other) for path in paths for other in paths)):
        raise ValueError('exact nonoverlapping private formal build paths required')
    return [run/path for path in paths]


def enroll(run, *, candidate, observer, targets):
    """Enroll exact absent paths under an owner-private, caller-owned fresh run."""
    run = private_path(run, directory=True)
    paths = target_paths(run, targets)
    if any(os.path.lexists(path) or path.resolve() != path for path in paths):
        raise ValueError('formal build enrollment must precede target creation')
    if (not re.fullmatch('[0-9a-f]{40}', candidate) or not observer.is_absolute()
            or observer.resolve(strict=True) != observer or observer.is_relative_to(run) or not observer.is_file()):
        raise ValueError('exact candidate and independently installed observer required')
    state = run/DIRECTORY
    state.mkdir(mode=0o700)
    write_once(state/'enrollment.json', dict(schema=1, candidate=candidate,
        run_identity=native_task.directory_identity(run), state_identity=native_task.directory_identity(state),
        observer=str(observer), observer_sha256=digest(observer), targets=[str(p.relative_to(run)) for p in paths]))
    ci_retention.fsync_directory(run)


def admission(run):
    run = private_path(run, directory=True)
    state = private_path(run/DIRECTORY, directory=True)
    row = json_file(private_path(state/'enrollment.json'))
    if (row['schema'] != 1 or row['run_identity'] != native_task.directory_identity(run)
            or row['state_identity'] != native_task.directory_identity(state)):
        raise ValueError('original formal build enrollment required')
    return state, row, target_paths(run, row['targets'])


def execute(run, argv, *, pass_fds=(), cwd=None, env=None):
    """Wait for the original child. No exit receipt is fabricated after loss of the executor."""
    state, _, targets = admission(run)
    with native_task.task_lock(state):
        for path in targets:
            if os.path.lexists(path) and any(private_path(path, directory=True).iterdir()):
                raise ValueError('a preallocated formal build directory must be empty')
        return native_task.launch(state, argv, pass_fds=pass_fds, cwd=cwd, env=env)


def checked_evidence(validate, terminal, targets, store_root):
    files = validate(terminal)
    if not files:
        raise ValueError('original formal receipt and evidence readback required')
    result = {}
    for name, expected in files.items():
        path = Path(name)
        if (not path.is_absolute() or path.resolve(strict=True) != path or not path.is_file()
                or any(path.is_relative_to(target) for target in (*targets, store_root))
                or digest(path) != expected):
            raise ValueError('original formal evidence is changed or inside a disposable build: '+str(path))
        result[str(path)] = expected
    return result


def retire(run, observer, validate):
    """Apply caller's original readback to each custody state. Return the original status."""
    state, enrollment, targets = admission(run)
    if enrollment['observer'] != str(observer) or enrollment['observer_sha256'] != digest(observer):
        raise ValueError('same installed host observer required')
    with native_task.task_lock(state):
        original_exit = private_path(state/'native-task-exit.json')
        terminal = json_file(original_exit)
        code, cancellation = terminal['returncode'], terminal['cancel_signal']
        if (type(code) is not int or type(terminal['process_group']) is not int
                or terminal['process_group'] <= 1 or code < 0 and -code not in native_task.SIGNALS
                or cancellation is not None and (type(cancellation) is not int or cancellation not in native_task.SIGNALS)):
            raise ValueError('original formal exit receipt required')
        status = 'passed' if code == 0 else 'cancelled' if cancellation or -code in native_task.SIGNALS else 'failed'
        managed = state/'store'
        result_path = state/'retired.json'
        settled = os.path.lexists(result_path)
        if not settled:
            native_task.all_workers_exited(terminal)
        files = checked_evidence(validate, terminal, targets, managed)
        retained = state/'retained.json'
        native_task.preserve(retained, dict(enrollment_sha256=digest(state/'enrollment.json'),
            terminal_sha256=digest(original_exit), executor_log_sha256=digest(private_path(state/'native-task.log')),
            original_evidence=files))
        if settled:
            previous = json_file(private_path(result_path))
            if (any(os.path.lexists(path) for path in targets)
                    or previous['candidate'] != enrollment['candidate'] or previous['returncode'] != code
                    or previous['status'] != status or previous['remaining_build_bytes'] != 0):
                raise ValueError('formal retirement result differs from original execution')
            return previous
        managed.mkdir(mode=0o700, exist_ok=True)
        return handoff(run, state, managed, targets, enrollment['candidate'], status, code,
                       observer, retained, files, validate, terminal)


def handoff(run, state, managed, targets, candidate, status, code, observer, retained, files, validate, terminal):
    """Journal exact inodes, then delegate durable unlink to the existing Store."""
    store = ci_retention.Store(managed)
    manifest = state/'handoff.json'
    with store.lock():
        if not os.path.lexists(manifest):
            native_task.observe(observer, run, (), targets)
            rows = []
            for path in targets:
                if not os.path.lexists(path) and code != 0:
                    continue  # An original unsuccessful child may have stopped before allocation.
                private_path(path, directory=True)
                rows.append(dict(path=str(path), identity=native_task.directory_identity(path),
                                 allocated_bytes=ci_retention.allocated_bytes(path)))
            identities = {tuple(row['identity']) for row in rows}
            if len(identities) != len(rows) or any(device != run.stat().st_dev for device, _ in identities):
                raise ValueError('formal builds require distinct original inodes on the run filesystem')
            payload = store.allocate(kind='build', candidate=candidate)
            write_once(manifest, dict(payload=payload.name, targets=rows, retained_sha256=digest(retained)))
        saved = json_file(private_path(manifest))
        paths = [Path(row['path']) for row in saved['targets']]
        if (saved['retained_sha256'] != digest(retained)
                or paths != [path for path in targets if path in paths]
                or code == 0 and paths != targets
                or any(os.path.lexists(path) for path in targets if path not in paths)):
            raise ValueError('existing formal handoff differs from original enrollment or evidence')
        payload, record, entry = store.read(saved['payload'])
        if entry['candidate'] != candidate or entry['kind'] != 'build':
            raise ValueError('existing formal handoff has different candidate')
        if entry['state'] == 'active':
            native_task.observe(observer, run, (), [*paths, payload])
            if native_task.directory_identity(private_path(payload, directory=True)) != entry['identity']:
                raise ValueError('formal handoff payload identity changed')
            slots = {str(i) for i in range(len(paths))}
            if any(path.name not in slots for path in payload.iterdir()):
                raise ValueError('unknown child in formal handoff payload')
            for index, row in enumerate(saved['targets']):
                path = Path(row['path'])
                destination = payload/str(index)
                if os.path.lexists(path) == os.path.lexists(destination):
                    raise ValueError('formal target move identity uncertain')
                current = path if os.path.lexists(path) else destination
                if native_task.directory_identity(private_path(current, directory=True)) != row['identity']:
                    raise ValueError('formal target identity changed')
            for index, path in enumerate(paths):
                if os.path.lexists(path):
                    path.rename(payload/str(index))
                ci_retention.fsync_directory(path.parent)
            ci_retention.fsync_directory(payload)
        if any(os.path.lexists(path) for path in targets) or checked_evidence(validate, terminal, targets, managed) != files:
            raise ValueError('original evidence differs after formal build handoff')
        native_task.observe(observer, run, (), [payload])
        receipt = state/'cleanup-receipt.json'
        native_task.preserve(receipt, dict(candidate=candidate, status=status,
            resource_cleanup_confirmed=True, evidence_sha256=digest(retained)))
        if entry['state'] == 'active':
            store.retire(payload.name, receipt, retained, now=int(time.time()))
            payload, record, entry = store.read(payload.name)
        if entry['receipt'] != json_file(receipt):
            raise ValueError('formal cleanup receipt changed')
        store.remove_retired(payload, record, entry)
        if checked_evidence(validate, terminal, targets, managed) != files:
            raise ValueError('original evidence differs after formal build deletion')
        value = dict(candidate=candidate, status=status, returncode=code, remaining_build_bytes=0,
                     target_count=len(paths), allocated_bytes=sum(row['allocated_bytes'] for row in saved['targets']))
        write_once(state/'retired.json', value)
        return value
