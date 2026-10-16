"""Real private PTY helper and original SDK orchestration; no VM or network."""
import asyncio
import base64
from contextlib import ExitStack
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shlex
import struct
import subprocess
import sys
import tempfile
import termios
import time
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

from e2b.connection_config import ConnectionConfig
from e2b.envd.process import process_pb
from e2b.sandbox_sync.commands import pty as sync_pty
from e2b.sandbox_async.commands import pty as async_pty
from packaging.version import Version
from protobuf import Oneof
from sdk_support import isolated_sdk_environment

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_pty as probe
import sdk_pty_compare as paired_probe


def winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))


def read_ready(fd, master, capture):
    data = bytearray()
    deadline = time.monotonic() + 10
    while b'\n' not in data:
        remaining = deadline - time.monotonic()
        readable = select.select([fd, master], [], [], max(0, remaining))[0]
        if remaining <= 0 or not readable:
            raise AssertionError('private PTY did not reach its FIFO barrier: ' + repr(capture.data[-2048:]))
        if master in readable:
            try:
                chunk = os.read(master, 16384)
            except OSError as error:
                raise AssertionError('PTY exited before ready: ' + repr(capture.data[-2048:])) from error
            if not chunk:
                raise AssertionError('PTY exited before ready: ' + repr(capture.data[-2048:]))
            capture(chunk)
        if fd in readable:
            data.extend(os.read(fd, 4096))
        assert len(data) <= 4096
    return json.loads(data)


class PrivatePtyProgramTests(unittest.TestCase):
    def test_exact_bootstrap_raw_all_bytes_merged_output_resize_exit_and_kill(self):
        for exit_code, report_ready in ((code, flag) for code in (0, 17, -9) for flag in (False, True)):
            with self.subTest(exit_code=exit_code, report_ready=report_ready), tempfile.TemporaryDirectory() as temp:
                directory = str(Path(temp).resolve())
                os.mkfifo(directory + '/ready', 0o600)
                Path(directory, 'program.py').write_text(probe.GUEST_PROGRAM)
                fifo = os.open(directory + '/ready', os.O_RDWR | os.O_NONBLOCK)
                master, slave = pty.openpty()
                child = None
                try:
                    winsize(slave, 24, 80)
                    # No host profiles: only the exact transmitted bootstrap and
                    # helper are tested here, not guest login-shell configuration.
                    child = subprocess.Popen(['/bin/bash', '--noprofile', '--norc', '-i'],
                        stdin=slave, stdout=slave, stderr=slave, cwd=directory,
                        env={**os.environ, 'SANDPROOF_PTY_VALUE': probe.VALUE}, start_new_session=True)
                    os.close(slave)
                    slave = None
                    config = probe.configuration(directory, max(exit_code, 0))
                    if report_ready:
                        config['report_ready'] = True
                    line = probe.bootstrap(config)
                    self.assertEqual(os.write(master, line), len(line))
                    capture = probe.Capture()
                    ready = read_ready(fifo, master, capture)
                    self.assertEqual(ready, dict(pid=child.pid, uid=os.getuid(), cwd=directory,
                        value=probe.VALUE, size=[24, 80], tty=[True]*3))
                    if exit_code == -9:
                        child.kill()
                    else:
                        winsize(master, 30, 100)
                        self.assertEqual(os.write(master, probe.PAYLOAD[:513]), 513)
                        self.assertEqual(os.write(master, probe.PAYLOAD[513:]), len(probe.PAYLOAD)-513)
                    deadline = time.monotonic() + 10
                    while True:
                        remaining = deadline - time.monotonic()
                        self.assertGreater(remaining, 0)
                        self.assertTrue(select.select([master], [], [], remaining)[0])
                        try:
                            chunk = os.read(master, 16384)
                        except OSError as error:
                            if error.errno != errno.EIO:
                                raise
                            break  # Linux PTY EOF; Darwin returns empty bytes.
                        if not chunk:
                            break
                        capture(chunk)
                    self.assertEqual(child.wait(timeout=5), exit_code)
                    if exit_code == -9:
                        self.assertNotIn(probe.HEADER, capture.data)
                        if report_ready:
                            self.assertTrue(capture.data.endswith(probe.READY_HEADER))
                    else:
                        result = probe.validate_frame(capture.data, ready, [30, 100])
                        self.assertEqual(result['received_bytes'], len(probe.PAYLOAD))
                        if report_ready:
                            paired_probe.frame(bytes(capture.data), ready, startup=True)
                    self.assertEqual(Path(directory, 'started').read_bytes(), b'once\n')
                finally:
                    if child is not None:
                        if child.poll() is None:
                            child.kill()
                        child.wait(timeout=5)
                    for fd in (slave, master, fifo):
                        if fd is not None:
                            os.close(fd)


