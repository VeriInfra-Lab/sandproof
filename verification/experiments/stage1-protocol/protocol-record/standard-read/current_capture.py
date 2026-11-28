"""Extract the two transparent StandardRecord reads from an exact candidate."""

import importlib.util
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile


STAGE = Path('verification/experiments/stage1-protocol')
HERE = STAGE / 'protocol-record/standard-read'
SOURCE = Path('controller-service/src/controller/lifecycle_inbox/recording.rs')
CRATE = STAGE / 'inbox-record/Cargo.toml'
MODULE = Path('SandproofStandardRecord')
RAW = 'standard-record-read.llbc'
EXTERNAL = {
    'core.cell.RefCell.borrow',
    'core.cell.Ref.Insts.CoreOpsDerefDeref.deref',
    'core.option.Option.is_some_and',
    'core.option.Option.Insts.CoreCloneClone.clone',
    'core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any',
    'alloc.string.String.Insts.CoreCloneClone.clone',
}


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def admit(root, directory, config):
    helpers = load('standard_read_ir_helpers', root / STAGE / 'unload-parent-link/revision_capture.py')
    syntax = load('standard_read_lean_tokens', root / STAGE / 'unload-parent-link/check_capture.py')
    raw = json.loads((directory / RAW).read_bytes())
    ir = raw['translated']
    options = ir['options']
    if (raw['has_errors'] or options['start_from'] != config['roots']
            or options['include'] != config['include'] or options['opaque'] != config['opaque']
            or options['exclude'] or any(options[name] for name in
                ('skip_borrowck', 'no_typecheck', 'monomorphize'))
            or {row['key'] for row in ir['target_information']} != {config['target']}):
        raise ValueError('unexpected standard read extraction scope or compiler checks')
    functions = {row['def_id']: row for row in ir['fun_decls'] if row}
    called = {}
    for name in config['roots']:
        row = helpers.declaration(ir['fun_decls'], name)
        call, = list(helpers.ir_values(row['body'], 'Call'))
        target, = list(helpers.regular_calls({'Call': call}))
        callee = functions[target]
        path = callee['item_meta']['name']
        if (row['item_meta']['opacity'] != 'Transparent' or set(row['body']) != {'Structured'}
                or callee['item_meta']['opacity'] != 'Transparent' or set(callee['body']) != {'Structured'}
                or path[:4] != row['item_meta']['name'][:4]
                or len(path) != 6 or 'Impl' not in path[4]
                or path[5] != {'Ident': [name.split('proof_standard_record_')[1], 0]}):
            raise ValueError('standard read forwarder changed its actual transparent target')
        called[name] = callee['def_id']
    translation = json.loads((directory / 'generated/translation.json').read_bytes())
    opaque = [row['lean_name'] for row in translation['functions'] if row['is_opaque']]
    if len(opaque) != len(EXTERNAL) or set(opaque) != EXTERNAL:
        raise ValueError('new opaque function in the actual read closure')
    for name in ('Types.lean', 'Funs.lean'):
        actual = directory / 'generated' / MODULE / name
        tokens = json.dumps(syntax.tokens(actual.read_bytes()), separators=(',', ':')).encode()
        if hashlib.sha256(tokens).hexdigest() != config['lean_token_sha256'][name]:
            raise ValueError('current generated standard read body differs: ' + name)
    return called


