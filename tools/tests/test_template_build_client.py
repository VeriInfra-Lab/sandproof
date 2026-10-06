import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock

from tools.template_build_client import BuildClient, request_window, saved_intent


class BuildClientIntentTests(unittest.TestCase):
    def client(self):
        client = BuildClient('http://127.0.0.1:1', 'synthetic-secret')
        client.request = Mock(return_value={'requestWindow': {'namespace': 17, 'sequence': 0}})
        return client

    def test_binding_is_shared_and_does_not_persist_the_credential(self):
        client = self.client()
        binding = client.intent_binding()
        self.assertEqual(binding, {'origin': 'http://127.0.0.1:1',
            'credential_sha256': hashlib.sha256(b'synthetic-secret').hexdigest()})
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'intent.json'
            intent = saved_intent(path, client, {'kind': 'dockerfile', 'timeoutSeconds': 30})
            self.assertEqual({key: intent[key] for key in binding}, binding)
            self.assertNotIn(client.key, path.read_text())
            client.request.assert_called_once_with('POST', '/sandproof/template-build-windows', {'durationMs': 30000})

    def test_legacy_unbound_or_malformed_intent_is_never_migrated(self):
        request = {'kind': 'dockerfile', 'timeoutSeconds': 30}
        for change in ('origin', 'credential_sha256', 'legacy', 'missing_window', 'invalid_window', 'empty_key'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as tmp:
                client = self.client()
                path = Path(tmp) / 'intent.json'
                intent = saved_intent(path, client, request)
                if change in ('origin', 'credential_sha256'):
                    del intent[change]
                elif change == 'legacy':
                    intent['build_mode'] = 'legacy-unbounded'
                elif change == 'missing_window':
                    del intent['requestWindow']
                elif change == 'invalid_window':
                    intent['requestWindow'] = {'namespace': 17, 'sequence': True}
                else:
                    intent['idempotency_key'] = ''
                path.write_text(json.dumps(intent))
                original = path.read_bytes()
                client.request.reset_mock()
                with self.assertRaises(ValueError):
                    saved_intent(path, client, request)
                client.request.assert_not_called()
                self.assertEqual(path.read_bytes(), original)

    def test_retry_reuses_original_window_and_key_without_any_request(self):
        client = self.client()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'intent.json'
            request = {'kind': 'oci', 'image': 'ubuntu:24.04', 'timeoutSeconds': 30}
            intent = saved_intent(path, client, request, token='original')
            original = path.read_bytes()
            client.request = Mock(side_effect=AssertionError('must not open a replacement window'))
            self.assertEqual(saved_intent(path, client, request), intent)
            client.request.assert_not_called()
            self.assertEqual(path.read_bytes(), original)

    def test_changed_origin_credential_request_or_key_preserves_original_intent(self):
        client = self.client()
        request = {'kind': 'oci', 'image': 'ubuntu:24.04', 'timeoutSeconds': 30}
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'intent.json'
            saved_intent(path, client, request, token='original')
            original = path.read_bytes()
            for other, changed, token in (
                (BuildClient('http://127.0.0.1:2', client.key), request, None),
                (BuildClient('http://127.0.0.1:1', 'another'), request, None),
                (client, request | {'image': 'debian:12'}, None),
                (client, request, 'replacement'),
            ):
                with self.subTest(binding=other.intent_binding(), token=token):
                    other.request = Mock(side_effect=AssertionError('must not dispatch'))
                    with self.assertRaisesRegex(ValueError, 'conflicts'):
                        saved_intent(path, other, changed, token=token)
                    other.request.assert_not_called()
                    self.assertEqual(path.read_bytes(), original)

    def test_failed_or_malformed_window_response_never_falls_back_to_submission(self):
        for result in (RuntimeError('BuildWindowsDisabled'), {'requestWindow': None},
                       {'requestWindow': {'namespace': 17, 'sequence': -1}}):
            with self.subTest(result=result), tempfile.TemporaryDirectory() as tmp:
                client = self.client()
                client.request = Mock(side_effect=result) if isinstance(result, Exception) else Mock(return_value=result)
                path = Path(tmp) / 'intent.json'
                with self.assertRaises((RuntimeError, ValueError)):
                    saved_intent(path, client, {'kind': 'oci', 'timeoutSeconds': 30})
                self.assertEqual(client.request.call_count, 1)
                self.assertEqual(client.request.call_args.args[1], '/sandproof/template-build-windows')
                self.assertFalse(path.exists())

    def test_dangling_intent_symlink_is_rejected_before_opening_a_window(self):
        client = self.client()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'intent.json'
            path.symlink_to(Path(tmp) / 'missing')
            with self.assertRaisesRegex(ValueError, 'invalid saved build intent'):
                saved_intent(path, client, {'kind': 'oci', 'timeoutSeconds': 30})
            client.request.assert_not_called()

    def test_window_shape_preserves_zero_and_maximum_but_rejects_nonidentities(self):
        for number in (0, 2**64 - 1):
            value = {'namespace': 17, 'sequence': number}
            self.assertEqual(request_window(value), value)
        for value in (None, {}, {'namespace': 17, 'sequence': -1},
                      {'namespace': 17, 'sequence': 2**64}, {'namespace': 17, 'sequence': True},
                      {'namespace': '17', 'sequence': 0}, {'namespace': 17, 'sequence': 0, 'other': 1}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                request_window(value)

    def test_cli_rejects_removed_mode_option_before_any_network_request(self):
        script = Path(__file__).resolve().parents[1] / 'template_build_client.py'
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'intent.json'
            result = subprocess.run([sys.executable, str(script), '--url', 'http://127.0.0.1:1',
                '--image', 'ubuntu:24.04', '--intent', str(path), '--build-mode', 'legacy-unbounded'],
                capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertIn('unrecognized arguments: --build-mode', result.stderr)
            self.assertFalse(path.exists())
