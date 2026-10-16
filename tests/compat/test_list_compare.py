"""Original SDK multi-handle List recording with prescribed RPC observations."""
import asyncio
import copy
from collections import deque
from contextlib import ExitStack
import errno
import os
from pathlib import Path
import pty
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
from e2b.sandbox.commands.main import ProcessInfo
from e2b.sandbox_sync.commands import command as sync_command, pty as sync_pty
from e2b.sandbox_async.commands import command as async_command, pty as async_pty
from packaging.version import Version
from protobuf import Oneof
from sdk_support import isolated_sdk_environment

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_list_compare as probe
import sdk_guest_compare as paired
from sdk_observations import Calls, value

INPUTS = dict(directory='/tmp/shared', users=['root', 'ubuntu'], mtime=1_700_000_000)
DIRECTORY = INPUTS['directory'] + '/list'


class PrivateProgramTests(unittest.TestCase):
    def test_exact_program_waits_for_release_records_once_and_exits_with_real_pipe_or_pty(self):
        for name, code in (('root', 0), ('ubuntu', 17), ('pty', -9)):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temp:
                directory = str(Path(temp).resolve())
                master = slave = child = None
                try:
                    command = probe.program(directory, name, max(code, 0))
                    if name == 'pty':
                        master, slave = pty.openpty()
                        child = subprocess.Popen(['/bin/bash', '--noprofile', '--norc', '-c', command],
                                                 stdin=slave, stdout=slave, stderr=slave, close_fds=True)
                        os.close(slave)
                        slave = None
                        reader = master
                    else:
                        child = subprocess.Popen(['/bin/bash', '--noprofile', '--norc', '-c', command],
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                        reader = child.stdout.fileno()
                    data = b''
                    while len(data) < len(probe.READY):
                        self.assertTrue(select.select([reader], [], [], 10)[0])
                        chunk = os.read(reader, 4096)
                        self.assertTrue(chunk)
                        data += chunk
                    self.assertEqual(data, probe.READY)
                    self.assertIsNone(child.poll())
                    self.assertEqual(Path(directory, 'started-' + name).read_bytes(), b'once\n')
                    if name == 'pty':
                        child.kill()
                        child.wait(10)
                        self.assertTrue(select.select([reader], [], [], 10)[0])
                        try:
                            self.assertEqual(os.read(reader, 4096), b'')
                        except OSError as error:
                            self.assertEqual(error.errno, errno.EIO)
                    else:
                        output, error = child.communicate(b'X', timeout=10)
                        self.assertEqual((output, error), (b'', b''))
                    self.assertEqual(child.returncode, code)
                finally:
                    if child is not None:
                        if child.poll() is None:
                            child.kill()
                        child.communicate(timeout=5)
                    for descriptor in (master, slave):
                        if descriptor is not None:
                            os.close(descriptor)


class ListedRpc:
    """A fixed event/List transcript with explicit release barriers, not a registry."""
    def __init__(self, asynchronous, pids, fault=None):
        self.asynchronous, self.pids, self.fault = asynchronous, pids, fault
        self.definitions = probe.specs(DIRECTORY)
        self.calls, self.closed = [], []
        self.done = {name: False for name in probe.NAMES}
        self.events = {name: asyncio.Event() for name in probe.NAMES}
        self.bootstrapped = asyncio.Event()
        self.started = []
        # Setup quiescence, two held snapshots, then each End's delayed removal
        # and two complete snapshots (including after retained Connect).
        self.lists = deque([['setup'], [], list(probe.NAMES), list(probe.NAMES),
            list(probe.NAMES), ['root', 'ubuntu'], ['root', 'ubuntu'], ['root', 'ubuntu'],
            ['root', 'ubuntu'], ['ubuntu'], ['ubuntu'], ['ubuntu'], ['ubuntu'], [], [], []])
        self.list_count = 0

    def info(self, name):
        if name == 'setup':
            return process_pb.ProcessInfo(pid=90123, config=process_pb.ProcessConfig(cmd='completed setup'))
        metadata = copy.deepcopy(self.definitions[name]['metadata'])
        metadata.pop('tag')
        if metadata['cwd'] is None:
            metadata.pop('cwd')
        return process_pb.ProcessInfo(pid=self.pids[name], config=process_pb.ProcessConfig(**metadata))

    def list(self, request, **kwargs):
        self.calls.append(('list', request, kwargs))
        names = self.lists.popleft()
        self.list_count += 1
        items = [self.info(name) for name in names]
        if self.asynchronous:
            items.reverse()
        if self.list_count == 3:
            if self.fault == 'extra':
                items.append(self.info('setup'))
            elif self.fault == 'missing':
                items.pop()
            elif self.fault == 'duplicate':
                items.append(items[0])
            elif self.fault == 'metadata':
                items[0].config.envs['UNEXPECTED'] = '4321'
        if self.list_count == 5 and self.fault == 'missing-survivor':
            items = [self.info('ubuntu'), self.info('pty')]
        if self.list_count == 7 and self.fault == 'resurrection':
            items.append(self.info('pty'))
        return process_pb.ListResponse(processes=items)

    def start(self, request, **kwargs):
        name = probe.NAMES[len(self.started)]
        self.started.append(name)
        self.calls.append(('start', request, kwargs))
        if self.fault == 'unknown-start' and name == 'ubuntu':
            raise TimeoutError('original Start outcome unknown')
        return self.stream(process_pb.StartResponse, name, True)

    def connect(self, request, **kwargs):
        name = next(name for name, pid in self.pids.items() if request.process.selector == Oneof('pid', pid))
        self.calls.append(('connect', request, kwargs))
        return self.stream(process_pb.ConnectResponse, name, False)

    def event(self, response, field, data):
        return response(event=process_pb.ProcessEvent(event=Oneof(field, data)))

    def stream(self, response, name, start):
        pid = self.pids[name] + (1 if self.fault == 'wrong-connect' and not start else 0)
        first = self.event(response, 'start', process_pb.ProcessEvent.StartEvent(pid=pid))
        def ready():
            data = (b'controlled shell prompt\r\n' if name == 'pty' else b'') + probe.READY
            if self.fault == 'bad-ready' and name == 'root':
                data = b'bad ready'
            channel = 'pty' if name == 'pty' else 'stdout'
            for chunk in (data[:3], data[3:]):
                yield self.event(response, 'data', process_pb.ProcessEvent.DataEvent(output=Oneof(channel, chunk)))
        def end():
            code = self.definitions[name]['code']
            if self.fault == 'wrong-end' and name == 'root':
                code = 17
            if self.fault == 'missing-end' and name == 'root':
                return
            fields = dict(exit_code=code, exited=True)
            if code:
                fields['error'] = 'signal: killed' if code == -1 else 'exit status 17'
            if self.fault == 'history' and not start and self.done[name]:
                yield self.event(response, 'data', process_pb.ProcessEvent.DataEvent(output=Oneof('stdout', b'history')))
            yield self.event(response, 'end', process_pb.ProcessEvent.EndEvent(**fields))
        async def asynchronous():
            try:
                yield first
                if start:
                    if name == 'pty':
                        await asyncio.wait_for(self.bootstrapped.wait(), 5)
                    for item in ready():
                        yield item
                await asyncio.wait_for(self.events[name].wait(), 5)
                for item in end():
                    yield item
            finally:
                self.closed.append((name, start))
        def synchronous():
            try:
                yield first
                if start:
                    yield from ready()
                assert self.done[name], 'terminal consumer must follow input or kill'
                yield from end()
            finally:
                self.closed.append((name, start))
        return asynchronous() if self.asynchronous else synchronous()

    def finish(self, name):
        self.done[name] = True
        self.events[name].set()

    def input(self, request, **kwargs):
        self.calls.append(('input', request, kwargs))
        name = next(name for name, pid in self.pids.items() if request.process.selector == Oneof('pid', pid))
        data = request.input.input
        if name == 'pty':
            assert data == Oneof('pty', (self.definitions[name]['command'] + '\n').encode())
            self.bootstrapped.set()
        else:
            assert data == Oneof('stdin', b'X')
            if self.fault == 'unknown-input' and name == 'root':
                raise TimeoutError('input outcome unknown')
            self.finish(name)
        return process_pb.SendInputResponse()

    def signal(self, request, **kwargs):
        self.calls.append(('kill', request, kwargs))
        name = next(name for name, pid in self.pids.items() if request.process.selector == Oneof('pid', pid))
        assert request.signal == process_pb.Signal.SIGKILL
        if self.fault == 'unknown-kill' and name == 'pty':
            raise TimeoutError('kill outcome unknown')
        self.finish(name)
        return process_pb.SendSignalResponse()


class OriginalListTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    def fixture(self, asynchronous, pids=None, fault=None):
        pids = pids or dict(root=4321, ubuntu=4322, pty=4323)
        backend = ListedRpc(asynchronous, pids, fault)
        wrap = AsyncMock if asynchronous else Mock
        rpc = SimpleNamespace(start=backend.start, connect=backend.connect,
            list=wrap(side_effect=backend.list), send_input=wrap(side_effect=backend.input),
            send_signal=wrap(side_effect=backend.signal))
        config = ConnectionConfig(api_key='synthetic', retries=0)
        command_module, pty_module = (async_command, async_pty) if asynchronous else (sync_command, sync_pty)
        with patch.object(command_module, 'create_rpc_client', return_value=rpc), \
             patch.object(pty_module, 'create_rpc_client', return_value=rpc):
            commands = command_module.Commands('http://example.invalid', config, Version('0.9.0'), Mock())
            terminal = pty_module.Pty('http://example.invalid', config, Version('0.9.0'), Mock())
        run = commands.run
        def foreground(command, **kwargs):
            if kwargs.get('background'):
                return run(command, **kwargs)
            assert command == 'mkdir -m 0777 ' + DIRECTORY
            return CommandResult(stdout='', stderr='', exit_code=0, error=None)
        async def asynchronous_run(*args, **kwargs):
            return await paired.call(foreground, *args, **kwargs)
        commands.run = wrap(side_effect=asynchronous_run if asynchronous else foreground)
        sandbox = SimpleNamespace(commands=commands, pty=terminal,
                                  files=SimpleNamespace(read=wrap(return_value='once\n')))
        return Calls(sandbox, asynchronous), backend

    async def run_case(self, asynchronous, pids=None):
        calls, backend = self.fixture(asynchronous, pids)
        await probe.list_case(calls, INPUTS)
        return calls.rows[0], backend

    async def test_both_original_sdks_compare_all_fields_and_entries_across_order_and_pid_changes(self):
        rows = []
        for asynchronous, pids in ((False, dict(root=4321, ubuntu=4322, pty=4323)),
                                   (True, dict(root=18123, ubuntu=99, pty=717))):
            row, backend = await self.run_case(asynchronous, pids)
            rows.append(row)
            self.assertEqual(backend.started, list(probe.NAMES))
            self.assertFalse(backend.lists)
            self.assertEqual(len(backend.closed), 7)
            starts = [item for item in backend.calls if item[0] == 'start']
            for name, (_, request, kwargs) in zip(probe.NAMES, starts):
                spec = probe.specs(DIRECTORY)[name]
                actual = value(ProcessInfo(pid=pids[name], tag=None, cmd=request.process.cmd,
                    args=list(request.process.args), envs=dict(request.process.envs),
                    cwd=request.process.cwd if request.process.has_field('cwd') else None))
                self.assertEqual(actual, dict(pid=pids[name], **spec['metadata']))
                self.assertEqual(kwargs['timeout_ms'], 60000)
                self.assertEqual(kwargs['headers']['Authorization'],
                                 'Basic cm9vdDo=' if name == 'root' else 'Basic dWJ1bnR1Og==')
            self.assertEqual([r[1].process.selector for r in backend.calls if r[0] == 'kill'],
                             [Oneof('pid', pids['pty'])])
            snapshots = [c for c in row['calls'] if c['method'] == 'commands.list']
            self.assertEqual([len(c['result']) for c in snapshots], [3, 3, 2, 2, 1, 1, 0, 0])
        saved = copy.deepcopy(rows)
        paired.compare([rows[0]], [rows[1]], 'complete List')
        self.assertEqual(rows, saved)

    async def test_missing_duplicate_extra_metadata_survivor_and_resurrection_are_rejected(self):
        for asynchronous in (False, True):
            for fault in ('missing', 'duplicate', 'extra', 'metadata', 'missing-survivor', 'resurrection'):
                calls, backend = self.fixture(asynchronous, fault=fault)
                with self.subTest(asynchronous=asynchronous, fault=fault), self.assertRaises(AssertionError):
                    await probe.list_case(calls, INPUTS)
                self.assertEqual(calls.rows[0]['status'], 'incomplete')
                self.assertEqual(backend.started, list(probe.NAMES))
                self.assertEqual(len(backend.closed), 3 if backend.list_count == 3 else 4)
                kills = [r[1].process.selector.value for r in backend.calls if r[0] == 'kill']
                self.assertEqual(set(kills), set(backend.pids.values()))
                self.assertEqual(len(kills), 3)

    async def test_unknown_start_input_signal_and_wrong_connect_never_replay_or_signal_wrong_pid(self):
        for asynchronous in (False, True):
            for fault in ('unknown-start', 'unknown-input', 'unknown-kill', 'wrong-connect'):
                calls, backend = self.fixture(asynchronous, fault=fault)
                with self.subTest(asynchronous=asynchronous, fault=fault), self.assertRaises(Exception):
                    await probe.list_case(calls, INPUTS)
                self.assertEqual(len(backend.started), 2 if fault == 'unknown-start' else 3)
                kills = [r[1].process.selector.value for r in backend.calls if r[0] == 'kill']
                self.assertEqual(set(kills), {backend.pids['root']} if fault == 'unknown-start' else set(backend.pids.values()))
                self.assertEqual(len(kills), len(set(kills)))
                self.assertEqual(calls.rows[0]['status'], 'incomplete')
                if fault == 'unknown-input':
                    sent = [r for r in backend.calls if r[0] == 'input' and r[1].process.selector.value == backend.pids['root']]
                    self.assertEqual(len(sent), 1)

    async def test_wrong_end_keeps_actual_terminal_without_signalling_the_finished_command(self):
        for asynchronous in (False, True):
            calls, backend = self.fixture(asynchronous, fault='wrong-end')
            with self.assertRaises(Exception):
                await probe.list_case(calls, INPUTS)
            kills = [r[1].process.selector.value for r in backend.calls if r[0] == 'kill']
            self.assertEqual(kills, [backend.pids['pty'], backend.pids['ubuntu']])
            terminal = next(c for c in calls.rows[0]['calls'] if c['method'] == 'handle.wait' and c['name'] == 'root')
            self.assertEqual(terminal['error']['fields']['exit_code'], 17)
            self.assertEqual(terminal['status'], 'incomplete')

    async def test_bad_readiness_missing_end_and_late_history_preserve_partial_rows_and_do_not_restart(self):
        for asynchronous in (False, True):
            for fault in ('bad-ready', 'missing-end', 'history'):
                calls, backend = self.fixture(asynchronous, fault=fault)
                with self.subTest(asynchronous=asynchronous, fault=fault), self.assertRaises(Exception):
                    await probe.list_case(calls, INPUTS)
                self.assertEqual(backend.started, ['root'] if fault == 'bad-ready' else list(probe.NAMES))
                self.assertEqual(calls.rows[0]['status'], 'incomplete')
                kills = [r[1].process.selector.value for r in backend.calls if r[0] == 'kill']
                self.assertEqual(len(kills), 1 if fault == 'bad-ready' else 3)
                self.assertEqual(len(kills), len(set(kills)))
                self.assertTrue(calls.rows[0]['subscriptions'][0]['channels']['stdout'])

    async def test_primary_failure_and_all_unknown_disconnects_are_retained_without_retries(self):
        original_operation = Calls.operation
        for asynchronous in (False, True):
            calls, backend = self.fixture(asynchronous, fault='wrong-end')
            attempted = []
            async def operation(instance, method, target, *args, **kwargs):
                if method == 'handle.disconnect':
                    label = kwargs['evidence']['subscription']
                    attempted.append(label)
                    if label in ('pty-late', 'root-start'):
                        original_target = target
                        async def unknown_close():
                            await paired.call(original_target)
                            raise RuntimeError('unknown disconnect ' + label)
                        target = unknown_close
                return await original_operation(instance, method, target, *args, **kwargs)
            with patch.object(Calls, 'operation', operation), self.assertRaises(BaseExceptionGroup) as raised:
                await probe.list_case(calls, INPUTS)
            self.assertEqual(len(raised.exception.exceptions), 3)
            self.assertEqual(attempted, ['pty-late', 'ubuntu-connected', 'pty-start', 'ubuntu-start', 'root-start'])
            self.assertEqual(len(backend.closed), 5)
            incomplete = [r for r in calls.rows[0]['calls'] if r['method'] == 'handle.disconnect'
                          and r['status'] == 'incomplete']
            self.assertEqual([r['error']['message'] for r in incomplete],
                             ['unknown disconnect pty-late', 'unknown disconnect root-start'])

    async def test_comparator_only_normalizes_valid_pid_references_list_order_and_sampling(self):
        original, _ = await self.run_case(False)
        changed = copy.deepcopy(original)
        for observation in changed['observations']:
            observation['calls'].insert(0, copy.deepcopy(observation['calls'][0]))
        paired.compare([original], [changed], 'different removal sampling count')
        mutations = [lambda r: r['pids'].__setitem__('root', True),
            lambda r: r['subscriptions'][0].__setitem__('pid', 77),
            lambda r: next(c for c in r['calls'] if c['method'] == 'commands.connect')['args'].__setitem__(0, 77),
            lambda r: next(c for c in r['calls'] if c['method'] == 'commands.connect')['result'].__setitem__('pid', 77),
            lambda r: next(c for c in r['calls'] if c['method'] == 'handle.wait').__setitem__('handle_pid', 77),
            lambda r: next(c for c in r['calls'] if c['method'] == 'commands.list')['result'].pop(),
            lambda r: next(c for c in r['calls'] if c['method'] == 'commands.list')['result'][0].__setitem__('tag', ''),
            lambda r: next(c for c in r['calls'] if c['method'] == 'commands.list')['result'][0]['envs'].__setitem__('PID', '4321'),
            lambda r: next(c for c in r['calls'] if c['method'] == 'commands.list')['result'][0].__setitem__('cwd', ''),
            lambda r: r['observations'][1]['calls'][0]['result'].pop(0),
            lambda r: r['observations'][1]['calls'][-1]['kwargs'].__setitem__('request_timeout', 0),
            lambda r: r['observations'][1]['calls'][-1].__setitem__('status', 'incomplete'),
            lambda r: r['observations'][0]['calls'].reverse(),
            lambda r: r['subscriptions'][0]['channels']['stdout'].append('4321'),
            lambda r: next(c for c in r['calls'] if c['method'] == 'commands.send_stdin')['args'].__setitem__(1, value(b'Y'))]
        for mutation in mutations:
            other = copy.deepcopy(original)
            mutation(other)
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                paired.compare([original], [other], 'changed List evidence')


class ObservationTests(unittest.IsolatedAsyncioTestCase):
    async def test_bounded_observation_rejects_deadline_count_invalid_rows_and_missing_survivors(self):
        definitions = probe.specs(DIRECTORY)
        pids = dict(root=1, ubuntu=2, pty=3)
        entries = [ProcessInfo(pid=pids[name], **definitions[name]['metadata']) for name in probe.NAMES]
        now = [0]
        async def pause(seconds):
            now[0] += seconds
        for fault in ('deadline', 'count', 'duplicate', 'missing'):
            evidence, now[0] = {}, 0
            async def listed(**_kwargs):
                if fault == 'deadline':
                    now[0] = probe.SECONDS
                return entries + [entries[0]] if fault == 'duplicate' else entries[1:] if fault == 'missing' else entries
            calls = Calls(SimpleNamespace(commands=SimpleNamespace(list=AsyncMock(side_effect=listed))), True)
            with self.subTest(fault=fault), patch.object(probe, 'ROUNDS', 2), self.assertRaises(AssertionError):
                await probe.observe_removal(calls, evidence, ['root', 'ubuntu'], pids, definitions,
                    retiring='pty', clock=lambda: now[0], pause=pause)
            self.assertEqual(evidence['status'], 'incomplete')
            self.assertTrue(evidence['calls'])
            self.assertLessEqual(len(evidence['calls']), 2)


if __name__ == '__main__':
    unittest.main()
