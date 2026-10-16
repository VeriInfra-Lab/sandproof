"""Original SDK handles driven by fixed tag transcripts; no Go/Rust VM."""
import asyncio
import copy
from collections import deque
from contextlib import ExitStack
import hashlib
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from wsgiref.simple_server import make_server
from unittest.mock import AsyncMock, Mock, patch

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b import AsyncSandbox, Sandbox
from e2b.connection_config import ConnectionConfig
from e2b.envd.process import process_connect, process_pb
from e2b.sandbox.commands.command_handle import CommandResult
from e2b.sandbox_sync.commands import command as sync_command
from e2b.sandbox_async.commands import command as async_command
from packaging.version import Version
from protobuf import Oneof
from sdk_support import isolated_sdk_environment
from test_sdk_envd_wire import QuietHandler, SyntheticEnvd, SyntheticProcess, ThreadedEnvd

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_guest_compare as paired
import sdk_tag_compare as probe
from sdk_observations import Calls

PARENT = '/tmp/shared/ubuntu'


class Transcript:
    def __init__(self, asynchronous, user, pid, fault):
        self.asynchronous, self.user, self.fault = asynchronous, user, fault
        self.tag = probe.TAGS[user]
        self.pids = dict(first=pid, second=pid + 1)
        self.definitions = probe.definitions(PARENT + '/tags', self.tag)
        self.calls, self.closed, self.started, self.signals = [], [], [], []
        self.done = {name: asyncio.Event() for name in probe.GENERATIONS}
        self.inputs = {name: bytearray() for name in probe.GENERATIONS}
        self.lists = deque([[], ['first'], ['first'], [], [], ['second'], ['second'], [], []])

    def start(self, request, **kwargs):
        name = probe.GENERATIONS[len(self.started)]
        self.started.append(name)
        self.calls.append(('start', request, kwargs))
        assert request.has_field('tag') and request.tag == self.tag
        if self.fault == 'unknown-start':
            raise TimeoutError('Start outcome unknown')
        return self.stream(process_pb.StartResponse, name, 'start')

    def connect(self, request, **kwargs):
        self.calls.append(('connect', request, kwargs))
        if request.process.selector.field == 'pid':
            assert request.process.selector == Oneof('pid', self.pids['first'])
            if self.fault == 'retired-success':
                return self.stream(process_pb.ConnectResponse, 'first', 'unexpected')
            raise ConnectError(Code.NOT_FOUND, f"process with pid {self.pids['first']} not found")
        assert request.process.selector == Oneof('tag', self.tag)
        name = self.started[-1]
        return self.stream(process_pb.ConnectResponse, name, 'late' if self.done[name].is_set() else 'resumed')

    def stream(self, response, name, label):
        def event(field, data):
            return response(event=process_pb.ProcessEvent(event=Oneof(field, data)))
        pid = self.pids[name] + (50 if self.fault == 'wrong-connect' and label == 'resumed' else 0)
        start = event('start', process_pb.ProcessEvent.StartEvent(pid=pid))

        def data(channel, raw):
            split = 3 if self.asynchronous else 1
            for chunk in (raw[:split], raw[split:]):
                yield event('data', process_pb.ProcessEvent.DataEvent(output=Oneof(channel, chunk)))

        def finish():
            if label != 'late' and label != 'unexpected':
                raw = f'{len(probe.INPUT)} {hashlib.sha256(probe.INPUT).hexdigest()}\n'.encode()
                yield from data('stdout', b'bad\n' if self.fault == 'output' else raw)
                yield from data('stderr', 'paired stderr 中文\n'.encode())
            if self.fault != 'missing-end':
                code = 17 if self.fault == 'wrong-end' else 0
                yield event('end', process_pb.ProcessEvent.EndEvent(
                    exit_code=code, exited=True, **({'error': 'exit status 17'} if code else {})))

        async def async_events():
            try:
                yield start
                if label == 'start':
                    for item in data('stdout', probe.READY):
                        yield item
                await asyncio.wait_for(self.done[name].wait(), 5)
                for item in finish():
                    yield item
            finally:
                self.closed.append((name, label))

        def sync_events():
            try:
                yield start
                if label == 'start':
                    yield from data('stdout', probe.READY)
                assert self.done[name].is_set(), 'terminal read must follow original input EOF'
                yield from finish()
            finally:
                self.closed.append((name, label))
        return async_events() if self.asynchronous else sync_events()

    def listing(self, request, **kwargs):
        self.calls.append(('list', request, kwargs))
        items = []
        for name in self.lists.popleft():
            metadata = copy.deepcopy(self.definitions[name]['metadata'])
            tag = metadata.pop('tag')
            items.append(process_pb.ProcessInfo(pid=self.pids[name], tag=tag,
                config=process_pb.ProcessConfig(**metadata)))
        if self.fault == 'tag-presence' and items:
            items[0].tag = None
        return process_pb.ListResponse(processes=items)

    def input(self, request, **kwargs):
        self.calls.append(('input', request, kwargs))
        assert request.process.selector == Oneof('tag', self.tag)
        if self.fault == 'unknown-input':
            raise TimeoutError('input outcome unknown')
        assert request.input.input.field == 'stdin'
        self.inputs[self.started[-1]].extend(request.input.input.value)
        return process_pb.SendInputResponse()

    def close(self, request, **kwargs):
        self.calls.append(('close', request, kwargs))
        assert request.process.selector == Oneof('tag', self.tag)
        self.done[self.started[-1]].set()
        return process_pb.CloseStdinResponse()

    def signal(self, request, **kwargs):
        self.calls.append(('signal', request, kwargs))
        if request.process.selector.field == 'tag':
            assert self.done[self.started[-1]].is_set()
            raise ConnectError(Code.NOT_FOUND, f'process with tag {self.tag} not found')
        pid = request.process.selector.value
        self.signals.append(pid)
        name = next(name for name, original in self.pids.items() if original == pid)
        self.done[name].set()
        return process_pb.SendSignalResponse()


class OriginalTagTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    def fixture(self, asynchronous, user='ubuntu', pid=4321, fault=None):
        backend = Transcript(asynchronous, user, pid, fault)
        wrap = AsyncMock if asynchronous else Mock
        rpc = SimpleNamespace(start=backend.start, connect=backend.connect,
            list=wrap(side_effect=backend.listing), send_input=wrap(side_effect=backend.input),
            close_stdin=wrap(side_effect=backend.close), send_signal=wrap(side_effect=backend.signal))
        module = async_command if asynchronous else sync_command
        with patch.object(module, 'create_rpc_client', return_value=rpc):
            commands = module.Commands('http://example.invalid', ConnectionConfig(api_key='synthetic', retries=0),
                                       Version('0.9.0'), Mock())
        commands.run = wrap(return_value=CommandResult(stdout='', stderr='', exit_code=0, error=None))
        sandbox = SimpleNamespace(commands=commands, files=SimpleNamespace(read=wrap(return_value='once\n')))
        return Calls(sandbox, asynchronous), backend

    async def test_original_sdk_empty_and_special_tags_reconnect_reuse_and_compare_exact_fields(self):
        for user in probe.TAGS:
            rows = []
            for asynchronous in (False, True):
                observed, backend = self.fixture(asynchronous, user, 4321 + 100 * len(rows))
                rows.append(await probe.tag_case(observed, user, PARENT))
                self.assertEqual(backend.started, ['first', 'second'])
                self.assertEqual(backend.signals, [])
                self.assertEqual({key: bytes(data) for key, data in backend.inputs.items()},
                                 dict(first=probe.INPUT, second=probe.INPUT))
                self.assertEqual(len(backend.closed), 6)
                self.assertFalse(backend.lists)
                for method, request, kwargs in backend.calls:
                    if method == 'start':
                        self.assertEqual(kwargs['timeout_ms'], 60000)
                        self.assertEqual(request.tag, probe.TAGS[user])
                    if method != 'list':
                        self.assertEqual(kwargs['headers']['Authorization'],
                                         'Basic cm9vdDo=' if user == 'root' else 'Basic dWJ1bnR1Og==')
            saved = copy.deepcopy(rows)
            paired.compare([rows[0]], [rows[1]], 'original SDK tag generations')
            self.assertEqual(rows, saved)

    async def test_unknown_start_input_and_wrong_connect_never_retry_or_target_unowned_pid(self):
        for asynchronous in (False, True):
            for fault, starts, signals in [('unknown-start', 1, []), ('unknown-input', 1, [4321]),
                                           ('wrong-connect', 1, [4321]), ('tag-presence', 1, [4321])]:
                observed, backend = self.fixture(asynchronous, fault=fault)
                with self.subTest(asynchronous=asynchronous, fault=fault), self.assertRaises(Exception):
                    await probe.tag_case(observed, 'ubuntu', PARENT)
                self.assertEqual(len(backend.started), starts)
                self.assertEqual(backend.signals, signals)
                self.assertEqual(observed.rows[0]['status'], 'incomplete')
                self.assertLessEqual(sum(method == 'input' for method, *_ in backend.calls), 1)

    async def test_real_end_prevents_cleanup_signal_but_truncation_keeps_original_responsibility(self):
        for asynchronous in (False, True):
            for fault, signals, starts in [('output', [], 1), ('wrong-end', [], 1),
                                           ('missing-end', [4321], 1), ('retired-success', [4322], 2)]:
                observed, backend = self.fixture(asynchronous, fault=fault)
                with self.subTest(asynchronous=asynchronous, fault=fault), self.assertRaises(Exception):
                    await probe.tag_case(observed, 'ubuntu', PARENT)
                self.assertEqual(backend.signals, signals)
                self.assertEqual(len(backend.started), starts)
                if fault == 'retired-success':
                    self.assertIn(('first', 'unexpected'), backend.closed)

    async def test_comparison_rejects_old_pid_alias_and_metadata_errors_without_rewriting_data(self):
        observed, _ = self.fixture(False)
        row = await probe.tag_case(observed, 'ubuntu', PARENT)
        mutations = [lambda r: r['pids'].__setitem__('second', r['pids']['first']),
            lambda r: r['subscriptions'][1].__setitem__('pid', 999),
            lambda r: r['calls'].__delitem__(next(i for i,c in enumerate(r['calls']) if c['method'] == 'process.start')),
            lambda r: r['observations'][1]['calls'][-1].__setitem__('result', [{'unexpected': True}]),
            lambda r: next(c for c in r['calls'] if c['method'] == 'commands.list')['result'][0].__setitem__('tag', None)]
        for mutate in mutations:
            changed = copy.deepcopy(row)
            mutate(changed)
            with self.assertRaises(AssertionError):
                paired.compare([row], [changed], 'damaged tag evidence')


