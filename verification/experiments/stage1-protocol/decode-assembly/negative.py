"""Require the real compiled field-swap failure, then extract the same Rust."""
import argparse
import json
import os
from pathlib import Path
import time
import traceback
import support
from check import check


def validate_native_commands(receipt):
    variants = ('original', support.MUTANT, 'restored')
    commands = receipt.get('commands', [])
    expected = [(v, c) for v in variants for c in ('test', 'clippy')]
    observed = [(r.get('variant'), r.get('argv', [None, None])[1]) for r in commands]
    if observed != expected or set(receipt.get('variants', {})) != set(variants):
        raise ValueError('all six original/mutant/restored native commands are required')
    for variant in variants:
        row = receipt['variants'][variant]
        if row.get('native_checked') is not True:
            raise ValueError('incomplete native variant')
        if variant != support.MUTANT and row['source_inputs'] != receipt['inputs']:
            raise ValueError('original/restored inputs differ')
    for row in commands:
        command = row['argv'][1]
        args = (['--lib', '--message-format=json', 'tests::actual_five_branch_fields_and_roundtrips',
                 '--', '--exact', '--nocapture'] if command == 'test'
                else ['--all-targets', '--', '-D', 'warnings'])
        argv = ['cargo', command, '--manifest-path', str(support.HERE.parent / 'envelope-schema/Cargo.toml'),
                '--locked', '--offline', *args]
        failed = row['variant'] == support.MUTANT and command == 'test'
        if (row['argv'] != argv or row['exit_code'] != (101 if failed else 0) or
                (row.get('property_rejected') is True) != failed):
            raise ValueError('native command/result is not the original full-field assertion')


def run(native, output):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='decode-assembly-real-swap-admission', complete=False,
        lean_counterexample=False, ning_acceptance=False, shared_acceptance=False)
    try:
        receipt = json.loads((native / 'result.json').read_text())
        if receipt.get('complete') is not True or receipt.get('kind') != 'decode-assembly-native-development':
            raise ValueError('complete original/mutant/restored native evidence required')
        validate_native_commands(receipt)
        for row in receipt['commands']:
            log = native / row['variant'] / (row['argv'][1] + '.log')
            if support.base.sha(log) != row['log_sha256']:
                raise ValueError('native log missing or changed')
        if support.base.snapshot(support.ROOT) != receipt['inputs']:
            raise ValueError('native evidence does not cover current complete source inputs')
        project = output / 'source'
        support.copy_source(project, receipt['inputs'])
        mutation = support.mutate(project)
        if mutation != receipt['mutation']:
            raise ValueError('native mutation differs from the original registered field swap')
        expected = dict(receipt['inputs'])
        expected[mutation['path']] = mutation['after']
        if receipt['variants'][support.MUTANT]['source_inputs'] != expected:
            raise ValueError('native variant changed more than the registered fields')
        record['library_inputs'] = support.libraries()
        if record['library_inputs'] != receipt['library_inputs']:
            raise ValueError('native library sources/features changed')
        raw, captured = support.capture(project, 'recorded_decode', output)
        record.update(capture=captured, native_result_sha256=support.base.sha(native / 'result.json'))
        try:
            check(raw, support.schema.inventory(support.ROOT))
        except ValueError as error:
            if str(error) != 'original input field name routed to another field':
                raise ValueError('unexpected failure is not the registered field-swap rejection') from error
            record['rejection'] = str(error)
        else:
            raise ValueError('actual swapped input fields were incorrectly admitted')
        for name, wanted in expected.items():
            if support.base.sha(project / name) != wanted:
                raise ValueError('tested mutant source changed')
        if support.base.snapshot(support.ROOT) != receipt['inputs'] or record['library_inputs'] != support.libraries():
            raise ValueError('current source/library changed during admission')
        record.update(complete=True, source_admission_rejected=True)
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-evidence', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.native_evidence.absolute(), args.output.absolute())
    print(json.dumps({k: result.get(k) for k in ('kind', 'complete', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] else 1)
