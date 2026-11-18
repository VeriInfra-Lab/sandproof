"""Capture the retention caller and both existing business providers in one type view."""
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
from probe_prepare_iterators import GIB, run
from revision_capture import declaration, ir_values
from source_first_match import (
    ARCHIVE, CODEC, CODEC_ROOT, DIGESTS, HERE as FIRST_MATCH, OUTER, PARENT, ROOT, SOURCE, TRAIT, TREE,
    borrowed_paths, digest, stage,
)


NAMESPACE = 'SandproofSourceRetentionProviders'
HERE = Path(__file__).resolve().parent / 'retention-providers'
PROVIDERS = ROOT / 'verification/experiments/stage1-protocol'
INNER = CODEC_ROOT + '::retains_with_saved_keys'
PROBE = 'crate::proof_source_work_eq'
MODES = {'providers': INNER, 'binding-only': INNER, 'parent': PARENT}
CODEC_DIGESTS = {
    'providers': '4d14975425b749c2d5cc423326c0f5a10eeef3ed350e17f67ccb173c6b2db14f',
    'binding-only': '36ed19d3ba79487946c2fe5cf93b234c56fee4c07fb6a32e75fe2c6492e08cf8',
}
PROBE_SOURCE = Path('controller-service/src/lib.rs')
PROBE_DIGEST = '87a6ee953ed5aa14fc408283515dc771847a2fa529b2c2380346cba55432a8b3'


def configuration(mode):
    return dict(roots=[MODES[mode]], retained_roots=[PROBE],
                include=['sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service', 'core::tuple'],
                opaque=[])


def stage_port(source, mode):
    if mode not in MODES:
        raise ValueError('unknown retention provider view')
    if stage(source, 'factored') != DIGESTS['factored']:
        raise ValueError('the original caller must be byte-identical to #944')
    codec = source / CODEC
    original = codec.read_text()
    old = (FIRST_MATCH / 'retains.rs.in').read_text()
    if original.count(old) != 1:
        raise ValueError('expected the original complete #944 caller')
    unchanged = old[old.index('fn retains_inventory_using<'):]
    if mode == 'binding-only':
        match = 'if Check::same_work(next, record) {'
        if unchanged.count(match) != 1:
            raise ValueError('the actual complete-work comparison must occur once')
        unchanged = unchanged.replace(match, 'if Check::same_binding(next, record) {')
    port = (HERE / 'key-port.rs.in').read_text()
    codec.write_text(original.replace(old, port + '\n\n' + unchanged))
    with (source / PROBE_SOURCE).open('ab') as file:
        file.write((PROVIDERS / 'source-work-match/probe.rs.in').read_bytes())
    expected = CODEC_DIGESTS['binding-only' if mode == 'binding-only' else 'providers']
    if digest(codec) != expected or digest(source / PROBE_SOURCE) != PROBE_DIGEST:
        raise ValueError('reviewed Rust port or original comparison probe changed')
    return expected


def only_call(function):
    body = function['body']['Structured']['body']
    call, = (item['call'] for item in ir_values(body, 'Call'))
    if call['dest']['kind'] != {'Local': 0}:
        raise ValueError('the parent must return the selected original result')
    return body, call


def parent_key_lookup(translated, call):
    key_trait = declaration(translated['trait_decls'], CODEC_ROOT + '::SavedKeyCheck')['def_id']
    reference, = (entry['Value'][1] for entry in call['func']['Regular']['generics']['trait_refs']
                  if 'Value' in entry and entry['Value'][1]['trait_decl_ref']['skip_binder']['id'] == key_trait)
    instance = indexed(translated['trait_impls'], reference['kind']['TraitImpl']['id'])
    method, = (row['skip_binder']['id'] for row in instance['methods'] if row['kind']['TraitMethod'] == [key_trait, 0])
    functions = translated['fun_decls']
    body, lookup = only_call(indexed(functions, method))
    callee = indexed(functions, call_kind(lookup)['Fun'])['item_meta']['name']
    prefix = [{'Ident': [name, 0]} for name in ('alloc', 'collections', 'btree', 'set')]
    if callee[:4] != prefix or callee[-1] != {'Ident': ['contains', 0]} or borrowed_paths(body, lookup) != [(1,), (2,)]:
        raise ValueError('the real parent must delegate original key and bytes to BTreeSet::contains')
    return method


