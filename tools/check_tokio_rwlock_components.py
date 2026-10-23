"""Original Guest RwLock wiring in private copies; native host only.

The production-function mutant is checked by the original rotation assertion.
It is not an extracted async Rust counterexample. Fresh test executables alone
are insufficient: the business rlib must be rebuilt for the same source copy.
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

try:
    from .check_owned_message_components import ROOT, artifact, run, sha
except ImportError:
    from check_owned_message_components import ROOT, artifact, run, sha

PACKAGE = 'sandproof-guest-service'


def fresh_build(log, source, target):
    executable = artifact(log, 'session')
    rows = []
    for line in log.read_text().splitlines():
        try:
            item = json.loads(line)
        except ValueError:
            continue
        if (item.get('reason') == 'compiler-artifact'
                and item.get('target', {}).get('name') == 'sandproof_guest_service'
                and item['target']['kind'] == ['lib'] and not item.get('executable')):
            rows.append(item)
    if executable['fresh'] or len(rows) != 1 or rows[0]['fresh']:
        raise ValueError('Both business library and test executable must be freshly built')
    manifest = source / 'sandbox-guest-service/Cargo.toml'
    if any(Path(item['manifest_path']) != manifest for item in [executable, rows[0]]):
        raise ValueError('Business library/test source copy changed')
    libraries = [Path(name).resolve() for name in rows[0]['filenames'] if name.endswith('.rlib')]
    binary = Path(executable['executable']).resolve()
    if len(libraries) != 1 or any(not path.is_relative_to(target) or not path.is_file()
                                  for path in [binary, *libraries]):
        raise ValueError('Business library or executable escaped the registered target')
    return binary, libraries[0]


def mutate_original(source):
    path = source / 'sandbox-guest-service/src/session.rs'
    before = '        Ok(AuthPermit { guard })'
    after = '''        let snapshot = Initialized {
            hash: guard.hash,
            token: Secret(Zeroizing::new(guard.token.0.to_string())),
            instance: guard.instance.clone(),
            template: guard.template.clone(),
            defaults: guard.defaults.clone(),
        };
        drop(guard);
        let detached = Arc::new(RwLock::new(Some(snapshot)));
        let guard = OwnedRwLockReadGuard::try_map(detached.read_owned().await, Option::as_ref)
            .map_err(|_| ServiceError::Unauthenticated)?;
        Ok(AuthPermit { guard })'''
    text = path.read_text()
    if text.count(before) != 1:
        raise ValueError('Ambiguous original authorize guard handoff')
    path.write_text(text.replace(before, after))


def reproduce(output):
    from check_tokio_rwlock_contract import check
    contract = check()
    for name in contract['inputs'] + ['verification/tokio-rwlock-inputs.sha256']:
        if subprocess.check_output(['git', 'show', 'HEAD:' + name], cwd=ROOT) != (ROOT / name).read_bytes():
            raise ValueError('Commit frozen RwLock inputs before reproduction: ' + name)
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=False)
    start = time.monotonic()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    archive = subprocess.check_output(['git', 'archive', 'HEAD'], cwd=ROOT)
    target = Path(os.environ.get('CARGO_TARGET_DIR', str(output / 'target'))).resolve()
    env = os.environ | {'CARGO_TARGET_DIR': str(target), 'RUST_TEST_THREADS': '1'}
    appendix = ROOT / 'verification/experiments/tokio-rwlock/session-tests.rs'
    record = dict(kind='original-source-and-conditional-api-native-evidence-not-async-refinement',
                  head=revision, contract_id=contract['contract_id'], appendix_sha256=sha(appendix),
                  variants={}, complete=False)
    error = None
    cleanup_source = ROOT
    try:
        for variant in ('correct', 'early-release-original'):
            source = output / variant
            source.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
                tree.extractall(source, filter='data')
            cleanup_source = source
            original = source / 'sandbox-guest-service/src/session.rs'
            if sha(original) != contract['production_sources']['sandbox-guest-service/src/session.rs']:
                raise ValueError('Original session source changed')
            suite = source / 'sandbox-guest-service/tests/session.rs'
            suite.write_text(suite.read_text() + appendix.read_text())
            if variant != 'correct':
                mutate_original(source)
            if run(['cargo', 'fmt', '--all', '--check'], source, env, output / (variant + '-fmt.log')):
                raise ValueError('RwLock fixture formatting failed')
            # Tar can restore timestamps older than a preceding mutant's rlib.
            # Invalidate only this owned package, preserving dependency caches.
            if run(['cargo', 'clean', '--package', PACKAGE], source, env, output / (variant + '-clean.log')):
                raise ValueError('Failed to invalidate the mutated business package')
            log = output / (variant + '-build.log')
            if run(['cargo', 'test', '--locked', '--offline', '-p', PACKAGE, '--test', 'session',
                    '--no-run', '--message-format=json'], source, env, log):
                raise ValueError('Compile failure is not semantic rejection')
            executable, library = fresh_build(log, source, target)
            binary = output / (variant + '-tests')
            shutil.copy2(executable, binary)
            row = dict(build_exit=0, fresh_binary=True, business_library_fresh=True,
                       binary_sha256=sha(binary), library_sha256=sha(library),
                       production_sha256=contract['production_sources']['sandbox-guest-service/src/session.rs'],
                       tested_source_sha256=sha(original), checks=[])
            marker = 'rotation_waits_for_original_dispatch_and_rejects_old_credentials_after_commit'
            arguments = [] if variant == 'correct' else [marker, '--exact']
            test_log = output / (variant + '-test.log')
            code = run([str(binary), *arguments, '--nocapture', '--test-threads=1'], source, env, test_log)
            text = test_log.read_text()
            if variant == 'correct':
                if code != 0 or 'test result: ok. 17 passed; 0 failed; 0 ignored;' not in text:
                    raise ValueError('All original session and six RwLock fixture tests must pass')
                lib_log = output / 'correct-private-build.log'
                if run(['cargo', 'test', '--locked', '--offline', '-p', PACKAGE, '--lib',
                        '--no-run', '--message-format=json'], source, env, lib_log):
                    raise ValueError('Original private authorization tests failed to build')
                built = artifact(lib_log, 'sandproof_guest_service')
                lib_binary = Path(built['executable']).resolve()
                if built['fresh'] or not lib_binary.is_relative_to(target):
                    raise ValueError('Need freshly built actual private authorization tests')
                retained = output / 'correct-private-tests'
                shutil.copy2(lib_binary, retained)
                for index, method in enumerate([
                        'signed_permit_blocks_rotation_and_wall_clock_is_read_after_snapshot_wait',
                        'authorization_lock_wait_uses_original_deadline_and_cancel_does_not_pin_a_reader']):
                    name = 'session::file_authorization::tests::' + method
                    private_log = output / ('correct-private-' + str(index) + '.log')
                    result = run([str(retained), name, '--exact', '--nocapture', '--test-threads=1'], source, env, private_log)
                    if result != 0 or 'test result: ok. 1 passed; 0 failed; 0 ignored;' not in private_log.read_text():
                        raise ValueError('Original mapped file-authorization regression failed: ' + name)
                    row['checks'].append(dict(marker=name, count=1, test_exit=0, log_sha256=sha(private_log)))
                clippy = output / 'clippy.log'
                if run(['cargo', 'clippy', '--locked', '--offline', '-p', PACKAGE,
                        '--all-targets', '--', '-D', 'warnings'], source, env, clippy):
                    raise ValueError('RwLock component Clippy failed')
                record['clippy'] = dict(exit_code=0, log_sha256=sha(clippy))
                row['private_binary_sha256'] = sha(retained)
            elif (code != 101 or 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' not in text
                  or 'is_pending()' not in text):
                raise ValueError('Wrong original-lock release did not fail the same native assertion')
            row['checks'].append(dict(marker='all session tests' if variant == 'correct' else marker,
                                      count=17 if variant == 'correct' else 1,
                                      test_exit=code, log_sha256=sha(test_log)))
            record['variants'][variant] = row
            (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
    except BaseException as failure:
        error = failure
    finally:
        # Never leave a mutant rlib available for a later baseline build.
        cleanup = run(['cargo', 'clean', '--package', PACKAGE], cleanup_source, env, output / 'final-package-clean.log')
        record['final_mutant_cache_cleanup'] = dict(exit_code=cleanup,
            log_sha256=sha(output / 'final-package-clean.log'))
        record.update(complete=error is None and cleanup == 0,
                      seconds=round(time.monotonic() - start, 3),
                      negative_scope='Actual private source mutation and original native assertion; not formal async Rust refutation')
        if error is not None:
            record['error'] = repr(error)
        (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    if error is not None:
        raise error
    if cleanup:
        raise ValueError('Mutated package cache was not quarantined')
    print(json.dumps(dict(complete=True, correct=19, native_wrong=1, seconds=record['seconds'])))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    reproduce(parser.parse_args().output.resolve())
