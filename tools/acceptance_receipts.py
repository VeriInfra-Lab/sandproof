"""Private, content-addressed acceptance evidence; no GitHub status writer.

Receipts are trusted local execution records, not cryptographic attestations.
Only an operator-controlled cache may be admitted. Source/command/tool changes,
missing logs and interrupted executions never produce reusable success.
"""
from contextlib import contextmanager
from collections import Counter
import fcntl
import hashlib
import importlib.util
import json
import os
import shutil
import stat
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
PROCESS_BATCH = str(os.getpid())+'-'+str(time.time_ns())
NATIVE_SCHEMA = 2
NATIVE_SCOPES = ('all', 'stage1-protocol', 'stage1-create-delivery',
                 'stage1-protocol-convergence', 'capture-caller', 'pause-publication', 'pause-stop',
                 'pause-dispatch', 'pause-receipt')
CAPTURE_CALLER_LABELS = ('functional', 'capture-caller')
PAUSE_PUBLICATION_LABELS = ('functional', 'pause-publication')
PAUSE_STOP_LABELS = ('functional', 'pause-stop')
PAUSE_DISPATCH_CHILD_SCOPES = ('capture-caller', 'pause-publication', 'pause-stop')
PAUSE_DISPATCH_LABELS = ('functional', *PAUSE_DISPATCH_CHILD_SCOPES, 'pause-dispatch')
PAUSE_RECEIPT_CHILD_SCOPES = (*PAUSE_DISPATCH_CHILD_SCOPES, 'pause-dispatch')
PAUSE_RECEIPT_LABELS = ('functional', *PAUSE_RECEIPT_CHILD_SCOPES, 'pause-receipt')
ARCHIVED_NATIVE_CASES = ('stage1-protocol', 'capture-caller', 'pause-publication', 'pause-stop',
                         'pause-dispatch', 'pause-receipt')
SOURCE_ARCHIVES = ('SANDPROOF_SQLITE_STATEMENTS_SOURCE_ARCHIVE',
                   'SANDPROOF_CHILD_OBSERVATION_SOURCE_ARCHIVE',
                   'SANDPROOF_WRITER_SOURCE_ARCHIVE',
                   'SANDPROOF_LIBRARY_SOURCE_ARCHIVE',
                   'SANDPROOF_STAGE1_CREATE_SOURCE_ARCHIVE')


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
        or name.startswith(('.cargo/','deploy/','references/','tests/','verification/extract-'))
        or name.split('/')[0].startswith(('sandbox-','controller-')))}
    for directory in [root / 'verification/extract-storage', *root.glob('sandbox-*'),
                      *root.glob('controller-*')]:
        if not directory.is_dir():
            continue
        for parent, dirs, files in os.walk(directory):
            dirs[:] = [d for d in dirs if d not in ('target', '.git', '.lake', 'node_modules')]
            for name in files:
                if name.endswith(('.rs','.sql','.lua','.proto')) or name in ('Cargo.toml', 'Cargo.lock', 'build.rs'):
                    selected.add(str((Path(parent) / name).relative_to(root)))
    if (root / '.cargo').is_dir():
        selected.update(str(p.relative_to(root)) for p in (root / '.cargo').rglob('*') if p.is_file())
    if any(Path(name).name=='build.rs' for name in selected):
        raise ValueError('owned build.rs requires explicit generated-input registration before cache reuse')
    result={name: sha(root / name) for name in sorted(selected)}
    for parent in (root,*root.parents):
        for name in ('.cargo/config','.cargo/config.toml','rustfmt.toml','.rustfmt.toml','clippy.toml','.clippy.toml'):
            path=parent/name
            if path.is_file():result['@configuration:'+str(path.resolve())]=sha(path)
    return result


def repository_inputs(root):
    names=subprocess.check_output(['git','ls-files','-z'],cwd=root).decode().split('\0')
    return {n:sha(root/n) for n in names if n}


def rules(root=ROOT):
    names = subprocess.check_output(['git', 'ls-files', '-z'], cwd=root).decode().split('\0')
    return {n: sha(root / n) for n in names if n and (
        n.startswith(('tools/', 'scripts/', '.github/', 'verification/experiments/',
                      'verification/fixtures/')) and not n.endswith('.lean'))}


