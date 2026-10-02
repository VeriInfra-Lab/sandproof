"""Reconcile one benchmark create in an operator-owned controller journal.

No database writes, guessed IDs, public-list inference, or create retries.
Destroyed is only a durable witness; the caller must still audit host resources.
"""
import json
import os
from pathlib import Path
import re
import sqlite3
import stat
import time


def load_runtime_intent(directory, *, task, runtime_attempt, build_id, template):
    """Read only this live trial's checkpoint, never discover a previous attempt.

    The expected correlation and template come from the supervisor's live
    environment, not from the file. The controller match still checks its own
    namespace, owner and immutable binding before any DELETE can be sent.
    """
    if (not isinstance(runtime_attempt, str)
            or not re.fullmatch(r'[0-9a-f]{32}', runtime_attempt)
            or not isinstance(build_id, str) or not re.fullmatch(r'[0-9a-f]{32}', build_id)
            or not isinstance(template, dict) or template.get('templateId') != 'tpl-' + build_id
            or not isinstance(task, str) or not task):
        raise ValueError('missing live runtime correlation')
    directory = Path(directory).absolute()
    if directory.resolve(strict=True) != directory:
        raise ValueError('runtime intent directory is not private')
    def unique_keys(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('duplicate runtime intent field')
            result[key] = value
        return result
    descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        info = os.fstat(descriptor)
        if info.st_uid != os.geteuid() or info.st_mode & 0o077:
            raise ValueError('runtime intent directory is not private')
        fd = os.open('runtime-intent.json', os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                     dir_fd=descriptor)
        with os.fdopen(fd, 'rb') as source:
            info = os.fstat(source.fileno())
            if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
                    or info.st_nlink != 1 or info.st_mode & 0o022 or info.st_size > 256 * 1024):
                raise ValueError('unsafe runtime intent file')
            raw = source.read(256 * 1024 + 1)
            if len(raw) > 256 * 1024:
                raise ValueError('runtime intent too large')
    finally:
        os.close(descriptor)
    intent = json.loads(raw, object_pairs_hook=unique_keys)
    expected = {'metadata': {'benchmark': 'terminal-bench-2.0', 'task': task,
        'build': build_id, 'revision': template.get('revision'), 'runtime_attempt': runtime_attempt},
        'template': template}
    if intent != expected:
        raise ValueError('runtime intent differs from live trial')
    return intent


def matching_runtime(database, intent, *, owner, namespace, sandbox_id=None):
    metadata, template = intent['metadata'], intent['template']
    if (not re.fullmatch(r'[0-9a-f]{32}', metadata.get('runtime_attempt', ''))
            or metadata.get('benchmark') != 'terminal-bench-2.0'
            or not re.fullmatch(r'[0-9a-f]{32}', metadata.get('build', ''))
            or template['templateId'] != 'tpl-' + metadata['build']
            or template['revision'] != metadata.get('revision')):
        raise ValueError('invalid runtime creation intent')
    matches = []
    # One read transaction includes both options and state. Missing files must
    # not silently create a new empty database and appear to prove absence.
    with sqlite3.connect(Path(database).resolve().as_uri() + '?mode=ro', uri=True) as db:
        rows = db.execute('SELECT s.sandbox_id,s.state_blob,o.options_json FROM sandboxes s '
                          'JOIN sandbox_create_options o USING (sandbox_id)').fetchall()
    for identity, state_blob, options_blob in rows:
        options = json.loads(options_blob)
        observed = options.get('metadata', {})
        if observed.get('runtime_attempt') != metadata['runtime_attempt']:
            continue
        state, bound = json.loads(state_blob), options['template']
        if (observed != metadata or state['owner'] != owner or bound['owner'] != owner
                or not re.fullmatch(r'sb-' + re.escape(namespace) + r'-[1-9][0-9]*', identity)
                or (sandbox_id is not None and identity != sandbox_id)
                or bound['template_id'] != template['templateId']
                or bound['revision'] != template['revision']
                or bound['kernel']['sha256'] != template['kernelSha256']
                or bound['rootfs']['sha256'] != template['rootfsSha256']
                or bound['resources'] != {
                    'cpu_count': template['cpuCount'], 'memory_mib': template['memoryMib'],
                    'disk_bytes': template['diskBytes'], 'host_memory_bytes': template['hostMemoryBytes']}):
            raise RuntimeError('runtime creation identity mismatch')
        matches.append({'sandbox_id': identity, 'phase': state['phase']})
    if len(matches) > 1:
        raise RuntimeError('ambiguous runtime creation identity')
    return matches[0] if matches else None


def recover_runtime_create(database, intent, *, owner, namespace, delete, deadline,
                           sandbox_id=None, monotonic=time.monotonic, sleep=time.sleep):
    """Wait for admission, delete the exact identity once, observe its tombstone.

    An absent row or a 404 is not evidence that a delayed create cannot commit.
    A lost DELETE receipt is observed without repeating the operation.
    """
    outcome = {'status': 'unknown', 'delete_dispatched': False, 'delete_acknowledged': False}
    while monotonic() < deadline:
        found = matching_runtime(database, intent, owner=owner, namespace=namespace, sandbox_id=sandbox_id)
        if found is not None:
            outcome['sandbox_id'] = found['sandbox_id']
            if found['phase'] == 'Destroyed':
                return outcome | {'status': 'destroyed'}
            if not outcome['delete_dispatched']:
                outcome['delete_dispatched'] = True
                try:
                    status = delete(found['sandbox_id'])
                    outcome['delete_http_status'] = status
                    outcome['delete_acknowledged'] = status in (200, 204)
                    if status not in (200, 204, 404):
                        raise RuntimeError('runtime deletion rejected')
                except OSError as error:
                    outcome['delete_error_type'] = type(error).__name__
        sleep(min(.5, max(0, deadline - monotonic())))
    raise TimeoutError('runtime identity or destruction remains unknown; retain original intent')
