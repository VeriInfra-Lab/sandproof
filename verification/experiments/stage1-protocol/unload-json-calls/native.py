"""Run the same real-Rust JSON assertions on correct/mutant/restored.

Run this on ning for acceptance. Elsewhere it is development evidence only;
this file does not sign proof_native_suite or whole-project acceptance.
"""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tarfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
TEST_PREFIX = 'controller::lifecycle_inbox::source_work::snapshot::json_call_contract::'
SOURCE = 'controller-service/src/controller/lifecycle_inbox/source_work/snapshot.rs'
FAILURES = {
    'origin-request': ('full_anchor_contract',
                       'complete original anchor must roundtrip without replacing execution_origin'),
    'options-empty': ('full_options_contract',
                      'complete original create options must roundtrip without dropping metadata'),
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check_assertions(name, exit_code, text):
    if name not in FAILURES:
        if exit_code != 0 or not re.search(r'^test result: ok\. 2 passed; 0 failed; 0 ignored;', text, re.M):
            raise ValueError('correct/restored assertions did not pass')
        return

    test, message = FAILURES[name]
    failed = re.escape(TEST_PREFIX + test)
    panic = (rf"^thread '{failed}' \([0-9]+\) panicked at {re.escape(SOURCE)}:[0-9]+:[0-9]+:\n"
             rf'assertion `left == right` failed: {re.escape(message)}$')
    if (exit_code != 101 or not re.search(rf'^test {failed} \.\.\. FAILED$', text, re.M)
            or not re.search(panic, text, re.M)
            or not re.search(r'^test result: FAILED\. 1 passed; 1 failed; 0 ignored;', text, re.M)):
        raise ValueError('mutant did not reach the same complete-value assertion')


def run(output, lock):
    os.umask(0o077)
    output.mkdir(mode=0o700)
    report = dict(schema=1, development_only=True, native_acceptance=False,
                  public_acceptance=False, complete=False, cases=[])
    try:
        spec = importlib.util.spec_from_file_location('json_call_capture', HERE / 'admit.py')
        capture = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(capture)
        profile = json.loads((HERE / 'profile.json').read_bytes())
        fixture = (HERE / 'native-fixture.rs.in').read_bytes()
        source_fixture = (HERE / 'native-source-fixture.rs.in').read_bytes()
        report['head'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        subprocess.run(['git', 'diff', '--quiet', report['head']], cwd=ROOT, check=True)
        capture.inputs(profile, report['head'])
        report['fixture_sha256'] = sha(HERE / 'native-fixture.rs.in')
        report['source_fixture_sha256'] = sha(HERE / 'native-source-fixture.rs.in')
        report['profile_sha256'] = sha(HERE / 'profile.json')
        version = subprocess.check_output(['cargo', '--version'], cwd=ROOT, text=True).strip()
        if not version.startswith('cargo 1.97.0 '):
            raise ValueError('use the existing fixed Cargo toolchain')
        report['cargo_version'] = version
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', report['head']], cwd=ROOT, stdout=stream, check=True)
        report['source_archive_sha256'] = sha(archive)
        cpus = sorted(os.sched_getaffinity(0))[:4]
        os.sched_setaffinity(0, cpus)
        report['cpus'] = cpus
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='2', CARGO_INCREMENTAL='0',
                   CARGO_PROFILE_DEV_OPT_LEVEL='0', CARGO_PROFILE_TEST_OPT_LEVEL='0',
                   CARGO_PROFILE_DEV_DEBUG='1', CARGO_PROFILE_TEST_DEBUG='1',
                   CARGO_PROFILE_DEV_DEBUG_ASSERTIONS='true', CARGO_PROFILE_TEST_DEBUG_ASSERTIONS='true',
                   CARGO_PROFILE_DEV_OVERFLOW_CHECKS='true', CARGO_PROFILE_TEST_OVERFLOW_CHECKS='true')
        for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUST_TEST_THREADS'):
            env.pop(name, None)

        def command(case, name, argv, seconds, source, target):
            log = output / case['name'] / (name + '.log')
            started = time.monotonic()
            row = dict(name=name, argv=list(map(str, argv)))
            case['commands'].append(row)
            with log.open('xb') as stream:
                child = subprocess.Popen(argv, cwd=source, env=dict(env, CARGO_TARGET_DIR=str(target)),
                    stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    child.wait(timeout=seconds)
                except BaseException:
                    row['interrupted'] = True
                    os.killpg(child.pid, signal.SIGTERM)
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(child.pid, signal.SIGKILL)
                        child.wait(timeout=5)
                    raise
                finally:
                    row.update(exit_code=child.returncode, seconds=time.monotonic()-started, log_sha256=sha(log))
            return row, log

        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for name in ('correct', 'origin-request', 'options-empty', 'restored'):
                directory = output / name
                directory.mkdir(mode=0o700)
                source, target = directory / 'source', directory / 'target'
                source.mkdir(mode=0o700)
                with tarfile.open(archive) as tar:
                    tar.extractall(source, filter='data')
                case = dict(name=name, commands=[], complete=False)
                report['cases'].append(case)
                capture.verify_archived_inputs(source, profile)
                case['mutation'] = capture.apply_variant(source, profile, name if name in profile['mutations'] else 'correct')
                rust = source / profile['source']
                rust.write_bytes(rust.read_bytes() + b'\n' + fixture)
                support = source / 'controller-service/src/controller/lifecycle_inbox/source_work/tests.rs'
                support.write_bytes(support.read_bytes() + b'\n' + source_fixture)
                case['support_sha256'] = sha(support)
                case['tested_source_sha256'] = sha(rust)
                build, log = command(case, 'compile', ['cargo', 'test', '--manifest-path', 'controller-service/Cargo.toml',
                    '--lib', '--locked', '--offline', '--no-run', '--message-format=json'], 600, source, target)
                if build['exit_code'] != 0:
                    raise ValueError('native variant did not compile: ' + name)
                artifacts = []
                for line in log.read_text().splitlines():
                    try:
                        row = json.loads(line)
                    except ValueError:
                        continue
                    if (row.get('reason') == 'compiler-artifact' and row.get('executable')
                            and row['target']['name'] == 'sandproof_controller_service'
                            and row['profile']['test'] is True):
                        artifacts.append(row)
                if len(artifacts) != 1 or artifacts[0]['fresh'] is not False:
                    raise ValueError('missing newly compiled business test binary: ' + name)
                artifact = artifacts[0]
                binary = Path(artifact['executable']).resolve(strict=True)
                if (Path(artifact['manifest_path']).resolve() != source / 'controller-service/Cargo.toml'
                        or Path(artifact['target']['src_path']).resolve() != source / 'controller-service/src/lib.rs'
                        or not binary.is_relative_to(target.resolve())):
                    raise ValueError('test binary is not bound to the actual isolated crate')
                case['artifact'] = artifact
                case['binary_sha256'] = sha(binary)
                test, log = command(case, 'assertions', [binary, 'json_call_contract'], 60, source, target)
                check_assertions(name, test['exit_code'], log.read_text())
                if (sha(rust) != case['tested_source_sha256'] or sha(binary) != case['binary_sha256']
                        or sha(support) != case['support_sha256']):
                    raise ValueError('native source/binary changed during the run')
                case['complete'] = True
        capture.inputs(profile, report['head'])
        report['complete'] = True
    except BaseException as error:
        report['error'] = repr(error)
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: value for key, value in report.items() if key != 'cases'}, indent=2))
    return not report['complete']


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    args = parser.parse_args()
    raise SystemExit(run(args.output.resolve(), args.lock.resolve(strict=True)))