def contract_inputs(root):
    """Root JSON contracts affect native checkers as well as formal proofs.

    Include newly prepared candidates; retain tracked names so deletion fails
    closed rather than silently dropping an input. Proof-only Lean/manifests
    stay in the formal closure, not this native execution configuration.
    """
    names = subprocess.check_output(['git', 'ls-files', '-z'], cwd=root).decode().split('\0')
    selected = {n for n in names if n and Path(n).parent == Path('verification')
                and Path(n).suffix == '.json'}
    selected.update(str(p.relative_to(root)) for p in (root / 'verification').glob('*.json'))
    result = {}
    for name in sorted(selected):
        path = root / name
        if path.is_symlink() or not path.is_file():
            raise ValueError('missing or non-regular native contract input: ' + name)
        result[name] = sha(path)
    return result


def source_archive_inputs(environment=None):
    """Bind original source archive bytes independently of machine paths."""
    environment = os.environ if environment is None else environment
    result = {}
    for name in SOURCE_ARCHIVES:
        value = environment.get(name)
        if not value:
            continue
        path = Path(value)
        if path.is_symlink() or not path.is_file():
            raise ValueError('missing or non-regular native source archive: ' + name)
        result[name] = sha(path)
    return result


def check_native_scope(scope):
    if scope not in NATIVE_SCOPES:
        raise ValueError('unknown native acceptance scope: ' + str(scope))


def stage1_registration():
    spec = importlib.util.spec_from_file_location('native_scope_registration',
        ROOT/'verification/experiments/stage1-protocol/registered.py')
    registered = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(registered)
    return registered


def stage1_native_labels(profile='full'):
    """Fixed full, Create or convergence scope; no caller-selected cases."""
    registered = stage1_registration()
    labels = tuple(registered.native_labels(profile))
    counts = {'full': 74, 'create-delivery': 12, 'protocol-convergence': 22}
    if profile not in counts:
        raise ValueError('unknown fixed native profile: ' + str(profile))
    count = counts[profile]
    if len(labels) != count or len(set(labels)) != count:
        raise ValueError(f'stage1 native scope requires the exact {count} registered cases')
    if profile == 'protocol-convergence' and not {
            'stage1-accepted-json-parser', 'stage1-sqlite-commit'} <= set(labels):
        raise ValueError('convergence is missing its JSON or SQLite component')
    result = ('functional', *(('stage1-protocol',) if profile == 'full' else ()), *labels)
    if len(set(result)) != count+(2 if profile == 'full' else 1):
        raise ValueError('stage1 native scope has duplicate component labels')
    return result


def scoped_native_labels(scope):
    check_native_scope(scope)
    profiles = {'stage1-protocol': 'full', 'stage1-create-delivery': 'create-delivery',
                'stage1-protocol-convergence': 'protocol-convergence'}
    if scope in profiles:
        return stage1_native_labels(profiles[scope])
    if scope == 'pause-dispatch':
        return PAUSE_DISPATCH_LABELS
    if scope == 'pause-receipt':
        return PAUSE_RECEIPT_LABELS
    return (PAUSE_PUBLICATION_LABELS if scope == 'pause-publication'
            else PAUSE_STOP_LABELS if scope == 'pause-stop' else CAPTURE_CALLER_LABELS)


def check_native_components(keys, *, scope='all', allow_additional=False):
    check_native_scope(scope)
    if scope == 'all':
        return
    keys = tuple(keys)
    if scope in ('pause-dispatch', 'pause-receipt') and not allow_additional and (
            len(keys) != len(scoped_native_labels(scope))
            or any(key.get('command') != ['component-case'] for key in keys)):
        raise ValueError(scope+' requires exactly its fixed component-case records')
    expected = set(scoped_native_labels(scope))
    labels = [key.get('label') for key in keys if key.get('command') == ['component-case']]
    counts = Counter(labels)
    if (not expected <= counts.keys() or any(counts[label] != 1 for label in expected)
            or (not allow_additional and set(counts) != expected)):
        raise ValueError(f'{scope} native scope requires exactly its {len(expected)} complete components')


