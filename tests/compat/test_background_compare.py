"""Real private input program and fixed SDK command handles; no product VM."""
import asyncio
import copy
from contextlib import ExitStack
import hashlib
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

from e2b.connection_config import ConnectionConfig
from e2b.envd.process import process_pb
from e2b.sandbox.commands.command_handle import CommandResult
from e2b.sandbox_sync.commands import command as sync_command
from e2b.sandbox_async.commands import command as async_command
from packaging.version import Version
from protobuf import Oneof
from sdk_support import isolated_sdk_environment

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_background_compare as background
import sdk_guest_compare as paired
from sdk_observations import Calls


class PrivateProgramTests(unittest.TestCase):
    def test_exact_program_barrier_stdin_empty_binary_exit_and_kill(self):
        for code in (0, 17, -9):
            with self.subTest(code=code), tempfile.TemporaryDirectory() as temp:
                directory = str(Path(temp).resolve())
                os.mkfifo(directory + '/ready', 0o600)
                fifo = os.open(directory + '/ready', os.O_RDWR | os.O_NONBLOCK)
                child = None
                try:
                    child = subprocess.Popen(['/bin/bash', '--noprofile', '--norc', '-c',
                        background.command(directory, max(code, 0))], stdin=subprocess.PIPE,
                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                    self.assertTrue(select.select([fifo], [], [], 10)[0], 'program did not reach FIFO')
                    self.assertEqual(os.read(fifo, 4096), b'ready\n')
                    self.assertIsNone(child.poll(), 'program must await input')
                    self.assertEqual(Path(directory, 'started').read_bytes(), b'once\n')
                    if code == -9:
                        child.kill()
                        stdout, stderr = child.communicate(timeout=10)
                        self.assertEqual((stdout, stderr), (b'', b''))
                    else:
                        data = b'' if code == 0 else background.TEXT.encode() + background.BINARY
                        stdout, stderr = child.communicate(data, timeout=10)
                        self.assertEqual(stdout, f'{len(data)} {hashlib.sha256(data).hexdigest()}\n'.encode())
                        self.assertEqual(stderr, 'paired stderr 中文\n'.encode())
                    self.assertEqual(child.returncode, code)
                finally:
                    if child is not None:
                        if child.poll() is None:
                            child.kill()
                        child.communicate(timeout=5)
                    os.close(fifo)


class ControlledRpc:
    """Predetermined event/input boundary; not an envd process implementation."""
    def __init__(self, asynchronous, mode, pid):
        self.asynchronous, self.mode, self.pid = asynchronous, mode, pid
        self.calls, self.closed, self.data = [], [], bytearray()
        self.done, self.event = False, asyncio.Event()
        self.wrong_pid = self.corruption = self.signal_error = None
        self.code = 17 if mode == 'reconnect-exit' else 0
        self.error = 'exit status 17' if self.code else None

    def start(self, request, **kwargs):
        self.calls.append(('start', request, kwargs))
        return self.stream(process_pb.StartResponse, False, False)

    def connect(self, request, **kwargs):
        self.calls.append(('connect', request, kwargs))
        assert request.process.selector == Oneof('pid', self.pid)
        return self.stream(process_pb.ConnectResponse, self.done, bool(self.wrong_pid))

    def envelope(self, response, field, value):
        return response(event=process_pb.ProcessEvent(event=Oneof(field, value)))

    def output(self, response, late):
        stdout = b'' if late or self.code == -1 else f'{len(self.data)} {hashlib.sha256(self.data).hexdigest()}\n'.encode()
        stderr = b'' if late or self.code == -1 else 'paired stderr 中文\n'.encode()
        if self.corruption == 'stdout' and not late:
            stdout += b'!'
        if self.corruption == 'history' and late:
            stdout = b'old output'
        for channel, data in (('stdout', stdout), ('stderr', stderr)):
            # Only split on UTF-8 character boundaries: the byte-decoding
            # contract is tested separately, not loosened by this helper.
            for chunk in (data[:3], data[3:]):
                if chunk:
                    yield self.envelope(response, 'data', process_pb.ProcessEvent.DataEvent(output=Oneof(channel, chunk)))
        if self.corruption == 'missing-end':
            return
        end = dict(exit_code=self.code, exited=True)
        if self.error is not None:
            end['error'] = self.error
        yield self.envelope(response, 'end', process_pb.ProcessEvent.EndEvent(**end))

    def stream(self, response, late, wrong):
        first = self.envelope(response, 'start', process_pb.ProcessEvent.StartEvent(pid=self.pid + int(wrong)))
        async def asynchronous():
            try:
                yield first
                await asyncio.wait_for(self.event.wait(), 5)
                for item in self.output(response, late):
                    yield item
            finally:
                self.closed.append((late, wrong))
        def synchronous():
            try:
                yield first
                assert self.done, 'wait must follow close stdin or kill'
                yield from self.output(response, late)
            finally:
                self.closed.append((late, wrong))
        return asynchronous() if self.asynchronous else synchronous()

    def input(self, request, **kwargs):
        self.calls.append(('input', request, kwargs))
        assert request.process.selector == Oneof('pid', self.pid)
        assert request.input.input.field == 'stdin'
        self.data.extend(request.input.input.value)
        return process_pb.SendInputResponse()

    def close(self, request, **kwargs):
        self.calls.append(('close-stdin', request, kwargs))
        assert request.process.selector == Oneof('pid', self.pid)
        self.finish()
        return process_pb.CloseStdinResponse()

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
            self.event.set()


class OriginalSdkTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    def fixture(self, asynchronous, mode='direct', pid=4321):
        backend = ControlledRpc(asynchronous, mode, pid)
        wrap = AsyncMock if asynchronous else Mock
        rpc = SimpleNamespace(start=backend.start, connect=backend.connect,
            send_input=wrap(side_effect=backend.input), close_stdin=wrap(side_effect=backend.close),
            send_signal=wrap(side_effect=backend.signal))
        module = async_command if asynchronous else sync_command
        with patch.object(module, 'create_rpc_client', return_value=rpc):
            commands = module.Commands('http://example.invalid', ConnectionConfig(api_key='synthetic', retries=0),
                                       Version('0.9.0'), Mock())
        run = commands.run
        def foreground(command, **kwargs):
            if kwargs.get('background'):
                return run(command, **kwargs)
            output = 'ready\n' if command.startswith('cat ') else ''
            assert command.startswith(('cat ', 'mkdir -m 0700 '))
            if output:
                kwargs['on_stdout'](output)
            return CommandResult(exit_code=0, error=None, stdout=output, stderr='')
        # The SDK's complete background and handle API is real; only setup and
        # FIFO-reader commands have predetermined outcomes in this RPC fixture.
        async def async_foreground(*args, **kwargs):
            return await paired.call(foreground, *args, **kwargs)
        commands.run = wrap(side_effect=async_foreground if asynchronous else foreground)
        sandbox = SimpleNamespace(commands=commands, files=SimpleNamespace(read=wrap(return_value='once\n')))
        return Calls(sandbox, asynchronous), backend

    async def run_case(self, asynchronous, mode='direct', pid=4321):
        calls, backend = self.fixture(asynchronous, mode, pid)
        await background.background_case(calls, 'ubuntu', '/tmp/shared/ubuntu', mode)
        return calls.rows[0], backend

    async def test_original_sdks_share_all_cases_and_only_public_pid_values_differ(self):
        for mode in background.MODES:
            rows = []
            for asynchronous, pid in ((False, 4321), (True, 8123)):
                with self.subTest(mode=mode, asynchronous=asynchronous):
                    row, backend = await self.run_case(asynchronous, mode, pid)
                    rows.append(row)
                    starts = [item for item in backend.calls if item[0] == 'start']
                    self.assertEqual(len(starts), 1)
                    request, kwargs = starts[0][1:]
                    self.assertEqual((request.process.cmd, list(request.process.args[:2])), ('/bin/bash', ['-l', '-c']))
                    self.assertTrue(request.stdin)
                    self.assertEqual(request.process.cwd, '/tmp/shared/ubuntu')
                    self.assertEqual(kwargs['timeout_ms'], 60000)
                    self.assertEqual(kwargs['headers']['Authorization'], 'Basic dWJ1bnR1Og==')
                    self.assertEqual(bytes(backend.data), background.TEXT.encode() + background.BINARY if mode == 'reconnect-exit' else b'')
                    self.assertEqual(len([item for item in backend.calls if item[0] == 'kill']), int(mode == 'kill'))
                    self.assertEqual(len(backend.closed), 3 if mode == 'reconnect-exit' else 2)
                    self.assertEqual(row['pid'], pid)
            preserved = copy.deepcopy(rows)
            paired.compare([rows[0]], [rows[1]], 'same calls, different original PID')
            self.assertEqual(rows, preserved)

    async def test_normalization_rejects_wrong_references_and_does_not_rewrite_other_pid_text(self):
        original, _ = await self.run_case(False, 'reconnect-exit')
        mutations = [lambda r: r.__setitem__('pid', True),
            lambda r: r['subscriptions'][1].__setitem__('pid', 99),
            lambda r: next(c for c in r['calls'] if c['method'] == 'commands.connect')['args'].__setitem__(0, 99),
            lambda r: next(c for c in r['calls'] if c['method'] == 'handle.send_stdin').__setitem__('handle_pid', 99),
            lambda r: next(c for c in r['calls'] if c['method'] == 'commands.connect')['result'].__setitem__('pid', 99),
            lambda r: r['calls'][0].__setitem__('status', 'incomplete'),
            lambda r: r['calls'][0]['kwargs'].__setitem__('timeout', 4321),
            lambda r: r['calls'][-3]['args'].append('PID 4321'),
            lambda r: next(c for c in r['calls'] if c['method'] == 'handle.wait')['error'].__setitem__('message', 'PID 4321')]
        for mutation in mutations:
            other = copy.deepcopy(original)
            mutation(other)
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                paired.compare([original], [other], 'changed evidence')
        other = copy.deepcopy(original)
        for item in other['subscriptions']:
            for channel, parts in item['callbacks'].items():
                item['callbacks'][channel] = [''.join(parts)]
        paired.compare([original], [other], 'callback segmentation only')

    async def test_wrong_reconnect_pid_closes_both_streams_and_only_signals_original(self):
        for asynchronous in (False, True):
            calls, backend = self.fixture(asynchronous, 'reconnect-exit')
            backend.wrong_pid = True
            with self.assertRaisesRegex(AssertionError, 'changed PID'):
                await background.background_case(calls, 'ubuntu', '/tmp/shared/ubuntu', 'reconnect-exit')
            self.assertEqual([r[1].process.selector for r in backend.calls if r[0] == 'kill'], [Oneof('pid', backend.pid)])
            self.assertEqual(len(backend.closed), 2)
            self.assertEqual(calls.rows[0]['status'], 'incomplete')
            self.assertEqual(calls.rows[0]['subscriptions'][1]['pid'], backend.pid + 1)

    async def test_corrupt_output_missing_end_or_retained_history_cannot_pass(self):
        for asynchronous in (False, True):
            for corruption in ('stdout', 'missing-end', 'history'):
                with self.subTest(asynchronous=asynchronous, corruption=corruption):
                    calls, backend = self.fixture(asynchronous)
                    backend.corruption = corruption
                    with self.assertRaises(Exception):
                        await background.background_case(calls, 'ubuntu', '/tmp/shared/ubuntu', 'direct')
                    self.assertEqual(calls.rows[0]['status'], 'incomplete')
                    self.assertEqual(len([r for r in backend.calls if r[0] == 'start']), 1)
                    if corruption != 'missing-end':
                        self.assertFalse(any(r[0] == 'kill' for r in backend.calls))
                    else:
                        self.assertEqual([r[1].process.selector for r in backend.calls if r[0] == 'kill'],
                                         [Oneof('pid', backend.pid)])

    async def test_actual_end_category_and_fields_fail_without_a_signal_after_end(self):
        for asynchronous in (False, True):
            for mode, code, error in [('direct', 17, 'exit status 17'),
                                     ('reconnect-exit', 0, None),
                                     ('direct', 0, 'unexpected terminal error'),
                                     ('reconnect-exit', 17, 'unexpected terminal error')]:
                with self.subTest(asynchronous=asynchronous, mode=mode, code=code, error=error):
                    calls, backend = self.fixture(asynchronous, mode)
                    backend.code, backend.error = code, error
                    with self.assertRaises(Exception):
                        await background.background_case(calls, 'ubuntu', '/tmp/shared/ubuntu', mode)
                    self.assertFalse(any(row[0] == 'kill' for row in backend.calls), 'original End already observed')
                    self.assertEqual(len([row for row in backend.calls if row[0] == 'start']), 1)
                    self.assertEqual(len(backend.closed), 2 if mode == 'reconnect-exit' else 1)
                    row = calls.rows[0]
                    self.assertEqual(row['status'], 'incomplete')
                    terminal = next(item for item in row['calls'] if item['method'] == 'handle.wait')
                    actual = terminal['error']['fields'] if 'error' in terminal else terminal['result']
                    self.assertEqual((actual['exit_code'], actual['error']), (code, error))
                    self.assertEqual(actual['stderr'], 'paired stderr 中文\n')

    async def test_unknown_signal_is_recorded_without_replay(self):
        for asynchronous in (False, True):
            calls, backend = self.fixture(asynchronous, 'kill')
            backend.signal_error = RuntimeError('unknown signal result')
            with self.assertRaisesRegex(RuntimeError, 'unknown signal result'):
                await background.background_case(calls, 'ubuntu', '/tmp/shared/ubuntu', 'kill')
            self.assertEqual(len([r for r in backend.calls if r[0] == 'kill']), 1)
            self.assertEqual(len(backend.closed), 1)
            signal = next(c for c in calls.rows[0]['calls'] if c['method'] == 'commands.kill')
            self.assertEqual(signal['status'], 'incomplete')
            self.assertEqual(signal['error']['message'], 'unknown signal result')

    async def test_unknown_start_never_invents_pid_or_control_requests(self):
        for asynchronous in (False, True):
            calls, backend = self.fixture(asynchronous)
            original = calls.sandbox.commands.run
            async def run(*args, **kwargs):
                if kwargs.get('background'):
                    raise RuntimeError('unknown Start result')
                return await paired.call(original, *args, **kwargs)
            calls.sandbox.commands.run = run
            with self.assertRaisesRegex(RuntimeError, 'unknown Start result'):
                await background.background_case(calls, 'ubuntu', '/tmp/shared/ubuntu', 'direct')
            row = calls.rows[0]
            self.assertNotIn('pid', row)
            self.assertEqual(backend.calls, [])
            self.assertEqual(row['calls'][-1]['status'], 'incomplete')

    async def test_cleanup_keeps_primary_and_every_close_error_without_repeating_close(self):
        calls, backend = self.fixture(True, 'reconnect-exit')
        commands = calls.sandbox.commands
        original_connect = commands.connect
        async def connect(*args, **kwargs):
            handle = await original_connect(*args, **kwargs)
            original = handle.disconnect
            async def fail_close():
                await original()
                raise OSError('close outcome unknown')
            handle.disconnect = fail_close
            return handle
        commands.connect = connect
        backend.wrong_pid = True
        backend.signal_error = RuntimeError('kill outcome unknown')
        with self.assertRaises(BaseExceptionGroup) as raised:
            await background.background_case(calls, 'ubuntu', '/tmp/shared/ubuntu', 'reconnect-exit')
        self.assertEqual([type(e) for e in raised.exception.exceptions], [AssertionError, RuntimeError, OSError])
        self.assertEqual(len(backend.closed), 2)
        self.assertEqual(len([r for r in backend.calls if r[0] == 'kill']), 1)
        disconnects = [r for r in calls.rows[0]['calls'] if r['method'] == 'handle.disconnect']
        self.assertEqual([r['subscription'] for r in disconnects], ['start', 'resumed'])


if __name__ == '__main__':
    unittest.main()
