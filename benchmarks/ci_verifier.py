"""Explicit CI scoring-entry variant; never modify the frozen source task.

This is not an original Terminal-Bench verifier run. Only the reviewed, common
Python/pytest bootstrap may be replaced; every other task file is retained.
The caller uses the returned directory to construct Harbor's ordinary Task and
Verifier *after* the original oracle has finished.
"""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat

from tools.benchmark_tasks import InvalidInput


MODE = 'ci-preinstalled-v1'
PYTHON = '/opt/sandproof-ci-verifier/bin/python'
PYTHON_VERSION = '3.13'
PACKAGES = {'pytest': '8.4.1', 'pytest-json-ctrf': '0.3.5'}
REQUIREMENTS_TEXT = '# Runtime: CPython 3.13\npytest==8.4.1\npytest-json-ctrf==0.3.5\n'
REQUIREMENTS_SHA256 = hashlib.sha256(REQUIREMENTS_TEXT.encode()).hexdigest()
# Content identity, not a task-name dispatch. Unknown launchers may contain
# actual scoring logic and cannot safely be replaced by this generic bootstrap.
BOOTSTRAP_SHA256 = '4770437ea96c3cc84684b4f99d55fb148fcac09f9ea1e8ef49de487716e6c334'


LAUNCHER = '''#!/bin/bash
# Sandproof CI scoring-entry variant; no dependency installation or downloads.
set -u
printf '0\\n' > /logs/verifier/reward.txt || exit 2
if [ "$PWD" = '/' ]; then
    echo 'Error: No working directory set.' >&2
    exit 2
fi
case "${PYTHONOPTIMIZE:-}" in
    ''|0) ;;
    *) echo 'Error: Optimized Python is not a valid scoring environment.' >&2; exit 2 ;;
esac
# Ambient pytest options/plugins must not skip or replace the frozen checks.
unset PYTEST_ADDOPTS PYTEST_PLUGINS PYTEST_DISABLE_PLUGIN_AUTOLOAD
/opt/sandproof-ci-verifier/bin/python -I - <<'PY'
from importlib.metadata import version
import sys

if sys.version_info[:2] != (3, 13) or sys.flags.optimize:
    raise RuntimeError('CI verifier requires non-optimized Python 3.13')
for package, expected in [('pytest', '8.4.1'), ('pytest-json-ctrf', '0.3.5')]:
    if version(package) != expected:
        raise RuntimeError('CI verifier dependency version differs: ' + package)
import pytest
raise SystemExit(pytest.main([
    '--ctrf', '/logs/verifier/ctrf.json', '/tests/test_outputs.py', '-rA',
]))
PY
code=$?
if [ "$code" -eq 0 ]; then
    printf '1\\n' > /logs/verifier/reward.txt
else
    printf '0\\n' > /logs/verifier/reward.txt
fi
'''


def _sha(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def _files(root):
    """Reject links and special files, including directories with link ancestry."""
    if root.is_symlink() or not root.is_dir() or root.resolve() != root:
        raise InvalidInput('CI verifier source must be an absolute nonsymlink task directory')
    result = {}
    for path in sorted(root.rglob('*')):
        mode = path.lstat().st_mode
        if stat.S_ISDIR(mode):
            continue
        if not stat.S_ISREG(mode):
            raise InvalidInput('CI verifier accepts only ordinary task files and directories')
        result[path.relative_to(root).as_posix()] = {
            'sha256': _sha(path), 'bytes': path.stat().st_size,
            'mode': '100755' if mode & 0o111 else '100644',
        }
    return result


def manifest_path(destination):
    destination = Path(destination)
    return destination.parent / (destination.name + '-manifest.json')


def prepare_ci_verifier_task(task, entry, destination):
    """Return a separately rooted task Path with just tests/test.sh changed.

    ``entry`` must be the already verified frozen task-lock entry. This helper
    rechecks its complete file inventory before and after copying. Failures
    preserve any partial destination for the enclosing trial's exact cleanup.
    It never executes task code, installs dependencies, or writes to the source.
    """
    source = Path(task.paths.task_dir)
    destination = Path(destination)
    if (not destination.is_absolute() or destination.resolve() != destination
            or destination.exists() or destination.is_symlink()
            or destination.is_relative_to(source) or source.is_relative_to(destination)):
        raise InvalidInput('CI verifier destination must be a new independent task directory')
    parent = destination.parent
    parent_info = parent.lstat()
    if (not stat.S_ISDIR(parent_info.st_mode) or parent_info.st_uid != os.geteuid()
            or parent_info.st_mode & 0o077):
        raise InvalidInput('CI verifier destination requires a private trial directory')
    if task.name != entry.get('name'):
        raise InvalidInput('CI verifier task identity differs from its frozen entry')
    expected = {}
    for row in entry['files']:
        path = PurePosixPath(row['path'])
        if (path.is_absolute() or '..' in path.parts or str(path) != row['path']
                or row['path'] in expected):
            raise InvalidInput('invalid CI verifier frozen file inventory')
        expected[row['path']] = {key: row[key] for key in ('sha256', 'bytes', 'mode')}
    original = _files(source)
    if original != expected:
        raise InvalidInput('CI verifier source differs from its complete frozen file inventory')
    if (original.get('tests/test.sh', {}).get('sha256') != BOOTSTRAP_SHA256
            or 'tests/test_outputs.py' not in original):
        raise InvalidInput('unsupported scoring entry; only the reviewed pytest bootstrap is replaceable')
    sidecar = manifest_path(destination)
    if sidecar.exists() or sidecar.is_symlink():
        raise InvalidInput('CI verifier variant manifest already exists')
    shutil.copytree(source, destination)
    destination.chmod(0o700)
    if _files(source) != original or _files(destination) != original:
        raise InvalidInput('CI verifier task changed while copying')
    script = destination / 'tests/test.sh'
    script.write_text(LAUNCHER)
    script.chmod(0o755)
    changed = _files(destination)
    if (set(changed) != set(original)
            or any(changed[path] != value for path, value in original.items() if path != 'tests/test.sh')
            or _files(source) != original):
        raise InvalidInput('CI verifier variant changed more than its scoring entry')
    evidence = dict(schema_version=1, mode=MODE, task=task.name, original_verifier=False,
                    changed_files=['tests/test.sh'], python=PYTHON, python_version=PYTHON_VERSION,
                    packages=PACKAGES, requirements_sha256=REQUIREMENTS_SHA256,
                    original_files=original, variant_files=changed,
                    source_launcher_sha256=original['tests/test.sh']['sha256'],
                    launcher_sha256=changed['tests/test.sh']['sha256'])
    descriptor = os.open(sidecar, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, 'w') as stream:
        json.dump(evidence, stream, sort_keys=True)
        stream.write('\n')
    return destination