class TagProgramTests(unittest.TestCase):
    def test_actual_private_program_waits_for_input_and_records_only_once(self):
        with tempfile.TemporaryDirectory() as directory:
            child = subprocess.Popen(['/bin/bash', '--noprofile', '--norc', '-c', probe.program(directory)],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                self.assertTrue(select.select([child.stdout], [], [], 10)[0])
                self.assertEqual(child.stdout.read(len(probe.READY)), probe.READY)
                self.assertIsNone(child.poll())
                output, error = child.communicate(probe.INPUT, timeout=10)
                self.assertEqual(child.returncode, 0)
                self.assertEqual(output, f'{len(probe.INPUT)} {hashlib.sha256(probe.INPUT).hexdigest()}\n'.encode())
                self.assertEqual(error, 'paired stderr 中文\n'.encode())
                self.assertEqual(Path(directory, 'started').read_bytes(), b'once\n')
            finally:
                if child.poll() is None:
                    child.kill()
                child.communicate(timeout=5)


class RejectedTagProcess(SyntheticProcess):
    def connect(self, request, _context):
        assert request.process.selector.field == 'tag'
        raise ConnectError(Code.NOT_FOUND, f'process with tag {request.process.selector.value} not found')

    def send_signal(self, request, _context):
        assert request.process.selector.field == 'tag'
        raise ConnectError(Code.NOT_FOUND, f'process with tag {request.process.selector.value} not found')


class TagWireTests(unittest.IsolatedAsyncioTestCase):
    async def test_original_generated_clients_preserve_tag_wire_and_parse_real_loopback_errors(self):
        with isolated_sdk_environment():
            app = SyntheticEnvd()
            app.process = RejectedTagProcess()
            app.process_app = process_connect.ProcessWSGIApplication(app.process)
            server = make_server('127.0.0.1', 0, app, server_class=ThreadedEnvd, handler_class=QuietHandler)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                url = f'http://127.0.0.1:{server.server_port}'
                for cls in (Sandbox, AsyncSandbox):
                    sandbox = await paired.call(cls.connect, 'probe-sandbox', api_url=url, sandbox_url=url,
                        api_key='synthetic-only', domain='example.invalid', retries=0, request_timeout=15)
                    calls = Calls(sandbox, cls is AsyncSandbox)
                    rpc = probe.TagRpc(calls, 'ubuntu')
                    for tag in probe.TAGS.values():
                        capture = probe.Capture(False, False)
                        handle = await rpc.stream('start', {'tag': tag, 'stdin': True, 'process': {'cmd': '/bin/true'}},
                            capture.callbacks(), generation='first', label='start')
                        try:
                            result = await paired.call(handle.wait, **({} if cls is AsyncSandbox else capture.callbacks()))
                            self.assertEqual((result.stdout, result.stderr, result.exit_code), ('synthetic\n', '', 0))
                        finally:
                            await paired.call(handle.disconnect)
                        failure = await rpc.stream('connect', {'process': {'tag': tag}}, {},
                            generation='first', label='missing', seconds=20, expected=ConnectError)
                        probe.not_found(failure, 'tag', tag)
                        failure = await rpc.unary('send_signal', {'process': {'tag': tag}},
                                                  generation='first', expected=ConnectError)
                        probe.not_found(failure, 'tag', tag)
                self.assertEqual([start.tag for start in app.process.starts], list(probe.TAGS.values()) * 2)
                self.assertTrue(all(start.has_field('tag') for start in app.process.starts))
                for record in app.records:
                    if record['path'].startswith('/process.Process/'):
                        self.assertEqual(record['authorization'], 'Basic dWJ1bnR1Og==')
                        self.assertEqual(record['access_token'], 'synthetic-token')
                        self.assertEqual(record['connect_timeout_ms'],
                            '60000' if record['path'].endswith('/Start') else
                            '20000' if record['path'].endswith('/Connect') else '15000')
            finally:
                await asyncio.to_thread(server.shutdown)
                server.server_close()
                thread.join(timeout=5)
                self.assertFalse(thread.is_alive())
