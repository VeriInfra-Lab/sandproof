"""Expose the original source save phases as typed call parameters."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

from frontier_capture import call_kind, indexed
from probe_prepare_iterators import GIB, ROOT, run
from revision_capture import declaration, ir_values, regular_calls
from source_save_capture import ARCHIVE, CODEC, CODEC_ROOT, MODES, SAVE, SOURCE, TREE, digest
from source_save_capture import configuration as concrete_configuration, stage as stage_save
from prepare_transaction_capture import checked_translation


HERE = Path(__file__).resolve().parent / 'save-order'
ORIGINAL_SOURCES = '''pub(super) struct Sources {
    pub config: SourceObservationConfig,
    pub node: sandproof_protocol::internal::NodeIdentity,
    pub max_sessions: u16,
    pub records: Vec<SourceWorkRecord>,
}'''
TYPED_SOURCES = '''pub(super) struct Sources<
    C = SourceObservationConfig,
    N = sandproof_protocol::internal::NodeIdentity,
    R = SourceWorkRecord,
> {
    pub config: C,
    pub node: N,
    pub max_sessions: u16,
    pub records: Vec<R>,
}'''
HEADER = '''pub(super) fn save(
    connection: &Connection,
    sources: &Sources,
    max_sessions: u16,
) -> Result<(), JournalError> {
'''
PHASES = {
    'load(connection)': 'Save::read(connection)',
    'reserved_records(connection, max_sessions)': 'Save::limit(connection, max_sessions)',
    'encode_candidate_records(sources)': 'Save::encode(sources)',
    'retains_saved_records(&current, sources, &keys)': 'Save::retains(&current, sources, &keys)',
    'write_candidate_records(connection, encoded)': 'Save::write(connection, encoded)',
}
SAVE_USING = CODEC_ROOT + '::save_using'
EFFECTS = CODEC_ROOT + '::SourceSaveEffects'
METHODS = ('read', 'limit', 'encode', 'retains', 'write')
ADAPTER_CALLS = ('load', 'reserved_records', 'encode_candidate_records',
                 'retains_saved_records', 'write_candidate_records')
NAMESPACE = 'SandproofSourceSaveOrder'
DIGESTS = {
    'factored': '73e3d7ad830db718e319146a822be76e80ca9ab508f85376876950d5c740cb19',
    'reordered': '482250ed534de41706a38d2b79ba603a6447b7595d6595ebfcc75c4883e5379e',
}


def factor(text):
    prefix, body = text.split(HEADER, 1)
    caller, rest = body.split('\nfn encode_candidate_records(', 1)
    if prefix.count(ORIGINAL_SOURCES) != 1 or not caller.endswith('}\n'):
        raise ValueError('actual source rows and save body required')
    for previous, forwarded in PHASES.items():
        if caller.count(previous) != 1:
            raise ValueError('exact source save phase required: ' + previous)
        caller = caller.replace(previous, forwarded)
    return (prefix.replace(ORIGINAL_SOURCES, TYPED_SOURCES) + (HERE / 'ports.rs.in').read_text()
            + caller + '\nfn encode_candidate_records(' + rest)


def stage(source, mode):
    if mode not in MODES:
        raise ValueError('expected a factored or reordered source save')
    stage_save(source, mode)
    path = source / CODEC
    path.write_text(factor(path.read_text()))
    sha = digest(path)
    if sha != DIGESTS[mode]:
        raise ValueError('exact typed source save required')
    return sha


def configuration(mode):
    cfg = concrete_configuration()
    cfg['roots'] = [SAVE if mode == 'parent' else SAVE_USING]
    cfg['include'] += [SAVE_USING]
    if mode == 'parent':
        cfg['include'] += [CODEC_ROOT + '::*::' + name for name in METHODS]
    return cfg


def check_adapter(translated, effects, method, body):
    functions = translated['fun_decls']
    invocation, = (row['call'] for row in ir_values(body, 'Call')
                   if call_kind(row['call']).get('Fun') == method)
    parameters = invocation['func']['Regular']['generics']['trait_refs']
    candidate, = (row['Value'][1] for row in parameters if 'Value' in row
                  and row['Value'][1]['trait_decl_ref']['skip_binder']['id'] == effects)
    instance = indexed(translated['trait_impls'], candidate['kind']['TraitImpl']['id'])
    found = []
    for index, rust_call in enumerate(ADAPTER_CALLS):
        entry, = (entry for entry in instance['methods'] if entry['kind'] == {'TraitMethod': [effects, index]})
        adapter = indexed(functions, entry['skip_binder']['id'])
        original = declaration(functions, CODEC_ROOT + '::' + rust_call)['def_id']
        if (adapter['item_meta']['opacity'] != 'Transparent'
                or list(regular_calls(adapter['body']['Structured']['body'])) != [original]):
            raise ValueError('native save must forward to the original ' + rust_call)
        found.append(original)
    return found


def admit(ir, generated, cfg, target, mode):
    root, = cfg['roots']
    translated = checked_translation(ir, cfg, target, root, None)
    functions = translated['fun_decls']
    method = declaration(functions, SAVE_USING)
    trait = declaration(translated['trait_decls'], EFFECTS)['def_id']
    references = {}
    called = []
    for row in ir_values(method['body']['Structured']['body'], 'Call'):
        kind = call_kind(row['call'])
        if 'Trait' not in kind:
            continue
        ref, index = kind['Trait']
        if 'Value' in ref:
            key, value = ref['Value']
            references[key] = value
        else:
            value = references[ref['Deduplicated']]
        if value['trait_decl_ref']['skip_binder']['id'] == trait:
            called.append(METHODS[index])
    order = (*METHODS[:-2], METHODS[-1], METHODS[-2]) if mode == 'reordered' else METHODS
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    matched, = (row for row in mapping if row['def_id'] == method['def_id'])
    if (tuple(called) != order or method['item_meta']['opacity'] != 'Transparent'
            or matched['is_opaque'] is not False or matched['source']['file'] != str(CODEC)):
        raise ValueError('original typed save and actual phase order are required')
    report = dict(save=method['def_id'], order=list(order), lean=matched['lean_name'])
    if mode == 'parent':
        parent = declaration(functions, SAVE)
        report['original_calls'] = check_adapter(translated, trait, method['def_id'],
                                                 parent['body']['Structured']['body'])
    return report


def capture(mode, output, tools, lock):
    # The capture workspace may be on a separate volume. Keep the original
    # persistent-workspace stop threshold in the shared bounded runner.
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or shutil.disk_usage(output.parent).free < 80 * GIB
            or shutil.disk_usage(ROOT).free < 64 * GIB):
        raise ValueError('proof role and actual capture filesystem budget required')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    os.umask(0o077)
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    for label, path in (('charon', charon), ('aeneas', aeneas)):
        if digest(path) != fixed[label]['binary_sha256']:
            raise ValueError('fixed extraction tool differs: ' + label)
    if subprocess.check_output(['git', 'rev-parse', SOURCE + '^{tree}'], text=True, cwd=ROOT).strip() != TREE:
        raise ValueError('original integration tree differs')
    output.mkdir()
    source = output / 'source'
    source.mkdir()
    report = dict(mode=mode, source_commit=SOURCE, source_tree=TREE, complete=False, commands=[],
                  driver_sha256=digest(Path(__file__)), cpus=cpus, public_acceptance=False)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', SOURCE], cwd=ROOT, stdout=stream, check=True)
        if digest(archive) != ARCHIVE:
            raise ValueError('original integration archive differs')
        with tarfile.open(archive) as contents:
            contents.extractall(source, filter='data')
        report['codec_sha256'] = stage(source, 'factored' if mode == 'parent' else mode)
        cfg = configuration(mode)
        selectors = [item for flag, key in [('--start-from', 'roots'), ('--include', 'include'),
                     ('--opaque', 'opaque')] for value in cfg[key] for item in (flag, value)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            commands = [
                ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                            *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                            '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
                ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
                            '-split-files', '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE,
                            '-dest', generated, ir], 120),
            ]
            for label, argv, seconds in commands:
                receipt = run(label, argv, seconds, source, output, env)
                report['commands'].append(receipt)
                if receipt['exit_code'] != 0 or receipt['bounded_stop']:
                    raise ValueError(label + ' did not complete')
                if label == 'charon':
                    report['ir_sha256'] = digest(ir)
        if digest(ir) != report['ir_sha256']:
            raise ValueError('original typed save IR changed')
        report['admission'] = admit(ir, generated, cfg, fixed['target'], mode)
        report['generated_sha256'] = {str(path.relative_to(generated)): digest(path)
                                      for path in sorted(generated.rglob('*')) if path.is_file()}
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(mode, report['codec_sha256'], report['admission'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=(*MODES, 'parent'), required=True)
    for name in ('output', 'tools', 'lock'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    capture(args.mode, args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True))
