"""Private, content-addressed acceptance evidence; no GitHub status writer.

Receipts are trusted local execution records, not cryptographic attestations.
Only an operator-controlled cache may be admitted. Source/command/tool changes,
missing logs and interrupted executions never produce reusable success.
"""
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
import shutil
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def tree_digest(directory, suffixes=None):
    directory = Path(directory)
    files = {}
    for parent, dirs, names in os.walk(directory):
        dirs[:] = [d for d in dirs if d not in ('.git','target','__pycache__')]
        for name in names:
            path = Path(parent) / name
            if path.is_file() and (suffixes is None or path.suffix in suffixes):
                files[str(path.relative_to(directory))] = sha(path)
    if not files:
        raise ValueError('missing runtime dependency tree: ' + str(directory))
    return digest(files)


def write_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix='.receipt-', dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(value, stream, indent=2)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


@contextmanager
def locked(directory):
    directory.mkdir(parents=True, exist_ok=True)
    with (directory / '.lock').open('a') as stream:
        fcntl.flock(stream, fcntl.LOCK_EX)
        yield


def rust_inputs(root):
    """Conservative full workspace closure, including added candidate files.

    Do not trim test sources from this set: candidates overlay test hooks.
    Paths outside tracked/package trees (targets, caches) are not inputs.
    """
    names = subprocess.check_output(['git', 'ls-files', '-z'], cwd=root).decode().split('\0')
    selected = {name for name in names if name and (
        name.endswith(('.rs', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'))
        or name.startswith('.cargo/'))}
    for directory in [root / 'verification/extract-storage', *root.glob('sandbox-*'),
                      *root.glob('controller-*')]:
        if not directory.is_dir():
            continue
        for parent, dirs, files in os.walk(directory):
            dirs[:] = [d for d in dirs if d not in ('target', '.git', '.lake', 'node_modules')]
            for name in files:
                if name.endswith('.rs') or name in ('Cargo.toml', 'Cargo.lock', 'build.rs'):
                    selected.add(str((Path(parent) / name).relative_to(root)))
    if (root / '.cargo').is_dir():
        selected.update(str(p.relative_to(root)) for p in (root / '.cargo').rglob('*') if p.is_file())
    return {name: sha(root / name) for name in sorted(selected)}


def rules(root=ROOT):
    names = subprocess.check_output(['git', 'ls-files', '-z'], cwd=root).decode().split('\0')
    return {n: sha(root / n) for n in names if n and (
        n.startswith(('tools/', 'scripts/', '.github/', 'verification/experiments/',
                      'verification/fixtures/')) and not n.endswith('.lean'))}


def native_key(label, command, source, root=ROOT, environment=None):
    # Absolute scratch paths do not change semantics, but argument order does.
    normalized = [str(c).replace(str(source), '$SOURCE') for c in command]
    runtime_path = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']) / 'runtime.json'
    env = os.environ if environment is None else environment
    options = {k:v for k,v in env.items() if k.startswith(('CARGO_PROFILE_', 'RUSTFLAGS',
        'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET', 'RUSTC_BOOTSTRAP',
        'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER'))}
    return dict(schema=1, label=label, command=normalized, options=options,
                rust=rust_inputs(source), rules=rules(root),
                runtime=json.loads(runtime_path.read_text()))


def load_success(directory, key):
    try:
        receipt = json.loads((directory / 'receipt.json').read_text())
        if (receipt.get('complete') is not True or receipt.get('key') != key
                or receipt.get('exit_code') != 0 or not receipt.get('files')):
            return None
        for name, expected in receipt['files'].items():
            path = Path(name)
            if path.is_absolute() or '..' in path.parts or sha(directory / path) != expected:
                return None
        return receipt
    except (OSError, ValueError, KeyError, TypeError):
        return None


def validate_native(cache):
    cache = Path(cache)
    seal = json.loads((cache/'complete.json').read_text())
    if (seal.get('complete') is not True or seal.get('exit_code') != 0
            or seal.get('rules') != rules() or not seal.get('commands')
            or seal.get('runtime') != json.loads((cache/'runtime.json').read_text())):
        raise ValueError('native suite is missing, incomplete or has changed rules/tools')
    for identity, expected in seal['commands'].items():
        if len(identity)!=64 or set(identity)-set('0123456789abcdef'):
            raise ValueError('invalid native entry identity')
        entry=cache/'commands'/identity
        if sha(entry/'receipt.json')!=expected:
            raise ValueError('changed native receipt: '+identity)
        receipt=json.loads((entry/'receipt.json').read_text())
        if load_success(entry,receipt['key']) is None or digest(receipt['key'])!=identity:
            raise ValueError('missing or corrupt native command evidence: '+identity)
    return seal


def native_run(label, command, source, log, environment=None, expected=0):
    """Run on the test host, or read its exact-input log on the proof host.

    Callers still check test counts and the precise negative assertion. The
    native suite is sealed only after every such caller returned successfully.
    """
    command = [str(c) for c in command]
    source, log = Path(source).resolve(), Path(log)
    cache = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']).resolve()
    key = native_key(label, command, source, environment=environment)
    entry = cache / 'commands' / digest(key)
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') == 'formal':
        seal = json.loads((cache / 'complete.json').read_text())
        if (seal.get('complete') is not True or seal.get('exit_code') != 0
                or seal.get('rules') != rules() or seal.get('runtime') != key['runtime']):
            raise ValueError('native suite missing, incomplete or has different acceptance rules')
        receipt = load_success(entry, key)
        if receipt is None or receipt.get('expected') != expected:
            raise ValueError('missing/stale native evidence: ' + label)
        if seal['commands'].get(entry.name) != sha(entry / 'receipt.json'):
            raise ValueError('native command was not part of the completed suite: ' + label)
        log.write_bytes((entry / 'command.log').read_bytes())
        return expected
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'native':
        raise ValueError('choose native (ning) or formal (a6000) acceptance role')
    with locked(entry):
        prior = load_success(entry, key)
        if prior is not None and prior.get('expected') == expected:
            log.write_bytes((entry / 'command.log').read_bytes())
            return expected
        started = time.monotonic()
        log.parent.mkdir(parents=True, exist_ok=True)
        with log.open('wb') as stream:
            code = subprocess.run(command, cwd=source, env=environment, stdout=stream,
                                  stderr=subprocess.STDOUT, check=False).returncode
        if code != expected:
            raise ValueError(f'{label}: expected {expected}, got {code}; see {log}')
        if native_key(label, command, source, environment=environment) != key:
            raise ValueError('native inputs changed during execution: ' + label)
        (entry / 'command.log').write_bytes(log.read_bytes())
        write_json(entry / 'receipt.json', dict(complete=True, exit_code=0, key=key,
            expected=expected, actual_exit=code, seconds=time.monotonic()-started,
            files={'command.log': sha(entry / 'command.log')}))
    return code


def native_case(label, source, output, action=None):
    """Keep a component's existing fresh-binary and assertion checks together."""
    cache = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']).resolve()
    key = native_key(label, ['component-case'], Path(source).resolve())
    entry = cache / 'commands' / digest(key)
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') == 'formal':
        seal = json.loads((cache / 'complete.json').read_text())
        receipt = load_success(entry, key)
        if (seal.get('complete') is not True or seal.get('exit_code') != 0
                or seal.get('rules') != rules() or receipt is None
                or seal.get('runtime') != key['runtime']
                or seal['commands'].get(entry.name) != sha(entry / 'receipt.json')):
            raise ValueError('missing/incomplete/changed native component: ' + label)
        return receipt
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'native' or action is None:
        raise ValueError('component cases must execute on the native test host')
    # Component drivers insist on fresh binaries. Reuse is at the completed
    # case boundary, never by relabelling a Cargo "fresh" artifact as new.
    with locked(entry):
        prior = load_success(entry, key)
        if prior:
            return prior
        start = time.monotonic()
        action()
        if native_key(label, ['component-case'], Path(source).resolve()) != key:
            raise ValueError('component inputs changed: ' + label)
        files = {}
        for path in sorted(Path(output).rglob('*')):
            if (not path.is_file() or path.is_symlink() or path.suffix not in ('.log', '.json', '.jsonl')
                    or any('target' in part or part in ('.git', '.lake') for part in path.relative_to(output).parts)):
                continue
            name = str(path.relative_to(output))
            target = entry / 'evidence' / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
            files['evidence/' + name] = sha(target)
        if not files:
            raise ValueError('component case has no original evidence')
        receipt = dict(complete=True, exit_code=0, key=key,
                       seconds=time.monotonic()-start, files=files)
        write_json(entry / 'receipt.json', receipt)
        return receipt