def current_native_seal(seal, runtime, *, scope='all'):
    check_native_scope(scope)
    actual_scope = seal.get('scope')
    if (actual_scope == 'pause-dispatch'
            and scope not in ('pause-dispatch', *PAUSE_DISPATCH_CHILD_SCOPES)):
        return False
    if (actual_scope == 'pause-receipt'
            and scope not in ('pause-receipt', *PAUSE_RECEIPT_CHILD_SCOPES)):
        return False
    # Whole packet admission is deliberately stricter than component reuse.
    # A complete full/all seal cannot stand in for explicit Create/convergence,
    # and neither finite seal can stand in for full/all or another scope.
    create = scope == 'stage1-create-delivery'
    if create:
        definition = stage1_registration().create_profile(ROOT)
        if (actual_scope != scope or seal.get('profile') != 'create-delivery'
                or seal.get('create_definition') != definition
                or seal.get('source_archives', {}).get('SANDPROOF_STAGE1_CREATE_SOURCE_ARCHIVE')
                    != definition['source_only']['archive_sha256']):
            return False
    elif scope == 'stage1-protocol-convergence':
        registered = stage1_registration()
        definition = registered.CONVERGENCE.definition(ROOT)
        source = registered.source_definition(ROOT)
        if (actual_scope != scope or seal.get('profile') != 'protocol-convergence'
                or digest(seal.get('convergence_definition')) != digest(definition)
                or seal.get('source_archives', {}).get('SANDPROOF_STAGE1_CREATE_SOURCE_ARCHIVE')
                    != source['archive_sha256']):
            return False
    elif actual_scope in ('stage1-create-delivery', 'stage1-protocol-convergence'):
        return False
    # Only these fixed packets may supply their affected old components.
    # validate_native still validates every parent receipt and
    # its complete current key before any child can consume its own case.
    dispatch_parent = ((actual_scope == 'pause-dispatch' and scope in PAUSE_DISPATCH_CHILD_SCOPES)
                       or (actual_scope == 'pause-receipt' and scope in PAUSE_RECEIPT_CHILD_SCOPES))
    return (seal.get('schema') == NATIVE_SCHEMA and seal.get('complete') is True
            and type(seal.get('exit_code')) is int and seal.get('exit_code') == 0
            and (actual_scope in ('all', scope) or dispatch_parent)
            and seal.get('scope_complete') is True
            and seal.get('full_native_suite_complete') is (actual_scope == 'all')
            and seal.get('rules') == rules() and seal.get('runtime') == runtime
            and seal.get('contracts') == contract_inputs(ROOT)
            and seal.get('source_archives') == source_archive_inputs())


def native_key(label, command, source, root=ROOT, environment=None):
    # Absolute scratch paths do not change semantics, but argument order does.
    normalized = [str(c).replace(str(source), '$SOURCE') for c in command]
    runtime_path = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']) / 'runtime.json'
    env = os.environ if environment is None else environment
    options = {k:v for k,v in env.items() if k.startswith(('CARGO_PROFILE_', 'RUSTFLAGS',
        'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET', 'RUSTC_BOOTSTRAP',
        'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER','CC','CXX','AR','LD','CFLAGS','CPPFLAGS',
        'PKG_CONFIG','SQLITE3','LIBSQLITE3')) or k in ('RUSTC','RUSTDOC','CARGO_BUILD_RUSTC')}
    result=dict(schema=NATIVE_SCHEMA, label=label, command=normalized, options=options,
                rust=rust_inputs(source), rules=rules(root),
                contracts=dict(review=contract_inputs(root), source=contract_inputs(source)),
                source_archives=source_archive_inputs(env),
                runtime=json.loads(runtime_path.read_text()))
    if label in ARCHIVED_NATIVE_CASES:
        # Committed/clean is admission, not cache identity. Reports and a new
        # commit with identical native inputs must not invalidate every proof.
        subprocess.check_output(['git', 'rev-parse', '--verify', 'HEAD'], cwd=source)
        if subprocess.check_output(['git', 'status', '--porcelain', '-z',
                                    '--untracked-files=normal'], cwd=source):
            raise ValueError(label + ' component requires a clean committed source')
        metadata = {}
        groups = {'source': (source, set(result['rust']) | set(result['contracts']['source'])),
                  'review': (root, set(result['rules']) | set(result['contracts']['review']))}
        for role, (base, names) in groups.items():
            metadata[role] = {}
            for name in sorted(names):
                path = Path(name.removeprefix('@configuration:')) if name.startswith('@configuration:') else base/name
                info = path.lstat()
                if not stat.S_ISREG(info.st_mode):
                    raise ValueError(label + ' native input is not a regular file: ' + name)
                metadata[role][name] = dict(kind='regular', executable=bool(info.st_mode & stat.S_IXUSR))
        result['input_metadata'] = metadata
    if label == 'functional':
        result['repository']=repository_inputs(source)
    return result


