import base64
import copy
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from tools import ci_dependencies as deps


ROOT = Path(__file__).resolve().parents[2]


class CiDependencyInputsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture = tempfile.TemporaryDirectory(prefix='ci-dependency-tests-')
        cls.checkout = Path(cls.fixture.name) / 'checkout'
        cls.checkout.mkdir()
        for path in deps.INPUT_PATHS:
            target = cls.checkout / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((ROOT / path).read_bytes())
        subprocess.run(['git', 'init', '-q', str(cls.checkout)], check=True)
        subprocess.run(['git', '-C', str(cls.checkout), 'add', '.'], check=True)
        subprocess.run(['git', '-C', str(cls.checkout), '-c', 'user.name=CI Fixture',
            '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture'], check=True)
        cls.commit = subprocess.check_output(['git', '-C', str(cls.checkout), 'rev-parse', 'HEAD'], text=True).strip()
        cls.bundle_path = Path(cls.fixture.name) / 'input.json'
        cls.bundle = deps.prepare_input(cls.checkout, cls.commit, cls.bundle_path)

    @classmethod
    def tearDownClass(cls):
        cls.fixture.cleanup()

    def damaged(self):
        return copy.deepcopy(self.bundle)

    def resign(self, bundle):
        bundle['input_sha256'] = deps.sha256(deps.canonical_json(bundle['manifest']))
        return bundle

    def replace_file(self, bundle, path, data):
        bundle['files'][path] = base64.b64encode(data).decode()
        bundle['manifest']['files'][path] = {'size': len(data), 'sha256': deps.sha256(data)}
        return self.resign(bundle)

    def test_approved_inputs_round_trip_without_checkout_secrets_or_artifacts(self):
        for path in ('private.key', '.cargo/credentials.toml', 'target/result', 'benchmarks/.venv/key'):
            destination = self.checkout / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text('never copied')
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'input.json'
            bundle = deps.prepare_input(self.checkout, self.commit, output)
            self.assertEqual(deps.load_input(output), bundle)
            self.assertEqual(set(bundle['files']), deps.INPUT_PATHS)
            self.assertEqual(bundle['manifest']['source_commit'], self.commit)
            self.assertNotIn(b'never copied', output.read_bytes())
            self.assertLess(output.stat().st_size, 1024 ** 2)
            for project in deps.PROJECTS:
                self.assertEqual(base64.b64decode(bundle['files'][project + '/uv.lock']),
                                 (ROOT / project / 'uv.lock').read_bytes())

    def test_internal_transport_is_fixed_manifest_metadata_not_repository_source(self):
        path = 'sandbox-internal/Cargo.toml'
        self.assertIn('sandbox-internal', deps.MEMBERS)
        self.assertIn(path, deps.INPUT_PATHS)
        files = deps.validate_input(self.bundle)
        self.assertEqual(files[path], (ROOT / path).read_bytes())
        self.assertNotIn('sandbox-internal/src/tls.rs', files)
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            deps.write_workspace(directory, files)
            self.assertEqual((directory / path).read_bytes(), files[path])
            self.assertEqual((directory / 'sandbox-internal/src/lib.rs').read_text(),
                             '// Dependency fetch metadata only.\n')
            self.assertFalse((directory / 'sandbox-internal/src/tls.rs').exists())
            self.assertEqual(deps.CARGO_ARGUMENTS,
                             ['fetch', '--locked', '--target', deps.PLATFORM['target']])

    def test_new_member_does_not_allow_unregistered_members_paths_or_git_sources(self):
        root = (ROOT / 'Cargo.toml').read_bytes()
        manifest = (ROOT / 'sandbox-internal/Cargo.toml').read_bytes()
        mutations = [
            ('Cargo.toml', root.replace(b'"sandbox-internal"', b'"unregistered-member"')),
            ('sandbox-internal/Cargo.toml', manifest.replace(
                b'path = "../sandbox-protocol"', b'path = "../../private"')),
            ('sandbox-internal/Cargo.toml', manifest.replace(
                b'path = "../sandbox-protocol"', b'path = "../unregistered-member"')),
            ('sandbox-internal/Cargo.toml', manifest +
                b'\n[dependencies.unregistered]\ngit = "https://example.invalid/code"\n'),
        ]
        for path, data in mutations:
            with self.subTest(path=path, data=data[-80:]), self.assertRaises(ValueError):
                deps.validate_input(self.replace_file(self.damaged(), path, data))
        for section in ('files', 'manifest'):
            bundle = self.damaged()
            files = bundle[section] if section == 'files' else bundle['manifest']['files']
            del files['sandbox-internal/Cargo.toml']
            with self.subTest(section=section), self.assertRaises(ValueError):
                deps.validate_input(self.resign(bundle))

    def test_center_store_fetches_only_registered_fixed_dependency_metadata(self):
        path = 'sandbox-center-store/Cargo.toml'
        self.assertIn('sandbox-center-store', deps.MEMBERS)
        files = deps.validate_input(self.bundle)
        self.assertEqual(files[path], (ROOT / path).read_bytes())
        self.assertNotIn('sandbox-center-store/src/store.rs', files)
        manifest = files[path]
        for data in (manifest.replace(b'../sandbox-protocol', b'../../private'),
                     manifest.replace(b'../sandbox-protocol', b'../unregistered-member'),
                     manifest.replace(b'name = "redis_ledger"', b'name = "../private"'),
                     manifest.replace(b'name = "redis_ledger"', b'name = "/private"'),
                     manifest.replace(b'name = "redis_ledger"', b'name = ""'),
                     manifest + b'\n[dependencies.unregistered]\ngit = "https://example.invalid/code"\n'):
            with self.subTest(data=data[-80:]), self.assertRaises(ValueError):
                deps.validate_input(self.replace_file(self.damaged(), path, data))
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            deps.write_workspace(directory, files)
            self.assertEqual((directory / path).read_bytes(), manifest)
            self.assertFalse((directory / 'sandbox-center-store/src/store.rs').exists())
            self.assertEqual((directory / 'sandbox-center-store/tests/redis_ledger.rs').read_text(),
                             '// Dependency fetch metadata only.\n')

    def test_center_core_registers_metadata_but_never_packages_source(self):
        path = 'sandbox-center-core/Cargo.toml'
        self.assertIn('sandbox-center-core', deps.MEMBERS)
        files = deps.validate_input(self.bundle)
        self.assertEqual(files[path], (ROOT / path).read_bytes())
        self.assertNotIn('sandbox-center-core/src/decision.rs', files)
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            deps.write_workspace(directory, files)
            self.assertEqual((directory / 'sandbox-center-core/src/lib.rs').read_text(),
                             '// Dependency fetch metadata only.\n')
            self.assertFalse((directory / 'sandbox-center-core/src/decision.rs').exists())

    def test_prepare_rejects_wrong_approval_dirty_blob_or_existing_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'input.json'
            for commit in ('main', 'a' * 40, self.commit[:12]):
                with self.subTest(commit=commit), self.assertRaises(ValueError):
                    deps.prepare_input(self.checkout, commit, output)
            target = self.checkout / 'Cargo.toml'
            original = target.read_bytes()
            try:
                target.write_bytes(original + b'\n# unreviewed\n')
                with self.assertRaises(ValueError):
                    deps.prepare_input(self.checkout, self.commit, output)
                self.assertFalse(output.exists())
            finally:
                target.write_bytes(original)
            output.write_text('preserve')
            with self.assertRaises(FileExistsError):
                deps.prepare_input(self.checkout, self.commit, output)
            self.assertEqual(output.read_text(), 'preserve')

    def test_prepare_rejects_linked_file_and_parent(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = self.checkout / 'Cargo.toml'
            original = target.read_bytes()
            linked = Path(temporary) / 'manifest'
            linked.write_bytes(original)
            try:
                target.unlink()
                target.symlink_to(linked)
                with self.assertRaises(ValueError):
                    deps.prepare_input(self.checkout, self.commit, Path(temporary) / 'input')
            finally:
                target.unlink()
                target.write_bytes(original)
            parent = self.checkout / 'tests'
            moved = Path(temporary) / 'tests'
            parent.rename(moved)
            try:
                parent.symlink_to(moved, target_is_directory=True)
                with self.assertRaises(ValueError):
                    deps.prepare_input(self.checkout, self.commit, Path(temporary) / 'input')
            finally:
                parent.unlink()
                moved.rename(parent)

    def test_manifest_cannot_change_recipe_platform_source_or_claimed_digest(self):
        for field, value in (('schema', True), ('source_commit', 'main'), ('platform', {'system': 'Linux'}),
                             ('recipe', {}), ('recipe_sha256', 'a' * 64)):
            bundle = self.damaged()
            bundle['manifest'][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                deps.validate_input(self.resign(bundle))
        bundle = self.damaged()
        bundle['input_sha256'] = 'a' * 64
        with self.assertRaises(ValueError):
            deps.validate_input(bundle)

    def test_missing_unknown_absolute_or_parent_paths_never_extract(self):
        for path in ('../key', '/root/key', 'tools/../Cargo.toml', '.git/config', 'target/output'):
            bundle = self.damaged()
            bundle['files'][path] = base64.b64encode(b'secret').decode()
            bundle['manifest']['files'][path] = {'size': 6, 'sha256': deps.sha256(b'secret')}
            with self.subTest(path=path), self.assertRaises(ValueError):
                deps.validate_input(self.resign(bundle))
        for section in ('files', 'manifest'):
            bundle = self.damaged()
            files = bundle[section] if section == 'files' else bundle['manifest']['files']
            del files['Cargo.lock']
            with self.subTest(section=section), self.assertRaises(ValueError):
                deps.validate_input(self.resign(bundle))

    def test_bad_encoding_size_hash_and_link_metadata_refuse(self):
        mutations = [lambda b: b['files'].__setitem__('Cargo.lock', '%%%'),
                     lambda b: b['manifest']['files']['Cargo.lock'].__setitem__('sha256', 'a' * 64),
                     lambda b: b['manifest']['files']['Cargo.lock'].__setitem__('size', True),
                     lambda b: b['manifest']['files']['Cargo.lock'].__setitem__('size', deps.MAX_FILE_BYTES + 1),
                     lambda b: b['manifest']['files']['Cargo.lock'].__setitem__('symlink', '/root/key')]
        for mutation in mutations:
            bundle = self.damaged()
            mutation(bundle)
            with self.assertRaises(ValueError):
                deps.validate_input(self.resign(bundle))
        with mock.patch.object(deps, 'MAX_PAYLOAD_BYTES', 1024):
            with self.assertRaises(ValueError):
                deps.validate_input(self.bundle)

    def test_load_rejects_oversize_duplicate_json_and_symlink(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'input'
            path.write_text('{"files":{},"files":{}}')
            with self.assertRaises(ValueError):
                deps.load_input(path)
            path.write_bytes(deps.canonical_json(self.bundle))
            with mock.patch.object(deps, 'MAX_INPUT_BYTES', 1024):
                with self.assertRaises(ValueError):
                    deps.load_input(path)
            link = Path(temporary) / 'link'
            link.symlink_to(path)
            with self.assertRaises((OSError, ValueError)):
                deps.load_input(link)
            fifo = Path(temporary) / 'fifo'
            os.mkfifo(fifo)
            with self.assertRaises(ValueError):
                deps.load_input(fifo)

    def test_corrupt_private_or_unlocked_sources_refuse_even_with_new_digest(self):
        mutations = [('rust-toolchain.toml', b'[toolchain]\nchannel="nightly"\n'),
                     ('Cargo.toml', b'[workspace]\nmembers=["../private"]\n'),
                     ('Cargo.lock', (ROOT / 'Cargo.lock').read_bytes().replace(
                         b'registry+https://github.com/rust-lang/crates.io-index', b'git+https://example.com/private')),
                     ('tests/compat/pyproject.toml', (ROOT / 'tests/compat/pyproject.toml').read_bytes().replace(
                         b'package = false', b'package = true')),
                     ('benchmarks/uv.lock', (ROOT / 'benchmarks/uv.lock').read_bytes().replace(
                         b'https://files.pythonhosted.org/', b'https://token:secret@files.pythonhosted.org/')),
                     ('tests/compat/uv.lock', (ROOT / 'tests/compat/uv.lock').read_bytes().replace(
                         b'packages%2Fpython-sdk', b'private'))]
        for path, data in mutations:
            with self.subTest(path=path), self.assertRaises(ValueError):
                deps.validate_input(self.replace_file(self.damaged(), path, data))

    def test_git_adapter_tamper_refuses_even_when_manifest_is_resigned(self):
        bundle = self.replace_file(self.damaged(), 'tools/ci_git.py', b'#!/usr/bin/env python3\nprint("changed")\n')
        with self.assertRaisesRegex(ValueError, 'controlled CI tool'):
            deps.validate_input(bundle)

    def test_warmup_runs_separate_locks_then_new_offline_workspaces_and_cleans_both(self):
        commands = []
        directories = []
        def run(arguments, **kwargs):
            if arguments[-1] == '--version':
                name = Path(arguments[0]).name
                version = {'uv': 'uv 0.11.31\n', 'cargo': 'cargo 1.97.0 (fixture)\n', 'rustc': 'rustc 1.97.0 (fixture)\n'}[name]
                return subprocess.CompletedProcess(arguments, 0, stdout=version)
            commands.append((arguments, kwargs))
            directory = kwargs['cwd']
            if directory not in directories:
                directories.append(directory)
                self.assertEqual((directory / 'git-bin/git').resolve(), ROOT / 'tools/ci_git.py')
            if arguments[0] == '/usr/local/bin/uv':
                project = Path(arguments[arguments.index('--project') + 1])
                self.assertEqual(kwargs['env']['CI_GIT_LOCK'], str(project / 'uv.lock'))
                (project / '.venv').mkdir()
                (project / '.venv' / 'temporary-installation').write_text('must disappear')
            (directory / 'target').mkdir(exist_ok=True)
            return subprocess.CompletedProcess(arguments, 0)
        output = io.StringIO()
        with mock.patch.object(deps, 'guest_environment', return_value={'PATH': '/usr/bin', 'HOME': '/home/runner'}), \
                mock.patch.object(deps.subprocess, 'run', side_effect=run), mock.patch('sys.stdout', output):
            self.assertEqual(deps.warmup(self.bundle_path), self.bundle['input_sha256'])
        self.assertEqual(len(directories), 2)
        self.assertNotEqual(directories[0], directories[1])
        self.assertTrue(all(not directory.exists() for directory in directories))
        syncs = [arguments for arguments, _ in commands if arguments[0] == '/usr/local/bin/uv']
        self.assertEqual(len(syncs), 4)
        self.assertTrue(all('--locked' in arguments and '--no-python-downloads' in arguments for arguments in syncs))
        self.assertTrue(all('--offline' not in arguments for arguments in syncs[:2]))
        self.assertTrue(all('--offline' in arguments for arguments in syncs[2:]))
        cargos = [arguments for arguments, _ in commands if arguments[0].endswith('/cargo')]
        self.assertEqual(cargos[0][1:], deps.CARGO_ARGUMENTS)
        self.assertEqual(cargos[1][1:], deps.CARGO_ARGUMENTS + ['--offline'])
        self.assertEqual(output.getvalue(), 'SANDPROOF_CI_DEPENDENCIES_READY ' + self.bundle['input_sha256'] + '\n')

    def test_failed_sync_or_bad_input_never_emits_ready(self):
        directories = []
        def fail(arguments, **kwargs):
            if arguments[-1] == '--version':
                versions = {'uv': 'uv 0.11.31', 'cargo': 'cargo 1.97.0', 'rustc': 'rustc 1.97.0'}
                return subprocess.CompletedProcess(arguments, 0, stdout=versions[Path(arguments[0]).name])
            directories.append(kwargs['cwd'])
            raise subprocess.CalledProcessError(1, arguments)
        output = io.StringIO()
        with mock.patch.object(deps, 'guest_environment', return_value={'PATH': '/usr/bin'}), \
                mock.patch.object(deps.subprocess, 'run', side_effect=fail), mock.patch('sys.stdout', output):
            self.assertEqual(deps.main(['warmup', '--input', str(self.bundle_path)]), 2)
        self.assertTrue(all(not directory.exists() for directory in directories))
        self.assertNotIn('READY', output.getvalue())

    def test_offline_provenance_or_cargo_failure_cleans_workspaces_and_never_ready(self):
        for stage in ('offline-sync', 'provenance', 'cargo'):
            directories = []
            def fail(arguments, **kwargs):
                if arguments[-1] == '--version':
                    versions = {'uv': 'uv 0.11.31', 'cargo': 'cargo 1.97.0', 'rustc': 'rustc 1.97.0'}
                    return subprocess.CompletedProcess(arguments, 0, stdout=versions[Path(arguments[0]).name])
                directories.append(kwargs['cwd'])
                refused = ((stage == 'offline-sync' and '--offline' in arguments and arguments[0].endswith('/uv'))
                           or (stage == 'provenance' and arguments[0].endswith('/python'))
                           or (stage == 'cargo' and arguments[0].endswith('/cargo')))
                if refused:
                    raise subprocess.CalledProcessError(1, arguments)
                return subprocess.CompletedProcess(arguments, 0)
            output = io.StringIO()
            with self.subTest(stage=stage), \
                    mock.patch.object(deps, 'guest_environment', return_value={'PATH': '/usr/bin'}), \
                    mock.patch.object(deps.subprocess, 'run', side_effect=fail), mock.patch('sys.stdout', output):
                with self.assertRaises(subprocess.CalledProcessError):
                    deps.warmup(self.bundle_path)
            self.assertTrue(all(not directory.exists() for directory in directories))
            self.assertNotIn('READY', output.getvalue())

    def test_wrong_installed_tool_version_never_emits_ready(self):
        output = io.StringIO()
        with mock.patch.object(deps, 'guest_environment', return_value={'PATH': '/usr/bin'}), \
                mock.patch.object(deps.subprocess, 'run', return_value=mock.Mock(stdout='uv 0.11.310\n')), \
                mock.patch('sys.stdout', output):
            with self.assertRaisesRegex(ValueError, 'tool version'):
                deps.warmup(self.bundle_path)
        self.assertNotIn('READY', output.getvalue())


class CiDependencyGuestTests(unittest.TestCase):
    def test_guest_rejects_root_wrong_account_platform_and_python(self):
        with mock.patch.object(deps.os, 'geteuid', return_value=0):
            with self.assertRaises(ValueError):
                deps.guest_environment()
        with mock.patch.object(deps.os, 'geteuid', return_value=1000), \
                mock.patch.object(deps.pwd, 'getpwuid', return_value=mock.Mock(pw_name='runner')), \
                mock.patch.object(deps.platform, 'system', return_value='Linux'), \
                mock.patch.object(deps.platform, 'machine', return_value='x86_64'), \
                mock.patch.object(deps.sys, 'version_info', (3, 12)), mock.patch.dict(os.environ, {'SECRET': 'never inherited'}):
            env = deps.guest_environment()
            self.assertNotIn('SECRET', env)
            self.assertEqual(env['RUSTUP_TOOLCHAIN'], '1.97.0')
            with mock.patch.object(deps.sys, 'version_info', (3, 13)):
                with self.assertRaises(ValueError):
                    deps.guest_environment()
            with mock.patch.object(deps.platform, 'machine', return_value='aarch64'):
                with self.assertRaises(ValueError):
                    deps.guest_environment()
            with mock.patch.object(deps.pwd, 'getpwuid', return_value=mock.Mock(pw_name='admin')):
                with self.assertRaises(ValueError):
                    deps.guest_environment()

    def test_provenance_check_accepts_original_source_and_rejects_changes(self):
        expected = {'e2b': {'version': '2.51.0', 'url': deps.PUBLIC_GIT['e2b'][0],
                           'commit_id': 'a' * 40, 'subdirectory': 'packages/python-sdk'}}
        direct = {'url': expected['e2b']['url'], 'subdirectory': expected['e2b']['subdirectory'],
                  'vcs_info': {'vcs': 'git', 'commit_id': 'a' * 40}}
        with tempfile.TemporaryDirectory() as temporary:
            installed = Path(temporary) / 'e2b-2.51.0.dist-info'
            installed.mkdir()
            (installed / 'METADATA').write_text('Name: e2b\nVersion: 2.51.0\n')
            script = 'import sys; sys.path.insert(0, sys.argv[2]);\n' + deps.PROVENANCE_CHECK
            args = [sys.executable, '-I', '-c', script, json.dumps(expected), temporary]
            (installed / 'direct_url.json').write_text(json.dumps(direct))
            self.assertEqual(subprocess.run(args, capture_output=True).returncode, 0)
            for field, value in (('url', 'https://example.com/e2b.git'), ('subdirectory', 'private'),
                                 ('vcs_info', {'vcs': 'git', 'commit_id': 'b' * 40})):
                bad = dict(direct, **{field: value})
                (installed / 'direct_url.json').write_text(json.dumps(bad))
                with self.subTest(field=field):
                    self.assertNotEqual(subprocess.run(args, capture_output=True).returncode, 0)
            (installed / 'direct_url.json').unlink()
            self.assertNotEqual(subprocess.run(args, capture_output=True).returncode, 0)


if __name__ == '__main__':
    unittest.main()