def admit(ir, generated, target, mode):
    translated = checked_translation(ir, configuration(mode), target, MODES[mode], PROBE)
    functions = translated['fun_decls']
    inner = declaration(functions, INNER)
    outer = declaration(functions, OUTER)
    trait = declaration(translated['trait_decls'], TRAIT)['def_id']
    body, dispatch = only_call(inner)
    if call_kind(dispatch) != {'Fun': outer['def_id']} or borrowed_paths(body, dispatch) != [(1,), (2,), (3,)]:
        raise ValueError('the original vectors and key carrier must reach the actual retention scan')
    instance, = (row for row in translated['trait_impls'] if row and row['impl_trait']['id'] == trait)
    methods = {entry['kind']['TraitMethod'][1]: indexed(functions, entry['skip_binder']['id'])
               for entry in instance['methods'] if entry['kind']['TraitMethod'][0] == trait}
    preserves = declaration(functions, WORK + '::retirement::preserves_saved_facts')
    names = json.loads((generated / 'translation.json').read_bytes())['functions']
    record = declaration(translated['type_decls'], WORK + '::SourceWorkRecord')['kind']['Struct']
    work = next(index for index, field in enumerate(record) if field['name'] == 'work')
    compare_site, compare_call = only_call(methods[1])
    preserve_site, preserve_call = only_call(methods[3])
    comparison = indexed(functions, call_kind(compare_call)['Fun'])
    if (borrowed_paths(compare_site, compare_call) != [(1, work), (2, work)]
            or call_kind(preserve_call) != {'Fun': preserves['def_id']}
            or borrowed_paths(preserve_site, preserve_call) != [(1,), (2,)]):
        raise ValueError('the original comparison and fact check must receive the complete values')
    for entry in (inner, outer, comparison, preserves, *methods.values()):
        if (entry['item_meta']['opacity'] != 'Transparent' or 'Structured' not in entry['body']
                or not any(not r['is_opaque'] for r in names if r['def_id'] == entry['def_id'])):
            raise ValueError('actual business provider must have a generated body')
    result = dict(inner=inner['def_id'], outer=outer['def_id'],
                  methods={str(i): row['def_id'] for i, row in sorted(methods.items())},
                  complete_work=comparison['def_id'], preserves=preserves['def_id'])
    if mode == 'parent':
        parent = declaration(functions, PARENT)
        body, call = only_call(parent)
        records = declaration(translated['type_decls'], CODEC_ROOT + '::Sources')['kind']['Struct']
        records = next(index for index, field in enumerate(records) if field['name'] == 'records')
        if call_kind(call) != {'Fun': inner['def_id']} or borrowed_paths(body, call) != [(1, records), (2, records), (3,)]:
            raise ValueError('original parent must pass both full vectors and the same key set')
        result['parent'] = parent['def_id']
        result['key_lookup'] = parent_key_lookup(translated, call)
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
    if any(digest(path) != fixed[name]['binary_sha256'] for name, path in (('charon', charon), ('aeneas', aeneas))):
        raise ValueError('fixed tools required')
    if subprocess.check_output(['git', 'rev-parse', SOURCE + '^{tree}'], text=True, cwd=ROOT).strip() != TREE:
        raise ValueError('original integration tree differs')
    output.mkdir()
    source = output / 'source'
    source.mkdir()
    report = dict(mode=mode, source_commit=SOURCE, source_tree=TREE, complete=False, commands=[], public_acceptance=False,
                  driver_sha256=digest(Path(__file__)), cpus=cpus)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', SOURCE], cwd=ROOT, stdout=stream, check=True)
        if digest(archive) != ARCHIVE:
            raise ValueError('original integration archive differs')
        with tarfile.open(archive) as inputs:
            inputs.extractall(source, filter='data')
        report['codec_sha256'] = stage_port(source, mode)
        report['comparison_probe_sha256'] = digest(source / PROBE_SOURCE)
        ir, generated = output / 'actual.llbc', output / 'generated'
        includes = [flag for name in configuration(mode)['include'] for flag in ('--include', name)]
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                        '--start-from', MODES[mode], '--start-from-if-exists', PROBE, *includes,
                        '--dest-file', ir, '--', '--manifest-path',
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
    print(report['admission'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=MODES, required=True)
    for argument in ('output', 'tools', 'lock'):
        parser.add_argument('--' + argument, type=Path, required=True)
    args = parser.parse_args()
    capture(args.mode, args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True))
