"""Fixed SDK data probe lifetimes under prescribed RPC events; no product VM."""
from contextlib import ExitStack
import ast
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import AsyncMock, Mock, patch

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.envd.process import process_pb as pb
from e2b.sandbox_sync.commands.command_handle import CommandHandle
from e2b.sandbox_async.commands.command_handle import AsyncCommandHandle
from protobuf import Oneof
from sdk_support import isolated_sdk_environment
from test_command_probe_cleanup import Transcript, end

import sdk_data_contract as probe


class DataProbeTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    def test_c02_bound_probe_sources_have_no_dependency_on_unbound_probes(self):
        root = Path(__file__).resolve().parents[2]
        tree = ast.parse((root / 'tests/system/sdk_c02_acceptance.py').read_text())
        sources = next(ast.literal_eval(node.value) for node in tree.body
                       if isinstance(node, ast.Assign)
                       and any(isinstance(target, ast.Name) and target.id == 'SOURCE_FILES'
                               for target in node.targets))
        with tempfile.TemporaryDirectory() as directory:
            for key in ('assertions_guard', 'sdk_server_smoke.py', 'sdk_data_contract.py', 'sdk_data_samples.py'):
                source = root / sources[key]
                shutil.copyfile(source, Path(directory) / source.name)
            # Enter command_cases too: a lazy import outside the bound set must fail.
            result = subprocess.run([sys.executable, '-I', '-c', '''
import asyncio, sys
from types import SimpleNamespace
sys.path.insert(0, sys.argv[1])
import sdk_data_contract as probe
def unavailable(*args, **kwargs):
    raise RuntimeError('controlled Start failure')
try:
    asyncio.run(probe.command_cases(SimpleNamespace(commands=SimpleNamespace(run=unavailable)),
                                   '/unused', 'ubuntu', asynchronous=False))
except RuntimeError as error:
    assert str(error) == 'controlled Start failure'
else:
    raise AssertionError('unexpected Start success')
''', directory], capture_output=True, text=True, timeout=15, cwd=directory)
            self.assertEqual(result.returncode, 0, result.stderr)

    def fixture(self, asynchronous, fault=None):
        ready = [('data', pb.ProcessEvent.DataEvent(output=Oneof('stdout', b'ready\n'))), None]
        starts = [(4321, ready), (4322, end('')), (4323, end('', code=-1, error='signal: killed'))]
        terminal = end('wrong' if fault == 'output' else 'done\n',
                       code=17 if fault == 'exit' else 0,
                       error='unexpected' if fault == 'error' else None)
        if fault == 'missing-end':
            terminal.pop()
        connects = [(9999 if fault == 'wrong-pid' else 4321, terminal)]
        if fault == 'unknown-start':
            starts[0] = ConnectError(Code.DEADLINE_EXCEEDED, '')
        elif fault == 'unknown-release':
            starts[1] = ConnectError(Code.DEADLINE_EXCEEDED, '')
        elif fault == 'unknown-kill-start':
            starts[2] = ConnectError(Code.DEADLINE_EXCEEDED, '')
        elif fault == 'unknown-connect':
            connects[0] = ConnectError(Code.DEADLINE_EXCEEDED, '')
        elif fault == 'kill-success-end':
            starts[2] = (4323, end(''))
        elif fault == 'kill-missing-end':
            starts[2] = (4323, [])
        tape = Transcript(asynchronous, starts, connects)
        if fault == 'marker':
            tape.sandbox.files.read.side_effect = ValueError('marker unavailable')
        if fault in ('unknown-kill', 'false-kill'):
            wrap = AsyncMock if asynchronous else Mock
            signal = tape.signal
            def failure(request, **kwargs):
                signal(request, **kwargs)
                raise ConnectError(Code.NOT_FOUND if fault == 'false-kill' else Code.DEADLINE_EXCEEDED, '')
            tape.sandbox.commands._rpc.send_signal = wrap(side_effect=failure)
        return tape

    async def invoke(self, tape):
        return await probe.command_cases(tape.sandbox, '/tmp/data-probe', 'ubuntu',
                                        asynchronous=tape.asynchronous)

    async def test_success_closes_all_subscriptions_and_only_signals_the_kill_case(self):
        for asynchronous in (False, True):
            tape = self.fixture(asynchronous)
            row = await self.invoke(tape)
            self.assertEqual(row, dict(reconnected_pid=4321, killed_pid=4323))
            self.assertEqual(tape.signals(), [Oneof('pid', 4323)])
            self.assertEqual(len(tape.closed), 4)
            self.assertEqual(len(set(tape.closed)), 4)
            self.assertEqual([name for name, _, _ in tape.calls],
                             ['start', 'connect', 'start', 'start', 'signal'])
            starts = [request.process.args[-1] for name, request, _ in tape.calls if name == 'start']
            self.assertEqual(starts, [probe.barrier_command('/tmp/data-probe'),
                                     "printf 'release\\n' > /tmp/data-probe/release.fifo", 'sleep 60'])
            for name, request, kwargs in tape.calls:
                if name == 'signal':
                    self.assertEqual(kwargs['timeout_ms'], 15000)
                elif name == 'connect':
                    self.assertEqual(request.process.selector, Oneof('pid', 4321))

    async def test_post_end_validation_failures_never_send_cleanup_signal(self):
        for asynchronous in (False, True):
            for fault in ('output', 'exit', 'error', 'marker'):
                with self.subTest(asynchronous=asynchronous, fault=fault):
                    tape = self.fixture(asynchronous, fault)
                    with self.assertRaises(Exception):
                        await self.invoke(tape)
                    self.assertEqual(tape.signals(), [])
                    self.assertEqual(len(tape.closed), 3)
                    self.assertEqual(len(tape.starts), 1)

    async def test_unknown_results_never_repeat_start_or_control_a_connect_reply(self):
        cases = [('unknown-start', 1, [], 1), ('unknown-connect', 1, [4321], 2),
                 ('wrong-pid', 1, [4321], 2), ('missing-end', 2, [4321], 3),
                 ('unknown-release', 2, [4321], 3), ('unknown-kill-start', 3, [], 4)]
        for asynchronous in (False, True):
            for fault, starts, signals, closed in cases:
                with self.subTest(asynchronous=asynchronous, fault=fault):
                    tape = self.fixture(asynchronous, fault)
                    with self.assertRaises(Exception):
                        await self.invoke(tape)
                    self.assertEqual(sum(name == 'start' for name, _, _ in tape.calls), starts)
                    self.assertEqual(tape.signals(), [Oneof('pid', pid) for pid in signals])
                    self.assertEqual(len(tape.closed), closed)

    async def test_explicit_kill_failure_and_missing_end_do_not_repeat_kill(self):
        for asynchronous in (False, True):
            for fault in ('unknown-kill', 'false-kill', 'kill-success-end', 'kill-missing-end'):
                with self.subTest(asynchronous=asynchronous, fault=fault):
                    tape = self.fixture(asynchronous, fault)
                    with self.assertRaises(Exception):
                        await self.invoke(tape)
                    self.assertEqual(tape.signals(), [Oneof('pid', 4323)])
                    self.assertEqual(len(tape.closed), 4)

    async def test_close_failures_preserve_primary_failure_without_post_end_signal(self):
        for asynchronous in (False, True):
            tape = self.fixture(asynchronous, 'marker')
            cls = AsyncCommandHandle if asynchronous else CommandHandle
            original = cls.disconnect
            calls = []
            def fail(handle):
                calls.append(handle)
                if handle.pid == 4321 and sum(item.pid == 4321 for item in calls) == 2:
                    raise RuntimeError('subscription close unknown')
            async def async_close(handle):
                await original(handle)
                fail(handle)
            def sync_close(handle):
                original(handle)
                fail(handle)
            with patch.object(cls, 'disconnect', async_close if asynchronous else sync_close):
                with self.assertRaises(BaseExceptionGroup) as caught:
                    await self.invoke(tape)
            self.assertEqual([type(error) for error in caught.exception.exceptions], [ValueError, RuntimeError])
            self.assertEqual(tape.signals(), [])
            self.assertEqual(len(calls), len({id(handle) for handle in calls}))
            self.assertEqual(len(tape.closed), 3)
