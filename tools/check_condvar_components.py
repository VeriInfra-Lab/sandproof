"""Native Condvar source/API evidence in private copies; run only on ning.

The wrong original-mutex variant is an API fixture mutant, not a production
Reaper mutation or a formal Rust refutation. Compilation and timeouts never
count as semantic rejection.
"""
import argparse
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time

from check_owned_message_components import ROOT, artifact, run, sha


def reproduce(output):
    from check_condvar_contract import check
    contract = check()
    for name in contract['inputs'] + ['verification/condvar-inputs.sha256']:
        if subprocess.check_output(['git', 'show', 'HEAD:' + name], cwd=ROOT) != (ROOT / name).read_bytes():
            raise ValueError('Commit frozen Condvar inputs before reproduction: ' + name)
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=False)
    start = time.monotonic()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    archive = subprocess.check_output(['git', 'archive', 'HEAD'], cwd=ROOT)
    target = Path(os.environ.get('CARGO_TARGET_DIR', str(output / 'target'))).resolve()
    env = os.environ | {'CARGO_TARGET_DIR': str(target), 'RUST_TEST_THREADS': '1'}
    appendix = ROOT / 'verification/experiments/condvar/reaper-tests.rs'
    record = dict(kind='conditional-model-and-original-source-api-components', head=revision,
                  contract_id=contract['contract_id'], appendix_sha256=sha(appendix), variants={},
                  negative_scope='Native API fixture mutant; no production Reaper mutation or formal Rust refutation')
    for variant in ('correct', 'wrong-original-mutex'):
        directory = output / variant
        directory.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
            tree.extractall(directory, filter='data')
        source = directory / 'sandbox-runtime/src/child_process/reaper.rs'
        original = source.read_bytes()
        if sha(source) != contract['production_sources']['sandbox-runtime/src/child_process/reaper.rs']:
            raise ValueError('Original Condvar production source changed')
        extra = appendix.read_text()
        if variant != 'correct':
            before = '        let guard = original.lock().unwrap();'
            if extra.count(before) != 1:
                raise ValueError('Ambiguous original mutex fixture mutation')
            extra = extra.replace(before, '        let guard = alternate.lock().unwrap();')
        source.write_text(original.decode() + '\n' + extra)
        build_log = output / (variant + '-build.log')
        code = run(['cargo', 'test', '--locked', '--offline', '-p', 'sandbox-runtime',
                    '--lib', '--no-run', '--message-format=json'], directory, env, build_log)
        if code:
            raise ValueError('Condvar compile failure is not semantic rejection')
        built = artifact(build_log, 'sandbox_runtime')
        if built['fresh'] or not Path(built['executable']).resolve().is_relative_to(target):
            raise ValueError('Condvar fixture needs a fresh binary inside the registered target')
        binary = output / (variant + '-tests')
        shutil.copy2(built['executable'], binary)
        checks = []
        markers = [('condvar_contract_trial', 4),
                   ('unknown_child_retains_capacity_until_exact_exit_is_reaped', 1)] if variant == 'correct' else [
                       ('condvar_contract_trial::timeout_reply_holds_the_original_mutex', 1)]
        for index, (marker, count) in enumerate(markers):
            log = output / (variant + '-test-' + str(index) + '.log')
            code = run([str(binary), marker, '--nocapture', '--test-threads=1'], directory, env, log)
            text = log.read_text()
            if variant == 'correct':
                if code != 0 or f'test result: ok. {count} passed; 0 failed; 0 ignored;' not in text:
                    raise ValueError('Missing successful original Condvar tests: ' + str(log))
            elif (code != 101 or 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' not in text
                  or 'Condvar reply changed the original mutex' not in text):
                raise ValueError('Wrong mutex did not fail the same original-mutex assertion: ' + str(log))
            checks.append(dict(marker=marker, count=count, test_exit=code, log_sha256=sha(log)))
        if variant == 'correct':
            for label, command in [('fmt', ['cargo', 'fmt', '--all', '--check']),
                                   ('clippy', ['cargo', 'clippy', '--locked', '--offline', '-p',
                                               'sandbox-runtime', '--all-targets', '--', '-D', 'warnings'])]:
                log = output / (label + '.log')
                if run(command, directory, env, log):
                    raise ValueError('Condvar fixture ' + label + ' failed')
                record[label] = dict(exit_code=0, log_sha256=sha(log))
        record['variants'][variant] = dict(build_exit=0, fresh_binary=True, binary_sha256=sha(binary),
            production_sha256=sha(ROOT / 'sandbox-runtime/src/child_process/reaper.rs'),
            tested_source_sha256=sha(source), checks=checks)
        (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
    record.update(complete=True, seconds=round(time.monotonic() - start, 3))
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(dict(complete=True, seconds=record['seconds'], correct=5, native_fixture_wrong=1)))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    reproduce(parser.parse_args().output.resolve())
