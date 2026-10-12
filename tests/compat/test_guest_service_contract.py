"""Guest replacement fixtures checked with the actual locked SDK schema.

This is protocol input validation, not a running Rust/upstream guest service.
The same data is intended for the Rust protocol adapter's subsequent tests.
"""
import base64
import json
from pathlib import Path
import unittest

from e2b.envd.filesystem import filesystem_pb
from e2b.envd.process import process_pb


ROOT = Path(__file__).resolve().parent
CONTRACT = json.loads((ROOT / 'guest-service-contract.json').read_text())


class GuestServiceProtocolContract(unittest.TestCase):
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


if __name__ == '__main__':
    unittest.main()
