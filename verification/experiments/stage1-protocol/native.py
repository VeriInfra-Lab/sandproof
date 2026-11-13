"""Private ordinary-host component, not an official full-suite native seal."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import re
import signal
import shutil
import subprocess
import sys
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
ENVELOPE_TESTS = (
    'independent_center_and_node_are_checked_before_the_remaining_shape',
    'request_retention_and_version_keep_original_rejection_order',
    'same_session_changed_full_intent_is_rejected',
    'all_registered_envelope_fields_remain_correlated',
    'current_version_flags_do_not_certify_terminal_truth',
    'matching_a_receipt_does_not_establish_input_validation',
    'renewal_shape_does_not_check_the_business_deadline',
    'unload_outcomes_are_exclusive_and_old_optional_shape_is_rejected',
    'original_continued_does_not_accept_expired_or_unstarted_running_facts',
    'lifecycle_outcome_contract_rejects_previous_wire_version',
)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    names = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'],
        cwd=ROOT,
    ).decode().split('\0')
    return {n: sha(ROOT / n) for n in sorted(set(names))
            if n and (ROOT / n).is_file() and not (ROOT / n).is_symlink()}


def native(output, target):
    output.mkdir(mode=0o700)
    target.mkdir(mode=0o700, exist_ok=True)
    if target.is_symlink() or not target.is_dir():
        raise ValueError('allocated target must be a real directory')
    started = time.monotonic()
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE='true',
               CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='4', RUST_TEST_THREADS='1',
               PYTHONDONTWRITEBYTECODE='1', CARGO_TERM_COLOR='never')
    for name in ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'LD_PRELOAD', 'LD_LIBRARY_PATH']:
        env.pop(name, None)
    record = dict(kind='stage1-protocol-component-not-full-native-seal', steps=[])

    def run(name, argv):
        t = time.monotonic()
        log = output / (name + '.log')
        with log.open('xb') as stream:
            result = subprocess.Popen(argv, cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
                                      stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                result.wait(timeout=1200)
            except BaseException:
                try:
                    os.killpg(result.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                result.wait()
                raise
        record['steps'].append(dict(name=name, argv=argv, exit_code=result.returncode,
                                    seconds=time.monotonic()-t, log_sha256=sha(log)))
        (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if result.returncode:
            raise RuntimeError('ordinary component failed: ' + name)
        return log

    try:
        record['environment'] = {n: env.get(n) for n in [
            'PATH', 'CARGO_HOME', 'CARGO_TARGET_DIR', 'CARGO_NET_OFFLINE', 'CARGO_BUILD_JOBS',
            'CARGO_INCREMENTAL', 'RUST_TEST_THREADS', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS']}
        record['versions'] = {n: subprocess.check_output(argv, cwd=ROOT, env=env, text=True)
                              for n, argv in [('rustc', ['rustc', '-vV']),
                                              ('cargo', ['cargo', '-vV']),
                                              ('clippy', ['cargo', 'clippy', '--version'])]}
        before = sources()
        record['source_sha256'] = before
        run('driver-tests', [sys.executable,
                            'verification/experiments/stage1-protocol/test_replay_formal.py'])
        run('adopted-source-views', [sys.executable,
                                   'verification/experiments/stage1-protocol/source_views_tests.py'])
        run('format', ['cargo', 'fmt', '-p', 'sandproof-protocol', '-p', 'sandproof-internal', '--check'])
        run('extract-format', ['cargo', 'fmt', '--manifest-path',
                              'verification/experiments/stage1-protocol/Cargo.toml', '--check'])
        packages = ['-p', 'sandproof-protocol', '-p', 'sandproof-internal']
        run('clean-own', ['cargo', 'clean', *packages])
        log = run('build', ['cargo', 'test', *packages, '--locked', '--offline',
                            '--no-run', '--message-format=json'])
        own = {'sandproof_protocol': 'sandbox-protocol/src/lib.rs',
               'sandproof_internal': 'sandbox-internal/src/lib.rs'}
        integration = {'current_envelope': 'sandbox-protocol/tests/current_envelope.rs'}
        expected_sources = own | integration
        libraries, tests, integration_tests = {}, [], {}
        compiled = output / 'compiled'
        compiled.mkdir()
        for line in log.read_text().splitlines():
            if not line.startswith('{'):
                continue
            item = json.loads(line)
            if item.get('reason') != 'compiler-artifact' or item['target']['name'] not in expected_sources:
                continue
            if Path(item['target']['src_path']).resolve() != ROOT / expected_sources[item['target']['name']]:
                raise ValueError('actual own library/test comes from a different source')
            package = 'sandbox-internal' if item['target']['name'] == 'sandproof_internal' else 'sandbox-protocol'
            if Path(item['manifest_path']).resolve() != ROOT / package / 'Cargo.toml':
                raise ValueError('actual own artifact has a foreign manifest')
            if item['fresh'] is not False:
                raise ValueError('expected actual fresh own library/test compilation')
            if item['target']['name'] in integration:
                if ('test' not in item['target']['kind'] or not item['profile']['test']
                        or not item['executable'] or item['target']['name'] in integration_tests):
                    raise ValueError('missing or duplicate actual integration program')
                integration_tests[item['target']['name']] = item
            elif item['profile']['test']:
                tests.append(item)
            elif 'lib' in item['target']['kind']:
                libraries[item['target']['name']] = item
            for name in item['filenames']:
                path = Path(name)
                if path.is_file():
                    if not path.resolve().is_relative_to(target.resolve()):
                        raise ValueError('compiler output escaped allocated target')
                    shutil.copy2(path, compiled / path.name)
        if set(libraries) != set(own) or len(tests) != 2:
            raise ValueError('missing actual two libraries and lib test programs')
        if set(integration_tests) != set(integration):
            raise ValueError('missing explicitly required current-envelope integration test')
        record.update(actual_libraries=libraries, actual_lib_tests=tests,
                      actual_integration_tests=integration_tests)
        test_log = run('test', ['cargo', 'test', *packages, '--locked', '--offline'])
        # Keep the original all-package execution. Bind its actual integration
        # section to the freshly compiled program, without running it twice.
        text = test_log.read_text()
        headers = list(re.finditer(r'^\s*Running tests/current_envelope\.rs \(([^\n]+)\)$', text, re.M))
        program = Path(integration_tests['current_envelope']['executable'])
        if len(headers) != 1 or (ROOT / headers[0].group(1)).resolve() != program.resolve():
            raise ValueError('current-envelope program was not explicitly observed in full test run')
        section = text[headers[0].end():]
        following = re.search(r'^\s*(?:Running |Doc-tests )', section, re.M)
        if following:
            section = section[:following.start()]
        passed = re.findall(r'^test ([a-z0-9_]+) \.\.\. ok$', section, re.M)
        if (sorted(passed) != sorted(ENVELOPE_TESTS)
                or f'test result: ok. {len(ENVELOPE_TESTS)} passed; 0 failed; 0 ignored;' not in section
                or sha(program) != sha(compiled / program.name)):
            raise ValueError('required integration assertions or actual program changed')
        record['integration_test_results'] = {'current_envelope': dict(
            test_names=passed, passed=len(ENVELOPE_TESTS), failed=0, ignored=0,
            test_source_sha256=before[integration['current_envelope']],
            executable_sha256=sha(program), test_log_sha256=sha(test_log))}
        run('clippy', ['cargo', 'clippy', *packages, '--all-targets', '--locked', '--offline',
                       '--', '-D', 'warnings'])
        run('docs', ['bash', 'scripts/python-entry.sh', 'tools/check_docs.py'])
        run('fmt-check', ['cargo', 'fmt', '-p', 'sandproof-protocol', '-p',
                          'sandproof-internal', '--check'])
        if sources() != before:
            raise ValueError('component source changed during verification')
        record['compiled_sha256'] = {p.name: sha(p) for p in compiled.iterdir() if p.is_file()}
        record['success'] = True
    except BaseException as error:
        record.update(success=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--target', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    result = native(args.output.absolute(), args.target.absolute())
    print(json.dumps({k: result.get(k) for k in ['kind', 'success', 'seconds', 'error']}))
    raise SystemExit(0 if result['success'] else 1)