class ControlledRpc:
    """Predetermined SDK event boundary, not an implementation of envd."""
    def __init__(self, asynchronous):
        self.asynchronous = asynchronous
        self.ready_event = asyncio.Event()
        self.calls, self.closed = [], []
        self.pid, self.done, self.data = 4321, False, bytearray()
        self.corruption = self.wrong_pid = self.signal_error = None
        self.config = self.ready = None
        self.code, self.error, self.resized = 0, None, None
        self.list_after_end = False
        self.end_override = None
        self.missing_end = False

    def start(self, request, **kwargs):
        self.calls.append(('start', request, kwargs))
        self.start_request = request
        return self.stream(process_pb.StartResponse, False, False)

    def connect(self, request, **kwargs):
        self.calls.append(('connect', request, kwargs))
        assert request.process.selector == Oneof('pid', self.pid)
        return self.stream(process_pb.ConnectResponse, self.done, bool(self.wrong_pid))

    def event(self, response, field, value):
        return response(event=process_pb.ProcessEvent(event=Oneof(field, value)))

    def output(self, response, late):
        if not late and self.code != -1:
            body = json.dumps(dict(ready=self.ready, resized=self.resized)).encode()
            data = probe.HEADER + struct.pack('!I', len(body)) + body + bytes(self.data)
            if self.corruption == 'binary':
                data = data[:-1] + bytes([data[-1] ^ 1])
            elif self.corruption == 'truncated':
                data = data[:-1]
            elif self.corruption == 'extra':
                data += b'!'
            for chunk in (b'controlled shell prompt\r\n', data[:7], data[7:]):
                yield self.event(response, 'data', process_pb.ProcessEvent.DataEvent(output=Oneof('pty', chunk)))
        if late and self.corruption == 'history':
            yield self.event(response, 'data', process_pb.ProcessEvent.DataEvent(output=Oneof('pty', b'old')))
        end = {'exit_code': self.code, 'exited': True}
        if self.error is not None:
            end['error'] = self.error
        if not self.missing_end:
            yield self.event(response, 'end', process_pb.ProcessEvent.EndEvent(**(self.end_override or end)))

    def stream(self, response, late, wrong):
        first = self.event(response, 'start', process_pb.ProcessEvent.StartEvent(pid=self.pid + int(wrong)))
        async def asynchronous():
            try:
                yield first
                await asyncio.wait_for(self.ready_event.wait(), 5)
                for event in self.output(response, late):
                    yield event
            finally:
                self.closed.append((late, wrong))
        def synchronous():
            try:
                yield first
                assert self.done, 'sync SDK must consume output after the input barrier'
                yield from self.output(response, late)
            finally:
                self.closed.append((late, wrong))
        return asynchronous() if self.asynchronous else synchronous()

    def input(self, request, **kwargs):
        self.calls.append(('input', request, kwargs))
        assert request.process.selector == Oneof('pid', self.pid)
        assert request.input.input.field == 'pty'
        value = request.input.input.value
        if self.config is None:
            self.config = json.loads(base64.b64decode(shlex.split(value.decode())[-1]))
            self.code = self.config['exit_code']
            self.error = 'exit status 17' if self.code else None
            self.ready = dict(pid=self.pid, uid=1000, cwd=self.config['directory'],
                              value=probe.VALUE, size=[24, 80], tty=[True]*3)
        else:
            self.data.extend(value)
            if len(self.data) == len(probe.PAYLOAD):
                self.finish()
        return process_pb.SendInputResponse()

    def update(self, request, **kwargs):
        self.calls.append(('resize', request, kwargs))
        assert request.process.selector == Oneof('pid', self.pid)
        self.resized = [request.pty.size.rows & 65535, request.pty.size.cols & 65535]
        return process_pb.UpdateResponse()

    def signal(self, request, **kwargs):
        self.calls.append(('kill', request, kwargs))
        assert request.process.selector == Oneof('pid', self.pid)
        assert request.signal == process_pb.Signal.SIGKILL
        if self.signal_error:
            raise self.signal_error
        self.code, self.error = -1, 'signal: killed'
        self.finish()
        return process_pb.SendSignalResponse()

    def finish(self):
        self.done = True
        if self.asynchronous:
            self.ready_event.set()

    def run(self, command, **kwargs):
        if command.startswith('cat '):
            return SimpleNamespace(exit_code=0, stdout=json.dumps(self.ready), stderr='')
        assert command.startswith('mkdir -m 0700 ')
        assert kwargs == dict(user='ubuntu', timeout=20)
        return SimpleNamespace(exit_code=0, stdout='', stderr='')

    def list(self, **kwargs):
        return [] if self.done and not self.list_after_end else [SimpleNamespace(pid=self.pid)]


class OriginalPtySdkTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    def sandbox(self, asynchronous):
        backend = ControlledRpc(asynchronous)
        wrap = AsyncMock if asynchronous else Mock
        rpc = SimpleNamespace(start=backend.start, connect=backend.connect,
            send_input=wrap(side_effect=backend.input), update=wrap(side_effect=backend.update),
            send_signal=wrap(side_effect=backend.signal))
        module = async_pty if asynchronous else sync_pty
        with patch.object(module, 'create_rpc_client', return_value=rpc):
            client = module.Pty('http://example.invalid', ConnectionConfig(api_key='synthetic', retries=0),
                                Version('0.9.0'), Mock())
        sandbox = SimpleNamespace(pty=client,
            commands=SimpleNamespace(run=wrap(side_effect=backend.run), list=wrap(side_effect=backend.list)),
            files=SimpleNamespace(read=wrap(return_value='once\n'), write=wrap()))
        return sandbox, backend

    async def test_original_public_pty_and_handles_consume_all_three_cases_in_both_modes(self):
        for asynchronous in (False, True):
            for mode in ('direct', 'reconnect-exit', 'kill'):
                with self.subTest(asynchronous=asynchronous, mode=mode):
                    sandbox, backend = self.sandbox(asynchronous)
                    result = await probe.pty_case(sandbox, 'ubuntu', 1000, mode, asynchronous)
                    self.assertEqual(result['pid'], backend.pid)
                    request = backend.start_request
                    self.assertEqual((request.process.cmd, list(request.process.args)), ('/bin/bash', ['-i', '-l']))
                    self.assertEqual(request.process.cwd, backend.config['directory'])
                    sandbox.files.write.assert_called_once_with(backend.config['directory'] + '/program.py',
                        probe.GUEST_PROGRAM, user='ubuntu', request_timeout=15)
                    self.assertEqual(dict(request.process.envs), dict(SANDPROOF_PTY_VALUE=probe.VALUE,
                        TERM='xterm-256color', LANG='C.UTF-8', LC_ALL='C.UTF-8'))
                    starts = [row for row in backend.calls if row[0] == 'start']
                    self.assertEqual(len(starts), 1)
                    self.assertEqual(starts[0][2]['timeout_ms'], 60000)
                    self.assertEqual(starts[0][2]['headers']['Authorization'], 'Basic dWJ1bnR1Og==')
                    self.assertEqual(len(backend.closed), 3 if mode == 'reconnect-exit' else 2)
                    self.assertEqual(len([row for row in backend.calls if row[0] == 'kill']), int(mode == 'kill'))
                    if mode != 'kill':
                        self.assertEqual(bytes(backend.data), probe.PAYLOAD)
                        self.assertEqual(result['observed_resize'], [30, 100])
                        self.assertEqual(result['requested_size'], [65560, 65616] if mode == 'reconnect-exit' else [24, 80])

    async def test_bad_binary_frames_and_replayed_history_fail_after_original_end(self):
        for asynchronous in (False, True):
            for corruption in ('binary', 'truncated', 'extra', 'history'):
                with self.subTest(asynchronous=asynchronous, corruption=corruption):
                    sandbox, backend = self.sandbox(asynchronous)
                    backend.corruption = corruption
                    with self.assertRaises(AssertionError):
                        await probe.pty_case(sandbox, 'ubuntu', 1000, 'direct', asynchronous)
                    self.assertEqual(len(backend.closed), 2)
                    self.assertFalse(any(row[0] == 'kill' for row in backend.calls))

    async def test_wrong_terminal_fields_fail_without_signalling_an_already_finished_pty(self):
        for asynchronous in (False, True):
            for mode, code, error in [('direct', 17, 'exit status 17'),
                                     ('reconnect-exit', 0, None),
                                     ('direct', 0, 'unexpected terminal error'),
                                     ('reconnect-exit', 17, 'unexpected terminal error')]:
                with self.subTest(asynchronous=asynchronous, mode=mode, code=code, error=error):
                    sandbox, backend = self.sandbox(asynchronous)
                    backend.end_override = dict(exit_code=code, exited=True)
                    if error is not None:
                        backend.end_override['error'] = error
                    with self.assertRaises(AssertionError) as raised:
                        await probe.pty_case(sandbox, 'ubuntu', 1000, mode, asynchronous)
                    self.assertEqual(raised.exception.args[0][0], (code, error, '', ''))
                    self.assertFalse(any(row[0] == 'kill' for row in backend.calls), 'actual End already received')
                    self.assertEqual(len([row for row in backend.calls if row[0] == 'start']), 1)
                    self.assertEqual(len(backend.closed), 2 if mode == 'reconnect-exit' else 1)

    async def test_missing_end_still_cleans_up_only_the_known_original_pty(self):
        for asynchronous in (False, True):
            sandbox, backend = self.sandbox(asynchronous)
            backend.missing_end = True
            with self.assertRaises(Exception):
                await probe.pty_case(sandbox, 'ubuntu', 1000, 'direct', asynchronous)
            self.assertEqual([row[1].process.selector for row in backend.calls if row[0] == 'kill'],
                             [Oneof('pid', backend.pid)])
            self.assertEqual(len(backend.closed), 1)

    async def test_wrong_reconnect_pid_closes_both_streams_and_kills_only_original_pid(self):
        for asynchronous in (False, True):
            sandbox, backend = self.sandbox(asynchronous)
            backend.wrong_pid = True
            with self.assertRaisesRegex(AssertionError, 'changed PID'):
                await probe.pty_case(sandbox, 'ubuntu', 1000, 'reconnect-exit', asynchronous)
            signals = [row[1].process.selector for row in backend.calls if row[0] == 'kill']
            self.assertEqual(signals, [Oneof('pid', backend.pid)])
            self.assertEqual(len(backend.closed), 2)

    async def test_unknown_kill_is_not_repeated_and_original_stream_is_closed(self):
        for asynchronous in (False, True):
            sandbox, backend = self.sandbox(asynchronous)
            backend.signal_error = RuntimeError('controlled unknown signal outcome')
            with self.assertRaisesRegex(Exception, 'unknown signal outcome'):
                await probe.pty_case(sandbox, 'ubuntu', 1000, 'kill', asynchronous)
            self.assertEqual(len([row for row in backend.calls if row[0] == 'kill']), 1)
            self.assertEqual(len(backend.closed), 1)

    async def test_unknown_start_does_not_replay_or_invent_a_pid_to_kill(self):
        for asynchronous in (False, True):
            sandbox, backend = self.sandbox(asynchronous)
            replacement = AsyncMock if asynchronous else Mock
            with patch.object(sandbox.pty, 'create', new=replacement(side_effect=RuntimeError('unknown Start'))) as create:
                with self.assertRaisesRegex(RuntimeError, 'unknown Start'):
                    await probe.pty_case(sandbox, 'ubuntu', 1000, 'direct', asynchronous)
                create.assert_called_once()
            self.assertEqual(backend.calls, [])
            self.assertEqual(backend.closed, [])

    async def test_end_does_not_invent_a_synchronous_list_removal_guarantee(self):
        for asynchronous in (False, True):
            sandbox, backend = self.sandbox(asynchronous)
            backend.list_after_end = True
            result = await probe.pty_case(sandbox, 'ubuntu', 1000, 'direct', asynchronous)
            self.assertTrue(result['listed_after_end'])
            self.assertFalse(any(row[0] == 'kill' for row in backend.calls))

    async def test_cleanup_attempts_all_handles_and_keeps_primary_failure(self):
        client = SimpleNamespace(kill=AsyncMock(side_effect=OSError('signal failed')))
        handles = [SimpleNamespace(disconnect=AsyncMock(side_effect=RuntimeError('close failed'))),
                   SimpleNamespace(disconnect=AsyncMock())]
        with self.assertRaises(ExceptionGroup) as caught:
            try:
                raise ValueError('primary')
            finally:
                await probe.close_case(client, 42, handles, finished=False, kill_attempted=False)
        self.assertEqual([type(error) for error in caught.exception.exceptions], [OSError, RuntimeError])
        self.assertIsInstance(caught.exception.__context__, ValueError)
        client.kill.assert_awaited_once_with(42, request_timeout=15)
        for handle in handles:
            handle.disconnect.assert_awaited_once()

    def test_callback_bound_rejects_instead_of_silently_truncating(self):
        capture = probe.Capture()
        capture(b'x' * probe.OUTPUT_LIMIT)
        for chunk in (b'!', 'text'):
            with self.assertRaises(ValueError):
                capture(chunk)
            self.assertEqual(len(capture.data), probe.OUTPUT_LIMIT)


if __name__ == '__main__':
    unittest.main()
