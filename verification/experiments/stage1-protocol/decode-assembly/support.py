"""Reuse the original complete input inventory and fixed library evidence."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/decode-assembly')
VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('decode_source_helpers', ROOT / HERE.parent / 'schema-leaves/formal.py')
schema = module('decode_schema_helpers', ROOT / HERE.parent / 'envelope-schema/formal.py')
bytes_module = module('decode_bytes_helpers', ROOT / HERE.parent / 'envelope-bytes/formal.py')
registry_module = module('decode_registry_helpers', ROOT / HERE.parent / 'envelope-writer/native.py')
serializer_library = module('decode_serialize_library', ROOT / HERE.parent / 'serde-binding/library.py')
parser_library = module('decode_parser_library', ROOT / HERE.parent / 'parser-binding/library.py')
canonical = module('decode_canonical', ROOT / 'tools/check_foundations_extraction.py').llbc_digest


def libraries():
    return dict(serializer=serializer_library.check(json.loads((ROOT / HERE.parent /
        'serde-binding/candidate.json').read_text()), ROOT),
        parser=parser_library.check(json.loads((ROOT / HERE.parent /
        'parser-binding/candidate.json').read_text()), ROOT))


MUTANT = 'swap-recorded-flags'
MUTATION_PATH = 'sandbox-protocol/src/lifecycle/authority.rs'
MUTATIONS = (
    ('    pub current: bool,\n', '    #[serde(rename(deserialize = "terminal"))]\n    pub current: bool,\n'),
    ('    pub terminal: bool,\n', '    #[serde(rename(deserialize = "current"))]\n    pub terminal: bool,\n'),
)


def mutate(project):
    path = project / MUTATION_PATH
    before = base.sha(path)
    text = path.read_text()
    for old, new in MUTATIONS:
        if text.count(old) != 1:
            raise ValueError('original Recorded boolean field not unique')
        text = text.replace(old, new)
    path.write_text(text)
    return dict(path=MUTATION_PATH, before=before, after=base.sha(path),
                replacements=[list(pair) for pair in MUTATIONS])


def copy_source(project, inputs):
    project.mkdir(parents=True, mode=0o700)
    for name, wanted in inputs.items():
        target = project / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, target)
        if base.sha(target) != wanted:
            raise ValueError('source changed while copying: ' + name)


def capture(project, root, output):
    if root not in ('accepted_decode', 'recorded_decode'):
        raise ValueError('unregistered actual decoder root')
    runtime = module('protocol_stable_tools', ROOT / HERE.parent / 'formal_runtime.py')
    tools = project / '.tools'
    if not tools.exists():
        tools.symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
    raw_path = project / ('artifacts/bytes-' + root + '.llbc')
    if raw_path.exists():
        raise ValueError('preserve previous extraction; use fresh output')
    (project / 'artifacts/extract-target').mkdir(parents=True, exist_ok=True)
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
    if base.sha(tools / 'aeneas/charon/bin/charon') != fixed['charon']['binary_sha256']:
        raise ValueError('fixed Charon changed')
    wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
        '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
        *runtime.tool_bind(ROOT), '--bind', str(project), str(VIRTUAL), '--chdir', str(VIRTUAL),
        '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL / 'artifacts/extract-target'),
        '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
        '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
    command = wrapper + ['.tools/aeneas/charon/bin/charon', 'cargo', '--preset=aeneas',
        '--error-on-warnings', '--sysroot', 'default', '--include', 'sandproof_protocol',
        '--start-from', 'crate::' + root, '--dest-file', '../../../../artifacts/' + raw_path.name,
        '--', '--manifest-path', str(HERE.parent / 'envelope-bytes/Cargo.toml'), '--lib', '--locked', '--offline']
    log = output / (root + '-charon.log')
    started = time.monotonic()
    with log.open('xb') as stream:
        result = subprocess.run(command, cwd=project, stdin=subprocess.DEVNULL,
            stdout=stream, stderr=subprocess.STDOUT,
            env=dict(os.environ, PYTHONDONTWRITEBYTECODE='1'), timeout=1800)
    record = dict(argv=command, exit_code=result.returncode, log_sha256=base.sha(log),
                  seconds=time.monotonic()-started, tool=fixed['charon'])
    (output / (root + '-capture.json')).write_text(json.dumps(record, indent=2)+'\n')
    if result.returncode:
        raise ValueError('extraction failure is not a property rejection')
    raw = json.loads(raw_path.read_text())
    if raw.get('has_errors') is not False:
        raise ValueError('incomplete actual extraction')
    record.update(llbc_sha256=base.sha(raw_path), canonical_llbc_sha256=canonical(raw))
    return raw, record
