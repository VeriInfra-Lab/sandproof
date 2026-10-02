"""Keep PyStan dependency setup guest-scoped and preserve original commands."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.oracle_repairs import prepare_repaired_task

BENCHMARKS = Path(__file__).parents[1]


class PyStanDependenciesTests(unittest.TestCase):
    def test_snapshot_guards_and_error_propagation_preserve_existing_sources(self):
        for failure in ('', 'distribution', 'codename', 'keyring', 'input',
                        'source_collision', 'preference_collision', 'update'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                for name in ('etc/apt/sources.list.d', 'etc/apt/preferences.d', 'usr/share/keyrings', 'app', 'bin'):
                    (root / name).mkdir(parents=True)
                existing = root / 'etc/apt/sources.list.d/ubuntu.sources'
                existing.write_text('original configured Ubuntu repositories\n')
                (root / 'etc/os-release').write_text(
                    f'ID={"debian" if failure == "distribution" else "ubuntu"}\n'
                    f'VERSION_CODENAME={"jammy" if failure == "codename" else "noble"}\n')
                if failure != 'keyring':
                    (root / 'usr/share/keyrings/ubuntu-archive-keyring.gpg').write_bytes(b'fixture keyring')
                for name in ('train_X.csv', 'train_y.csv', 'test_X.csv', 'meta_public.json', 'gp_rstan.R'):
                    if failure != 'input' or name != 'train_y.csv':
                        (root / 'app' / name).touch()
                source = root / 'etc/apt/sources.list.d/sandproof-pystan-snapshot.list'
                preference = root / 'etc/apt/preferences.d/sandproof-pystan-curl'
                if failure in ('source_collision', 'preference_collision'):
                    (source if failure == 'source_collision' else preference).write_text('do not replace\n')
                apt = root / 'bin/apt-get'
                apt.write_text('#!/bin/bash\nprintf "%s\\n" "$*" > "$REPAIR_TEST_LOG"\n'
                               + ('exit 42\n' if failure == 'update' else 'exit 0\n'))
                apt.chmod(0o700)
                # Run the actual helper with filesystem paths relocated to this
                # isolated fixture; the apt stub never accesses host packages.
                script = (BENCHMARKS / 'guest_pystan_dependencies.sh').read_text()
                for prefix in ('/etc/', '/usr/share/keyrings/', '/app/'):
                    script = script.replace(prefix, str(root) + prefix)
                helper = root / 'helper.sh'
                helper.write_text(script)
                log = root / 'commands'
                result = subprocess.run(['/bin/bash', str(helper)], capture_output=True, env={
                    **os.environ, 'PATH': str(root / 'bin') + ':/usr/bin:/bin', 'REPAIR_TEST_LOG': str(log)})
                self.assertEqual(result.returncode, 42 if failure == 'update' else 1 if failure else 0)
                self.assertEqual(existing.read_text(), 'original configured Ubuntu repositories\n')
                if failure not in ('', 'update'):
                    self.assertFalse(log.exists())
                    for target in (source, preference):
                        if target.exists():
                            self.assertEqual(target.read_text(), 'do not replace\n')
                else:
                    self.assertEqual(log.read_text(), '-o APT::Update::Error-Mode=any update\n')
                    self.assertEqual(source.read_text(), 'deb [check-valid-until=no signed-by='
                        + str(root / 'usr/share/keyrings/ubuntu-archive-keyring.gpg') + '] '
                        'https://snapshot.ubuntu.com/ubuntu/20251001T000000Z noble-updates main\n')
                    self.assertEqual(preference.read_text(),
                        'Package: curl libcurl4t64\nPin: version 8.5.0-2ubuntu10.6\nPin-Priority: 1001\n')

    def test_original_pip_commands_inherit_constraint_and_setup_failure_stops(self):
        script = 'set -e\npython -m pip install --upgrade pip setuptools wheel\npython analysis.py\n'
        for failure in ('', 'setup', 'pip'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source, bin_dir = root / 'source', root / 'bin'
                (source / 'solution').mkdir(parents=True)
                (source / 'tests').mkdir()
                bin_dir.mkdir()
                (source / 'solution/solve.sh').write_text(script)
                (source / 'solution/analysis.py').write_text('unchanged sampling program\n')
                (source / 'tests/test.sh').write_text('original verifier\n')
                (source / 'task.toml').write_text('[agent]\ntimeout_sec = 1800.0\n')
                with patch.dict('benchmarks.oracle_repairs.REPAIRS', {
                        'rstan-to-pystan': hashlib.sha256(script.encode()).hexdigest()}):
                    repaired = prepare_repaired_task('rstan-to-pystan', source, root / 'repaired')
                for command in ('bash', 'python'):
                    stub = bin_dir / command
                    stub.write_text(f'#!{sys.executable}\nimport json,os,sys\nfrom pathlib import Path\n'
                        'name=Path(sys.argv[0]).name\n'
                        'with open(os.environ["REPAIR_TEST_LOG"],"a") as log:\n'
                        ' log.write(json.dumps([name,sys.argv[1:],os.environ.get("PIP_CONSTRAINT")])+"\\n")\n'
                        'failure=os.environ["REPAIR_TEST_FAIL"]\n'
                        'raise SystemExit(42 if (name=="bash" and failure=="setup") or '
                        '("pip" in sys.argv and failure=="pip") else 0)\n')
                    stub.chmod(0o700)
                log = root / 'commands'
                result = subprocess.run(['/bin/bash', str(repaired / 'solution/solve.sh')], env={
                    **os.environ, 'PATH': str(bin_dir), 'REPAIR_TEST_LOG': str(log),
                    'REPAIR_TEST_FAIL': failure, 'PIP_CONSTRAINT': 'inherited-fixture'})
                self.assertEqual(result.returncode, 42 if failure else 0)
                expected = [['bash', ['/solution/guest_pystan_dependencies.sh'], 'inherited-fixture']]
                if failure != 'setup':
                    expected.append(['python', ['-m', 'pip', 'install', '--upgrade', 'pip', 'setuptools', 'wheel'],
                                     '/solution/pystan-constraints.txt'])
                    if not failure:
                        expected.append(['python', ['analysis.py'], '/solution/pystan-constraints.txt'])
                self.assertEqual([json.loads(line) for line in log.read_text().splitlines()], expected)
                self.assertEqual((source / 'solution/solve.sh').read_text(), script)
                for relative in ('solution/analysis.py', 'tests/test.sh', 'task.toml'):
                    self.assertEqual((source / relative).read_bytes(), (repaired / relative).read_bytes())
                evidence = json.loads((root / 'oracle-repair.json').read_text())
                for name in ('guest_pystan_dependencies.sh', 'pystan-constraints.txt'):
                    self.assertEqual((repaired / 'solution' / name).read_bytes(), (BENCHMARKS / name).read_bytes())
                    self.assertEqual(evidence['repaired_files'][name], hashlib.sha256((BENCHMARKS / name).read_bytes()).hexdigest())


if __name__ == '__main__':
    unittest.main()
