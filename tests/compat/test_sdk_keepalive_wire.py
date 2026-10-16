"""Pinned SDK keepalive headers/events over loopback; no timer or envd process."""
from contextlib import ExitStack
import inspect
import threading
import unittest
from wsgiref.simple_server import make_server

from e2b import AsyncSandbox, Sandbox
from e2b.envd.process import process_connect, process_pb as pb
from e2b.sandbox.commands.command_handle import CommandExitException, PtySize
from protobuf import Oneof

from sdk_support import isolated_sdk_environment
from test_sdk_envd_wire import QuietHandler, SyntheticEnvd, SyntheticProcess, ThreadedEnvd


async def call(function, *args, **kwargs):
    result = function(*args, **kwargs)
    return await result if inspect.isawaitable(result) else result


class KeepaliveProcess(SyntheticProcess):
    """Insert prescribed wire messages around existing fixture data, without sleeps."""
    def __init__(self):
        super().__init__()
        self.pty = False
        self.keepalives = 0

    def events(self, response):
        for message in super().events(response):
            event = message.event.event
            if self.pty and event.field == 'data':
                event.value.output = Oneof('pty', event.value.output.value)
            yield message
            if event.field != 'end':
                for _ in range(2):
                    self.keepalives += 1
                    yield response(event=pb.ProcessEvent(event=Oneof('keepalive', pb.ProcessEvent.KeepAlive())))


class KeepaliveEnvd(SyntheticEnvd):
    def __init__(self):
        super().__init__()
        self.headers = []
        self.process = KeepaliveProcess()
        self.process_app = process_connect.ProcessWSGIApplication(self.process)

    def __call__(self, environ, start_response):
        self.headers.append((environ['PATH_INFO'], environ.get('HTTP_KEEPALIVE_PING_INTERVAL')))
        return super().__call__(environ, start_response)


class OriginalSdkKeepaliveTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())
        self.app = KeepaliveEnvd()
        self.server = make_server('127.0.0.1', 0, self.app,
                                 server_class=ThreadedEnvd, handler_class=QuietHandler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)

    def close_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
        self.assertFalse(self.thread.is_alive())

    async def connect(self, cls):
        url = f'http://127.0.0.1:{self.server.server_port}'
        return await call(cls.connect, 'probe-sandbox', api_url=url, sandbox_url=url,
                          api_key='synthetic-only', domain='example.invalid', retries=0,
                          request_timeout=15)

    async def stream(self, sandbox, asynchronous, operation, output):
        if operation == 'foreground':
            return await call(sandbox.commands.run, 'synthetic', timeout=15,
                              on_stdout=output.append)
        if operation == 'background':
            function, args = sandbox.commands.run, ('synthetic',)
            options = {'background': True}
        elif operation == 'connect':
            function, args, options = sandbox.commands.connect, (4321,), {}
        elif operation == 'pty_start':
            function, args, options = sandbox.pty.create, (PtySize(rows=24, cols=80),), {}
        else:
            assert operation == 'pty_connect'
            function, args, options = sandbox.pty.connect, (4321,), {}
        pty = operation.startswith('pty_')
        if asynchronous:
            options['on_data' if pty else 'on_stdout'] = output.append
        # A finite local transcript also permits checking the SDK's unlimited
        # Connect deadline without waiting on a real process or a real clock.
        timeout = 0 if operation in ('connect', 'pty_connect') else 15
        handle = await call(function, *args, timeout=timeout, **options)
        try:
            self.assertEqual(handle.pid, 4321)
            wait_options = {} if asynchronous else {'on_pty' if pty else 'on_stdout': output.append}
            return await call(handle.wait, **wait_options)
        finally:
            await call(handle.disconnect)

    def check_request(self, operation, before):
        method = 'Connect' if operation in ('connect', 'pty_connect') else 'Start'
        self.assertEqual(self.app.headers[before:], [(f'/process.Process/{method}', '50')])
        expected_timeout = None if method == 'Connect' else '15000'
        self.assertEqual(self.app.records[-1]['connect_timeout_ms'], expected_timeout)

    async def test_sync_async_command_and_pty_send_50_seconds_and_ignore_keepalive_payloads(self):
        payload = 'a中b'.encode()
        for cls in (Sandbox, AsyncSandbox):
            sandbox = await self.connect(cls)
            for operation in ('foreground', 'background', 'connect', 'pty_start', 'pty_connect'):
                with self.subTest(client=cls.__name__, operation=operation):
                    self.app.process.pty = operation.startswith('pty_')
                    # Split a UTF-8 codepoint across data messages with keepalives between.
                    self.app.process.stdout_chunks = [payload[:2], payload[2:]]
                    output, before = [], len(self.app.headers)
                    ticks = self.app.process.keepalives
                    result = await self.stream(sandbox, cls is AsyncSandbox, operation, output)
                    expected = '' if self.app.process.pty else payload.decode()
                    self.assertEqual((result.stdout, result.stderr, result.exit_code, result.error),
                                     (expected, '', 0, None))
                    self.assertEqual((b'' if self.app.process.pty else '').join(output),
                                     payload if self.app.process.pty else payload.decode())
                    self.check_request(operation, before)
                    self.assertEqual(self.app.process.keepalives - ticks, 6)

    async def test_keepalive_is_neither_end_nor_success_and_never_replays_start(self):
        for cls in (Sandbox, AsyncSandbox):
            sandbox = await self.connect(cls)
            for operation in ('foreground', 'background', 'connect', 'pty_start', 'pty_connect'):
                for failure in ('missing_end', 'nonzero'):
                    with self.subTest(client=cls.__name__, operation=operation, failure=failure):
                        self.app.process.pty = operation.startswith('pty_')
                        self.app.process.stdout_chunks = []
                        self.app.process.omit_end = failure == 'missing_end'
                        self.app.process.exit_code = 17 if failure == 'nonzero' else 0
                        output, before = [], len(self.app.headers)
                        error = Exception if failure == 'missing_end' else CommandExitException
                        with self.assertRaises(error) as raised:
                            await self.stream(sandbox, cls is AsyncSandbox, operation, output)
                        if failure == 'missing_end':
                            self.assertEqual(str(raised.exception), 'Command ended without an end event')
                        else:
                            self.assertEqual((raised.exception.stdout, raised.exception.stderr,
                                              raised.exception.exit_code, raised.exception.error),
                                             ('', '', 17, None))
                        self.check_request(operation, before)
                        self.assertEqual(output, [])
