"""Deterministic native evidence for original Notify wiring, not async refinement.

Only explicit test appendices and the single notify_waiters->notify_one fault
are added in private copies. Both variants must compile; the mutant must fail
the correct version's immediate original-waiter assertion, never a timeout.
"""
import argparse
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import time

from check_owned_message_components import ROOT, artifact, run, sha


def reproduce(output):
    from check_notify_contract import check
    contract = check()
    for name in contract['inputs'] + ['verification/notify-inputs.sha256']:
        if subprocess.check_output(['git', 'show', 'HEAD:' + name], cwd=ROOT) != (ROOT/name).read_bytes():
            raise ValueError('Commit frozen Notify inputs before reproduction: ' + name)
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    archive = subprocess.check_output(['git', 'archive', 'HEAD'], cwd=ROOT)
    target = Path(os.environ.get('CARGO_TARGET_DIR', str(output/'target'))).resolve()
    env = os.environ | {'CARGO_TARGET_DIR': str(target), 'RUST_TEST_THREADS': '1'}
    receipt = dict(commit=revision, contract_id=contract['contract_id'],
                   proof_kind='conditional-model-and-original-source-component', variants={})

    def build(directory, package, name, tag):
        log = output / (tag + '-build.log')
        code = run(['cargo', 'test', '--locked', '--offline', '-p', package,
                    '--lib', '--no-run', '--message-format=json'], directory, env, log)
        if code:
            raise ValueError('Compile failure is not a semantic rejection: ' + str(log))
        built = artifact(log, name)
        if built['fresh']:
            raise ValueError('Notify fixture needs a freshly compiled test module: ' + tag)
        binary = Path(built['executable']).resolve()
        if not binary.is_relative_to(target):
            raise ValueError('Notify executable escaped registered target')
        retained = output / (tag + '-tests')
        shutil.copy2(binary, retained)
        return retained

    for variant in ('correct', 'one-instead-of-all'):
        directory = output / variant
        directory.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
            tree.extractall(directory, filter='data')
        files = {}
        for filename, appendices in [
            ('sandbox-guest-runtime/src/pty.rs', ['pty-tests.rs', 'primitive-tests.rs']),
            ('sandbox-guest-service/src/command/registry.rs', ['registry-tests.rs']),
        ]:
            path = directory / filename
            if path.read_bytes() != (ROOT/filename).read_bytes():
                raise ValueError('Production Notify input differs from committed source')
            files[filename] = dict(production_sha256=sha(path), appendices={})
            if variant != 'correct' and filename.endswith('/pty.rs'):
                text = path.read_text()
                old = 'self.closed.notify_waiters();'
                if text.count(old) != 1:
                    raise ValueError('Ambiguous reviewed original broadcast')
                path.write_text(text.replace(old, 'self.closed.notify_one();'))
            for name in appendices:
                appendix = ROOT / 'verification/experiments/notify' / name
                files[filename]['appendices'][name] = sha(appendix)
                with path.open('a') as stream:
                    stream.write('\n' + appendix.read_text())
            files[filename]['tested_sha256'] = sha(path)
        binary = build(directory, 'sandproof-guest-runtime', 'sandproof_guest_runtime', variant)
        checks = []
        if variant == 'correct':
            for marker, count in [('notify_contract_trial', 3), ('notify_primitive_trial', 5)]:
                log = output / (variant + '-' + marker + '.log')
                code = run([str(binary), marker, '--nocapture', '--test-threads=1'], directory, env, log)
                if code != 0 or f'test result: ok. {count} passed; 0 failed; 0 ignored;' not in log.read_text():
                    raise ValueError('Missing successful original Notify tests: ' + str(log))
                checks.append(dict(filter=marker, count=count, test_exit=code, log_sha256=sha(log)))
            record = build(directory, 'sandproof-guest-service', 'sandproof_guest_service', 'correct-record')
            log = output / 'correct-record-test.log'
            code = run([str(record), 'notify_record_trial', '--nocapture', '--test-threads=1'], directory, env, log)
            if code != 0 or 'test result: ok. 1 passed; 0 failed; 0 ignored;' not in log.read_text():
                raise ValueError('Original Record binding regression failed: ' + str(log))
            checks.append(dict(filter='notify_record_trial', count=1, test_exit=code,
                               binary_sha256=sha(record), log_sha256=sha(log)))
            log = output / 'correct-clippy.log'
            code = run(['cargo', 'clippy', '--locked', '--offline', '-p', 'sandproof-guest-runtime',
                        '-p', 'sandproof-guest-service', '--lib', '--tests', '--', '-D', 'warnings'], directory, env, log)
            if code:
                raise ValueError('Original Notify fixture Clippy failed: ' + str(log))
            receipt['clippy'] = dict(exit_code=code, log_sha256=sha(log))
        else:
            log = output / 'one-instead-of-all-test.log'
            marker = 'pty::notify_contract_trial::close_notifies_every_enabled_original_waiter'
            code = run([str(binary), marker, '--exact', '--nocapture', '--test-threads=1'], directory, env, log)
            text = log.read_text()
            if (code != 101 or 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' not in text
                    or 'close left an enabled original write waiter unnotified' not in text):
                raise ValueError('Wrong notification did not fail the same original-waiter assertion: ' + str(log))
            checks.append(dict(filter=marker, test_exit=code, log_sha256=sha(log)))
        receipt['variants'][variant] = dict(fresh_binary=True, build_exit=0,
            binary_sha256=sha(binary), sources=files, checks=checks)
        (output/'result.json').write_text(json.dumps(receipt, indent=2)+'\n')
    receipt.update(complete=True, seconds=round(time.monotonic()-started, 3))
    (output/'result.json').write_text(json.dumps(receipt, indent=2)+'\n')
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    reproduce(parser.parse_args().output.resolve())
