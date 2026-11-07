"""Frozen-source admission for the actual unchanged live_native Rust function."""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent
BASE = '2be04389e45482cb056c54e716dd097456d6168a'
SOURCE = 'controller-service/src/controller/lifecycle_inbox/source_work/snapshot.rs'
OWNED = ('verification/experiments/stage1-protocol/source-live/',
         'verification/aeneas/stage1-protocol/source-live/',
         'SandboxModel/Lifecycle/Protocol/SourceLive/')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def mode(path):
    return '100755' if path.stat().st_mode & 0o111 else '100644'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def source_guard(root=None):
    root = ROOT if root is None else root
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
    require(subprocess.run(['git', 'merge-base', '--is-ancestor', BASE, head], cwd=root,
                           check=False).returncode == 0, 'source base differs')
    changed = subprocess.check_output(['git', 'diff', '--name-only', '-z', 'HEAD'], cwd=root).decode().split('\0')
    changed += subprocess.check_output(['git', 'ls-files', '--others', '--exclude-standard', '-z'],
                                       cwd=root).decode().split('\0')
    require(all(n.startswith(OWNED) for n in filter(None, changed)), 'unowned input changed')
    data = json.loads((HERE / 'inventory-native.json').read_text())
    require(data['source_base'] == BASE and data['source'] == SOURCE, 'native source contract differs')
    for section in ('inputs', 'providers', 'templates', 'drivers'):
        for name, digest in data[section].items():
            path = (HERE if section in ('templates', 'drivers') else root) / name
            require(path.is_file() and not path.is_symlink() and sha(path) == digest,
                    'reviewed input changed: ' + name)
    return head, data


def recipes():
    return json.loads((HERE / 'mutations.json').read_text())


def mutate(source, view):
    data = recipes()
    require(hashlib.sha256(source.encode()).hexdigest() == data['source_sha256'],
            'original Rust body changed')
    if view == 'original':
        return source
    require(view in data['views'], 'unknown actual Rust wrong view')
    start = source.index('fn live_native(')
    end = source.index('\npub(super) fn current<', start)
    original = source[start:end]
    changed = original
    for edit in data['views'][view]['edits']:
        require(changed.count(edit['before']) == 1, 'actual mutation site differs: ' + view)
        changed = changed.replace(edit['before'], edit['after'], 1)
    result = source[:start] + changed + source[end:]
    require(result != source and hashlib.sha256(result.encode()).hexdigest() ==
            data['views'][view]['sha256'], 'actual Rust wrong body differs')
    return result


def restore(source, view):
    data = recipes()
    require(view in data['views'] and hashlib.sha256(source.encode()).hexdigest() ==
            data['views'][view]['sha256'], 'restoration source is not the exact wrong view')
    for edit in reversed(data['views'][view]['edits']):
        require(source.count(edit['after']) == 1, 'actual restoration site differs')
        source = source.replace(edit['after'], edit['before'], 1)
    require(hashlib.sha256(source.encode()).hexdigest() == data['source_sha256'],
            'restored actual Rust differs from original')
    return source
