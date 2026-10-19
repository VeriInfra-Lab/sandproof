"""Actual temporary bundle bytes; no binary execution, image or VM."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest

from tools import guest_artifacts as artifacts
from tools.system_acceptance import write_once

VERSIONS = dict(service_package_version='0.1.0', worker_package_version='0.1.0',
                config_version=3, worker_protocol_version=2, pid_format_version=2)
ROOT = Path(__file__).resolve().parents[2]


def manifest(directory):
    for name in ('source.tar', *artifacts.BINARIES.values()):
        path = directory / name
        path.write_bytes(('private bytes: ' + name).encode())
        path.chmod(0o500)
    fingerprint = artifacts.fingerprint(directory / 'source.tar', artifacts.SOURCE_LIMIT)
    return dict(schema_version=1, kind='rust-guest-v1', versions=dict(VERSIONS),
                source=dict(commit='a'*40, tree='b'*40, archive=fingerprint,
                            cargo_lock_sha256='c'*64, input_lock_sha256='d'*64, recipe_sha256='e'*64),
                build=dict(target=artifacts.TARGET, rustc=fingerprint, cargo=fingerprint,
                           rustc_version='rustc 1.97.0', cargo_version='cargo 1.97.0'),
                artifacts={role: artifacts.fingerprint(directory / name, artifacts.BINARY_LIMIT)
                           for role, name in artifacts.BINARIES.items()})


class GuestArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.root.chmod(0o700)
        self.manifest = manifest(self.root)
        self.path = self.root / 'manifest.json'

    def publish(self, raw=None):
        if raw is None:
            write_once(self.path, self.manifest)
        else:
            self.path.write_bytes(raw)
        return hashlib.sha256(self.path.read_bytes()).hexdigest()

    def test_separate_pin_binds_both_binaries_source_and_exact_worker_fragment(self):
        pin = self.publish()
        self.assertEqual(artifacts.load(self.root, pin), self.manifest)
        self.assertEqual(artifacts.worker_configuration(self.manifest), {
            'package_version': '0.1.0', 'protocol_version': 2,
            'artifact': self.manifest['artifacts']['worker']})
        with self.assertRaisesRegex(ValueError, 'trusted input lock'):
            artifacts.load(self.root, '0'*64)
        for name in ('source.tar', *artifacts.BINARIES.values()):
            path = self.root / name
            original = path.read_bytes()
            path.chmod(0o700)
            path.write_bytes(b'!' + original[1:])
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'changed'):
                artifacts.load(self.root, pin)
            path.write_bytes(original)
            path.chmod(0o500)

    def test_shape_rejects_missing_extra_old_boolean_and_foreign_bindings(self):
        for key in self.manifest:
            bad = copy.deepcopy(self.manifest)
            del bad[key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                artifacts.validate(bad)
        for path, value in [
            (('schema_version',), True), (('kind',), 'center-host-v1'),
            (('build', 'target'), 'aarch64-unknown-linux-gnu'),
            (('versions', 'worker_protocol_version'), True),
            (('versions', 'worker_package_version'), 'latest'),
            (('source', 'commit'), 'HEAD'), (('source', 'archive', 'bytes'), 0),
            (('artifacts', 'worker', 'sha256'), 'A'*64),
            (('artifacts', 'service', 'bytes'), artifacts.BINARY_LIMIT + 1),
            (('build', 'cargo_version'), 'x'*4097),
        ]:
            bad = copy.deepcopy(self.manifest)
            current = bad
            for key in path[:-1]: current = current[key]
            current[path[-1]] = value
            with self.subTest(path=path), self.assertRaises(ValueError): artifacts.validate(bad)
        bad = dict(self.manifest, extra=1)
        with self.assertRaises(ValueError): artifacts.validate(bad)

    def test_wrong_file_types_links_permissions_sizes_and_duplicate_keys_refuse(self):
        pin = self.publish()
        worker = self.root / artifacts.BINARIES['worker']
        worker.chmod(0o600)
        with self.assertRaises(ValueError): artifacts.load(self.root, pin)
        worker.chmod(0o500)
        alias = self.root / 'alias'
        os.link(worker, alias)
        with self.assertRaises(ValueError): artifacts.load(self.root, pin)
        alias.unlink()
        worker.rename(alias)
        worker.symlink_to(alias)
        with self.assertRaises((OSError, ValueError)): artifacts.load(self.root, pin)
        worker.unlink()
        worker.mkdir()
        with self.assertRaises(ValueError): artifacts.load(self.root, pin)
        with self.assertRaises(ValueError): artifacts.fingerprint(alias, 1)
        raw = json.dumps(self.manifest).replace('"schema_version": 1', '"schema_version": 1,"schema_version": 1').encode()
        self.path.write_bytes(raw)
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            artifacts.load(self.root, hashlib.sha256(raw).hexdigest())

    def test_report_pairs_reject_worker_service_or_input_lock_drift(self):
        common = dict(target_os='linux', target_arch='x86_64')
        service = dict(common, component='sandproof-guest-service', package_version='0.1.0',
                       **{key: value for key, value in VERSIONS.items() if key != 'service_package_version'})
        worker = dict(common, component='sandproof-guest-worker', package_version='0.1.0', worker_protocol_version=2)
        artifacts.check_reports(VERSIONS, service, worker)
        for report in (service, worker):
            for key in report:
                original = report[key]
                report[key] = True if type(original) is int else 'foreign'
                with self.subTest(key=key), self.assertRaises(ValueError): artifacts.check_reports(VERSIONS, service, worker)
                report[key] = original
        with self.assertRaises(ValueError): artifacts.check_reports(VERSIONS, dict(service, extra=1), worker)
        with self.assertRaises(ValueError): artifacts.check_reports(dict(VERSIONS, config_version=4), service, worker)

    def configuration(self):
        config = json.loads((ROOT / 'sandbox-guest-service/src/daemon/tests/configuration.json').read_bytes())
        config['worker'] = artifacts.worker_configuration(self.manifest)
        path = self.root / 'configuration.json'
        write_once(path, config)
        return path, config

    def test_shared_template_identity_binds_actual_config_release_and_binary_bytes(self):
        pin = self.publish()
        config, _ = self.configuration()
        config_pin = hashlib.sha256(config.read_bytes()).hexdigest()
        identity = artifacts.template_identity(self.root, pin, config, config_pin)
        fixture = json.loads((ROOT / 'tests/compat/guest-template-identity.json').read_bytes())
        self.assertEqual(identity, fixture)
        # No mutable configuration values or operator-local paths enter the
        # descriptor. Exact configuration bytes are still pinned separately.
        self.assertEqual(identity['configuration'], artifacts.fingerprint(config, artifacts.CONFIGURATION_LIMIT))
        with self.assertRaisesRegex(ValueError, 'configuration differs'):
            artifacts.template_identity(self.root, pin, config, '0'*64)
        config.write_bytes(config.read_bytes() + b' ')
        with self.assertRaisesRegex(ValueError, 'configuration differs'):
            artifacts.template_identity(self.root, pin, config, config_pin)
        config.chmod(0o644)
        with self.assertRaisesRegex(ValueError, 'owner-only'):
            artifacts.template_identity(self.root, pin, config, hashlib.sha256(config.read_bytes()).hexdigest())

    def test_template_identity_refuses_configuration_version_and_worker_mixing(self):
        pin = self.publish()
        path, config = self.configuration()
        for key, value in [
            (('version',), 1), (('version',), True),
            (('worker',), None), (('worker', 'protocol_version'), True),
            (('worker', 'protocol_version'), 1), (('worker', 'package_version'), '0.2.0'),
            (('worker', 'artifact', 'sha256'), 'e'*64), (('worker', 'artifact', 'bytes'), True),
            (('worker', 'extra'), 1),
        ]:
            bad = copy.deepcopy(config)
            current = bad
            for part in key[:-1]: current = current[part]
            current[key[-1]] = value
            path.write_text(json.dumps(bad))
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                artifacts.template_identity(self.root, pin, path, hashlib.sha256(path.read_bytes()).hexdigest())
        raw = json.dumps(config).replace('"version": 3', '"version": 3,"version": 3').encode()
        path.write_bytes(raw)
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            artifacts.template_identity(self.root, pin, path, hashlib.sha256(raw).hexdigest())
        path.write_bytes(b' ' * (artifacts.CONFIGURATION_LIMIT + 1))
        with self.assertRaisesRegex(ValueError, 'size'):
            artifacts.template_identity(self.root, pin, path, hashlib.sha256(path.read_bytes()).hexdigest())
