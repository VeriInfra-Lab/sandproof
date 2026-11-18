"""Preserve the first matching complete source record in the real save path."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

from frontier_capture import call_kind, indexed
from prepare_transaction_capture import WORK, checked_translation
from probe_prepare_iterators import GIB, ROOT, run
from revision_capture import declaration, ir_values, regular_calls
from source_save_order import ARCHIVE, CODEC, CODEC_ROOT, SOURCE, TREE, digest, stage as stage_save


HERE = Path(__file__).resolve().parent / 'first-match'
ORIGINAL = '''fn retains_saved_records(current: &Sources, sources: &Sources, keys: &BTreeSet<[u8; 32]>) -> bool {
    // Existing responsibilities cannot be dropped by a general source update.
    // The later GC settlement must have its own checked removal transaction.
    !current.records.iter().any(|record| {
        !keys.contains(record.work.requirement.work_binding.as_bytes())
            || sources
                .records
                .iter()
                .find(|next| next.work == record.work)
                .is_none_or(|next| !super::retirement::preserves_saved_facts(record, next))
    })
}
'''
MODES = ('factored', 'binding-only')
INNER = CODEC_ROOT + '::retains_record_using'
OUTER = CODEC_ROOT + '::retains_inventory_using'
PARENT = CODEC_ROOT + '::retains_saved_records'
TRAIT = CODEC_ROOT + '::SavedRecordChecks'
METHODS = ('has_key', 'same_work', 'same_binding', 'preserves')
NAMESPACE = 'SandproofSourceFirstMatch'
DIGESTS = {
    'factored': 'ac11351a2e3e528fd4e9baabbe7a619cefcbcfd6d9a87ebea788ffaaef49d91f',
    'binding-only': 'c8b1d4501e788eba76097e447de18a7e636e9612d5b31fa8f6c1d6fea66522b2',
}


def stage(source, mode):
    if mode not in MODES:
        raise ValueError('unknown source record search view')
    stage_save(source, 'factored')
    path = source / CODEC
    text = path.read_text()
    if text.count(ORIGINAL) != 1:
        raise ValueError('original first complete match and retention required')
    body = (HERE / 'retains.rs.in').read_text()
    if mode == 'binding-only':
        tested = 'if Check::same_work(next, record) {'
        if body.count(tested) != 1:
            raise ValueError('actual complete comparison must occur once')
        body = body.replace(tested, 'if Check::same_binding(next, record) {')
    path.write_text(text.replace(ORIGINAL, body))
    sha = digest(path)
    if sha != DIGESTS[mode]:
        raise ValueError('original first match candidate differs')
    return sha


def configuration(mode):
    cfg = dict(roots=[OUTER], retained_roots=[], include=[OUTER, INNER],
               opaque=['sandproof_controller_service', 'sandbox_controller_core',
                       'sandproof_protocol', 'rusqlite', 'serde_json'])
    if mode == 'parent':
        cfg['roots'] = [PARENT]
        cfg['include'] += [PARENT, CODEC_ROOT + '::Sources', WORK + '::SourceWorkRecord',
                          WORK + '::SourceWork', 'sandproof_controller_service::capture::source::SourceObservationRequirement',
                          'sandproof_protocol::lifecycle::digest::ContentDigest',
                          'sandproof_protocol::lifecycle::digest::*::as_bytes',
                          *(CODEC_ROOT + '::*::' + name for name in METHODS)]
        cfg['opaque'] += [WORK + '::retirement::preserves_saved_facts']
    return cfg


def borrowed_paths(body, call):
    bindings = {place['kind']['Local']: value['Ref'] for place, value in ir_values(body, 'Assign')}

    def origin(place):
        kind = place['kind']
        if 'Local' in kind:
            local = kind['Local']
            if local not in bindings:
                return (local,)
            reference = bindings[local]
            if reference['kind'] != 'Shared':
                raise ValueError('expected a shared borrow of the original record')
            return origin(reference['place'])
        source, projection = kind['Projection']
        if projection == 'Deref':
            return origin(source)
        return (*origin(source), projection['Field'][1])

    return [origin(arg['Move']) for arg in call['args']]


def check_parent(translated, parent, outer, trait):
    functions = translated['fun_decls']
    body = parent['body']['Structured']['body']
    call, = (row['call'] for row in ir_values(body, 'Call'))
    if call_kind(call) != {'Fun': outer} or call['dest']['kind'] != {'Local': 0}:
        raise ValueError('parent must return the actual retention scan')
    reference, = (r['Value'][1] for r in call['func']['Regular']['generics']['trait_refs'] if 'Value' in r
                  and r['Value'][1]['trait_decl_ref']['skip_binder']['id'] == trait)
    instance = indexed(translated['trait_impls'], reference['kind']['TraitImpl']['id'])
    keep = declaration(functions, WORK + '::retirement::preserves_saved_facts')['def_id']
    effects = {entry['kind']['TraitMethod'][1]: indexed(functions, entry['skip_binder']['id'])
               for entry in instance['methods'] if entry['kind']['TraitMethod'][0] == trait}
    if (set(effects) != set(range(len(METHODS)))
            or any(row['item_meta']['opacity'] != 'Transparent' for row in effects.values())
            or list(regular_calls(effects[3]['body']['Structured']['body'])) != [keep]):
        raise ValueError('the real dictionary must delegate to the original saved facts check')
    types = translated['type_decls']
    sources = declaration(types, CODEC_ROOT + '::Sources')['kind']['Struct']
    records = next(i for i, field in enumerate(sources) if field['name'] == 'records')
    record = declaration(types, WORK + '::SourceWorkRecord')['kind']['Struct']
    work = next(i for i, field in enumerate(record) if field['name'] == 'work')
    if borrowed_paths(body, call) != [(1, records), (2, records), (3,)]:
        raise ValueError('the parent must pass both entire record vectors and the original set')
    for index, paths in ((1, [(1, work), (2, work)]), (3, [(1,), (2,)])):
        body = effects[index]['body']['Structured']['body']
        member, = (item['call'] for item in ir_values(body, 'Call'))
        if member['dest']['kind'] != {'Local': 0} or borrowed_paths(body, member) != paths:
            raise ValueError('original comparison and guard must forward the complete values in place')
        if index == 1:
            equality = indexed(functions, call_kind(member)['Fun'])
            if equality['item_meta']['name'][-1] != {'Ident': ['eq', 0]}:
                raise ValueError('the complete work must flow to the real equality')
    return {name: effects[index]['def_id'] for index, name in enumerate(METHODS)}


def admit(ir, generated, target, mode):
    cfg = configuration(mode)
    root, = cfg['roots']
    translated = checked_translation(ir, cfg, target, root, None)
    functions = translated['fun_decls']
    inner, outer = (declaration(functions, name) for name in (INNER, OUTER))
    trait = declaration(translated['trait_decls'], TRAIT)['def_id']
    references = [entry for row in ir_values(translated, 'trait_refs') for entry in row]
    references += [entry[0] for entry in ir_values(translated, 'Trait') if isinstance(entry, list)]
    by_id = {item['Value'][0]: item['Value'][1] for item in references
             if 'Value' in item and 'trait_decl_ref' in item['Value'][1]}
    checks = []
    for row in ir_values(inner['body']['Structured']['body'], 'Call'):
        kind = call_kind(row['call'])
        if 'Trait' not in kind:
            continue
        reference, slot = kind['Trait']
        if 'Value' in reference:
            value = reference['Value'][1]
        else:
            value = by_id[reference['Deduplicated']]
        if value['trait_decl_ref']['skip_binder']['id'] == trait:
            checks.append(METHODS[slot])
    middle = 'same_binding' if mode == 'binding-only' else 'same_work'
    if checks != ['has_key', middle, 'preserves'] or list(regular_calls(
            outer['body']['Structured']['body'])).count(inner['def_id']) != 1:
        raise ValueError('original full work lookup and preserved-record call order differ')
    names = json.loads((generated / 'translation.json').read_bytes())['functions']
    for item in (inner, outer):
        mapped = [r for r in names if r['def_id'] == item['def_id']]
        if (item['item_meta']['opacity'] != 'Transparent' or not mapped
                or any(row['is_opaque'] or row['source']['file'] != str(CODEC) for row in mapped)):
            raise ValueError('required actual source retention body')
    result = dict(comparison=middle, inner=inner['def_id'], outer=outer['def_id'])
    if mode == 'parent':
        result['original_methods'] = check_parent(translated, declaration(functions, PARENT), outer['def_id'], trait)
    return result


def capture(mode, output, tools, lock):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or shutil.disk_usage(output.parent).free < 80 * GIB
            or shutil.disk_usage(ROOT).free < 64 * GIB):
        raise ValueError('proof role and actual capture filesystem budget required')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    os.umask(0o077)
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(digest(path) != fixed[name]['binary_sha256']
           for name, path in (('charon', charon), ('aeneas', aeneas))):
        raise ValueError('fixed tools required')
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
        with tarfile.open(archive) as inputs:
            inputs.extractall(source, filter='data')
        report['codec_sha256'] = stage(source, 'factored' if mode == 'parent' else mode)
        cfg = configuration(mode)
        options = [item for key, flag in (('roots', '--start-from'), ('include', '--include'), ('opaque', '--opaque'))
                   for value in cfg[key] for item in (flag, value)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                        *options, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                        '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
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
            raise ValueError('actual first-match IR changed')
        report['admission'] = admit(ir, generated, fixed['target'], mode)
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
