"""Real Git archive/files and bounded CLI fixtures, with controlled compilation.

These checks do not build Rust release binaries or attest a Linux guest image.
"""
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

from tools import guest_artifacts as artifacts, prepare_guest as preparation
from tools.tests.test_guest_artifacts import VERSIONS


def executable(path, text):
    path.write_text('#!/bin/sh\nprintf %s\\\\n ' + shlex.quote(text) + '\n')
    path.chmod(0o700)


class GuestPreparationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.root.chmod(0o700)
        self.checkout = self.root / 'checkout'
        for name in ('checkout', 'cargo_home', 'toolchain_bin'):
            (self.root / name).mkdir(mode=0o700)
        self.config = dict(candidate='a'*40, timeout_seconds=60, max_log_bytes=1024**2,
                           output=str(self.root / 'release'), **{name: str(self.root / name)
                           for name in ('checkout', 'cargo_home', 'toolchain_bin')})
        (self.checkout / 'tools').mkdir()
        for name in ('prepare_guest.py', 'guest_artifacts.py', '__init__.py'):
            (self.checkout / 'tools' / name).write_bytes((preparation.ROOT / 'tools' / name).read_bytes())
        (self.checkout / 'references').mkdir()
        self.lock = b'controlled Cargo lock input\n'
        (self.checkout / 'Cargo.lock').write_bytes(self.lock)
        (self.checkout / 'Cargo.toml').write_text('[workspace]\n')
        (self.checkout / 'main.rs').write_text('// committed source\n')
        (self.checkout / 'rust-toolchain.toml').write_text('[toolchain]\nchannel="1.97.0"\n')
        self.baseline = {'code_baseline': {'rust_toolchain': '1.97.0', 'rust_guest': dict(VERSIONS),
                                         'cargo_lock_sha256': hashlib.sha256(self.lock).hexdigest()}}
        self.save_baseline()
        self.git('init', '-q')
        self.commit()
        executable(self.root / 'toolchain_bin/rustc',
                   'rustc 1.97.0 (fixture)\nrelease: 1.97.0\nhost: x86_64-unknown-linux-gnu')
        executable(self.root / 'toolchain_bin/cargo', 'cargo 1.97.0 (fixture)')
        self.build_calls = []
        self.worker_protocol = 2

    def git(self, *args):
        return subprocess.check_output(['/usr/bin/git', '-C', str(self.checkout),
            '-c', 'core.hooksPath=/dev/null', '-c', 'user.name=Guest test',
            '-c', 'user.email=guest-test@example.invalid', *args], stderr=subprocess.PIPE,
            env=dict(os.environ, GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1'), timeout=10).decode().strip()

    def save_baseline(self):
        (self.checkout / 'references/v1-baseline.json').write_text(json.dumps(self.baseline))

    def commit(self):
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')
        self.config['candidate'] = self.git('rev-parse', 'HEAD')

    def controlled_compile(self, runner, args, **kwargs):
        self.build_calls.append((args, kwargs))
        self.assertEqual(Path(kwargs['cwd']), Path(self.config['output']) / 'source')
        target = Path(kwargs['env']['CARGO_TARGET_DIR']) / artifacts.TARGET / 'release'
        target.mkdir(parents=True)
        common = dict(target_os='linux', target_arch='x86_64')
        reports = {
            'service': dict(common, component='sandproof-guest-service', package_version='0.1.0',
                            **{key: value for key, value in VERSIONS.items() if key != 'service_package_version'}),
            'worker': dict(common, component='sandproof-guest-worker', package_version='0.1.0',
                           worker_protocol_version=self.worker_protocol),
        }
        for role, name in artifacts.BINARIES.items():
            executable(target / name, json.dumps(reports[role]))
            os.link(target / name, target / (name + '.cargo-alias'))

    def prepare(self, compile=None):
        with patch.object(preparation, 'ROOT', self.checkout), \
             patch.object(preparation.platform, 'system', return_value='Linux'), \
             patch.object(preparation.platform, 'machine', return_value='x86_64'), \
             patch.object(preparation.Commands, 'run', autospec=True,
                          side_effect=compile or self.controlled_compile):
            return preparation.prepare(self.config)

    def test_exact_archive_excludes_working_tree_changes_and_locks_both_actual_cli_outputs(self):
        (self.checkout / 'main.rs').write_text('// dirty source must not be built\n')
        (self.checkout / 'untracked.rs').write_text('// excluded\n')
        with patch.dict(os.environ, {'APP_PRIVATE_KEY': 'never-inherit', 'RUSTFLAGS': 'foreign',
                                     'HTTPS_PROXY': 'http://unapproved.invalid'}):
            receipt = self.prepare()
        output = Path(self.config['output'])
        manifest = artifacts.load(output, receipt['manifest_sha256'])
        self.assertEqual(manifest['source']['commit'], self.config['candidate'])
        self.assertEqual(manifest['source']['tree'], self.git('rev-parse', 'HEAD^{tree}'))
        self.assertEqual((output / 'source/main.rs').read_text(), '// committed source\n')
        self.assertFalse((output / 'source/untracked.rs').exists())
        self.assertEqual(receipt['worker'], artifacts.worker_configuration(manifest))
        self.assertEqual(len(self.build_calls), 1)
        args, kwargs = self.build_calls[0]
        self.assertEqual(args[1:], ['build', '--release', '--locked', '--offline', '--target', artifacts.TARGET,
                         '-p', 'sandproof-guest-service', '-p', 'sandproof-guest-runtime', '--bins'])
        self.assertEqual(kwargs['env']['CARGO_NET_OFFLINE'], 'true')
        self.assertEqual(kwargs['env']['CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS'], 'false')
        self.assertFalse({'APP_PRIVATE_KEY', 'RUSTFLAGS', 'HTTPS_PROXY'} & kwargs['env'].keys())
        self.assertEqual((output / 'manifest.json').stat().st_mode & 0o777, 0o600)
        for name in artifacts.BINARIES.values():
            self.assertEqual((output / name).stat().st_nlink, 1)
        with self.assertRaisesRegex(ValueError, 'new guest output'):
            self.prepare()
        self.assertEqual(len(self.build_calls), 1)

    def test_recipe_change_is_rejected_before_compilation(self):
        (self.checkout / 'tools/prepare_guest.py').write_text('# changed installed recipe\n')
        with self.assertRaisesRegex(ValueError, 'recipe differs'):
            self.prepare()
        self.assertEqual(self.build_calls, [])
        self.assertFalse((Path(self.config['output']) / 'manifest.json').exists())

    def test_wrong_lock_or_toolchain_and_non_linux_never_start_a_build(self):
        self.baseline['code_baseline']['cargo_lock_sha256'] = '0'*64
        self.save_baseline()
        self.commit()
        with self.assertRaisesRegex(ValueError, 'Cargo lock'):
            self.prepare()
        self.assertEqual(self.build_calls, [])
        self.config['output'] = str(self.root / 'other')
        with patch.object(preparation.platform, 'system', return_value='Darwin'), self.assertRaisesRegex(ValueError, 'Linux'):
            preparation.prepare(self.config)
        self.assertFalse((self.root / 'other').exists())

    def test_mixed_worker_version_has_no_manifest(self):
        self.worker_protocol = 1
        with self.assertRaisesRegex(ValueError, 'versions differ'):
            self.prepare()
        self.assertFalse((Path(self.config['output']) / 'manifest.json').exists())

    def test_wrong_native_toolchain_is_refused_before_compilation(self):
        executable(self.root / 'toolchain_bin/rustc',
                   'rustc 1.96.0 (fixture)\nrelease: 1.96.0\nhost: x86_64-unknown-linux-gnu')
        with self.assertRaisesRegex(ValueError, 'fixed Linux Rust/Cargo'):
            self.prepare()
        self.assertEqual(self.build_calls, [])
        self.assertFalse((Path(self.config['output']) / 'manifest.json').exists())

    def test_compile_failure_retains_private_run_never_retries_or_publishes(self):
        def fail(runner, args, **kwargs):
            self.build_calls.append((args, kwargs))
            raise RuntimeError('controlled offline cache miss')
        with self.assertRaisesRegex(RuntimeError, 'cache miss'):
            self.prepare(fail)
        self.assertEqual(len(self.build_calls), 1)
        output = Path(self.config['output'])
        self.assertTrue((output / 'source.tar').exists())
        self.assertFalse((output / 'manifest.json').exists())
        with self.assertRaisesRegex(ValueError, 'new guest output'):
            self.prepare()

    def test_expired_build_or_changed_compiler_cannot_publish(self):
        def expire(runner, args, **kwargs):
            self.controlled_compile(runner, args, **kwargs)
            runner.deadline = time.monotonic() - 1
        with self.assertRaises((TimeoutError, ValueError)):
            self.prepare(expire)
        self.assertFalse((Path(self.config['output']) / 'manifest.json').exists())
        self.config['output'] = str(self.root / 'changed-compiler')
        def change(runner, args, **kwargs):
            self.controlled_compile(runner, args, **kwargs)
            executable(self.root / 'toolchain_bin/cargo', 'changed')
        with self.assertRaisesRegex(ValueError, 'compiler changed'):
            self.prepare(change)
        self.assertFalse((Path(self.config['output']) / 'manifest.json').exists())

    def test_foreign_ref_existing_output_and_invalid_budgets_reject(self):
        for candidate in ('HEAD', 'main', 'a'*39, 'A'*40):
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                preparation.configuration(dict(self.config, candidate=candidate))
        for changes in ({'timeout_seconds': True}, {'max_log_bytes': 0},
                        {'output': str(self.checkout / 'nested')}, {'extra': 1}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                preparation.configuration(dict(self.config, **changes))
        (self.root / 'release').symlink_to(self.root / 'absent')
        with self.assertRaises(ValueError): preparation.configuration(self.config)
        self.assertEqual(self.build_calls, [])
