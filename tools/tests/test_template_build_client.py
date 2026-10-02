import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock

from tools.template_build_client import BuildClient, saved_intent


class BuildClientIntentTests(unittest.TestCase):
    def test_binding_is_shared_and_does_not_persist_the_credential(self):
        client = BuildClient('http://127.0.0.1:12345', 'synthetic-secret')
        binding = client.intent_binding()
        self.assertEqual(binding, {'origin': 'http://127.0.0.1:12345',
            'credential_sha256': hashlib.sha256(b'synthetic-secret').hexdigest()})
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'intent.json'
            intent = saved_intent(path, client, {'kind': 'dockerfile'}, build_mode='legacy-unbounded')
            self.assertEqual({key: intent[key] for key in binding}, binding)
            self.assertNotIn(client.key, path.read_text())

    def test_unbound_intent_is_preserved_without_a_request_or_migration(self):
        for mode in ('windowed', 'legacy-unbounded'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                client = BuildClient('http://127.0.0.1:1', 'synthetic-secret')
                client.request = Mock(return_value={'requestWindow': {'namespace': 17, 'sequence': 0}})
                path = Path(tmp) / 'intent.json'
                request = {'kind': 'dockerfile', 'timeoutSeconds': 30}
                intent = saved_intent(path, client, request, build_mode=mode)
                del intent['origin'], intent['credential_sha256']
                path.write_text(json.dumps(intent))
                original = path.read_bytes()
                client.request.reset_mock()
                with self.assertRaisesRegex(ValueError, 'saved build intent conflicts'):
                    saved_intent(path, client, request, build_mode=mode)
                client.request.assert_not_called()
                self.assertEqual(path.read_bytes(), original)
