"""One partial real upload on the lifecycle scenario's EXISTING VM.

The sibling runtime only installs the debug environment before data work starts.
This helper never creates a VM, restarts managers, changes a ledger or sends a
replacement write. Reports contain identity/digests, never bearer credentials.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import hashlib
import http.client
import json
import os
from pathlib import Path
import select
import socket
import sqlite3
import stat
import sys
import time
from urllib.parse import urlencode, urlsplit


PREFIX = bytes(range(256)) * 16
MAGIC = 'sandproof-data-drain-v1'
CONTROL_LIMITS = {'selected.arm': 512, 'unload-busy': 8704}


def _file_identity(value):
    return (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns,
            value.st_ctime_ns, value.st_mode, value.st_uid, value.st_nlink)


def _control_stamp(directory_fd, name, limit, owner):
    before = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
    if (not stat.S_ISREG(before.st_mode) or before.st_uid != owner or before.st_nlink != 1
            or before.st_mode & 0o077 or not 0 < before.st_size <= limit):
        raise ValueError('exact original private data-drain control file required')
    descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
                         dir_fd=directory_fd)
    try:
        if _file_identity(os.fstat(descriptor)) != _file_identity(before):
            raise ValueError('data-drain control replaced before protected read')
        data = bytearray()
        while len(data) <= limit:
            part = os.read(descriptor, limit + 1 - len(data))
            if not part:
                break
            data.extend(part)
        if (len(data) != before.st_size
                or _file_identity(os.fstat(descriptor)) != _file_identity(before)
                or _file_identity(os.stat(name, dir_fd=directory_fd, follow_symlinks=False))
                   != _file_identity(before)):
            raise ValueError('data-drain control changed during protected read')
        return dict(identity=_file_identity(before), sha256=hashlib.sha256(data).hexdigest())
    finally:
        os.close(descriptor)


def _require_busy(raw, arm, saved, accepted):
    if len(raw) > 8704 or not raw.startswith(arm):
        raise ValueError('driver Busy receipt belongs to another original relay')
    actual = json.loads(raw[len(arm):])
    if actual != saved or actual['accepted_version'] != accepted['accepted_version']:
        raise ValueError('driver Busy receipt differs from exact admitted Unload')


def _remaining(deadline):
    from lifecycle_unload_runtime import remaining
    return remaining(deadline)


def _private_directory(path):
    path = Path(path)
    value = path.lstat()
    if (not path.is_absolute() or path.resolve(strict=True) != path
            or not stat.S_ISDIR(value.st_mode) or value.st_uid != 0
            or value.st_mode & 0o077):
        raise ValueError('original root-private data-drain directory required')
    return path


def _alive(source):
    from lifecycle_unload_runtime import original_process
    return original_process(source)


def _route_key(source):
    return source['pid'], source['start_time_ticks'], source['boot_id']


def _live_routes(native, running):
    from lifecycle_unload_runtime import original_json
    from tools.template_build_config import bounded_data_route_limits
    position = running['context']['position']
    binding = running['context']['binding']
    directory = _private_directory(Path(native['node_state_dir']) / 'helper-gates')
    result = {}
    maximum = bounded_data_route_limits(native)['max_data_routes_per_slot']
    for index in range(maximum):
        marker = directory / f'{position}.lock.route-{index}.json'
        try:
            record = original_json(marker, 16384)
        except FileNotFoundError:
            continue
        if (set(record) != {'version', 'context', 'process', 'command_digest',
                            'route_index', 'control'} or record['version'] != 1
                or record['route_index'] != index
                or record['context']['position'] != position
                or record['context']['vm'] != binding['vm']
                or type(record['control']) is not bool):
            raise ValueError('exact original route cell/context required')
        gate = record['context']['helper_gate']
        gate_path = directory / f'{position}.lock'
        meta = gate_path.lstat()
        if (gate['path'] != str(gate_path) or (gate['device'], gate['inode']) != (meta.st_dev, meta.st_ino)
                or gate['max_data_routes_per_slot'] != maximum
                or record['context']['uplink_index'] != native['bounded_host']['uplink_base'] + position
                or not stat.S_ISREG(meta.st_mode) or meta.st_uid != 0 or meta.st_nlink != 1
                or meta.st_mode & 0o077):
            raise ValueError('original route helper gate/physical position differs')
        if record['control'] is False and _alive(record['process']):
            result[_route_key(record['process'])] = (marker, record)
    return result


def _capture_state(native, running, deadline):
    """Read one original SQL snapshot; no manager stop or database mutation."""
    database = Path(native['database'])
    before = database.stat()
    local = running['local']
    sandbox = 'sb-' + native['namespace'] + '-' + str(local['sandbox'])
    with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True,
                         timeout=min(2, _remaining(deadline))) as connection:
        connection.execute('BEGIN')
        rows = connection.execute('SELECT owner_id,state_blob FROM sandboxes '
                                  'WHERE sandbox_id=?', (sandbox,)).fetchall()
        if len(rows) != 1 or rows[0][0] != 'tenant-' + str(local['owner']):
            raise ValueError('original data-drain sandbox owner required')
        state = json.loads(rows[0][1])
        binding = running['context']['binding']
        if (state['owner'] != local['owner'] or state['terminal'] is not False
                or state['generation'] != binding['generation'] or state['current_vm'] != binding['vm']):
            raise ValueError('original running generation/VM changed while data drain is held')
        matches = []
        for encoded, dispatch in connection.execute(
                'SELECT action_json,dispatch_state FROM outbox WHERE sandbox_id=?', (sandbox,)):
            action = json.loads(encoded)
            # Actual controller serialization is an externally tagged Command.
            command = action['command']['logical']['command']
            if not isinstance(command, dict) or len(command) != 1:
                raise ValueError('original tagged action command required')
            name = next(iter(command))
            if name in ('FreezeVm', 'SealSnapshot'):
                matches.append(dict(action=action, state=dispatch))
    after = database.stat()
    if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
        raise ValueError('original data-drain database replaced')
    return state, matches


class InflightDataDrain:
    """Finite context owned by the original public scenario, not a supervisor."""
    def __init__(self, client, running, native, host_config, gate_directory, run_identity):
        self.client, self.running = client, running
        self.native, self.host_config = native, host_config
        self.deadline = client.deadline
        self.directory = _private_directory(gate_directory)
        metadata = self.directory.lstat()
        self._directory_identity = metadata.st_dev, metadata.st_ino
        parent = _private_directory(self.directory.parent).lstat()
        self._parent_identity = parent.st_dev, parent.st_ino
        self._directory_owner = 0
        self._control_files = {}
        self._controls_retired = False
        if len(os.fsencode(self.directory / '2147483647.sock')) >= 108:
            raise ValueError('original data-drain Unix socket path exceeds Linux limit')
        if (not isinstance(run_identity, str) or len(run_identity) != 64
                or any(c not in '0123456789abcdef' for c in run_identity)):
            raise ValueError('exact original run identity required')
        self.run_identity = run_identity
        self.path = '/tmp/sandproof-unload-data-drain-' + run_identity[:16]
        self.connection = self.peer = None
        self.pidfd = None
        self.accepted = None
        self.report = dict(prefix_bytes=len(PREFIX), prefix_sha256=hashlib.sha256(PREFIX).hexdigest(),
                           declared_bytes=len(PREFIX) + 1, tail_sent=False, completed=False,
                           upload_result='unacknowledged partial request; never replayed')
        _, baseline = _capture_state(native, running, self.deadline)
        self.baseline_actions = {json.dumps(row['action'], sort_keys=True) for row in baseline}

    def begin(self):
        """Actual guest bytes plus the exact live durable route are the barrier."""
        from lifecycle_unload_runtime import open_original_candidate_pidfd
        sdk = self.client.sdk()
        baseline = _live_routes(self.native, self.running)
        address = urlsplit(self.client.base_url)
        if address.scheme != 'http' or address.hostname != '127.0.0.1' or not address.port:
            raise ValueError('original loopback public center required')
        connection = http.client.HTTPConnection(address.hostname, address.port,
            timeout=min(10, _remaining(self.deadline)))
        self.connection = connection
        connection.putrequest('POST', '/files?' + urlencode({'path': self.path}))
        connection.putheader('X-Access-Token', self.client.data_token)
        connection.putheader('Content-Type', 'application/octet-stream')
        connection.putheader('Content-Length', str(len(PREFIX) + 1))
        connection.endheaders()
        connection.send(PREFIX)  # The final byte is NEVER sent or retried.
        for _ in range(128):
            _remaining(self.deadline)
            try:
                value = bytes(sdk.files.read(self.path, format='bytes',
                    request_timeout=min(3, _remaining(self.deadline))))
            except Exception as error:
                # Only original SDK NotFound means the writer has not opened
                # this new file yet. All other errors remain actual failures.
                from e2b.exceptions import FileNotFoundException
                if not isinstance(error, FileNotFoundException):
                    raise
                continue
            if not PREFIX.startswith(value) or len(value) > len(PREFIX):
                raise AssertionError('actual partial upload bytes differ')
            if value == PREFIX:
                break
        else:
            raise TimeoutError('real guest upload prefix not observed')
        for _ in range(128):
            _remaining(self.deadline)
            candidates = {key: item for key, item in _live_routes(self.native, self.running).items()
                          if key not in baseline}
            if len(candidates) == 1:
                self.marker, self.route = next(iter(candidates.values()))
                break
        else:
            raise RuntimeError('one exact newly accepted live upload route not observed')
        if select.select([connection.sock], [], [], 0)[0]:
            raise RuntimeError('partial upload already has a reply/EOF; no in-flight work inferred')
        self.source = self.route['process']
        self.pidfd = open_original_candidate_pidfd(self.source)
        self.arm_bytes = ('\n'.join((MAGIC, self.run_identity, str(self.source['pid']),
            str(self.source['start_time_ticks']), self.source['boot_id'])) + '\n').encode()
        arm = self.directory / 'selected.arm'
        descriptor = os.open(arm, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, 'wb') as output:
            output.write(self.arm_bytes)
            output.flush()
            os.fsync(output.fileno())
        self._remember_control('selected.arm', self.arm_bytes)
        self.report.update(guest_prefix_observed=True, original_route=self.route,
                           actual_admission='guest exact prefix + original durable live route')
        return self.report.copy()

    def wait_held(self, accepted):
        """Called AFTER the original client's Unload acceptance, before release."""
        from lifecycle_unload_runtime import original_json, pidfd_exited, checked_source_records
        self.accepted = accepted
        path = self.directory / f"{self.source['pid']}.sock"
        gate_deadline = min(self.deadline, time.monotonic() + 8)
        while True:
            _remaining(gate_deadline)
            try:
                self.peer = socket.socket(socket.AF_UNIX)
                self.peer.settimeout(_remaining(gate_deadline))
                self.peer.connect(str(path))
                break
            except (FileNotFoundError, ConnectionRefusedError):
                self.peer.close()
                self.peer = None
                time.sleep(.005)  # Polls an explicit gate; no completion inferred.
        echo = bytearray()
        while len(echo) < len(self.arm_bytes):
            part = self.peer.recv(len(self.arm_bytes) - len(echo))
            if not part:
                raise RuntimeError('original revocation gate truncated its binding')
            echo.extend(part)
        if bytes(echo) != self.arm_bytes:
            raise ValueError('original revocation gate identity differs')
        while True:
            _remaining(gate_deadline)
            if pidfd_exited(self.pidfd) or not _alive(self.source):
                raise AssertionError('held original relay exited before gate release')
            if original_json(self.marker, 16384) != self.route:
                raise ValueError('original upload route changed while held')
            state, actions = _capture_state(self.native, self.running, gate_deadline)
            actions = [row for row in actions if json.dumps(row['action'], sort_keys=True)
                       not in self.baseline_actions]
            if any(action['state'] != 'ready' for action in actions):
                raise AssertionError('Freeze/Full dispatched before actual data drain')
            if state['phase'] == 'Pausing':
                database = Path(self.native['database'])
                with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True) as connection:
                    connection.execute('BEGIN')
                    sources = checked_source_records(connection, self.host_config, gate_deadline)
                original = [record for record in sources if
                    record['work']['anchor']['request']['request']['binding']['key'] == accepted['key']
                    and record['work']['anchor']['request']['request']['binding']['session'] == self.client.session
                    and record['work']['anchor']['request']['accepted_version'] == accepted['accepted_version']]
                if len(original) != 1 or original[0]['capture'] is None:
                    raise ValueError('driver must actually admit this exact original Full before release')
                progress = self.directory / 'unload-busy'
                try:
                    from lifecycle_unload_runtime import checked_file
                    checked_file(progress, 8704, mode=0o600)
                    raw = progress.read_bytes()
                except FileNotFoundError:
                    time.sleep(.005)
                    continue
                saved = original[0]['work']['anchor']['request']
                _require_busy(raw, self.arm_bytes, saved, accepted)
                self._remember_control('unload-busy', raw)
                break
            if state['phase'] != 'Running':
                raise AssertionError('unexpected original phase while data drain is held')
            time.sleep(.005)
        self.client.assert_data_closed()
        self.report.update(revocation_gate_observed=True, original_relay_live_while_held=True,
                           native_pausing_observed=True, exact_driver_busy_return_observed=True,
                           freeze_dispatched_while_held=False)
        return self.report.copy()

    def release(self):
        if self.peer is None or self.accepted is None:
            raise RuntimeError('original accepted Unload and reached gate required')
        self.peer.sendall(b'g')
        self.peer.close()
        self.peer = None
        from lifecycle_unload_runtime import pidfd_exited
        while not pidfd_exited(self.pidfd, min(1000, int(_remaining(self.deadline) * 1000))):
            _remaining(self.deadline)
        self.connection.close()
        self.connection = None
        self.report.update(original_relay_exit_observed=True, gate_released=True)

    def assert_resumed(self):
        # The public scenario already checks old-token 401 and new-token SDK
        # success/RAM continuity. This adds exact captured partial-write bytes.
        sdk = self.client.sdk()
        value = bytes(sdk.files.read(self.path, format='bytes',
            request_timeout=min(10, _remaining(self.deadline))))
        if value != PREFIX:
            raise AssertionError('resumed VM lost or extended the accepted partial write')
        self.client.assert_previous_tokens_closed()
        self.report.update(resumed_exact_prefix=True, completed=True)
        return self.report.copy()

    def _open_control_directory(self):
        _private_directory(self.directory)
        descriptor = os.open(self.directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        value = os.fstat(descriptor)
        current = self.directory.lstat()
        if ((value.st_dev, value.st_ino) != self._directory_identity
                or (current.st_dev, current.st_ino) != self._directory_identity
                or value.st_uid != self._directory_owner or value.st_mode & 0o077):
            os.close(descriptor)
            raise ValueError('original data-drain control directory changed')
        return descriptor

    def _remember_control(self, name, expected):
        descriptor = self._open_control_directory()
        try:
            stamp = _control_stamp(descriptor, name, CONTROL_LIMITS[name], self._directory_owner)
        finally:
            os.close(descriptor)
        if (stamp['sha256'] != hashlib.sha256(expected).hexdigest()
                or stamp['identity'][2] != len(expected)
                or (name in self._control_files and self._control_files[name] != stamp)):
            raise ValueError('data-drain control differs from the originally observed bytes')
        self._control_files[name] = stamp
        self.report['control_file_identities'] = dict(self._control_files)

    def cleanup_after_success(self, runtime):
        """Retire only our two exact controls AFTER original product GC succeeds."""
        if (getattr(runtime, 'physical_complete', None) is not True
                or getattr(runtime, 'gc_complete', None) is not True
                or self.report.get('completed') is not True):
            raise ValueError('product physical/GC success and completed data-drain probe required')
        if self._controls_retired:
            raise ValueError('original data-drain controls already retired')
        if set(self._control_files) != set(CONTROL_LIMITS):
            raise ValueError('both original data-drain controls must be registered before retirement')
        _private_directory(self.directory.parent)
        parent_fd = os.open(self.directory.parent,
            os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        parent = os.fstat(parent_fd)
        if ((parent.st_dev, parent.st_ino) != self._parent_identity
                or parent.st_uid != self._directory_owner or parent.st_mode & 0o077):
            os.close(parent_fd)
            raise ValueError('original data-drain control parent changed')
        try:
            descriptor = self._open_control_directory()
        except BaseException:
            os.close(parent_fd)
            raise
        removed = []
        try:
            entry = os.stat(self.directory.name, dir_fd=parent_fd, follow_symlinks=False)
            if (entry.st_dev, entry.st_ino) != self._directory_identity:
                raise ValueError('original data-drain directory entry changed')
            if set(os.listdir(descriptor)) != set(CONTROL_LIMITS):
                raise ValueError('unregistered data-drain control entries preserved')
            # Check BOTH complete original identities/hashes before first unlink.
            for name, limit in CONTROL_LIMITS.items():
                if _control_stamp(descriptor, name, limit, self._directory_owner) != self._control_files[name]:
                    raise ValueError('changed data-drain control preserved before retirement')
            for name, limit in CONTROL_LIMITS.items():
                if _control_stamp(descriptor, name, limit, self._directory_owner) != self._control_files[name]:
                    raise ValueError('data-drain control changed before exact unlink')
                os.unlink(name, dir_fd=descriptor)
                removed.append(name)
            os.fsync(descriptor)
            entry = os.stat(self.directory.name, dir_fd=parent_fd, follow_symlinks=False)
            if (entry.st_dev, entry.st_ino) != self._directory_identity or os.listdir(descriptor):
                raise ValueError('original empty data-drain directory changed before retirement')
            os.rmdir(self.directory.name, dir_fd=parent_fd)
            os.fsync(parent_fd)
            self._controls_retired = True
            result = dict(state='exact-controls-retired-after-product-gc', files=list(removed),
                          original_files=dict(self._control_files), directory_fsynced=True,
                          directory_retired=True, parent_fsynced=True,
                          directory_identity=self._directory_identity, parent_identity=self._parent_identity)
            self.report['control_cleanup'] = result
            return result
        except BaseException:
            # If an unlink/fsync itself fails, record its exact partial prefix;
            # never synthesize success, recreate controls, or touch product data.
            self.report['control_cleanup'] = dict(state='unknown-preserved-original-evidence',
                removed_files=list(removed), original_files=dict(self._control_files))
            raise
        finally:
            os.close(descriptor)
            os.close(parent_fd)

    def close(self):
        # A failed observer releases only its own debug gate/socket. Original
        # product Destroy and runtime cleanup remain the outer owner's job.
        if self.peer is not None:
            self.peer.close()
            self.peer = None
        if self.connection is not None:
            self.connection.close()
            self.connection = None
        if self.pidfd is not None:
            os.close(self.pidfd)
            self.pidfd = None


if __name__ == '__main__':
    if sys.argv[1:] != ['--self-test']:
        raise SystemExit('Import from the original lifecycle scenario; only --self-test runs standalone')
    import copy
    import tempfile
    import unittest
    from unittest.mock import patch

    class DataDrainOracleTests(unittest.TestCase):
        def control_probe(self, root):
            # Same-owner unprivileged filesystem fixture only. Production
            # construction still requires root-private parent and directory.
            directory = Path(root) / 'gate'
            directory.mkdir(mode=0o700)
            probe = object.__new__(InflightDataDrain)
            probe.directory = directory
            meta, parent = directory.stat(), directory.parent.stat()
            probe._directory_identity = meta.st_dev, meta.st_ino
            probe._parent_identity = parent.st_dev, parent.st_ino
            probe._directory_owner = meta.st_uid
            probe._control_files = {}
            probe._controls_retired = False
            probe.report = {'completed': False}
            probe.peer = probe.connection = probe.pidfd = None
            for name in CONTROL_LIMITS:
                content = ('original ' + name).encode()
                path = directory / name
                path.write_bytes(content)
                path.chmod(0o600)
                probe._remember_control(name, content)
            return probe

        def test_control_cleanup_requires_product_success_and_close_only_closes(self):
            from types import SimpleNamespace
            with tempfile.TemporaryDirectory() as root, patch(__name__ + '._private_directory', Path):
                probe = self.control_probe(root)
                probe.close()
                for physical, gc, complete in ((False, True, True), (True, False, True), (True, True, False)):
                    probe.report['completed'] = complete
                    with self.assertRaises(ValueError):
                        probe.cleanup_after_success(SimpleNamespace(physical_complete=physical, gc_complete=gc))
                    self.assertEqual(set(os.listdir(probe.directory)), set(CONTROL_LIMITS))
                probe.report['completed'] = True
                result = probe.cleanup_after_success(SimpleNamespace(physical_complete=True, gc_complete=True))
                self.assertTrue(result['directory_retired'] and result['directory_fsynced'] and result['parent_fsynced'])
                self.assertFalse(probe.directory.exists())
                self.assertTrue(Path(root).is_dir())

        def test_control_cleanup_refuses_changed_or_replaced_files_before_first_unlink(self):
            from types import SimpleNamespace
            for replacement in (False, True):
                with self.subTest(replacement=replacement), tempfile.TemporaryDirectory() as root, \
                        patch(__name__ + '._private_directory', Path):
                    probe = self.control_probe(root)
                    path = probe.directory / 'unload-busy'
                    content = path.read_bytes()
                    if replacement:
                        path.rename(Path(root) / 'retained-original')
                        path.write_bytes(content)
                        path.chmod(0o600)
                    else:
                        path.write_bytes(b'changed')
                    probe.report['completed'] = True
                    with self.assertRaises(ValueError):
                        probe.cleanup_after_success(SimpleNamespace(physical_complete=True, gc_complete=True))
                    self.assertEqual(set(os.listdir(probe.directory)), set(CONTROL_LIMITS))

        def test_control_cleanup_refuses_unknown_entries_or_replaced_directory(self):
            from types import SimpleNamespace
            for replacement in (False, True):
                with self.subTest(replacement=replacement), tempfile.TemporaryDirectory() as root, \
                        patch(__name__ + '._private_directory', Path):
                    probe = self.control_probe(root)
                    if replacement:
                        probe.directory.rename(Path(root) / 'retained-directory')
                        probe.directory.mkdir(mode=0o700)
                    else:
                        (probe.directory / 'unknown').write_bytes(b'keep')
                    probe.report['completed'] = True
                    with self.assertRaises(ValueError):
                        probe.cleanup_after_success(SimpleNamespace(physical_complete=True, gc_complete=True))
                    self.assertTrue(probe.directory.is_dir())

        def test_busy_binds_every_original_field_and_relay(self):
            saved = {'accepted_version': 2, 'request': {'binding': {'session': 's', 'key':
                {'owner': '1', 'window': 1, 'sequence': 2}}, 'operation': {'kind': 'unload'}}}
            arm = b'fixed-original-relay\n'
            raw = arm + json.dumps(saved).encode()
            _require_busy(raw, arm, saved, {'accepted_version': 2})
            for field in ('session', 'key'):
                wrong = copy.deepcopy(saved)
                wrong['request']['binding'][field] = 'wrong'
                with self.assertRaises(ValueError):
                    _require_busy(arm + json.dumps(wrong).encode(), arm, saved, {'accepted_version': 2})
            with self.assertRaises(ValueError):
                _require_busy(raw, b'other-relay\n', saved, {'accepted_version': 2})
            with self.assertRaises(ValueError):
                _require_busy(raw, arm, saved, {'accepted_version': 3})

        def test_real_readonly_sql_preserves_ready_and_dispatched_actions(self):
            with tempfile.TemporaryDirectory() as root:
                database = Path(root) / 'db'
                state = {'phase': 'Pausing', 'owner': 2, 'terminal': False,
                         'generation': 1, 'current_vm': 7}
                action = {'command': {'logical': {'command': {'FreezeVm': {'vm': 7}}}}}
                with sqlite3.connect(database) as connection:
                    connection.execute('CREATE TABLE sandboxes(sandbox_id,owner_id,state_blob)')
                    connection.execute('CREATE TABLE outbox(sandbox_id,action_json,dispatch_state)')
                    connection.execute('INSERT INTO sandboxes VALUES(?,?,?)',
                        ('sb-fixed-1', 'tenant-2', json.dumps(state)))
                    connection.execute('INSERT INTO outbox VALUES(?,?,?)',
                        ('sb-fixed-1', json.dumps(action), 'dispatched'))
                native = {'database': str(database), 'namespace': 'fixed'}
                running = {'local': {'sandbox': 1, 'owner': 2},
                           'context': {'binding': {'generation': 1, 'vm': 7}}}
                with patch(__name__ + '._remaining', return_value=2):
                    actual, actions = _capture_state(native, running, 10)
                self.assertEqual(actual, state)
                self.assertEqual(actions, [{'action': action, 'state': 'dispatched'}])
                running['local']['owner'] = 3
                with patch(__name__ + '._remaining', return_value=2), self.assertRaises(ValueError):
                    _capture_state(native, running, 10)

    unittest.main(argv=[sys.argv[0]])
