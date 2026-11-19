"""Extract the real single-row save orchestration and its zero-update control."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

from frontier_capture import call_kind, indexed
from prepare_transaction_capture import checked_translation
from probe_prepare_iterators import GIB, run
from revision_capture import declaration, ir_values
from source_retention_providers import ARCHIVE, CODEC, CODEC_ROOT, ROOT, SOURCE, TREE, digest, stage_port


HERE = Path(__file__).resolve().parent / 'row-write'
NAMESPACE = 'SandproofSourceRowWrite'
CORE = CODEC_ROOT + '::write_candidate_record_using'
PARENT = CODEC_ROOT + '::write_candidate_record'
MODES = {'core': CORE, 'zero-ok': CORE, 'parent': PARENT}
PORTS = ('existing', 'vacant', 'digest', 'update', 'capacity', 'corrupt')
CODEC_DIGESTS = {
    'core': 'b7cd78749e67bf12ed39c1c6dbc738af618607891516b07e8d398f64720c6d45',
    'zero-ok': '993f36c7297ec974cfa13447450ae1527c33d296e85bffc26581ff5de534afad',
}


def stage(source, mode):
    if mode not in MODES:
        raise ValueError('unknown row-write capture')
    stage_port(source, 'providers')
    file = source / CODEC
    old = file.read_text()
    start = old.index('fn write_candidate_record(\n')
    end = old.index('// Only final native Retire', start)
    original = old[start:end]
    row = (HERE / 'row.rs.in').read_text()
    for statement in (
            'SELECT id FROM controller_source_work_records WHERE work_key=?1',
            'SELECT min(id) FROM controller_source_work_records WHERE work_key IS NULL',
            'UPDATE controller_source_work_records SET state_json=?1,sha256=?2,work_key=?3 WHERE id=?4 AND (work_key IS NULL OR work_key=?3)'):
        if original.count(statement) != 1 or row.count(statement) != 1:
            raise ValueError('the original SQL literal must be preserved: ' + statement)
    if mode == 'zero-ok':
        check = 'Effects::update(connection, bytes, &sha, &key, id)? != 1'
        if row.count(check) != 1:
            raise ValueError('the real update count check must occur once')
        row = row.replace(check, check.replace('!=', '>'))
    file.write_text(old[:start] + row + '\n' + old[end:])
    actual = digest(file)
    if actual != CODEC_DIGESTS['zero-ok' if mode == 'zero-ok' else 'core']:
        raise ValueError('the reviewed concrete SQL adapter and row control changed')
    return actual


def configuration(mode):
    cfg = dict(roots=[MODES[mode]], retained_roots=[], include=[CORE], opaque=[])
    if mode == 'parent':
        cfg['include'] += [PARENT]
        cfg['opaque'] += ['rusqlite', 'sha2', 'sandproof_controller_service::JournalError',
                          *(CODEC_ROOT + '::*::' + port for port in PORTS)]
    return cfg


def parent_operands(translated, parent, core, trait):
    body = parent['body']['Structured']['body']
    deref, save = (row['call'] for row in ir_values(body, 'Call'))
    functions = translated['fun_decls']
    borrowed = indexed(functions, call_kind(deref)['Fun'])['item_meta']['name']
    if borrowed[-1] != {'Ident': ['deref', 0]} or call_kind(save) != {'Fun': core} or save['dest']['kind'] != {'Local': 0}:
        raise ValueError('only the original Vec borrow and returned save call are expected')
    assignments = {left['kind']['Local']: right for left, right in ir_values(body, 'Assign')}

    def origin(place):
        kind = place['kind']
        if 'Projection' in kind:
            source, projection = kind['Projection']
            if projection != 'Deref':
                raise ValueError('a row adapter may only dereference the original inputs')
            return origin(source)
        local = kind['Local']
        if local not in assignments:
            return local
        value = assignments[local]
        if 'Ref' in value and value['Ref']['kind'] == 'Shared':
            return origin(value['Ref']['place'])
        if 'Use' in value and 'Copy' in value['Use'][0]:
            return origin(value['Use'][0]['Copy'])
        raise ValueError('unsupported row adapter operand')

    if ([origin(arg['Move']) for arg in deref['args']] != [3]
            or [origin(arg['Move']) for arg in save['args']] != [1, 2, deref['dest']['kind']['Local']]):
        raise ValueError('the exact connection, key and byte vector must reach the original row writer')
    ref, = (item['Value'][1] for item in save['func']['Regular']['generics']['trait_refs']
            if 'Value' in item and item['Value'][1]['trait_decl_ref']['skip_binder']['id'] == trait)
    instance = indexed(translated['trait_impls'], ref['kind']['TraitImpl']['id'])
    methods = {method['kind']['TraitMethod'][1]: method['skip_binder']['id'] for method in instance['methods']
               if method['kind']['TraitMethod'][0] == trait}
    if set(methods) != set(range(len(PORTS))) or any(
            indexed(functions, methods[slot])['item_meta']['name'][-1] != {'Ident': [name, 0]}
            for slot, name in enumerate(PORTS)):
        raise ValueError('the parent must select all six original SqliteSourceRow adapters')
    return {name: methods[slot] for slot, name in enumerate(PORTS)}


def admit(ir, generated, target, mode):
    translated = checked_translation(ir, configuration(mode), target, MODES[mode], None)
    functions = translated['fun_decls']
    core = declaration(functions, CORE)
    body = core['body']['Structured']['body']
    trait = declaration(translated['trait_decls'], CODEC_ROOT + '::SourceRowEffects')['def_id']
    refs = [entry for row in ir_values(translated, 'trait_refs') for entry in row]
    refs += [entry[0] for entry in ir_values(translated, 'Trait') if isinstance(entry, list)]
    resolved = {entry['Value'][0]: entry['Value'][1] for entry in refs
                if 'Value' in entry and 'trait_decl_ref' in entry['Value'][1]}
    ports = []
    for item in ir_values(body, 'Call'):
        kind = call_kind(item['call'])
        if 'Trait' in kind:
            reference, slot = kind['Trait']
            value = reference['Value'][1] if 'Value' in reference else resolved[reference['Deduplicated']]
            if value['trait_decl_ref']['skip_binder']['id'] == trait:
                ports.append(slot)
    if ports != [0, 1, 4, 2, 3, 5, 2, 3, 5]:
        raise ValueError('the actual first caller, fallback, hash, write and error constructors must remain ordered')
    names = json.loads((generated / 'translation.json').read_bytes())['functions']
    entry, = (row for row in names if row['def_id'] == core['def_id'])
    if entry['source']['file'] != str(CODEC) or entry['is_opaque'] or core['item_meta']['opacity'] != 'Transparent':
        raise ValueError('actual source row writer must stay transparent')
    result = dict(core=core['def_id'], ports=[PORTS[i] for i in ports])
    if mode == 'parent':
        parent = declaration(functions, PARENT)
        result['adapters'] = parent_operands(translated, parent, core['def_id'], trait)
        result['parent'] = parent['def_id']
    return result


def capture(mode, output, tools, lock):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or shutil.disk_usage(output.parent).free < 80 * GIB or shutil.disk_usage(ROOT).free < 64 * GIB):
        raise ValueError('proof role and actual capture filesystem budget required')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    os.umask(0o077)
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(digest(path) != fixed[name]['binary_sha256'] for name, path in (('charon', charon), ('aeneas', aeneas))):
        raise ValueError('fixed tools required')
    if subprocess.check_output(['git', 'rev-parse', SOURCE + '^{tree}'], text=True, cwd=ROOT).strip() != TREE:
        raise ValueError('original integration tree differs')
    output.mkdir()
    source = output / 'source'
    source.mkdir()
    report = dict(mode=mode, source_commit=SOURCE, source_tree=TREE, complete=False, commands=[],
                  public_acceptance=False, driver_sha256=digest(Path(__file__)), cpus=cpus)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', SOURCE], cwd=ROOT, stdout=stream, check=True)
        if digest(archive) != ARCHIVE:
            raise ValueError('original integration archive differs')
        with tarfile.open(archive) as inputs:
            inputs.extractall(source, filter='data')
        report['codec_sha256'] = stage(source, mode)
        cfg = configuration(mode)
        args = [flag for option, names in (('--include', cfg['include']), ('--opaque', cfg['opaque']))
                for name in names for flag in (option, name)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                        '--start-from', MODES[mode], *args, '--dest-file', ir, '--', '--manifest-path',
                        'controller-service/Cargo.toml', '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                        '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE, '-dest', generated, ir], 120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for label, argv, seconds in commands:
                result = run(label, argv, seconds, source, output, env)
                report['commands'].append(result)
                if result['exit_code'] != 0 or result['bounded_stop']:
                    raise ValueError(label + ' did not complete')
                if label == 'charon':
                    report['ir_sha256'] = digest(ir)
        if digest(ir) != report['ir_sha256']:
            raise ValueError('actual IR changed during capture')
        report['admission'] = admit(ir, generated, fixed['target'], mode)
        report['generated_sha256'] = {str(file.relative_to(generated)): digest(file)
                                      for file in sorted(generated.rglob('*')) if file.is_file()}
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(mode, report['admission'])


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', choices=MODES, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.mode, args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True))
