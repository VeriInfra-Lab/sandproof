"""Capture the complete retention and multi-row writer in the same source view."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil

from joint_inputs import HERE, MODES, NAMESPACE, ROOT, configuration, retention, row_provider, stage


def admit(row, ir, generated, mode, target):
    from frontier_capture import call_kind, indexed
    from revision_capture import declaration, ir_values, regular_calls
    from source_first_match import borrowed_paths
    from source_row_write import parent_operands
    from prepare_source_save_order import check_forwarded_call
    cfg = configuration(row, mode)
    raw = json.loads(ir.read_bytes())
    translated = raw['translated']
    options = translated['options']
    if (raw.get('has_errors') is not False or options['exclude'] != []
            or any(options[name] is not False for name in ('skip_borrowck', 'no_typecheck', 'remove_unused_clauses', 'monomorphize'))
            or any(options[key] != cfg[name] for key, name in (('start_from', 'roots'), ('include', 'include'),
                ('opaque', 'opaque'), ('start_from_if_exists', 'retained_roots')))
            or {entry['key'] for entry in translated['target_information']} != {target}):
        raise ValueError('actual capture options or target differ')
    functions = translated['fun_decls']
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']

    def actual(name, count=1):
        function = declaration(functions, row.CODEC_ROOT + '::' + name)
        records = [item for item in mapping if item['def_id'] == function['def_id']]
        if (function['item_meta']['opacity'] != 'Transparent' or len(records) != count
                or any(item['is_opaque'] is not False or item['source']['file'] != str(retention.CODEC) for item in records)):
            raise ValueError('missing original transparent body: ' + name)
        return function

    bridge = actual('save_sql_retaining_using')
    save = actual('save_using')
    retains = actual('retains_with_saved_keys')
    actual('retains_inventory_using', 3)
    actual('retains_record_using', 3)
    writer = actual('write_candidate_records', 3)
    single = actual('write_candidate_record')
    core = actual('write_candidate_record_using')
    check_forwarded_call(bridge, save['def_id'], ('Shared', 'Shared', 'Copy'))
    if mode == 'parent':
        check_forwarded_call(actual('save'), bridge['def_id'], ('Shared', 'Shared', 'Copy'))
    dispatch, = (item['call'] for item in ir_values(bridge['body']['Structured']['body'], 'Call'))
    effects = declaration(translated['trait_decls'], row.CODEC_ROOT + '::SourceSaveEffects')['def_id']
    instance_ref, = (entry['Value'][1] for entry in dispatch['func']['Regular']['generics']['trait_refs']
                     if 'Value' in entry and entry['Value'][1]['trait_decl_ref']['skip_binder']['id'] == effects)
    instance = indexed(translated['trait_impls'], instance_ref['kind']['TraitImpl']['id'])
    for index, name, destination in ((3, 'retains', retains), (4, 'write', writer)):
        entry, = (item for item in instance['methods'] if item['kind'] == {'TraitMethod': [effects, index]})
        adapter = indexed(functions, entry['skip_binder']['id'])
        if (adapter['item_meta']['opacity'] != 'Transparent'
                or list(regular_calls(adapter['body']['Structured']['body'])) != [destination['def_id']]):
            raise ValueError('the selected dictionary must directly forward ' + name)
        if name == 'write':
            check_forwarded_call(adapter, destination['def_id'], ('Shared', 'Move'))
        else:
            sources = declaration(translated['type_decls'], row.CODEC_ROOT + '::Sources')['kind']['Struct']
            records = next(i for i, field in enumerate(sources) if field['name'] == 'records')
            body = adapter['body']['Structured']['body']
            call, = (item['call'] for item in ir_values(body, 'Call'))
            if (call_kind(call) != {'Fun': destination['def_id']} or call['dest']['kind'] != {'Local': 0}
                    or borrowed_paths(body, call) != [(1, records), (2, records), (3,)]):
                raise ValueError('same current and candidate vectors and key carrier required')
    trait = declaration(translated['trait_decls'], row.CODEC_ROOT + '::SourceRowEffects')['def_id']
    return dict(bridge=bridge['def_id'], save=save['def_id'], writer=writer['def_id'],
                row_adapters=parent_operands(translated, single, core['def_id'], trait))


def capture(args):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('proof-machine role is required')
    if min(shutil.disk_usage(path).free for path in (args.output.parent, ROOT)) < 80 * 1024**3:
        raise ValueError('80 GiB of free capture and workspace capacity is required')
    os.umask(0o077)
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = args.tools / 'charon/bin/charon', args.tools / 'bin/aeneas'
    for name, path in (('charon', charon), ('aeneas', aeneas)):
        if retention.sha(path) != fixed[name]['binary_sha256']:
            raise ValueError('the registered fixed tool differs: ' + name)
    args.output.mkdir(mode=0o700)
    row = row_provider(args.output)
    from probe_prepare_iterators import run
    cfg = configuration(row, args.mode)
    report = dict(complete=False, mode=args.mode, source=retention.SOURCE, public_acceptance=False, commands=[], configuration=cfg)
    try:
        source = retention.snapshot(args.output)
        stage(row, source, args.mode)
        report['driver'] = {name: retention.sha(HERE / name) for name in ('joint_capture.py', 'joint_inputs.py', 'bridge.rs.in')}
        report['rust'] = {str(name): retention.sha(source / name) for name in (retention.CODEC, retention.LIB)}
        selectors = [part for flag, key in (('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
                    ('--include', 'include'), ('--opaque', 'opaque')) for value in cfg[key] for part in (flag, value)]
        ir, generated = args.output / 'actual.llbc', args.output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(args.output / 'target'))
        for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(name, None)
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default', *selectors,
                        '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml', '--lib', '--locked',
                        '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
                        '-namespace', NAMESPACE, '-subdir', NAMESPACE, '-dest', generated, ir], 120),
        ]
        with args.lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for label, argv, limit in commands:
                receipt = run(label, argv, limit, source, args.output, env)
                report['commands'].append(receipt)
                if receipt['exit_code'] or receipt['bounded_stop']:
                    raise ValueError(label + ' did not complete')
        report['ir'] = retention.sha(ir)
        report['admission'] = admit(row, ir, generated, args.mode, fixed['target'])
        report['generated'] = {str(path.relative_to(generated)): retention.sha(path)
                               for path in sorted(generated.rglob('*')) if path.is_file()}
        report['complete'] = True
    finally:
        retention.write_json(args.output / 'result.json', report)
    print(args.mode, 'completed fixed extraction and source admission')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=MODES, required=True)
    for name in ('output', 'tools', 'lock'):
        parser.add_argument('--' + name, type=Path, required=True)
    capture(parser.parse_args())