def load_success(directory, key):
    try:
        receipt = json.loads((directory / 'receipt.json').read_text())
        if (not isinstance(receipt,dict) or receipt.get('complete') is not True or receipt.get('key') != key
                or type(receipt.get('exit_code')) is not int or receipt.get('exit_code') != 0
                or not isinstance(receipt.get('files'),dict) or not receipt.get('files')):
            return None
        for name, expected in receipt['files'].items():
            path = Path(name)
            if (path.is_absolute() or '..' in path.parts or (directory/path).is_symlink()
                    or not (directory/path).resolve().is_relative_to(directory.resolve())
                    or sha(directory / path) != expected):
                return None
        return receipt
    except (OSError, ValueError, KeyError, TypeError):
        return None


def validate_native(cache, *, scope='all'):
    check_native_scope(scope)
    cache = Path(cache)
    seal = json.loads((cache/'complete.json').read_text())
    if (not current_native_seal(seal,json.loads((cache/'runtime.json').read_text()),scope=scope)
            or not seal.get('commands')):
        raise ValueError('native suite is missing, incomplete or has changed rules/tools')
    keys = []
    for identity, expected in seal['commands'].items():
        if len(identity)!=64 or set(identity)-set('0123456789abcdef'):
            raise ValueError('invalid native entry identity')
        entry=cache/'commands'/identity
        if sha(entry/'receipt.json')!=expected:
            raise ValueError('changed native receipt: '+identity)
        receipt=json.loads((entry/'receipt.json').read_text())
        if load_success(entry,receipt['key']) is None or digest(receipt['key'])!=identity:
            raise ValueError('missing or corrupt native command evidence: '+identity)
        keys.append(receipt['key'])
    dispatch_parent = seal['scope'] in ('pause-dispatch', 'pause-receipt')
    if dispatch_parent:
        check_native_components(keys, scope=seal['scope'])
        for key in keys:
            current = native_key(key['label'], ['component-case'], ROOT)
            if key != current or seal.get('rust') != current['rust']:
                raise ValueError(seal['scope']+' parent has foreign current inputs: ' + key['label'])
        if seal.get('repository_sha256') != digest(repository_inputs(ROOT)):
            raise ValueError(seal['scope']+' functional source inventory changed')
    current_keys = {}
    if scope == 'stage1-protocol-convergence':
        prototype = native_key('current-component', ['component-case'], ROOT)
        for key in keys:
            label = key['label']
            current = dict(prototype, label=label)
            if label == 'functional':
                current['repository'] = repository_inputs(ROOT)
            current_keys[label] = current
            if key != current and label not in seal.get('convergence_reuse', {}):
                raise ValueError('foreign native inputs have no current admission: '+label)
    if 'convergence_reuse' in seal:
        if scope != 'stage1-protocol-convergence':
            raise ValueError('fixed convergence reuse cannot satisfy another scope')
        for label, reference in seal['convergence_reuse'].items():
            if label not in current_keys:
                raise ValueError('reuse admission has no original component: '+label)
            key = current_keys[label]
            original, entry = convergence_reuse_case(cache, label, key, reference=reference)
            if (seal['commands'].get(entry.name) != sha(entry/'receipt.json')
                    or original['key']['label'] != label):
                raise ValueError('fixed original receipt absent from current native seal')
    check_native_components(keys, scope=scope,
                            allow_additional=seal['scope']=='all' or dispatch_parent)
    return seal


def convergence_reuse_case(cache, label, key, *, reference=None):
    """Read one fixed original receipt; never rewrite its key or execution."""
    definition = stage1_registration().CONVERGENCE
    plans = definition.native_reuse_definition(ROOT)
    if label not in plans['components']:
        raise ValueError('component has no fixed original receipt: '+label)
    plan = plans['components'][label]
    entry = cache/'commands'/plan['identity']
    receipt_file = entry/'receipt.json'
    if (entry.is_symlink() or receipt_file.is_symlink() or not receipt_file.is_file()
            or sha(receipt_file) != plan['receipt_sha256']):
        raise ValueError('missing or changed fixed original receipt: '+label)
    original = json.loads(receipt_file.read_text())
    if (digest(original['key']) != entry.name or original['key']['label'] != label
            or original['key']['command'] != ['component-case']
            or load_success(entry, original['key']) is None):
        raise ValueError('incomplete fixed original component evidence: '+label)
    if not definition.native_reuse_matches(ROOT, label, original['key'], key,
                                           sys.modules[__name__]):
        if reference is not None:
            raise ValueError('fixed original execution inputs no longer match: '+label)
        return None, None
    relative = 'admissions/'+label+'.json'
    path = cache/relative
    admission = dict(schema=1, kind='fixed-original-native-current-admission', label=label,
        original_identity=entry.name, original_receipt_sha256=plan['receipt_sha256'],
        original_execution_commit=plan['execution_commit'], current_key_sha256=digest(key),
        definition_sha256=sha(ROOT/definition.NATIVE_REUSE_MANIFEST))
    if reference is not None:
        if (reference != dict(identity=entry.name, file=relative, sha256=sha(path))
                or path.is_symlink() or json.loads(path.read_text()) != admission):
            raise ValueError('missing or changed current original-receipt admission: '+label)
    else:
        if path.is_symlink():
            raise ValueError('current admission must be a regular file')
        write_json(path, admission)
    return original, entry


