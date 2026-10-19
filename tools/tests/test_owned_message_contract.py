"""Reject source, use-point, library and proof-inventory drift (not Rust proof)."""
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from tools.check_owned_message_contract import ROOT, check, digest


class OwnedMessageAdmissionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name) / 'source'
        self.cargo = Path(temporary.name) / 'cargo'
        self.environment = json.loads((ROOT / 'verification/owned-message-environment.json').read_text())
        for name in self.environment['inputs']:
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, destination)
        # Only these unit tests use synthetic library files. Production
        # admission checks locked archives and actual reviewed source hashes.
        lock = 'version = 4\n'
        for name, library in self.environment['libraries'].items():
            registry = self.cargo / f"registry/src/unit-test/{name}-{library['version']}"
            for filename in library['reviewed_sources']:
                path = registry / filename
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('// synthetic admission fixture only\n')
                library['reviewed_sources'][filename] = digest(path)
            archive = self.cargo / f"registry/cache/unit-test/{name}-{library['version']}.crate"
            archive.parent.mkdir(parents=True, exist_ok=True)
            archive.write_bytes(b'synthetic locked archive')
            lock += f'[[package]]\nname = "{name}"\nversion = "{library["version"]}"\nchecksum = "{digest(archive)}"\n'
        (self.root / 'Cargo.lock').write_text(lock)
        mocked = patch('tools.check_owned_message_contract.check_b1',
                       return_value={'tokio': {'version': '1.49.0'}})
        mocked.start()
        self.addCleanup(mocked.stop)
        self.seal()

    def seal(self):
        (self.root / 'verification/owned-message-environment.json').write_text(json.dumps(self.environment))
        (self.root / 'verification/owned-message-inputs.sha256').write_text(''.join(
            digest(self.root / name) + '  ' + name + '\n' for name in self.environment['inputs']))

    def test_original_registration_admitted(self):
        self.assertEqual(len(check(self.root, self.cargo)['use_points']), 11)

    def test_changed_output_capture_rejected(self):
        path = self.root / 'sandbox-guest-service/src/command/body.rs'
        path.write_text(path.read_text().replace('            output,', '            output: None,'))
        with self.assertRaisesRegex(ValueError, 'Frozen owned-message input changed'):
            check(self.root, self.cargo)

    def test_missing_raw_input_rejected(self):
        path = self.root / 'verification/owned-message-inputs.sha256'
        path.write_text('\n'.join(path.read_text().splitlines()[:-1]) + '\n')
        with self.assertRaisesRegex(ValueError, 'Missing or duplicate'):
            check(self.root, self.cargo)

    def test_changed_library_source_rejected(self):
        library = self.environment['libraries']['bytes']
        path = self.cargo / f"registry/src/unit-test/bytes-{library['version']}/src/bytes.rs"
        path.write_text('// changed library\n')
        with self.assertRaisesRegex(ValueError, 'Reviewed message library source changed'):
            check(self.root, self.cargo)

    def test_changed_locked_archive_rejected(self):
        library = self.environment['libraries']['bytes']
        path = self.cargo / f"registry/cache/unit-test/bytes-{library['version']}.crate"
        path.write_bytes(b'changed archive')
        with self.assertRaisesRegex(ValueError, 'Locked message library archive changed'):
            check(self.root, self.cargo)

    def test_source_anchor_change_rejected_after_resealing(self):
        path = self.root / 'sandbox-guest-service/src/command/streams.rs'
        path.write_text(path.read_text().replace('subscriber.data.try_send(output)', 'other_sender.try_send(output)'))
        self.seal()
        with self.assertRaisesRegex(ValueError, 'Reviewed message call anchor changed'):
            check(self.root, self.cargo)

    def test_missing_audit_rejected_after_resealing(self):
        path = self.root / 'SandboxModel/Boundaries/OwnedMessage/Audit.lean'
        path.write_text('\n'.join(path.read_text().splitlines()[:-1]) + '\n')
        self.seal()
        with self.assertRaisesRegex(ValueError, 'theorem audit'):
            check(self.root, self.cargo)

    def test_unknown_public_result_rejected(self):
        self.environment['use_points'][0]['public_results'] = ['Invented.result']
        self.seal()
        with self.assertRaisesRegex(ValueError, 'Unknown public result'):
            check(self.root, self.cargo)

    def test_missing_business_provider_rejected(self):
        self.environment['use_points'][0]['caller_obligations'] = ''
        self.seal()
        with self.assertRaisesRegex(ValueError, 'business provider'):
            check(self.root, self.cargo)

    def test_relabelled_as_extracted_async_rust_rejected(self):
        self.environment['proof_kind'] = 'extracted-async-rust'
        self.seal()
        with self.assertRaisesRegex(ValueError, 'not extracted async Rust'):
            check(self.root, self.cargo)
