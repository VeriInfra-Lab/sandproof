"""Capture the actual collector and its original full persisted-key guard variant."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil

from collector_inputs import HERE, MODES, NAMESPACE, ROOT, SOURCE_FILES, configuration, retention, row_provider, stage

from load_capture import admit as parent_admit


def admit(row, ir, generated, target, config):
    from revision_capture import declaration, regular_calls
    checked = parent_admit(row, ir, generated, target, config)
    translated = json.loads(ir.read_bytes())['translated']
    mapping = json.loads((generated/'translation.json').read_bytes())['functions']
    functions = translated['fun_decls']
    lookup = {}
    for name, opaque, count in (('scan_loaded_records',False,1),('read_loaded_rows',False,3),
            ('decode_loaded_record',False,1),('checked_source_payload',True,1),('encode_load_anchor',True,1)):
        item = declaration(functions,row.CODEC_ROOT+'::'+name)
        found = [entry for entry in mapping if entry['def_id']==item['def_id']]
        if (len(found)!=count or any(entry['is_opaque'] is not opaque or entry['source']['file']!=str(retention.CODEC)
                for entry in found)):
            raise ValueError('actual source collector or boundary differs: '+name)
        lookup[name]=item
    for caller, callee in (('scan_loaded_records','read_loaded_rows'),('read_loaded_rows','decode_loaded_record'),
                           ('decode_loaded_record','checked_source_payload'),('decode_loaded_record','encode_load_anchor')):
        calls = regular_calls(lookup[caller]['body']['Structured']['body'])
        if list(calls).count(lookup[callee]['def_id']) != 1:
            raise ValueError('the original collector forwarding differs: '+caller+' -> '+callee)
    return {**checked,**{name: item['def_id'] for name,item in lookup.items()}}


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
    cfg = configuration(row, args.reference, args.bounds_reference, args.cursor_reference, args.collector_reference)
    report = dict(complete=False, mode=args.mode, source=retention.SOURCE, public_acceptance=False, commands=[], configuration=cfg)
    try:
        source = retention.snapshot(args.output)
        stage(row, source, args.mode)
        report['rules'] = {name: retention.sha(HERE / name) for name in ('collector_capture.py', 'collector_inputs.py')}
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
        report['admission'] = admit(row, ir, generated, fixed['target'], cfg)
        report['generated'] = {str(path.relative_to(generated)): retention.sha(path)
                               for path in sorted(generated.rglob('*')) if path.is_file()}
        report['complete'] = True
    finally:
        retention.write_json(args.output / 'result.json', report)
    print(args.mode, 'completed fixed extraction and source admission')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=MODES, required=True)
    for name in ('reference', 'bounds-reference', 'cursor-reference', 'collector-reference', 'output', 'tools', 'lock'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    for name in ("reference", "bounds_reference", "cursor_reference", "collector_reference", "output", "tools", "lock"):
        setattr(args, name, getattr(args, name).resolve())
    capture(args)
