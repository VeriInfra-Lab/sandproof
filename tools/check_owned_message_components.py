"""Deterministic checks of original Guest value ownership in an isolated copy.

The helper is real but is aborted/reaped before message tests. No driver,
authorization, output protocol, quarantine/record release policy or actual
async machine refinement is claimed. Both versions compile; the mutant edits
only the actual Producer::frame output capture and fails the original test.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import time

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command, source, env, log):
    with log.open('xb') as stream:
        return subprocess.run(command, cwd=source, env=env, stdout=stream,
                              stderr=subprocess.STDOUT, check=False, timeout=900).returncode


def artifact(log, name):
    rows = [json.loads(line) for line in log.read_text().splitlines() if line.startswith('{')]
    rows = [r for r in rows if r.get('reason') == 'compiler-artifact'
            and r.get('target', {}).get('name') == name and r.get('executable')]
    if len(rows) != 1:
        raise ValueError('Missing unambiguous real Rust executable: ' + name)
    return rows[0]


def reproduce(output):
    from check_owned_message_contract import check
    contract = check()
    for name in contract['inputs'] + ['verification/owned-message-inputs.sha256']:
        committed = subprocess.check_output(['git', 'show', 'HEAD:' + name], cwd=ROOT)
        if committed != (ROOT/name).read_bytes():
            raise ValueError('Commit the frozen message inputs before reproduction: ' + name)
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    archive = subprocess.check_output(['git', 'archive', 'HEAD'], cwd=ROOT)
    # Only the original committed production code is copied. Test appendices
    # are explicit proof fixtures, not replacement production algorithms.
    receipt = dict(commit=revision, contract_id=contract['contract_id'], proof_kind='conditional-model-and-original-source-component',
                   rustc=subprocess.check_output(['rustc', '-vV'], text=True), variants={})
    target = Path(os.environ.get('CARGO_TARGET_DIR', str(output / 'target'))).resolve()
    env = os.environ | {'CARGO_TARGET_DIR': str(target), 'RUST_TEST_THREADS': '1'}
    for kind in ('correct', 'early-output-drop'):
        directory = output / kind
        directory.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
            tree.extractall(directory, filter='data')
        files = {}
        for stem in ('streams', 'body'):
            path = directory / f'sandbox-guest-service/src/command/{stem}.rs'
            original = ROOT / f'sandbox-guest-service/src/command/{stem}.rs'
            if path.read_bytes() != original.read_bytes():
                raise ValueError('Commit the production inputs before reproduction')
            files[stem] = {'production_sha256': sha(path)}
            if kind == 'early-output-drop' and stem == 'body':
                old = '            _lease: Arc::clone(&self.lease),\n            output,\n'
                new = '            _lease: Arc::clone(&self.lease),\n            output: { drop(output); None },\n'
                text = path.read_text()
                if text.count(old) != 1:
                    raise ValueError('Expected exactly one reviewed actual frame capture')
                path.write_text(text.replace(old, new))
            appendix = ROOT / f'verification/experiments/owned-message/{stem}-tests.rs'
            files[stem]['test_appendix_sha256'] = sha(appendix)
            with path.open('a') as stream:
                stream.write('\n' + appendix.read_text())
            files[stem]['tested_sha256'] = sha(path)
        if kind == 'correct':
            log = output / 'worker-build.log'
            code = run(['cargo', 'build', '--locked', '--offline', '-p', 'sandproof-guest-runtime',
                        '--bin', 'sandproof-guest-worker', '--message-format=json'], directory, env, log)
            if code:
                raise ValueError('Real worker build failed; not a semantic rejection: ' + str(log))
            worker = Path(artifact(log, 'sandproof-guest-worker')['executable'])
            worker_copy = output / 'sandproof-guest-worker'
            shutil.copy2(worker, worker_copy)
            env['SANDPROOF_OWNED_MESSAGE_WORKER'] = str(worker_copy)
            receipt['worker_sha256'] = sha(worker_copy)
        build_log = output / (kind + '-build.log')
        code = run(['cargo', 'test', '--locked', '--offline', '-p', 'sandproof-guest-service',
                    '--lib', '--no-run', '--message-format=json'], directory, env, build_log)
        if code:
            raise ValueError(kind + ': compile failure is not a semantic rejection: ' + str(build_log))
        built = artifact(build_log, 'sandproof_guest_service')
        if built['fresh']:
            raise ValueError('Need a freshly compiled original test module per variant')
        binary = Path(built['executable']).resolve()
        if not binary.is_relative_to(target):
            raise ValueError('Executable escaped the registered target')
        retained = output / (kind + '-test-binary')
        shutil.copy2(binary, retained)
        command = [str(retained)]
        if kind == 'correct':
            command += ['owned_message_contract_tests', '--nocapture', '--test-threads=1']
        else:
            command += ['command::body::owned_message_contract_tests::last_bytes_clone_keeps_output_and_original_leases',
                        '--exact', '--nocapture', '--test-threads=1']
        test_log = output / (kind + '-test.log')
        code = run(command, directory, env, test_log)
        text = test_log.read_text()
        if kind == 'correct':
            clippy_log = output / 'correct-clippy.log'
            clippy = run(['cargo', 'clippy', '--locked', '--offline', '-p', 'sandproof-guest-service',
                          '--lib', '--tests', '--', '-D', 'warnings'], directory, env, clippy_log)
            if clippy:
                raise ValueError('Actual-source fixture Clippy failed: ' + str(clippy_log))
            receipt['clippy'] = dict(exit_code=clippy, log_sha256=sha(clippy_log))
            if code != 0 or not re.search(r'test result: ok\. 10 passed; 0 failed; 0 ignored;', text):
                raise ValueError('All ten actual-source/primitive tests must execute and pass: ' + str(test_log))
        elif (code != 101 or 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' not in text
              or 'held frame released its byte permit early' not in text):
            raise ValueError('Mutant did not fail the same original held-byte assertion: ' + str(test_log))
        receipt['variants'][kind] = dict(build_exit=0, fresh_binary=True, test_exit=code,
            binary_sha256=sha(retained), test_log_sha256=sha(test_log), sources=files)
        (output / 'result.json').write_text(json.dumps(receipt, indent=2) + '\n')
    receipt['seconds'] = round(time.monotonic() - started, 3)
    receipt['complete'] = True
    (output / 'result.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    reproduce(args.output.resolve())
