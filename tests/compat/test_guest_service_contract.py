"""Guest replacement fixtures checked with the actual locked SDK schema.

This is protocol input validation, not a running Rust/upstream guest service.
The same data is intended for the Rust protocol adapter's subsequent tests.
"""
import asyncio
import base64
from io import BytesIO
import json
from pathlib import Path
import threading
import unittest
from urllib.parse import parse_qs, urlsplit
from wsgiref.simple_server import make_server

from e2b import Sandbox
from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.connection_config import ConnectionConfig
from e2b.envd.client_async import create_rpc_client as async_client
from e2b.envd.client_sync import create_rpc_client as sync_client
from e2b.envd.filesystem import filesystem_pb
from e2b.envd.process import process_connect, process_pb
from e2b.envd.rpc import handle_rpc_exception
from e2b.exceptions import RateLimitException

from sdk_support import isolated_sdk_environment
from test_sdk_envd_wire import QuietHandler, SyntheticEnvd


ROOT = Path(__file__).resolve().parent
CONTRACT = json.loads((ROOT / 'guest-service-contract.json').read_text())


class GuestServiceProtocolContract(unittest.TestCase):
    def test_start_capacity_eos_reaches_original_sync_and_async_sdk_error_parser(self):
        sample = next(case for case in CONTRACT['start_refusals']
                      if case['id'] == 'pid-writer-capacity')
        payload = json.dumps(sample['end_stream'], separators=(',', ':')).encode()
        wire = b'\x02' + len(payload).to_bytes(4, 'big') + payload
        requests = []

        def app(environ, start_response):
            self.assertEqual(environ['PATH_INFO'], '/process.Process/Start')
            requests.append(environ['wsgi.input'].read(int(environ['CONTENT_LENGTH'])))
            start_response(f"{sample['status']} OK", [
                ('Content-Type', sample['content_type']), ('Content-Length', str(len(wire)))])
            return [wire]

        async def exercise(url):
            for asynchronous in (False, True):
                factory, kind = ((async_client, process_connect.ProcessClient) if asynchronous
                                 else (sync_client, process_connect.ProcessClientSync))
                client = factory(kind, url, ConnectionConfig())
                stream = client.start(process_pb.StartRequest.from_json(
                    '{"process":{"cmd":"/bin/true"}}'), timeout_ms=5000)
                try:
                    with self.assertRaises(ConnectError) as raised:
                        if asynchronous:
                            await anext(stream)
                        else:
                            next(stream)
                    expected = sample['end_stream']['error']
                    self.assertEqual(raised.exception.code, Code.RESOURCE_EXHAUSTED)
                    self.assertEqual(raised.exception.message, expected['message'])
                    self.assertIsInstance(handle_rpc_exception(raised.exception), RateLimitException)
                finally:
                    if asynchronous:
                        await stream.aclose()
                    else:
                        stream.close()

        with isolated_sdk_environment(), make_server(
                '127.0.0.1', 0, app, handler_class=QuietHandler) as server:
            thread = threading.Thread(target=server.serve_forever)
            thread.start()
            try:
                asyncio.run(exercise(f'http://127.0.0.1:{server.server_port}'))
                self.assertEqual(len(requests), 2, 'capacity rejection must not replay Start')
            finally:
                server.shutdown()
                thread.join(timeout=5)
                self.assertFalse(thread.is_alive())

    def test_sources_match_the_existing_sdk_contract(self):
        sdk = json.loads((ROOT / 'contract.json').read_text())
        self.assertEqual(CONTRACT['sdk']['version'], sdk['version'])
        self.assertEqual(CONTRACT['sdk']['revision'], sdk['git_revision'])
        self.assertIn(CONTRACT['runtime_revision'], sdk['upstream_sources']['runtime_lifetime'])

    def test_requests_and_responses_roundtrip_through_pinned_generated_schema(self):
        modules = {'process': process_pb, 'filesystem': filesystem_pb}
        for case in CONTRACT['rpcs']:
            with self.subTest(case=case['id']):
                schema = modules[case['service']]
                for suffix, documents in [('Request', [case['request']]), ('Response', case['responses'])]:
                    message_type = getattr(schema, case['method'] + suffix)
                    for document in documents:
                        message = message_type.from_json(json.dumps(document))
                        # Fixtures use canonical proto JSON so a misspelled or
                        # ignored field cannot silently disappear in both legs.
                        self.assertEqual(json.loads(message.to_json()), document)
                        wire = message.to_binary()
                        restored = message_type.from_binary(wire)
                        self.assertEqual(json.loads(restored.to_json()), json.loads(message.to_json()))
                        self.assertEqual(restored.to_binary(), wire)

    def test_binary_input_and_stdout_stderr_are_not_utf8_normalized(self):
        cases = {c['id']: c for c in CONTRACT['rpcs']}
        request = process_pb.SendInputRequest.from_json(json.dumps(cases['command-input']['request']))
        self.assertEqual(request.input.input.value, b'\x00\xffA\n')
        stream = [process_pb.StartResponse.from_json(json.dumps(x)) for x in cases['command-start']['responses']]
        self.assertEqual(stream[1].event.event.value.output.value, b'out')
        self.assertEqual(stream[2].event.event.value.output.value, b'err')
        self.assertEqual(stream[-1].event.event.value.exit_code, 17)
        for case in CONTRACT['http']:
            if 'body_base64' in case:
                self.assertEqual(base64.b64decode(case['body_base64'], validate=True), b'\x00\xffA\n')

    def test_pty_and_stdin_presence_are_explicit(self):
        cases = {c['id']: c for c in CONTRACT['rpcs']}
        pipes = process_pb.StartRequest.from_json(json.dumps(cases['command-start']['request']))
        pty = process_pb.StartRequest.from_json(json.dumps(cases['pty-start']['request']))
        self.assertTrue(pipes.has_field('stdin'))
        self.assertFalse(pipes.stdin)
        self.assertIsNone(pipes.pty)
        self.assertEqual((pty.pty.size.cols, pty.pty.size.rows), (80, 24))
        self.assertTrue(pty.stdin)

    def test_file_size_uses_int64_json_and_timestamp_schema(self):
        case = next(c for c in CONTRACT['rpcs'] if c['id'] == 'file-stat')
        entry = filesystem_pb.StatResponse.from_json(json.dumps(case['responses'][0])).entry
        self.assertEqual(entry.size, 4)
        self.assertEqual(entry.mode, 0o644)
        self.assertEqual(entry.owner, 'ubuntu')
        self.assertIsNotNone(entry.modified_time)

    def test_file_http_samples_match_fixed_sdk_requests(self):
        cases = {case['id']: case for case in CONTRACT['http']}
        upload, download = cases['file-upload'], cases['file-download']
        payload = base64.b64decode(upload['body_base64'], validate=True)
        app = SyntheticEnvd()
        # Preset the download response: the shared fixture is not a filesystem.
        app.file_contents['/tmp/sample.bin'] = base64.b64decode(
            download['body_base64'], validate=True)
        with isolated_sdk_environment(), make_server(
                '127.0.0.1', 0, app, handler_class=QuietHandler) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                url = f'http://127.0.0.1:{server.server_port}'
                sandbox = Sandbox.connect(
                    'probe-sandbox', api_url=url, sandbox_url=url,
                    api_key='synthetic-local-only', domain='example.invalid',
                    debug=False, request_timeout=5, retries=0)
                sandbox.files.write('/tmp/sample.bin', BytesIO(payload), user='ubuntu')
                self.assertEqual(app.records[-1]['body'], payload)
                self.assertEqual(app.records[-1]['content_type'], upload['content_type'])
                content = sandbox.files.read('/tmp/sample.bin', format='bytes', user='ubuntu')
                self.assertEqual(content, app.file_contents['/tmp/sample.bin'])
                for sample, actual in zip((upload, download), app.records[-2:], strict=True):
                    with self.subTest(sample=sample['id']):
                        expected = urlsplit(sample['path'])
                        self.assertEqual((actual['method'], actual['path']),
                                         (sample['method'], expected.path))
                        self.assertEqual(actual['query'], parse_qs(expected.query))
                        self.assertEqual(actual['access_token'], 'synthetic-token')
            finally:
                server.shutdown()
                thread.join(timeout=5)
                self.assertFalse(thread.is_alive())


if __name__ == '__main__':
    unittest.main()
