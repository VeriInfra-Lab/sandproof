"""Scoring-entry integrity tests; no VM, task execution, or dependency download."""
import ast
import hashlib
from importlib.metadata import PackageNotFoundError
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from benchmarks import ci_verifier as variant
from tools.benchmark_tasks import InvalidInput


class CiVerifierTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.source = self.root / 'arbitrary-task'
        self.source.mkdir()
        for path, value in {
            'instruction.md': 'Frozen instruction\n',
            'task.toml': '[agent]\ntimeout_sec=900\n[verifier]\ntimeout_sec=900\n',
            'environment/Dockerfile': 'FROM fixture:fixed\n',
            'solution/solve.sh': '#!/bin/sh\nprintf original-solution\n',
            'tests/test.sh': '#!/bin/sh\necho synthetic-bootstrap\n',
            'tests/test_outputs.py': 'def test_original():\n    assert False\n',
            'tests/support/data.json': '{"unchanged":true}\n',
        }.items():
            target = self.source / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(value)
        (self.source / 'solution/solve.sh').chmod(0o755)
        self.task = SimpleNamespace(name='arbitrary-task', paths=SimpleNamespace(task_dir=self.source))
        self.repin()
        self.destination = self.root / 'variant'

    def repin(self):
        self.snapshot = variant._files(self.source)
        self.entry = dict(name='arbitrary-task', files=[dict(path=path, **row)
            for path, row in self.snapshot.items()])

    def prepare(self):
        # Synthetic inputs never masquerade as an upstream accepted bootstrap.
        with patch.object(variant, 'BOOTSTRAP_SHA256', self.snapshot['tests/test.sh']['sha256']):
            return variant.prepare_ci_verifier_task(self.task, self.entry, self.destination)

    def test_only_scoring_launcher_changes_and_original_is_untouched(self):
        result = self.prepare()
        self.assertEqual(result, self.destination)
        self.assertEqual(variant._files(self.source), self.snapshot)
        changed = variant._files(result)
        self.assertEqual(set(changed), set(self.snapshot))
        for path, original in self.snapshot.items():
            with self.subTest(path=path):
                if path != 'tests/test.sh':
                    self.assertEqual(changed[path], original)
                    self.assertEqual((result / path).read_bytes(), (self.source / path).read_bytes())
        self.assertEqual((result / 'tests/test.sh').read_text(), variant.LAUNCHER)
        self.assertTrue((result / 'tests/test.sh').stat().st_mode & 0o111)
        self.assertEqual(stat.S_IMODE(result.stat().st_mode), 0o700)
        sidecar = variant.manifest_path(result)
        self.assertEqual(stat.S_IMODE(sidecar.stat().st_mode), 0o600)
        evidence = json.loads(sidecar.read_bytes())
        self.assertEqual(evidence['mode'], 'ci-preinstalled-v1')
        self.assertFalse(evidence['original_verifier'])
        self.assertEqual(evidence['changed_files'], ['tests/test.sh'])
        self.assertEqual(evidence['original_files'], self.snapshot)
        self.assertEqual(evidence['variant_files'], changed)
        self.assertEqual(evidence['source_launcher_sha256'], self.snapshot['tests/test.sh']['sha256'])
        self.assertEqual(evidence['launcher_sha256'], hashlib.sha256(variant.LAUNCHER.encode()).hexdigest())
        self.assertEqual(evidence['packages'], {'pytest': '8.4.1', 'pytest-json-ctrf': '0.3.5'})
        self.assertEqual(evidence['requirements_sha256'], variant.REQUIREMENTS_SHA256)
        self.assertEqual(variant.REQUIREMENTS_SHA256,
                         hashlib.sha256(variant.REQUIREMENTS_TEXT.encode()).hexdigest())

    def test_unknown_bootstrap_is_not_replaced(self):
        with self.assertRaisesRegex(InvalidInput, 'unsupported scoring entry'):
            variant.prepare_ci_verifier_task(self.task, self.entry, self.destination)
        self.assertFalse(self.destination.exists())

    def test_selected_task_names_do_not_control_the_transformation(self):
        self.task.name = self.entry['name'] = 'another-unrelated-task'
        self.prepare()
        self.assertEqual((self.destination / 'tests/test.sh').read_text(), variant.LAUNCHER)

    def test_identity_mismatch_rejected(self):
        self.entry['name'] = 'different-task'
        with self.assertRaisesRegex(InvalidInput, 'identity differs'):
            self.prepare()
        self.assertFalse(self.destination.exists())

    def test_changed_assertion_or_solution_or_extra_file_rejected(self):
        for path in ('tests/test_outputs.py', 'solution/solve.sh', 'environment/extra'):
            with self.subTest(path=path):
                target = self.source / path
                original = target.read_bytes() if target.exists() else None
                target.write_text('unexpected changed content\n')
                with self.assertRaisesRegex(InvalidInput, 'complete frozen file inventory'):
                    self.prepare()
                self.assertFalse(self.destination.exists())
                if original is None:
                    target.unlink()
                else:
                    target.write_bytes(original)

    def test_missing_test_outputs_is_not_replaced(self):
        (self.source / 'tests/test_outputs.py').unlink()
        self.repin()
        with self.assertRaisesRegex(InvalidInput, 'unsupported scoring entry'):
            self.prepare()

    def test_file_and_directory_symlinks_rejected(self):
        for name, target in (('escape', self.source / 'tests/test_outputs.py'),
                             ('directory-escape', self.root)):
            with self.subTest(name=name):
                link = self.source / name
                link.symlink_to(target)
                with self.assertRaisesRegex(InvalidInput, 'ordinary task files'):
                    self.prepare()
                link.unlink()
                self.assertFalse(self.destination.exists())

    def test_special_file_rejected_without_opening_it(self):
        pipe = self.source / 'pipe'
        os.mkfifo(pipe)
        with self.assertRaisesRegex(InvalidInput, 'ordinary task files'):
            self.prepare()
        self.assertFalse(self.destination.exists())

    def test_destination_cannot_overwrite_or_nest_in_original(self):
        existing = self.root / 'existing'
        existing.mkdir()
        for destination in (existing, self.source / 'nested', self.source.parent, Path('relative')):
            with self.subTest(destination=destination):
                self.destination = destination
                with self.assertRaisesRegex(InvalidInput, 'new independent'):
                    self.prepare()
        self.assertEqual(variant._files(self.source), self.snapshot)

    def test_destination_parent_must_be_private(self):
        public = self.root / 'public'
        public.mkdir(mode=0o755)
        self.destination = public / 'variant'
        with self.assertRaisesRegex(InvalidInput, 'private trial directory'):
            self.prepare()

    def test_manifest_cannot_overwrite_evidence(self):
        sidecar = variant.manifest_path(self.destination)
        sidecar.write_text('existing evidence')
        with self.assertRaisesRegex(InvalidInput, 'manifest already exists'):
            self.prepare()
        self.assertEqual(sidecar.read_text(), 'existing evidence')
        self.assertFalse(self.destination.exists())

    def test_invalid_or_duplicate_inventory_paths_rejected(self):
        for name in ('../outside', '/absolute', 'tests//test.sh', 'tests/test.sh'):
            with self.subTest(name=name):
                extra = dict(self.entry['files'][0], path=name)
                self.entry['files'].append(extra)
                with self.assertRaisesRegex(InvalidInput, 'invalid CI verifier frozen file inventory'):
                    self.prepare()
                self.entry['files'].pop()

    def test_copy_error_does_not_touch_source(self):
        with patch.object(variant.shutil, 'copytree', side_effect=OSError('copy failed')):
            with self.assertRaises(OSError):
                self.prepare()
        self.assertEqual(variant._files(self.source), self.snapshot)

    def test_launcher_is_valid_shell_and_pins_runtime_without_downloads(self):
        launcher = self.root / 'launcher.sh'
        launcher.write_text(variant.LAUNCHER)
        subprocess.run(['bash', '-n', str(launcher)], check=True)
        python_code = variant.LAUNCHER.split("<<'PY'\n", 1)[1].split('\nPY\n', 1)[0]
        tree = ast.parse(python_code)
        self.assertFalse(any(isinstance(node, ast.Assert) for node in ast.walk(tree)))
        self.assertIn("sys.version_info[:2] != (3, 13) or sys.flags.optimize", python_code)
        self.assertIn("('pytest', '8.4.1')", python_code)
        self.assertIn("('pytest-json-ctrf', '0.3.5')", python_code)
        self.assertIn("'/tests/test_outputs.py', '-rA'", python_code)
        self.assertIn("'--ctrf', '/logs/verifier/ctrf.json'", python_code)
        self.assertIn(variant.PYTHON + ' -I -', variant.LAUNCHER)
        self.assertIn('PYTHONOPTIMIZE', variant.LAUNCHER)
        for forbidden in ('apt-get ', 'curl ', 'uvx ', 'pip ', 'wget ', 'http://', 'https://'):
            self.assertNotIn(forbidden, variant.LAUNCHER)
        self.assertIn("printf '0\\n' > /logs/verifier/reward.txt", variant.LAUNCHER)
        self.assertIn("printf '1\\n' > /logs/verifier/reward.txt", variant.LAUNCHER)

    def test_runtime_preflight_rejects_missing_or_changed_packages_and_optimization(self):
        # Execute only dependency checks, never a mocked pytest/scoring run.
        code = variant.LAUNCHER.split("<<'PY'\n", 1)[1].split('\nPY\n', 1)[0]
        tree = ast.parse(code)
        boundary = next(index for index, node in enumerate(tree.body)
                        if isinstance(node, ast.Import) and node.names[0].name == 'pytest')
        preflight = compile(ast.Module(body=tree.body[:boundary], type_ignores=[]), '<preflight>', 'exec')
        for versions, missing, python_version, optimized in (
            ({'pytest': '8.4.1', 'pytest-json-ctrf': '0.3.5'}, None, (3, 13), 0),
            ({'pytest': '8.4.2', 'pytest-json-ctrf': '0.3.5'}, None, (3, 13), 0),
            ({'pytest': '8.4.1', 'pytest-json-ctrf': '0.3.6'}, None, (3, 13), 0),
            ({'pytest': '8.4.1', 'pytest-json-ctrf': '0.3.5'}, 'pytest', (3, 13), 0),
            ({'pytest': '8.4.1', 'pytest-json-ctrf': '0.3.5'}, 'pytest-json-ctrf', (3, 13), 0),
            ({'pytest': '8.4.1', 'pytest-json-ctrf': '0.3.5'}, None, (3, 12), 0),
            ({'pytest': '8.4.1', 'pytest-json-ctrf': '0.3.5'}, None, (3, 13), 1),
        ):
            with self.subTest(versions=versions, missing=missing, python=python_version, optimize=optimized):
                def version(package):
                    if package == missing:
                        raise PackageNotFoundError(package)
                    return versions[package]
                valid = versions == variant.PACKAGES and missing is None and python_version == (3, 13) and not optimized
                with (patch('importlib.metadata.version', side_effect=version),
                      patch.object(sys, 'version_info', python_version),
                      patch.object(sys, 'flags', SimpleNamespace(optimize=optimized))):
                    if valid:
                        exec(preflight, {})
                    else:
                        with self.assertRaises((RuntimeError, PackageNotFoundError)):
                            exec(preflight, {})


if __name__ == '__main__':
    unittest.main()
