"""Extract the already executed real omission variant and check the same binding."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import traceback

import admit
from check import check


def run(native, output):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='serde-binding-real-omission-admission', complete=False,
                  lean_counterexample=False, ning_acceptance=False, shared_acceptance=False)
    try:
        receipt = json.loads((native / 'result.json').read_text())
        if receipt.get('complete') is not True or receipt['kind'] != 'serde-binding-native-development':
            raise ValueError('completed original/mutant/restored native evidence required')
        for row in receipt['commands']:
            log = native / row['variant'] / (row['argv'][1] + '.log')
            if admit.base.sha(log) != row['log_sha256']:
                raise ValueError('native evidence changed')
        failures = [r for r in receipt['commands'] if r.get('property_rejected') is True]
        if len(failures) != 1 or failures[0]['variant'] != 'omit-recorded-terminal':
            raise ValueError('wrong real omission assertion')
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
        if (mutation['path'] != 'sandbox-protocol/src/lifecycle/authority.rs' or
                mutation['original'] != '    pub terminal: bool,\n' or
                mutation['replacement'] != '    #[serde(skip_serializing)]\n    pub terminal: bool,\n'):
            raise ValueError('unreviewed native mutation')
        changed = project / mutation['path']
        text = changed.read_text()
        if text.count(mutation['original']) != 1:
            raise ValueError('original terminal field no longer unique')
        changed.write_text(text.replace(mutation['original'], mutation['replacement']))
        source_inputs = receipt['variants']['omit-recorded-terminal']['source_inputs']
        for name, wanted in source_inputs.items():
            if admit.base.sha(project / name) != wanted:
                raise ValueError('actual tested source changed: ' + name)
        tools = project / '.tools'
        if not tools.exists():
            tools.symlink_to(admit.ROOT / '.tools', target_is_directory=True)
        raw_path = project / 'artifacts/bytes-recorded_encode.llbc'
        if raw_path.exists():
            raise ValueError('preserve previous extraction; use fresh output')
        (project / 'artifacts/extract-target').mkdir(parents=True)
        fixed = json.loads((admit.ROOT / 'verification/foundations-environment.json').read_text())
        if admit.base.sha(tools / 'aeneas/charon/bin/charon') != fixed['charon']['binary_sha256']:
            raise ValueError('fixed Charon changed')
        wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
            '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
            '--bind', str(project), str(admit.VIRTUAL), '--chdir', str(admit.VIRTUAL),
            '--setenv', 'CARGO_TARGET_DIR', str(admit.VIRTUAL / 'artifacts/extract-target'),
            '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
            '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
        command = wrapper + ['.tools/aeneas/charon/bin/charon', 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default', '--include', 'sandproof_protocol',
            '--start-from', 'crate::recorded_encode', '--dest-file',
            '../../../../artifacts/bytes-recorded_encode.llbc', '--', '--manifest-path',
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
        schema = admit.bytes_module.schema.inventory(admit.ROOT)
        try:
            check(raw, schema)
        except ValueError as error:
            expected = 'missing/extra standard field call: RecordedLifecycleIntent'
            if str(error) != expected:
                raise ValueError('unexpected admission failure, not registered field omission') from error
            record['rejection'] = str(error)
        else:
            raise ValueError('real omitted field was incorrectly admitted')
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
