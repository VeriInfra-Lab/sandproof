"""One partial real upload on the lifecycle scenario's EXISTING VM.

The sibling runtime only installs the debug environment before data work starts.
This helper never creates a VM, restarts managers, changes a ledger or sends a
replacement write. Reports contain identity/digests, never bearer credentials.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_runpy.run_path(str(_GuardPath(__file__).with_name('_assertions_required.py')))[
    'require_assertions']()

import hashlib
import http.client
import json
import os
from pathlib import Path
import socket
import sqlite3
import stat
import sys
import time
from urllib.parse import urlencode, urlsplit


PREFIX = bytes(range(256)) * 16
MAGIC = 'sandproof-data-drain-v1'


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
    position = running['context']['position']
    binding = running['context']['binding']
    directory = _private_directory(Path(native['node_state_dir']) / 'helper-gates')
    result = {}
    maximum = native['bounded_host']['max_data_routes_per_slot']
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
                or record['control'] not in (False, True)):
            raise ValueError('exact original route cell/context required')
        if _alive(record['process']):
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
            except FileNotFoundError:
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
                state = {'phase': 'Pausing'}
                action = {'command': {'logical': {'command': {'FreezeVm': {'vm': 7}}}}}
                with sqlite3.connect(database) as connection:
                    connection.execute('CREATE TABLE sandboxes(sandbox_id,owner_id,state_blob)')
                    connection.execute('CREATE TABLE outbox(sandbox_id,action_json,dispatch_state)')
                    connection.execute('INSERT INTO sandboxes VALUES(?,?,?)',
                        ('sb-fixed-1', 'tenant-2', json.dumps(state)))
                    connection.execute('INSERT INTO outbox VALUES(?,?,?)',
                        ('sb-fixed-1', json.dumps(action), 'dispatched'))
                native = {'database': str(database), 'namespace': 'fixed'}
                running = {'local': {'sandbox': 1, 'owner': 2}}
                with patch(__name__ + '._remaining', return_value=2):
                    actual, actions = _capture_state(native, running, 10)
                self.assertEqual(actual, state)
                self.assertEqual(actions, [{'action': action, 'state': 'dispatched'}])
                running['local']['owner'] = 3
                with patch(__name__ + '._remaining', return_value=2), self.assertRaises(ValueError):
                    _capture_state(native, running, 10)

    unittest.main(argv=[sys.argv[0]])