def convergence_reuse_publication(cache, keys, *, scope):
    """Seal only original receipts actually used in this current batch."""
    if os.environ.get('SANDPROOF_CONVERGENCE_REUSE_FIXED17') != '1':
        return None
    if scope != 'stage1-protocol-convergence':
        raise ValueError('fixed convergence reuse requires its exact native scope')
    plans = stage1_registration().CONVERGENCE.native_reuse_definition(ROOT)
    result = {}
    prototype = native_key('current-component', ['component-case'], ROOT)
    for identity, key in keys.items():
        label = key['label']
        if label in plans['components'] and identity == plans['components'][label]['identity']:
            current = dict(prototype, label=label)
            path = cache/'admissions'/(label+'.json')
            reference = dict(identity=identity, file=str(path.relative_to(cache)), sha256=sha(path))
            convergence_reuse_case(cache, label, current, reference=reference)
            result[label] = reference
        else:
            current = dict(prototype, label=label)
            if label == 'functional':
                current['repository'] = repository_inputs(ROOT)
            if key != current:
                raise ValueError('foreign native inputs have no current admission: '+label)
    return result


def record_used(entry, reused=False):
    filename=os.environ.get('SANDPROOF_NATIVE_USED')
    if not filename:return
    cache=Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']).resolve()
    path=Path(filename).resolve()
    if not path.is_relative_to(cache):return
    with path.open('a') as stream:
        fcntl.flock(stream,fcntl.LOCK_EX)
        stream.write(json.dumps(dict(identity=entry.name,sha256=sha(entry/'receipt.json'),reused=reused))+'\n')


def command_reusable(entry, receipt):
    if receipt.get('batch')==os.environ.get('SANDPROOF_NATIVE_BATCH',PROCESS_BATCH):return True
    filename=os.environ.get('SANDPROOF_NATIVE_PREVIOUS') or str(entry.parents[1]/'complete.json')
    if not Path(filename).resolve().is_relative_to(entry.parents[1].resolve()):
        filename=str(entry.parents[1]/'complete.json')
    try:
        prior=json.loads(Path(filename).read_text())
        return (prior.get('complete') is True and prior.get('exit_code')==0
                and prior.get('commands',{}).get(entry.name)==sha(entry/'receipt.json'))
    except (OSError,ValueError,AttributeError,TypeError):return False


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
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE', 'formal') == 'formal':
        seal = json.loads((cache / 'complete.json').read_text())
        if not current_native_seal(seal,key['runtime']):
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
        if prior is not None and prior.get('expected') == expected and command_reusable(entry,prior):
            log.write_bytes((entry / 'command.log').read_bytes())
            record_used(entry,reused=True)
            return expected
        if (entry/'receipt.json').exists():
            history=entry/'history'/sha(entry/'receipt.json');history.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(entry/'receipt.json',history/'receipt.json')
            if (entry/'command.log').is_file():shutil.copyfile(entry/'command.log',history/'command.log')
            (entry/'receipt.json').unlink()
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
            batch=os.environ.get('SANDPROOF_NATIVE_BATCH',PROCESS_BATCH),
            expected=expected, actual_exit=code, seconds=time.monotonic()-started,
            files={'command.log': sha(entry / 'command.log')}))
        record_used(entry)
    return code


