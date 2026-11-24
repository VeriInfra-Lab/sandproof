"""Prepare the real SQLite encoder regression for the Ning acceptance role."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile

from encoder_inputs import HERE, MODES, retention, row_provider, stage
import native as parent

CASE = parent.GROUP + 'proof_source_save_stores_the_exact_key_and_json_from_one_record'
REJECTION = 'the encoded SQL row must keep its complete key'


def report_views(output, work, mode, row):
    from verify import compiled_test
    evidence = output / mode
    evidence.mkdir(mode=0o700)
    source = work / mode / 'source'
    source.mkdir(mode=0o700, parents=True)
    with tarfile.open(output / 'source.tar') as contents:
        contents.extractall(source, filter='data')
    if mode != 'original':
        stage(row, source, mode)
    staged_rust = {str(name): retention.sha(source / name) for name in (retention.CODEC, retention.LIB)}
    with (source / parent.TESTS).open('ab') as tests:
        tests.write(b'\n' + (HERE / 'native-encoder.rs.in').read_bytes())
    inputs = parent.inventory(source)
    report = dict(inputs=inputs, staged_rust_before_fixture=staged_rust, compiled=False, executed=False)
    if work == output:
        return report
    target = work / mode / 'target'
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(target), CARGO_TERM_COLOR='never')
    for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(name, None)
    try:
        compile_log = evidence / 'compile.log'
        compiled = parent.command(['cargo', 'test', '--locked', '--offline', '-p', 'sandproof-controller-service',
                                   '--features', 'rusqlite/hooks', '--lib', '--no-run', '--message-format=json'],
                                  source, compile_log, env)
        report['compilation'] = compiled
        if compiled['exit_code'] != 0:
            raise ValueError('actual native view failed to compile: ' + mode)
        binary = compiled_test(evidence, source, target)
        report.update(compiled=True, executable_sha256=retention.sha(binary))
        log = evidence / 'assertions.log'
        asserted = parent.command([binary, '--exact', CASE, '--nocapture'], source, log, env)
        report['assertions'] = asserted
        messages = log.read_text()
        if mode == 'zero-row-key':
            valid = (asserted['exit_code'] == 101 and REJECTION in messages and CASE in messages
                     and 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' in messages)
        else:
            valid = (asserted['exit_code'] == 0 and 'test ' + CASE + ' ... ok' in messages
                     and 'test result: ok. 1 passed; 0 failed; 0 ignored;' in messages)
        if not valid:
            raise ValueError('the same exact SQLite assertion did not produce its required result: ' + mode)
        if inputs != parent.inventory(source) or report['executable_sha256'] != retention.sha(binary):
            raise ValueError('actual source or executable changed during native checks')
        report.update(executed=True, property_rejected=mode == 'zero-row-key')
        return report
    finally:
        retention.write_json(evidence / 'result.json', report)


def main(args):
    os.umask(0o077)
    retention.helpers()
    from tools.ci_retention import Store
    from tools.system_acceptance import private_path
    output = args.output.resolve()
    rules = {file.name: retention.sha(file) for file in HERE.iterdir() if file.is_file()}
    candidate, identity = None, None
    if args.prepare_only:
        if args.work is not None:
            raise ValueError('--work is only used for actual native tests')
        work = output
    else:
        if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'native' or sys.platform != 'linux' or args.work is None:
            raise ValueError('the Linux Ning role and an allocated acceptance payload are required')
        work = private_path(args.work.absolute(), directory=True)
        candidate = parent.candidate()
        store = Store(work.parent.parent)
        with store.lock():
            payload, _, record = store.read(work.name)
            identity = [work.stat().st_dev, work.stat().st_ino]
            if (payload != work or any(work.iterdir()) or record['state'] != 'active' or record['kind'] != 'acceptance'
                    or record['candidate'] != candidate['commit'] or record['identity'] != identity
                    or output.is_relative_to(store.root) or work.is_relative_to(output)):
                raise ValueError('an empty active allocation for the exact candidate and independent evidence are required')
        fixed = json.loads((retention.ROOT / 'verification/foundations-environment.json').read_bytes())
        compiler = subprocess.check_output(['rustc', '-vV'], cwd=retention.ROOT, text=True)
        if compiler.splitlines()[0] != fixed['production_compiler']:
            raise ValueError('registered production compiler required')
    output.mkdir(mode=0o700)
    report = dict(complete=False, prepare_only=args.prepare_only, candidate=candidate, rules=rules, views={},
                  source=retention.SOURCE, allocation_identity=identity, resource_cleanup_confirmed=False)
    if not args.prepare_only:
        report['compiler'] = compiler
    try:
        row = row_provider(output)
        # Use the same official Git archive as both formal variants.
        with (output / 'source.tar').open('xb') as stream:
            subprocess.run(['git', 'archive', retention.SOURCE], cwd=retention.ROOT, stdout=stream, check=True)
        if retention.sha(output / 'source.tar') != retention.SOURCE_ARCHIVE:
            raise ValueError('exact source archive required')
        for mode in ('original', *MODES):
            report['views'][mode] = report_views(output, work, mode, row)
        if rules != {file.name: retention.sha(file) for file in HERE.iterdir() if file.is_file()}:
            raise ValueError('native rules changed during checking')
        if candidate is not None and candidate != parent.candidate():
            raise ValueError('actual candidate changed during native run')
        report['complete'] = True
    finally:
        retention.write_json(output / 'result.json', report)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--work', type=Path)
    parser.add_argument('--prepare-only', action='store_true')
    main(parser.parse_args())
