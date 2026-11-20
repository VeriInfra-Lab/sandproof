"""Extract the already executed real unknown-field variant and check the same binding."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import traceback

import formal as admit
from binding import report


def validate_native_commands(receipt):
    """Reject a pruned/duplicate receipt even if its complete flag is retained."""
    variants = ('original', 'allow-operation-unknown', 'restored')
    manifest = str(admit.HERE / 'Cargo.toml')
    commands = receipt.get('commands', [])
    expected = [(variant, command) for variant in variants for command in ('test', 'clippy')]
    observed = [(row.get('variant'), row.get('argv', [None, None])[1]) for row in commands]
    if observed != expected or set(receipt.get('variants', {})) != set(variants):
        raise ValueError('all six original/mutant/restored native commands are required')
    for variant in variants:
        if receipt['variants'][variant].get('native_checked') is not True:
            raise ValueError('incomplete native variant')
        if variant != 'allow-operation-unknown' and receipt['variants'][variant]['source_inputs'] != receipt['inputs']:
            raise ValueError('original/restored source inputs differ')
    for row in commands:
        command = row['argv'][1]
        args = (['--lib', '--message-format=json', '--', '--nocapture'] if command == 'test'
                else ['--all-targets', '--', '-D', 'warnings'])
        argv = ['cargo', command, '--manifest-path', manifest, '--locked', '--offline', *args]
        failed = row['variant'] == 'allow-operation-unknown' and command == 'test'
        if (row['argv'] != argv or row['exit_code'] != (101 if failed else 0) or
                (row.get('property_rejected') is True) != failed):
            raise ValueError('native command/result is not the original registered assertion')


def run(native, output):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='parser-binding-real-unknown-admission', complete=False,
                  lean_counterexample=False, ning_acceptance=False, shared_acceptance=False)
    try:
        receipt = json.loads((native / 'result.json').read_text())
        if receipt.get('complete') is not True or receipt['kind'] != 'parser-binding-native-development':
            raise ValueError('completed original/mutant/restored native evidence required')
        validate_native_commands(receipt)
        for row in receipt['commands']:
            log = native / row['variant'] / (row['argv'][1] + '.log')
            if admit.base.sha(log) != row['log_sha256']:
                raise ValueError('native evidence changed')
        failures = [r for r in receipt['commands'] if r.get('property_rejected') is True]
        if len(failures) != 1 or failures[0]['variant'] != 'allow-operation-unknown':
            raise ValueError('wrong real unknown-field assertion')
        if admit.base.snapshot(admit.ROOT) != receipt['inputs']:
            raise ValueError('native evidence does not cover current complete source inputs')
        project = output / 'source'
        project.mkdir(mode=0o700)
        for name, wanted in receipt['inputs'].items():
            target = project / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(admit.ROOT / name, target)
            if admit.base.sha(target) != wanted:
                raise ValueError('current source changed while copying')
        mutation = receipt['mutation']
        if (mutation['path'] != 'sandbox-protocol/src/lifecycle/request.rs' or
                mutation['original'] != '#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]\n' or
                mutation['replacement'] != '#[serde(tag = "kind", rename_all = "snake_case")]\n'):
            raise ValueError('unreviewed native mutation')
        changed = project / mutation['path']
        text = changed.read_text()
        if text.count(mutation['original']) != 1:
            raise ValueError('original operation serde attributes no longer unique')
        changed.write_text(text.replace(mutation['original'], mutation['replacement']))
        source_inputs = receipt['variants']['allow-operation-unknown']['source_inputs']
        for name, wanted in source_inputs.items():
            if admit.base.sha(project / name) != wanted:
                raise ValueError('actual tested source changed: ' + name)
        runtime = admit.module('protocol_stable_tools', admit.ROOT / admit.HERE.parent / 'formal_runtime.py')
        tools = project / '.tools'
        if not tools.exists():
            tools.symlink_to(runtime.tool_root(admit.ROOT), target_is_directory=True)
        raw_path = project / 'artifacts/bytes-recorded_decode.llbc'
        if raw_path.exists():
            raise ValueError('preserve previous extraction; use fresh output')
        (project / 'artifacts/extract-target').mkdir(parents=True)
        fixed = json.loads((admit.ROOT / 'verification/foundations-environment.json').read_text())
        if admit.base.sha(tools / 'aeneas/charon/bin/charon') != fixed['charon']['binary_sha256']:
            raise ValueError('fixed Charon changed')
        wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
            '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
            *runtime.tool_bind(admit.ROOT), '--bind', str(project), str(admit.VIRTUAL), '--chdir', str(admit.VIRTUAL),
            '--setenv', 'CARGO_TARGET_DIR', str(admit.VIRTUAL / 'artifacts/extract-target'),
            '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
            '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
        command = wrapper + ['.tools/aeneas/charon/bin/charon', 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default', '--include', 'sandproof_protocol',
            '--start-from', 'crate::recorded_decode', '--dest-file',
            '../../../../artifacts/bytes-recorded_decode.llbc', '--', '--manifest-path',
            str(admit.HERE.parent / 'envelope-bytes/Cargo.toml'), '--lib', '--locked', '--offline']
        log = output / 'charon.log'
        with log.open('xb') as stream:
            result = subprocess.run(command, cwd=project, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record.update(command=command, exit_code=result.returncode, log_sha256=admit.base.sha(log),
                      native_result_sha256=admit.base.sha(native / 'result.json'))
        if result.returncode:
            raise ValueError('mutant extraction failure is not a property rejection')
        raw = json.loads(raw_path.read_text())
        if raw.get('has_errors') is not False:
            raise ValueError('incomplete mutant extraction')
        try:
            report(raw)
        except ValueError as error:
            expected = 'operation unknown-field rejection visitor missing'
            if str(error) != expected:
                raise ValueError('unexpected admission failure, not registered field unknown-field') from error
            record['rejection'] = str(error)
        else:
            raise ValueError('real allow-unknown operation was incorrectly admitted')
        for name, wanted in source_inputs.items():
            if admit.base.sha(project / name) != wanted:
                raise ValueError('tested source changed during extraction')
        if admit.base.snapshot(admit.ROOT) != receipt['inputs']:
            raise ValueError('current inputs changed during extraction')
        record.update(complete=True, source_admission_rejected=True,
                      llbc_sha256=admit.base.sha(raw_path), canonical_llbc_sha256=admit.canonical(raw))
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
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
