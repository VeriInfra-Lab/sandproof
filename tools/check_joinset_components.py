"""Original JoinSet wiring in private copies; conditional evidence, not async refinement.

Only explicit test appendices and one reviewed drain-loop removal per mutant
are allowed. Every mutant must freshly compile and fail the positive version's
immediate original-task assertion. Compile failures and timeouts are failures.
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

GUEST_DRAIN = '''        while let Some(result) = tasks.join_next().await {
            report.record(result.unwrap_or(Outcome::Failed));
        }
'''
CENTER_DRAIN = '''    while let Some(stopped) = tasks.join_next().await {
        let completion = match stopped {
            Ok(completion) => completion,
            Err(_) => Err("center worker stopped abnormally"),
        };
        result = result.and(completion);
    }
'''


def reproduce(output):
    from check_joinset_contract import check
    contract = check()
    for name in contract['inputs'] + ['verification/joinset-inputs.sha256']:
        committed = subprocess.check_output(['git', 'show', 'HEAD:' + name], cwd=ROOT)
        if committed != (ROOT/name).read_bytes():
            raise ValueError('Commit frozen JoinSet inputs before reproduction: ' + name)
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    archive = subprocess.check_output(['git', 'archive', 'HEAD'], cwd=ROOT)
    target = Path(os.environ.get('CARGO_TARGET_DIR', str(output/'target'))).resolve()
    env = os.environ | {'CARGO_TARGET_DIR': str(target), 'RUST_TEST_THREADS': '1'}
    receipt = dict(commit=revision, contract_id=contract['contract_id'],
                   proof_kind='conditional-model-and-original-source-component',
                   rustc=subprocess.check_output(['rustc', '-vV'], text=True), variants={})

    def build(directory, package, kind, name, tag, fresh=True):
        log = output/(tag+'-build.log')
        command = ['cargo', 'test', '--locked', '--offline', '-p', package, '--'+kind]
        if kind != 'lib':
            command.append(name)
        code = run(command+['--no-run', '--message-format=json'], directory, env, log)
        if code:
            raise ValueError('Compile failure is not a semantic rejection: ' + str(log))
        built = artifact(log, name)
        if fresh and built['fresh']:
            raise ValueError('JoinSet variant needs a freshly compiled original test module: ' + tag)
        binary = Path(built['executable']).resolve()
        if not binary.is_relative_to(target):
            raise ValueError('JoinSet executable escaped registered target')
        retained = output/(tag+'-tests')
        shutil.copy2(binary, retained)
        return retained, dict(fresh=not built['fresh'], sha256=sha(retained),
                              build_log_sha256=sha(log))

    def test(binary, directory, marker, count, tag, wrong=False):
        log = output/(tag+'.log')
        code = run([str(binary), marker, '--nocapture', '--test-threads=1'], directory, env, log)
        text = log.read_text()
        if wrong:
            if (code != 101 or 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' not in text
                    or 'returned before original task was joined' not in text):
                raise ValueError('Wrong drain did not fail the same immediate original-task assertion: '+str(log))
        elif code != 0 or f'test result: ok. {count} passed; 0 failed; 0 ignored;' not in text:
            raise ValueError('Missing successful original JoinSet tests: '+str(log))
        return dict(filter=marker, count=count, test_exit=code, log_sha256=sha(log))

    for variant in ('correct', 'guest-no-drain', 'center-no-drain'):
        directory = output/variant
        directory.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
            tree.extractall(directory, filter='data')
        files = {}
        for filename, appendices in [
            ('sandbox-guest-service/src/listener.rs', ['guest-tests.rs', 'primitive-tests.rs']),
            ('sandbox-center/src/runtime.rs', ['center-tests.rs']),
        ]:
            path = directory/filename
            if path.read_bytes() != (ROOT/filename).read_bytes():
                raise ValueError('Original JoinSet source differs from committed production input')
            files[filename] = dict(production_sha256=sha(path), appendices={})
            text = path.read_text()
            mutation = ((variant == 'guest-no-drain' and filename.endswith('/listener.rs'))
                        or (variant == 'center-no-drain' and filename.endswith('/runtime.rs')))
            if mutation:
                old = GUEST_DRAIN if variant == 'guest-no-drain' else CENTER_DRAIN
                new = '        drop(tasks);\n' if variant == 'guest-no-drain' else '    drop(tasks);\n'
                if text.count(old) != 1:
                    raise ValueError('Ambiguous original drain-loop mutation')
                text = text.replace(old, new)
                files[filename]['mutation'] = dict(before=old, after=new)
            for name in appendices:
                appendix = ROOT/'verification/experiments/joinset'/name
                files[filename]['appendices'][name] = sha(appendix)
                text += '\n'+appendix.read_text()
            path.write_text(text)
            files[filename]['tested_sha256'] = sha(path)
        checks, binaries = [], {}
        if variant != 'center-no-drain':
            binary, binaries['guest'] = build(directory, 'sandproof-guest-service', 'lib',
                                              'sandproof_guest_service', variant+'-guest')
            checks.append(test(binary, directory, 'joinset_contract_trial', 1,
                               variant+'-guest-drain', wrong=variant != 'correct'))
            if variant == 'correct':
                checks.append(test(binary, directory, 'joinset_primitive_trial', 8, 'correct-primitives'))
        if variant != 'guest-no-drain':
            binary, binaries['center'] = build(directory, 'sandproof-center', 'bin',
                                               'sandproof-center', variant+'-center')
            checks.append(test(binary, directory, 'joinset_center_trial', 1,
                               variant+'-center-drain', wrong=variant != 'correct'))
        if variant == 'correct':
            binary, binaries['protected'] = build(directory, 'sandproof-internal', 'test',
                                                  'secure_probe', 'correct-protected', fresh=False)
            checks.append(test(binary, directory,
                'connections_are_bounded_before_handshake_and_shutdown_releases_original_slots',
                1, 'correct-protected-drain'))
            log = output/'correct-clippy.log'
            code = run(['cargo', 'clippy', '--locked', '--offline', '-p', 'sandproof-guest-service',
                        '-p', 'sandproof-center', '--all-targets', '--', '-D', 'warnings'], directory, env, log)
            if code:
                raise ValueError('Original JoinSet fixture Clippy failed: '+str(log))
            receipt['clippy'] = dict(exit_code=code, log_sha256=sha(log))
        receipt['variants'][variant] = dict(sources=files, binaries=binaries, checks=checks)
        (output/'result.json').write_text(json.dumps(receipt, indent=2)+'\n')
    receipt.update(complete=True, seconds=round(time.monotonic()-started, 3))
    (output/'result.json').write_text(json.dumps(receipt, indent=2)+'\n')
    print(json.dumps(dict(complete=True, correct_cases=11, wrong_variants=2,
                         seconds=receipt['seconds'], evidence=str(output))))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    reproduce(parser.parse_args().output.resolve())
