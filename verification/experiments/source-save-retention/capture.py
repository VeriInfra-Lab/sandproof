"""Capture the real save control flow together with complete retention providers."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil

import inputs as v


def borrowed_paths(body, call):
    from revision_capture import ir_values
    bindings = {place['kind']['Local']: value['Ref'] for place, value in ir_values(body, 'Assign')}

    def origin(place):
        kind = place['kind']
        if 'Local' in kind:
            local = kind['Local']
            if local not in bindings:
                return (local,)
            reference = bindings[local]
            if reference['kind'] != 'Shared':
                raise ValueError('the original shared input is required')
            return origin(reference['place'])
        source, projection = kind['Projection']
        return origin(source) if projection == 'Deref' else (*origin(source), projection['Field'][1])

    return [origin(argument['Move']) for argument in call['args']]


def configuration(mode):
    codec = 'sandproof_controller_service::controller::lifecycle_inbox::source_work::codec'
    root = codec + ('::save' if mode == 'parent' else '::save_retaining_using')
    return dict(roots=[root], retained_roots=['crate::proof_source_work_eq'],
                include=['sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service', 'core::tuple'],
                opaque=['rusqlite', 'serde_json', *[codec + '::' + name for name in
                        ('load', 'reserved_records', 'encode_candidate_records', 'write_candidate_records')]])


def admit(ir, generated, mode):
    v.helpers()
    from frontier_capture import call_kind, indexed
    from prepare_transaction_capture import checked_translation
    from revision_capture import declaration, ir_values, regular_calls
    from prepare_source_save_order import check_forwarded_call
    cfg = configuration(mode)
    fixed = json.loads((v.ROOT / 'verification/foundations-environment.json').read_bytes())
    translated = checked_translation(ir, cfg, fixed['target'], cfg['roots'][0], cfg['retained_roots'][0])
    functions = translated['fun_decls']
    codec = cfg['roots'][0].rsplit('::', 1)[0]
    save = declaration(functions, codec + '::save_using')
    bridge = declaration(functions, codec + '::save_retaining_using')
    methods = ('read', 'limit', 'encode', 'retains', 'write')
    trait = declaration(translated['trait_decls'], codec + '::SourceSaveEffects')['def_id']
    references, called = {}, []

    def collect(value):
        if isinstance(value, dict):
            item = value.get('Value')
            if (isinstance(item, list) and len(item) == 2 and isinstance(item[0], int)
                    and isinstance(item[1], dict) and 'trait_decl_ref' in item[1]):
                if item[0] in references and references[item[0]] != item[1]:
                    raise ValueError('ambiguous global trait reference')
                references[item[0]] = item[1]
            for child in value.values():
                collect(child)
        elif isinstance(value, list):
            for child in value:
                collect(child)

    collect(translated)
    for item in ir_values(save['body']['Structured']['body'], 'Call'):
        kind = call_kind(item['call'])
        if 'Trait' in kind:
            ref, index = kind['Trait']
            if 'Value' in ref:
                key, value = ref['Value']
                references[key] = value
            else:
                value = references[ref['Deduplicated']]
            if value['trait_decl_ref']['skip_binder']['id'] == trait:
                called.append(methods[index])
    if tuple(called) != methods:
        raise ValueError('the actual five source-save phases must retain their original order')
    check_forwarded_call(bridge, save['def_id'], ('Shared', 'Shared', 'Copy'))
    inner = declaration(functions, codec + '::retains_with_saved_keys')
    required = [save, bridge, inner]
    sources = declaration(translated['type_decls'], codec + '::Sources')['kind']['Struct']
    records = next(i for i, field in enumerate(sources) if field['name'] == 'records')
    adapter, = (row for row in functions if row and row['item_meta']['opacity'] == 'Transparent'
                and row['item_meta']['name'][-1] == {'Ident': ['retains', 0]}
                and list(regular_calls(row['body']['Structured']['body'])) == [inner['def_id']])
    body = adapter['body']['Structured']['body']
    call, = (row['call'] for row in ir_values(body, 'Call'))
    if borrowed_paths(body, call) != [(1, records), (2, records), (3,)] or call['dest']['kind'] != {'Local': 0}:
        raise ValueError('the checked adapter must pass the original complete record vectors and key carrier')
    required.append(adapter)
    if mode == 'parent':
        parent = declaration(functions, codec + '::save')
        check_forwarded_call(parent, bridge['def_id'], ('Shared', 'Shared', 'Copy'))
        dispatch, = (row['call'] for row in ir_values(parent['body']['Structured']['body'], 'Call'))
        key_trait = declaration(translated['trait_decls'], codec + '::SavedKeyCheck')['def_id']
        reference, = (entry['Value'][1] for entry in dispatch['func']['Regular']['generics']['trait_refs']
                      if 'Value' in entry and entry['Value'][1]['trait_decl_ref']['skip_binder']['id'] == key_trait)
        instance = indexed(translated['trait_impls'], reference['kind']['TraitImpl']['id'])
        method, = (row['skip_binder']['id'] for row in instance['methods'] if row['kind']['TraitMethod'] == [key_trait, 0])
        lookup_body = indexed(functions, method)['body']['Structured']['body']
        lookup, = (row['call'] for row in ir_values(lookup_body, 'Call'))
        callee = indexed(functions, call_kind(lookup)['Fun'])['item_meta']['name']
        prefix = [{'Ident': [name, 0]} for name in ('alloc', 'collections', 'btree', 'set')]
        if (callee[:4] != prefix or callee[-1] != {'Ident': ['contains', 0]}
                or borrowed_paths(lookup_body, lookup) != [(1,), (2,)]):
            raise ValueError('the parent must delegate the complete original key to BTreeSet::contains')
        required.append(parent)
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    for function in required:
        actual, = (row for row in mapping if row['def_id'] == function['def_id'])
        if function['item_meta']['opacity'] != 'Transparent' or actual['is_opaque'] is not False:
            raise ValueError('the actual business caller must have a transparent generated body')
    return dict(save=save['def_id'], bridge=bridge['def_id'], retention=inner['def_id'], order=list(methods))


def capture(args):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('proof-machine role is required')
    if min(shutil.disk_usage(args.output.parent).free, shutil.disk_usage(v.ROOT).free) < 80 * 1024**3:
        raise ValueError('80 GiB of free capture/workspace capacity is required')
    os.umask(0o077)
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    save = v.helpers()
    from probe_prepare_iterators import run
    fixed = json.loads((v.ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = args.tools / 'charon/bin/charon', args.tools / 'bin/aeneas'
    for name, path in (('charon', charon), ('aeneas', aeneas)):
        if v.sha(path) != fixed[name]['binary_sha256']:
            raise ValueError('the registered fixed tool differs: ' + name)
    args.output.mkdir(mode=0o700)
    report = dict(complete=False, mode=args.mode, source=v.SOURCE, public_acceptance=False, commands=[])
    try:
        source = v.snapshot(args.output)
        report['rust'] = v.stage(source, args.mode)
        report['driver'] = {p.name: v.sha(p) for p in (Path(__file__), v.HERE / 'inputs.py', v.HERE / 'bridge.rs.in')}
        cfg = configuration(args.mode)
        selectors = [item for flag, key in (('--start-from', 'roots'), ('--include', 'include'), ('--opaque', 'opaque'))
                     for value in cfg[key] for item in (flag, value)]
        ir, generated = args.output / 'actual.llbc', args.output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(args.output / 'target'))
        for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(name, None)
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default', *selectors,
                        '--start-from-if-exists', cfg['retained_roots'][0], '--dest-file', ir, '--', '--manifest-path',
                        'controller-service/Cargo.toml', '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
                        '-namespace', v.NAMESPACE, '-subdir', v.NAMESPACE, '-dest', generated, ir], 120),
        ]
        with args.lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for label, argv, limit in commands:
                receipt = run(label, argv, limit, source, args.output, env)
                report['commands'].append(receipt)
                if receipt['exit_code'] != 0 or receipt['bounded_stop']:
                    raise ValueError(label + ' did not complete')
        report['ir'] = v.sha(ir)
        report['admission'] = admit(ir, generated, args.mode)
        report['generated'] = {str(p.relative_to(generated)): v.sha(p) for p in sorted(generated.rglob('*')) if p.is_file()}
        report['complete'] = True
    finally:
        v.write_json(args.output / 'result.json', report)
    print(args.mode, 'completed fixed extraction and source admission')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=v.MODES, required=True)
    for name in ('output', 'tools', 'lock'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    capture(args)
