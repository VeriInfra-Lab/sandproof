"""Offline preparation is not a VM execution or promotion attestation."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools import ci_e2e_artifacts as artifacts, ci_e2e_prepare as preparation


VERSIONS = {'internal_protocol': 2, 'center_store': 6, 'host_store': 16}


class Preparation(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.root.chmod(0o700)
        for name in ('checkout', 'cargo_home', 'uv_cache', 'rustup_home', 'cargo_target', 'toolchain_bin'):
            (self.root / name).mkdir(mode=0o700)
        self.target = self.root / 'cargo_target/release'
        self.target.mkdir(mode=0o700)
        self.config = dict(candidate='a' * 40, uv='/approved/uv', cargo='/approved/cargo',
                           candidate_manifest=str(self.root / 'candidate-artifacts.json'),
                           **{name: str(self.root / name) for name in (
                               'checkout', 'cargo_home', 'uv_cache', 'rustup_home',
                               'cargo_target', 'toolchain_bin')},
                           center=str(self.target / 'sandproof-center'),
                           host=str(self.target / 'sandproof-host'),
                           window_helper=str(self.target / 'sandproof-window-reducer'))
        self.calls = []

    def build(self, arguments, **kwargs):
        self.calls.append((arguments, kwargs))
        if '--bin' in arguments:
            binary = self.target / arguments[arguments.index('--bin') + 1]
            binary.write_bytes(b'approved same-candidate output: ' + binary.name.encode())
            binary.chmod(0o700)

    def prepare(self, *, git=None, build=None, versions=None):
        with patch.object(preparation, 'git', side_effect=git or ['a' * 40, '', 'a' * 40, '']), \
                patch.object(preparation.subprocess, 'run', side_effect=build or self.build), \
                patch.object(preparation, 'contract_versions', return_value=versions or VERSIONS), \
                patch.object(artifacts, 'trusted_path', side_effect=lambda path: path):
            return preparation.prepare(self.config)

    def test_exact_two_services_and_helper_build_offline_without_inherited_credentials(self):
        with patch.dict(os.environ, {'APP_PRIVATE_KEY': 'must-not-enter-candidate',
                                     'HTTPS_PROXY': 'http://unreviewed-proxy.invalid'}):
            manifest = self.prepare()
        self.assertEqual(set(manifest['artifacts']), {'center', 'host'})
        self.assertEqual(manifest['candidate'], 'a' * 40)
        self.assertEqual(manifest['versions'], VERSIONS)
        self.assertEqual(len(self.calls), 4)
        self.assertEqual(self.calls[0][0][:4], ['/approved/uv', 'sync', '--locked', '--offline'])
        self.assertIn('--no-python-downloads', self.calls[0][0])
        self.assertIn('--no-managed-python', self.calls[0][0])
        self.assertIn('--no-config', self.calls[0][0])
        self.assertEqual([args[-1] for args, _ in self.calls[1:]],
                         ['sandproof-center', 'sandproof-host', 'sandproof-window-reducer'])
        for arguments, kwargs in self.calls:
            self.assertIn('--locked', arguments)
            self.assertIn('--offline', arguments)
            self.assertEqual(kwargs['env']['CARGO_NET_OFFLINE'], 'true')
            self.assertNotIn('APP_PRIVATE_KEY', kwargs['env'])
            self.assertNotIn('HTTPS_PROXY', kwargs['env'])
        self.assertEqual(Path(self.config['candidate_manifest']).stat().st_mode & 0o777, 0o600)

    def test_missing_or_exposed_cache_fails_before_any_install_or_build(self):
        for name in ('cargo_home', 'uv_cache', 'rustup_home'):
            cache = Path(self.config[name])
            cache.chmod(0o755)
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.prepare()
            cache.chmod(0o700)
        self.assertEqual(self.calls, [])
        self.assertFalse(Path(self.config['candidate_manifest']).exists())

    def test_unrelated_single_server_or_mixed_output_is_never_sealed(self):
        self.config['host'] = str(self.target / 'sandproof-server')
        with self.assertRaisesRegex(ValueError, 'exact candidate build target'):
            self.prepare()
        self.assertEqual(self.calls, [])

    def test_fixed_cache_miss_never_retries_online_or_publishes_partial_artifacts(self):
        def fail(arguments, **kwargs):
            self.calls.append((arguments, kwargs))
            raise subprocess.CalledProcessError(1, arguments)
        with self.assertRaises(subprocess.CalledProcessError):
            self.prepare(build=fail)
        self.assertEqual(len(self.calls), 1)
        self.assertIn('--offline', self.calls[0][0])
        self.assertFalse(Path(self.config['candidate_manifest']).exists())

    def test_checkout_change_or_duplicate_original_receipt_does_not_publish_again(self):
        with self.assertRaisesRegex(ValueError, 'modified during preparation'):
            self.prepare(git=['a' * 40, '', 'a' * 40, ' M Cargo.lock'])
        self.assertFalse(Path(self.config['candidate_manifest']).exists())
        with self.assertRaisesRegex(ValueError, 'modified during preparation'):
            self.prepare(git=['a' * 40, '', 'b' * 40])
        self.assertFalse(Path(self.config['candidate_manifest']).exists())
        manifest = self.prepare()
        count = len(self.calls)
        with self.assertRaisesRegex(ValueError, 'receipt already exists'):
            self.prepare()
        self.assertEqual(len(self.calls), count)
        self.assertEqual(json.loads(Path(self.config['candidate_manifest']).read_text()), manifest)

    def test_missing_host_or_invalid_metadata_cannot_produce_a_dual_artifact_receipt(self):
        def incomplete(arguments, **kwargs):
            if arguments[-1] != 'sandproof-host':
                self.build(arguments, **kwargs)
        with self.assertRaises(FileNotFoundError):
            self.prepare(build=incomplete)
        self.assertFalse(Path(self.config['candidate_manifest']).exists())

    def test_both_actual_binaries_must_report_identical_contract_versions(self):
        with patch.object(preparation, 'git', side_effect=['a' * 40, '', 'a' * 40, '']), \
                patch.object(preparation.subprocess, 'run', side_effect=self.build), \
                patch.object(preparation, 'contract_versions', side_effect=[VERSIONS, dict(VERSIONS, host_store=14)]) as versions:
            with self.assertRaisesRegex(ValueError, 'versions differ'):
                preparation.prepare(self.config)
        self.assertEqual([call.args[0] for call in versions.call_args_list],
                         [self.config['center'], self.config['host']])
        self.assertFalse(Path(self.config['candidate_manifest']).exists())
        with self.assertRaises(ValueError):
            self.prepare(versions=dict(VERSIONS, host_store=True))
        self.assertFalse(Path(self.config['candidate_manifest']).exists())

    def test_metadata_is_bounded_and_nonzero_exit_is_not_a_version_response(self):
        binary = self.root / 'metadata-child'
        def run(text):
            binary.write_text('#!/bin/sh\n' + text)
            binary.chmod(0o700)
            return preparation.contract_versions(str(binary), {'PATH': '/usr/bin:/bin'})
        self.assertEqual(run("printf '%s' '" + json.dumps(VERSIONS) + "'\n"), VERSIONS)
        with self.assertRaisesRegex(ValueError, 'fixed size'):
            run("printf '%s' '" + 'x' * 1025 + "'\n")
        with self.assertRaisesRegex(ValueError, 'command failed'):
            run("printf '%s' '{}'\nexit 3\n")
        with self.assertRaisesRegex(ValueError, 'duplicate JSON key'):
            run("printf '%s' '{\"host_store\":15,\"host_store\":16}'\n")
        with patch.object(preparation.time, 'monotonic', side_effect=[0, 6]), \
                self.assertRaisesRegex(TimeoutError, 'metadata deadline'):
            run('exec /bin/sleep 30\n')


if __name__ == '__main__':
    unittest.main()
