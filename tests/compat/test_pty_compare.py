"""Original sync/async PTY SDK against controlled events in the paired probe."""
import asyncio
import copy
from contextlib import ExitStack
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

from e2b.connection_config import ConnectionConfig
from e2b.envd.process import process_pb
from e2b.sandbox.commands.command_handle import CommandResult
from e2b.sandbox_sync.commands import pty as sync_pty
from e2b.sandbox_async.commands import pty as async_pty
from packaging.version import Version
from protobuf import Oneof
from sdk_support import isolated_sdk_environment
from test_pty_probe import ControlledRpc

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_pty_compare as probe
import sdk_guest_compare as paired
from sdk_observations import Calls, value


class PairedRpc(ControlledRpc):
    def __init__(self, asynchronous, pid):
        super().__init__(asynchronous)
        self.pid, self.bootstrapped = pid, asyncio.Event()
        self.startup = b'controlled shell prompt\r\n'
        self.bad_ready = False

    def input(self, request, **kwargs):
        first = self.config is None
        result = super().input(request, **kwargs)
        if first:
            assert self.config['report_ready'] is True
            if self.asynchronous:
                self.bootstrapped.set()
        return result

    def output(self, response, late):
        for event in super().output(response, late):
            item = event.event.event
            if item.field == 'data' and item.value.output.value == b'controlled shell prompt\r\n':
                continue  # The startup bytes are emitted before the ready barrier.
            if self.corruption == 'missing-end' and item.field == 'end':
                continue
            if self.corruption == 'end-error' and item.field == 'end':
                event = self.event(response, 'end', process_pb.ProcessEvent.EndEvent(
                    exit_code=item.value.exit_code, exited=True, error='unexpected terminal error'))
            if self.corruption == 'wrong-exit' and item.field == 'end':
                code = 0 if item.value.exit_code else 17
                fields = dict(exit_code=code, exited=True)
                if code:
                    fields['error'] = 'exit status 17'
                event = self.event(response, 'end', process_pb.ProcessEvent.EndEvent(**fields))
            yield event

    def stream(self, response, late, wrong):
        first = self.event(response, 'start', process_pb.ProcessEvent.StartEvent(pid=self.pid + int(wrong)))
        start = response is process_pb.StartResponse
        def prefix():
            data = self.startup + probe.READY_HEADER
            for chunk in (data[:5], data[5:-3], data[-3:]):
                yield self.event(response, 'data', process_pb.ProcessEvent.DataEvent(output=Oneof('pty', chunk)))
        async def asynchronous():
            try:
                yield first
                if start:
                    await asyncio.wait_for(self.bootstrapped.wait(), 5)
                    for item in prefix():
                        yield item
                await asyncio.wait_for(self.ready_event.wait(), 5)
                for item in self.output(response, late):
                    yield item
            finally:
                self.closed.append((late, wrong))
        def synchronous():
            try:
                yield first
                if start:
                    assert self.config is not None
                    yield from prefix()
                assert self.done, 'terminal read must follow input or kill'
                yield from self.output(response, late)
            finally:
                self.closed.append((late, wrong))
        return asynchronous() if self.asynchronous else synchronous()

    def run(self, command, **kwargs):
        if command.startswith('cat '):
            ready = dict(self.ready)
            if self.bad_ready:
                ready['pid'] += 1
            return CommandResult(exit_code=0, error=None, stdout=probe.ready_text(ready), stderr='')
        assert command.startswith('mkdir -m 0700 ')
        assert kwargs['user'] == 'ubuntu' and kwargs['timeout'] == 20
        return CommandResult(exit_code=0, error=None, stdout='', stderr='')


class PairedPtyTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    def fixture(self, asynchronous, pid=4321):
        backend = PairedRpc(asynchronous, pid)
        wrap = AsyncMock if asynchronous else Mock
        rpc = SimpleNamespace(start=backend.start, connect=backend.connect,
            send_input=wrap(side_effect=backend.input), update=wrap(side_effect=backend.update),
            send_signal=wrap(side_effect=backend.signal))
        module = async_pty if asynchronous else sync_pty
        with patch.object(module, 'create_rpc_client', return_value=rpc):
            client = module.Pty('http://example.invalid', ConnectionConfig(api_key='synthetic', retries=0),
                                Version('0.9.0'), Mock())
        sandbox = SimpleNamespace(pty=client,
            commands=SimpleNamespace(run=wrap(side_effect=backend.run)),
            files=SimpleNamespace(read=wrap(return_value='once\n'), write=wrap(return_value=None)))
        return Calls(sandbox, asynchronous), backend

    async def run_case(self, asynchronous, mode='direct', pid=4321):
        calls, backend = self.fixture(asynchronous, pid)
        await probe.pty_case(calls, 'ubuntu', 1000, '/tmp/shared/ubuntu', mode)
        return calls.rows[0], backend

    async def test_all_modes_both_sdks_keep_startup_bytes_binary_sizes_terminal_and_original_pid(self):
        for mode in ('direct', 'reconnect-exit', 'kill'):
            rows = []
            for asynchronous, pid in ((False, 4321), (True, 18123)):
                with self.subTest(mode=mode, asynchronous=asynchronous):
                    row, backend = await self.run_case(asynchronous, mode, pid)
                    rows.append(row)
                    request = backend.start_request
                    self.assertEqual((request.process.cmd, list(request.process.args)), ('/bin/bash', ['-i', '-l']))
                    self.assertEqual(request.process.cwd, '/tmp/shared/ubuntu/pty-' + mode)
                    self.assertEqual(dict(request.process.envs), dict(SANDPROOF_PTY_VALUE=probe.VALUE,
                        TERM='xterm-256color', LANG='C.UTF-8', LC_ALL='C.UTF-8'))
                    self.assertEqual(len([r for r in backend.calls if r[0] == 'start']), 1)
                    self.assertEqual(len(backend.closed), 3 if mode == 'reconnect-exit' else 2)
                    self.assertEqual(row['pid'], pid)
                    if mode != 'kill':
                        self.assertEqual(bytes(backend.data), probe.PAYLOAD)
                        self.assertEqual(backend.resized, [30, 100])
                        self.assertEqual(request.pty.size.rows, 65560 if mode == 'reconnect-exit' else 24)
            preserved = copy.deepcopy(rows)
            paired.compare([rows[0]], [rows[1]], 'paired PTY SDK modes with different PID widths')
            self.assertEqual(rows, preserved)

    async def test_comparison_keeps_startup_errors_inputs_and_metadata_and_checks_every_pid_reference(self):
        original, _ = await self.run_case(False, 'reconnect-exit')
        mutations = [lambda r: r.__setitem__('pid', True),
            lambda r: r['ready'].__setitem__('pid', 99),
            lambda r: r['subscriptions'][1].__setitem__('pid', 99),
            lambda r: next(c for c in r['calls'] if c['method'] == 'pty.connect')['args'].__setitem__(0, 99),
            lambda r: next(c for c in r['calls'] if c['method'] == 'pty.connect')['result'].__setitem__('pid', 99),
            lambda r: next(c for c in r['calls'] if c['method'] == 'handle.wait').__setitem__('handle_pid', 99),
            lambda r: r['subscriptions'][0]['chunks'].insert(0, value(b'PID 4321')),
            lambda r: next(c for c in r['calls'] if c['method'] == 'handle.wait')['error'].__setitem__('message', 'PID 4321'),
            lambda r: next(c for c in r['calls'] if c['method'] == 'pty.resize')['args'][1].__setitem__('rows', 4321),
            lambda r: r.__setitem__('uid', 4321),
            lambda r: r['calls'][0].__setitem__('status', 'incomplete')]
        for mutate in mutations:
            other = copy.deepcopy(original)
            mutate(other)
            with self.subTest(mutate=mutate), self.assertRaises(AssertionError):
                paired.compare([original], [other], 'changed raw evidence')
        other = copy.deepcopy(original)
        for subscription in other['subscriptions']:
            data = b''.join(probe.base64.b64decode(c['base64']) for c in subscription['chunks'])
            subscription['chunks'] = [value(data)]
        paired.compare([original], [other], 'segmentation only')

    async def test_wrong_ready_or_reconnect_never_signals_the_returned_wrong_pid(self):
        for asynchronous in (False, True):
            for fault in ('ready', 'connect'):
                calls, backend = self.fixture(asynchronous)
                backend.bad_ready = fault == 'ready'
                backend.wrong_pid = fault == 'connect'
                with self.subTest(asynchronous=asynchronous, fault=fault), self.assertRaises(AssertionError):
                    await probe.pty_case(calls, 'ubuntu', 1000, '/tmp/shared/ubuntu', 'reconnect-exit')
                self.assertEqual([r[1].process.selector for r in backend.calls if r[0] == 'kill'], [Oneof('pid', backend.pid)])
                self.assertEqual(len(backend.closed), 1 if fault == 'ready' else 2)
                self.assertEqual(calls.rows[0]['status'], 'incomplete')

    async def test_ready_and_frame_json_are_exact_bytes_not_permissively_reencoded(self):
        original, _ = await self.run_case(False)
        other = copy.deepcopy(original)
        ready_call = next(c for c in other['calls'] if c.get('pty_ready'))
        ready_call['result']['stdout'] = json.dumps(other['ready'], separators=(',', ':')) + '\n'
        with self.assertRaises(AssertionError):
            paired.compare([original], [other], 'changed readiness encoding')
        other = copy.deepcopy(original)
        captured = other['subscriptions'][0]
        data = b''.join(probe.base64.b64decode(c['base64']) for c in captured['chunks'])
        prefix, _, rest = data.partition(probe.HEADER)
        size = probe.struct.unpack('!I', rest[:4])[0]
        body = rest[4:4+size].replace(b'"pid": 4321', b'"pid": 4321, "pid": 4321')
        captured['chunks'] = [value(prefix + probe.HEADER + probe.struct.pack('!I', len(body)) + body + rest[4+size:])]
        with self.assertRaisesRegex(AssertionError, 'metadata or binary'):
            paired.compare([original], [other], 'duplicate JSON key')

    async def test_binary_corruption_truncation_extra_history_and_missing_end_fail_without_start_replay(self):
        for asynchronous in (False, True):
            for corruption in ('binary', 'truncated', 'extra', 'history', 'missing-end', 'end-error'):
                calls, backend = self.fixture(asynchronous)
                backend.corruption = corruption
                with self.subTest(asynchronous=asynchronous, corruption=corruption), self.assertRaises(Exception):
                    await probe.pty_case(calls, 'ubuntu', 1000, '/tmp/shared/ubuntu', 'direct')
                self.assertEqual(len([r for r in backend.calls if r[0] == 'start']), 1)
                self.assertEqual(calls.rows[0]['status'], 'incomplete')
                if corruption != 'missing-end':
                    self.assertFalse(any(r[0] == 'kill' for r in backend.calls), 'End was already observed')

    async def test_unknown_signal_is_not_retried_and_partial_bytes_are_preserved(self):
        for asynchronous in (False, True):
            calls, backend = self.fixture(asynchronous)
            backend.signal_error = RuntimeError('unknown signal result')
            with self.assertRaisesRegex(RuntimeError, 'unknown signal result'):
                await probe.pty_case(calls, 'ubuntu', 1000, '/tmp/shared/ubuntu', 'kill')
            self.assertEqual(len([r for r in backend.calls if r[0] == 'kill']), 1)
            self.assertEqual(len(backend.closed), 1)
            row = calls.rows[0]
            self.assertTrue(row['subscriptions'][0]['chunks'])
            signal = next(c for c in row['calls'] if c['method'] == 'pty.kill')
            self.assertEqual(signal['status'], 'incomplete')
            self.assertEqual(signal['error']['message'], 'unknown signal result')

    async def test_unexpected_success_or_nonzero_end_keeps_evidence_without_an_extra_signal(self):
        for asynchronous in (False, True):
            for mode in ('direct', 'reconnect-exit'):
                calls, backend = self.fixture(asynchronous)
                backend.corruption = 'wrong-exit'
                with self.subTest(asynchronous=asynchronous, mode=mode), self.assertRaises(Exception):
                    await probe.pty_case(calls, 'ubuntu', 1000, '/tmp/shared/ubuntu', mode)
                self.assertFalse(any(r[0] == 'kill' for r in backend.calls))
                terminal = next(c for c in calls.rows[0]['calls'] if c['method'] == 'handle.wait')
                self.assertEqual(terminal['status'], 'incomplete')
                if mode == 'direct':
                    self.assertEqual(terminal['error']['fields']['exit_code'], 17)
                else:
                    self.assertEqual(terminal['result']['exit_code'], 0)

    async def test_unknown_start_has_one_attempt_and_no_invented_control_identity(self):
        for asynchronous in (False, True):
            calls, backend = self.fixture(asynchronous)
            wrap = AsyncMock if asynchronous else Mock
            calls.sandbox.pty.create = wrap(side_effect=RuntimeError('unknown Start'))
            with self.assertRaisesRegex(RuntimeError, 'unknown Start'):
                await probe.pty_case(calls, 'ubuntu', 1000, '/tmp/shared/ubuntu', 'direct')
            calls.sandbox.pty.create.assert_called_once()
            self.assertNotIn('pid', calls.rows[0])
            self.assertEqual(backend.calls, [])

    async def test_unknown_disconnect_keeps_all_cleanup_failures_and_does_not_repeat_close(self):
        calls, backend = self.fixture(True)
        connect = calls.sandbox.pty.connect
        async def broken_connect(*args, **kwargs):
            handle = await connect(*args, **kwargs)
            close = handle.disconnect
            async def broken_close():
                await close()
                raise OSError('unknown disconnect')
            handle.disconnect = broken_close
            return handle
        calls.sandbox.pty.connect = broken_connect
        backend.wrong_pid = True
        backend.signal_error = RuntimeError('unknown cleanup signal')
        with self.assertRaises(BaseExceptionGroup) as raised:
            await probe.pty_case(calls, 'ubuntu', 1000, '/tmp/shared/ubuntu', 'reconnect-exit')
        self.assertEqual([type(e) for e in raised.exception.exceptions], [AssertionError, RuntimeError, OSError])
        self.assertEqual(len(backend.closed), 2)
        self.assertEqual(len([r for r in backend.calls if r[0] == 'kill']), 1)

    async def test_callback_bound_and_early_extra_data_fail_instead_of_truncating(self):
        capture = probe.Capture(True)
        with self.assertRaisesRegex(AssertionError, 'followed ready'):
            capture(probe.READY_HEADER + b'unexpected')
        with self.assertRaisesRegex(AssertionError, 'budget'):
            probe.Capture()(b'x' * (probe.OUTPUT_LIMIT + 1))


if __name__ == '__main__':
    unittest.main()
