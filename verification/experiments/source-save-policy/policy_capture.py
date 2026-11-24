"""Capture the original maintenance reader in the same save capacity parent."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil

from policy_inputs import CHECKPOINT, CONFIG, HERE, MODES, NAMESPACE, POLICY_ROOT, ROOT, SOURCE_FILES, configuration, retention, row_provider, stage


def admit(row, ir, generated, target):
    from frontier_capture import indexed
    from prepare_source_save_order import check_forwarded_call
    from revision_capture import declaration, regular_calls

    raw = json.loads(ir.read_bytes())
    translated = raw['translated']
    options = translated['options']
    cfg = configuration(row)
    keys = (('start_from', 'roots'), ('start_from_if_exists', 'retained_roots'),
            ('include', 'include'), ('opaque', 'opaque'))
    if (raw['has_errors'] or {entry['key'] for entry in translated['target_information']} != {target}
            or options['exclude'] != [] or any(options[key] != cfg[group] for key, group in keys)
            or any(options[key] is not False for key in ('skip_borrowck', 'no_typecheck', 'remove_unused_clauses', 'monomorphize'))):
        raise ValueError('actual source count target or extraction options differ')
    functions = translated['fun_decls']
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']

    def actual(name, source=retention.CODEC):
        entry = declaration(functions, name)
        mapped, = (item for item in mapping if item['def_id'] == entry['def_id'])
        if (mapped['is_opaque'] is not False or mapped['source']['file'] != str(source)
                or 'Structured' not in entry['body']):
            raise ValueError('missing actual transparent original source count function')
        return entry

    save, count, per_session = (actual(row.CODEC_ROOT + '::' + name) for name in ('save', 'reserved_records', 'records_per_session'))
    limit, = (entry for entry in mapping if '.JournalSourceSave.Insts.' in entry['lean_name']
              and entry['lean_name'].endswith('.limit') and entry['is_opaque'] is False)
    check_forwarded_call(indexed(functions, limit['def_id']), count['def_id'], ('Shared', 'Copy'))
    if list(regular_calls(count['body']['Structured']['body'])).count(per_session['def_id']) != 1:
        raise ValueError('actual source count must call the original per-session calculation once')
    policy = actual(POLICY_ROOT, CHECKPOINT)
    read = actual(POLICY_ROOT + '::read', CONFIG)
    reader = actual(POLICY_ROOT + '::read_using', CONFIG)
    check_forwarded_call(policy, read['def_id'], ('Shared',))
    for call, callee in ((per_session, policy), (read, reader)):
        if list(regular_calls(call['body']['Structured']['body'])).count(callee['def_id']) != 1:
            raise ValueError('a real policy forwarding call changed')
    return dict(policy=policy['def_id'], reader=reader['def_id'], save=save['def_id'], limit=limit['def_id'], count=count['def_id'], per_session=per_session['def_id'])


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
        report['driver'] = {name: retention.sha(HERE / name) for name in ('policy_capture.py', 'policy_inputs.py')}
        report['rust'] = {str(name): retention.sha(source / name) for name in SOURCE_FILES}
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