def native_case(label, source, output, action=None, *, scope='all'):
    """Keep a component's existing fresh-binary and assertion checks together."""
    check_native_scope(scope)
    labels = scoped_native_labels(scope)
    if scope != 'all' and label not in labels:
        raise ValueError('component is outside requested native scope: ' + label)
    cache = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']).resolve()
    key = native_key(label, ['component-case'], Path(source).resolve())
    entry = cache / 'commands' / digest(key)
    if label in ARCHIVED_NATIVE_CASES:
        archive = entry/'evidence/source.tar'
        if (archive.is_symlink() or (archive.exists() and not archive.is_file())
                or not archive.parent.resolve().is_relative_to(entry.resolve())):
            raise ValueError(label + ' cached source.tar must be inside its regular evidence directory')
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE', 'formal') == 'formal':
        seal = (validate_native(cache, scope=scope) if scope != 'all'
                else json.loads((cache / 'complete.json').read_text()))
        receipt = load_success(entry, key)
        if (receipt is None and scope == 'stage1-protocol-convergence'
                and label in seal.get('convergence_reuse', {})):
            receipt, entry = convergence_reuse_case(cache, label, key,
                reference=seal['convergence_reuse'][label])
        if (label in ARCHIVED_NATIVE_CASES and receipt is not None
                and 'evidence/source.tar' not in receipt['files']):
            receipt = None
        if (not current_native_seal(seal,key['runtime'],scope=scope) or receipt is None
                or seal['commands'].get(entry.name) != sha(entry / 'receipt.json')):
            raise ValueError('missing/incomplete/changed native component: ' + label)
        return receipt
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'native' or action is None:
        raise ValueError('component cases must execute on the native test host')
    # Component drivers insist on fresh binaries. Reuse is at the completed
    # case boundary, never by relabelling a Cargo "fresh" artifact as new.
    with locked(entry):
        prior = load_success(entry, key)
        if (label in ARCHIVED_NATIVE_CASES and prior is not None
                and 'evidence/source.tar' not in prior['files']):
            prior = None
        if prior:
            record_used(entry,reused=True)
            return prior
        if os.environ.get('SANDPROOF_CONVERGENCE_REUSE_FIXED17') == '1':
            if scope != 'stage1-protocol-convergence':
                raise ValueError('fixed convergence reuse requires its exact native scope')
            plans = stage1_registration().CONVERGENCE.native_reuse_definition(ROOT)
            if label in plans['components']:
                prior, original_entry = convergence_reuse_case(cache, label, key)
                if prior is not None:
                    record_used(original_entry, reused=True)
                    return prior
        start = time.monotonic()
        if label in ARCHIVED_NATIVE_CASES:
            (entry/'receipt.json').unlink(missing_ok=True)
        action()
        if native_key(label, ['component-case'], Path(source).resolve()) != key:
            raise ValueError('component inputs changed: ' + label)
        files = {}
        for path in sorted(Path(output).rglob('*')):
            parts = path.relative_to(output).parts
            suffixes = ('.log', '.json', '.jsonl')
            if label in ('capture-caller', 'pause-publication', 'pause-stop', 'pause-dispatch', 'pause-receipt'):
                # Original CLI logs use .stdout/.stderr, including semantic
                # names such as ignore-capture-target. Do not discard a log
                # merely because its filename contains the word "target".
                suffixes += ('.stdout', '.stderr')
                build = any(part in ('.git', '.lake', 'target', 'extract-target', 'formal-target')
                            or part.startswith(('target-', 'cargo-target')) for part in parts[:-1])
            else:
                build = any('target' in part or part in ('.git', '.lake') for part in parts)
            if (not path.is_file() or path.is_symlink() or path.suffix not in suffixes or build):
                continue
            name = str(path.relative_to(output))
            target = entry / 'evidence' / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
            files['evidence/' + name] = sha(target)
        if label in ARCHIVED_NATIVE_CASES:
            # One explicitly named raw input, not arbitrary archives or Cargo
            # targets. Historical native source remains verifiable after docs
            # or proof-only commits without changing the receipt schema.
            archive = Path(output)/'source.tar'
            if archive.is_symlink() or not archive.is_file():
                raise ValueError(label + ' component requires its original regular source.tar')
            expected = sha(archive)
            target = entry/'evidence/source.tar'
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(archive, target)
            if sha(target) != expected or sha(archive) != expected:
                raise ValueError(label + ' original source archive changed while copying')
            files['evidence/source.tar'] = expected
        if not files:
            raise ValueError('component case has no original evidence')
        receipt = dict(complete=True, exit_code=0, key=key,
                       seconds=time.monotonic()-start, files=files)
        write_json(entry / 'receipt.json', receipt)
        record_used(entry)
        return receipt
