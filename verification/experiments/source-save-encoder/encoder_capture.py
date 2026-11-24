"""Extract the actual encoder and its parent under the fixed toolchain."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil

from encoder_inputs import HERE, MODES, NAMESPACE, ROOT, configuration, retention, row_provider, stage


def admit(row, ir, generated, target):
    from frontier_capture import indexed
    from prepare_source_save_order import check_forwarded_call
    from revision_capture import declaration, regular_calls

    cfg = configuration(row)
    raw = json.loads(ir.read_bytes())
    translated = raw['translated']
    options = translated['options']
    selectors = (('start_from', 'roots'), ('start_from_if_exists', 'retained_roots'),
                 ('include', 'include'), ('opaque', 'opaque'))
    if (raw.get('has_errors') is not False or options['exclude'] != []
            or any(options[name] is not False for name in
                   ('skip_borrowck', 'no_typecheck', 'remove_unused_clauses', 'monomorphize'))
            or any(options[key] != cfg[name] for key, name in selectors)
            or {entry['key'] for entry in translated['target_information']} != {target}):
        raise ValueError('encoder extraction options or target differ')

    functions = translated['fun_decls']
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']

    def find(name, count=1):
        item = declaration(functions, row.CODEC_ROOT + '::' + name)
        mapped = [entry for entry in mapping if entry['def_id'] == item['def_id']]
        if (item['item_meta']['opacity'] != 'Transparent' or len(mapped) != count
                or any(entry['is_opaque'] is not False or entry['source']['file'] != str(retention.CODEC)
                       or entry['lean_file'] != NAMESPACE + '/Funs.lean' for entry in mapped)):
            raise ValueError('missing original transparent definition: ' + name)
        return item

    encoding = find('encode_candidate_records', 3)
    save = find('save')
    bridge = find('save_sql_retaining_using')
    check_forwarded_call(save, bridge['def_id'], ('Shared', 'Shared', 'Copy'))
    adapter, = (entry for entry in mapping if '.JournalSourceSave.Insts.' in entry['lean_name']
                and entry['lean_name'].endswith('.encode') and entry['is_opaque'] is False)
    check_forwarded_call(indexed(functions, adapter['def_id']), encoding['def_id'], ('Shared',))
    forwards = [declaration(functions, row.CODEC_ROOT + '::' + name)
                for name in ('encode_anchor_json', 'encode_record_json')]
    calls = list(regular_calls(encoding['body']['Structured']['body']))
    if any(calls.count(function['def_id']) != 1 for function in forwards):
        raise ValueError('the exact original two JSON calls must occur in the encoded loop')
    return dict(save=save['def_id'], encoder=encoding['def_id'], adapter=adapter['def_id'],
                json=[function['def_id'] for function in forwards])


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
    cfg = configuration(row)
    report = dict(complete=False, mode=args.mode, source=retention.SOURCE, public_acceptance=False, commands=[], configuration=cfg)
    try:
        source = retention.snapshot(args.output)
        stage(row, source, args.mode)
        report['driver'] = {name: retention.sha(HERE / name) for name in ('encoder_capture.py', 'encoder_inputs.py')}
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
        report['admission'] = admit(row, ir, generated, fixed['target'])
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