def capture(root, directory, config):
    archive = load('standard_read_source_archive', root / STAGE / 'archive.py')
    runtime = load('standard_read_fixed_runtime', root / STAGE / 'formal_runtime.py')
    directory.mkdir(mode=0o700)
    tar, manifest = directory / 'source.tar', directory / 'source-manifest.json'
    source = archive.build(root, tar, manifest)
    if archive.verify(tar, manifest) != source:
        raise ValueError('current standard read archive changed')
    project = directory / 'source'
    project.mkdir(mode=0o700)
    with tarfile.open(tar) as contents:
        contents.extractall(project, filter='data')
    installed = runtime.tool_root(root) / 'aeneas'
    binaries = {'charon': installed / 'charon/bin/charon', 'aeneas': installed / 'bin/aeneas'}
    if any(archive.sha(path) != config['tools'][name]['binary_sha256'] for name, path in binaries.items()):
        raise ValueError('fixed standard read tool differs')
    compile_options = [argument for flag, key in (
        ('--include', 'include'), ('--opaque', 'opaque'), ('--start-from', 'roots')
    ) for entry in config[key] for argument in (flag, entry)]
    raw, generated = directory / RAW, directory / 'generated'
    steps = (
        ('charon', [binaries['charon'], 'cargo', '--preset=aeneas', '--error-on-warnings',
                    '--sysroot', 'default', *compile_options, '--dest-file', raw, '--',
                    '--manifest-path', CRATE, '--lib', '--locked', '--offline', '--target', config['target']]),
        ('aeneas', [binaries['aeneas'], '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
                    '-split-files', '-emit-json', '-namespace', 'SandproofStandardRecordRust',
                    '-subdir', MODULE, '-dest', generated, raw]),
    )
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='4',
               CARGO_TARGET_DIR=str(directory / 'target'))
    for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(name, None)
    report = dict(schema=2, complete=False, commit=source['commit'], tree=source['tree'],
                  source_sha256=source['archive_sha256'], tools=config['tools'], commands=[])
    recording = project / SOURCE
    original = recording.read_bytes()
    forwarder = (project / HERE / 'root.rs.in').read_bytes()
    try:
        recording.write_bytes(original + b'\n' + forwarder)
        for name, args in steps:
            if name == 'aeneas':
                recording.write_bytes(original)
            args = list(map(str, args))
            log = directory / (name + '.log')
            with log.open('xb') as stream:
                result = subprocess.run(args, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                        stdout=stream, stderr=subprocess.STDOUT, timeout=900)
            report['commands'].append(dict(name=name, argv=args, exit_code=result.returncode,
                                           log_sha256=archive.sha(log)))
            result.check_returncode()
        report['targets'] = admit(root, directory, config)
        report['files'] = {str(path.relative_to(directory)): archive.sha(path) for path in (
            raw, tar, manifest, *sorted(p for p in generated.rglob('*') if p.is_file()))}
        report['complete'] = True
    finally:
        recording.write_bytes(original)
        if archive.sha(recording) != source['files'][str(SOURCE)]['sha256']:
            report['complete'] = False
            raise ValueError('standard read source restoration failed')
        (directory / 'result.json').write_text(json.dumps(report, indent=2) + '\n')


def evidence(root, directory, config, sha):
    archive = load('standard_read_archive_validator', root / STAGE / 'archive.py')
    receipt = directory / 'result.json'
    report = json.loads(receipt.read_bytes())
    source = archive.verify(directory / 'source.tar', directory / 'source-manifest.json')
    if (report.get('schema') != 2 or report.get('complete') is not True
            or report['commit'] != source['commit'] or report['tree'] != source['tree']
            or report['source_sha256'] != source['archive_sha256'] or report['tools'] != config['tools']
            or source['commit'] != archive.git(root, 'rev-parse', 'HEAD').decode().strip()
            or source['tree'] != archive.git(root, 'rev-parse', 'HEAD^{tree}').decode().strip()
            or [(row['name'], row['exit_code']) for row in report['commands']] !=
                [('charon', 0), ('aeneas', 0)]):
        raise ValueError('incomplete current standard read capture')
    snapshot, _ = archive.committed_inputs(root)
    if source['files'] != snapshot['files']:
        raise ValueError('current standard read source archive differs from the candidate')
    observed = {name: source['files'][name] for name in config['source_inputs']}
    if any(row['sha256'] != config['source_inputs'][name]
           or row['mode'] != config['source_modes'][name] for name, row in observed.items()):
        raise ValueError('current standard read consumed a different source')
    expected = {RAW, 'source.tar', 'source-manifest.json', 'generated/translation.json'} | {
        f'generated/{MODULE}/{name}' for name in
        ('Types.lean', 'Funs.lean', 'TypesExternal_Template.lean', 'FunsExternal_Template.lean')}
    if set(report['files']) != expected:
        raise ValueError('missing or extra standard read extraction artifact')
    docs = dict(report['files'])
    docs.update({row['name'] + '.log': row['log_sha256'] for row in report['commands']})
    result = {str(directory / name): digest for name, digest in docs.items()}
    if any(sha(Path(name)) != digest for name, digest in result.items()):
        raise ValueError('current standard read extraction evidence changed')
    if report['targets'] != admit(root, directory, config):
        raise ValueError('current standard read forwarder evidence changed')
    result[str(receipt)] = sha(receipt)
    return result
