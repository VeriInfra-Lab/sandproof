"""Capture the observation parent, its actual save adapter, and the same writer."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

import source_row_sql_composition as row
import source_save_row_list_capture as save
import observation_slot_capture as slot
from frontier_capture import call_kind, indexed
from prepare_observation_slot import parent_relation
from revision_capture import ir_values, regular_calls
from prepare_transaction_capture import checked_translation
from probe_prepare_iterators import GIB, run
from source_save_order import check_adapter
from revision_capture import declaration


SOURCE, TREE, ARCHIVE, STAGED = save.SOURCE, save.TREE, save.ARCHIVE, save.STAGED
NAMESPACE = 'SandproofObservationSqlSave'
WORK = Path(slot.finish.prepare.SOURCE)
root = slot.configuration('parent')
includes = [save.ROOT, save.SAVE_USING, save.EFFECTS, save.WRITE_ROWS, row.PARENT, row.CORE,
            row.CODEC_ROOT + '::JournalSourceSave']
includes += [row.CODEC_ROOT + '::*::' + name for name in (
    'read', 'limit', 'encode', 'retains', 'write')]
CFG = dict(roots=root['roots'], retained_roots=root['retained_roots'],
           include=list(dict.fromkeys([*root['include'], *includes])),
           opaque=[name for name in root['opaque'] if name != save.ROOT])


def admit(ir, generated, target):
    retained, = CFG['retained_roots']
    translated = checked_translation(ir, CFG, target, CFG['roots'][0], retained)
    functions = translated['fun_decls']
    selected = slot.check_parent(translated)
    relation = parent_relation(translated, selected)
    codec, using, rows, writer = (declaration(functions, name) for name in
        (save.ROOT, save.SAVE_USING, save.WRITE_ROWS, row.PARENT))
    parent = indexed(functions, selected['parent'])['body']['Structured']['body']
    (finish,) = (item['call'] for item in ir_values(parent, 'Call')
                 if call_kind(item['call']).get('Fun') == selected['tail'])
    save_trait = declaration(translated['trait_decls'], slot.WORK + '::SaveSourceInputs')['def_id']
    (dictionary,) = (item['Value'][1] for item in finish['func']['Regular']['generics']['trait_refs']
                     if 'Value' in item and item['Value'][1]['trait_decl_ref']['skip_binder']['id'] == save_trait)
    instance = indexed(translated['trait_impls'], dictionary['kind']['TraitImpl']['id'])
    (method,) = instance['methods']
    adapter = indexed(functions, method['skip_binder']['id'])
    if (method['kind'] != {'TraitMethod': [save_trait, 0]}
            or adapter['item_meta']['opacity'] != 'Transparent'
            or list(regular_calls(adapter['body']['Structured']['body'])) != [codec['def_id']]):
        raise ValueError('the same selected dictionary must call the real codec')
    trait = declaration(translated['trait_decls'], save.EFFECTS)['def_id']
    phases = check_adapter(translated, trait, using['def_id'], codec['body']['Structured']['body'])
    (write,) = [value['call'] for value in ir_values(rows['body']['Structured']['body'], 'Call')
                if call_kind(value['call']).get('Fun') == writer['def_id']]
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    for item, count in ((adapter, 1), (codec, 1), (using, 1), (rows, 3), (writer, 1)):
        emitted = [entry for entry in mapping if entry['def_id'] == item['def_id']]
        if len(emitted) != count or any(entry['is_opaque'] for entry in emitted):
            raise ValueError('all links through the original codec and row loop must remain transparent')
    return dict(parent=selected, relation=relation, adapter=adapter['def_id'], codec=codec['def_id'],
                phases=phases, writer=writer['def_id'], original_operands=write['args'])


def capture(out, tools, lock):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or min(shutil.disk_usage(path).free for path in (out.parent, row.ROOT)) < 80 * GIB):
        raise ValueError('original start budget is required')
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    os.umask(0o077)
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    fixed = json.loads((row.ROOT / 'verification/foundations-environment.json').read_bytes())
    for name, binary in (('charon', charon), ('aeneas', aeneas)):
        if row.digest(binary) != fixed[name]['binary_sha256']:
            raise ValueError('fixed tools required')
    if subprocess.check_output(['git', 'rev-parse', SOURCE + '^{tree}'], cwd=row.ROOT, text=True).strip() != TREE:
        raise ValueError('fixed original source tree required')
    out.mkdir()
    source = out / 'source'
    source.mkdir()
    archive = out / 'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git', 'archive', SOURCE], cwd=row.ROOT, stdout=stream, check=True)
    if row.digest(archive) != ARCHIVE:
        raise ValueError('the original source archive differs')
    with tarfile.open(archive) as inputs:
        inputs.extractall(source, filter='data')
    for name, original in slot.finish.prepare.SOURCE_INPUTS.items():
        if row.digest(source / name) != original:
            raise ValueError('the original parent input differs: ' + name)
    codec = row.stage(source, 'core')
    if codec != STAGED:
        raise ValueError('the reviewed SQL staging differs')
    method = source / WORK
    method.write_text(slot.factor(method.read_text(), 'parent'))
    selectors = [part for flag, group in (('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
                                          ('--include', 'include'), ('--opaque', 'opaque'))
                 for pattern in CFG[group] for part in (flag, pattern)]
    ir, generated = out / 'actual.llbc', out / 'generated'
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
               CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(out / 'target'))
    for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(key, None)
    commands = [
        ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                    *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                    '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
        ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                     '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE, '-dest', generated, ir], 120),
    ]
    report = {'source': SOURCE, 'tree': TREE, 'archive': row.digest(archive),
              'codec_sha256': codec, 'work_sha256': row.digest(method), 'options': CFG, 'driver': row.digest(Path(__file__)), 'commands': [], 'complete': False}
    try:
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for name, command, limit in commands:
                item = run(name, command, limit, source, out, env)
                report['commands'].append(item)
                if item['exit_code'] or item['bounded_stop']:
                    raise ValueError(name + ' did not complete')
                if name == 'charon':
                    report['ir_sha256'] = row.digest(ir)
        report['generated_sha256'] = {str(p.relative_to(generated)): row.digest(p)
                                      for p in generated.rglob('*') if p.is_file()}
        report['admission'] = admit(ir, generated, fixed['target'])
        report['complete'] = True
    finally:
        (out / 'result.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True))
