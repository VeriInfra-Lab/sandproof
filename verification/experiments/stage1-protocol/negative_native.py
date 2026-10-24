"""Run unchanged protocol assertions against isolated, actually compiled wrong Rust."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import re
import signal
import shutil
import subprocess
import tarfile
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
MUTATIONS = [
    dict(name='same-incarnation', source='sandbox-protocol/src/lifecycle/request.rs',
         old='target_incarnation.get() <= source_incarnation.get()',
         new='target_incarnation.get() < source_incarnation.get()', profile='request', target='lifecycle_contracts',
         test_source='sandbox-protocol/tests/lifecycle_contracts.rs', test='resume_allows_number_gaps_but_never_old_or_same_incarnation'),
    dict(name='key-only-replay', source='sandbox-protocol/src/lifecycle/request.rs',
         old='        self == original\n',
         new='        self.binding.key == original.binding.key\n', profile='request', target='lifecycle_contracts',
         test_source='sandbox-protocol/tests/lifecycle_contracts.rs', test='replay_compares_complete_original_input'),
    dict(name='skip-capture-binding', source='sandbox-protocol/src/lifecycle/manifest.rs',
         old='if self.binding != *expected {',
         new='if self.binding != *expected && false {', profile='manifest', target='lifecycle_contracts',
         test_source='sandbox-protocol/tests/lifecycle_contracts.rs', test='full_manifest_has_exact_original_binding_and_complete_files'),
    dict(name='session-only-envelope', source='sandbox-protocol/src/lifecycle/authority.rs',
         old='        self.intent == *original\n',
         new='        self.intent.request.binding.session == original.request.binding.session\n',
         profile='request', target='current_envelope',
         test_source='sandbox-protocol/tests/current_envelope.rs',
         test='same_session_changed_full_intent_is_rejected',
         failure_message='same session cannot replace the complete original accepted intent'),
]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(output, target, archive):
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
    result = dict(kind='stage1-isolated-native-negatives-not-formal-proof',
                  archive_sha256=sha(archive), variants=[])

    def save():
        (output / 'progress.json').write_text(json.dumps(result, indent=2)+'\n')

    def command(folder, name, argv, cwd):
        log = folder / (name + '.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            proc = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                proc.wait(timeout=1200)
            except BaseException:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                proc.wait()
                raise
        return dict(argv=argv, exit_code=proc.returncode,
                    seconds=time.monotonic()-begin, log_sha256=sha(log))

    def snapshot(source):
        files = {}
        for path in sorted(source.rglob('*')):
            if path.is_symlink() or not (path.is_file() or path.is_dir()):
                raise ValueError('source snapshot contains an unsupported file type')
            if path.is_file():
                files[str(path.relative_to(source))] = dict(
                    sha256=sha(path), mode=path.stat().st_mode & 0o777)
        return files

    def clean(folder, label, source, row):
        row[label] = command(folder, label, ['cargo', 'clean', '-p', 'sandproof-protocol'], source)
        save()
        if row[label]['exit_code'] != 0:
            raise ValueError('own package cleanup failed: ' + label)

    def build(folder, label, source, mutation, row):
        clean(folder, 'clean-' + label, source, row)
        args = ['cargo', 'test', '-p', 'sandproof-protocol', '--test', mutation['target'],
                '--locked', '--offline', '--no-run', '--message-format=json']
        build_error = None
        try:
            row[label] = command(folder, label, args, source)
        except BaseException as error:
            build_error = error
            log = folder / (label + '.log')
            row[label] = dict(argv=args, exit_code=None, error=repr(error),
                              log_sha256=sha(log) if log.is_file() else None)
        library, test = None, None
        compiled = folder / ('compiled' if label == 'build' else label + '-compiled')
        compiled.mkdir(mode=0o700)
        artifacts = []
        artifact_errors = []
        for line in (folder / (label + '.log')).read_text().splitlines():
            if not line.startswith('{'):
                continue
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                artifact_errors.append('incomplete compiler JSON output')
                continue
            if item.get('reason') != 'compiler-artifact':
                continue
            t = item['target']
            if t['name'] == 'sandproof_protocol' and 'lib' in t['kind']:
                expected = source / 'sandbox-protocol/src/lib.rs'
                library = item
            elif t['name'] == mutation['target'] and 'test' in t['kind']:
                expected = source / mutation['test_source']
                test = item
            else:
                continue
            if (Path(t['src_path']).resolve() != expected.resolve()
                    or Path(item['manifest_path']).resolve() != source / 'sandbox-protocol/Cargo.toml'
                    or item['fresh'] is not False):
                artifact_errors.append('actual library/test is stale or from another source')
            artifacts.append(item)
            # Preserve emitted artifacts even when a later test compilation fails.
            for name in item['filenames']:
                path = Path(name)
                if path.is_file():
                    if not path.resolve().is_relative_to(target.resolve()):
                        artifact_errors.append('compiler output escaped allocated target')
                        continue
                    shutil.copy2(path, compiled / path.name)
        artifact_key = 'compiler_artifacts' if label == 'build' else label + '_compiler_artifacts'
        hashes_key = 'compiled_sha256' if label == 'build' else label + '_compiled_sha256'
        row[artifact_key] = artifacts
        if artifact_errors:
            row[label + '_artifact_errors'] = artifact_errors
        row[hashes_key] = {p.name: sha(p) for p in compiled.iterdir() if p.is_file()}
        save()
        if build_error is not None:
            raise build_error
        if artifact_errors:
            raise ValueError('; '.join(artifact_errors))
        if row[label]['exit_code']:
            raise ValueError('Rust must compile before its assertion is tested: ' + label)
        if library is None or test is None or len(artifacts) != 2 or not test['executable']:
            raise ValueError('missing actual own library or selected test program')
        return test

    def assertion(folder, label, source, mutation, row, program, *, wrong):
        args = [program['executable'], mutation['test'], '--exact', '--nocapture', '--test-threads=1']
        key = 'assertion' if label == 'assertions' else label
        row[key] = command(folder, label, args, source)
        text = (folder / (label + '.log')).read_text()
        if wrong:
            message = mutation.get('failure_message', 'assertion')
            valid = (row[key]['exit_code'] == 101 and message in text
                     and 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' in text
                     and 'running 1 test' in text
                     and mutation['test'] in text
                     and re.search(r'^\s*' + re.escape(mutation['test']) + r'\s*$', text, re.M))
        else:
            valid = (row[key]['exit_code'] == 0
                     and 'test result: ok. 1 passed; 0 failed; 0 ignored;' in text
                     and 'test ' + mutation['test'] + ' ... ok' in text)
        save()
        if not valid:
            raise ValueError('unchanged exact assertion did not produce its required result: ' + label)

    try:
        result['versions'] = {n: subprocess.check_output(args, text=True, env=env)
                              for n, args in [('rustc', ['rustc', '-vV']),
                                              ('cargo', ['cargo', '-vV'])]}
        result['environment'] = {n: env.get(n) for n in ['CARGO_HOME', 'CARGO_TARGET_DIR',
            'CARGO_NET_OFFLINE', 'CARGO_BUILD_JOBS', 'RUST_TEST_THREADS', 'RUSTFLAGS']}
        for mutation in MUTATIONS:
            folder = output / mutation['name']
            folder.mkdir(mode=0o700)
            source = folder / 'source'
            source.mkdir(mode=0o700)
            with tarfile.open(archive) as tar:
                tar.extractall(source, filter='data')
            before = snapshot(source)
            file = source / mutation['source']
            original = file.read_bytes()
            if original.decode().count(mutation['old']) != 1:
                raise ValueError('mutation must match one exact actual production expression')
            test_file = source / mutation['test_source']
            wrong = original.decode().replace(mutation['old'], mutation['new'], 1).encode()
            row = dict(mutation, original_source_sha256=hashlib.sha256(original).hexdigest(),
                       wrong_source_sha256=hashlib.sha256(wrong).hexdigest(),
                       test_source_sha256=sha(test_file))
            result['variants'].append(row)
            (folder / 'source-before.json').write_text(json.dumps(before, indent=2)+'\n')
            mutated = False
            try:
                normal = build(folder, 'baseline_build', source, mutation, row)
                assertion(folder, 'baseline_assertion', source, mutation, row, normal, wrong=False)
                if snapshot(source) != before:
                    raise ValueError('baseline execution changed its source')
                mutated = True
                file.write_bytes(wrong)
                shutil.copy2(file, folder / 'mutation.rs')
                expected_wrong = dict(before)
                expected_wrong[mutation['source']] = dict(before[mutation['source']], sha256=sha(file))
                if snapshot(source) != expected_wrong or sha(test_file) != row['test_source_sha256']:
                    raise ValueError('mutant changed more than its single expression or original test')
                program = build(folder, 'build', source, mutation, row)
                assertion(folder, 'assertions', source, mutation, row, program, wrong=True)
                if snapshot(source) != expected_wrong or sha(test_file) != row['test_source_sha256']:
                    raise ValueError('wrong source or unchanged test changed during execution')
                file.write_bytes(original)
                restored = build(folder, 'restored_build', source, mutation, row)
                assertion(folder, 'restored_assertion', source, mutation, row, restored, wrong=False)
                after = snapshot(source)
                row['source_changes_after_restore'] = {n: dict(before=before.get(n), after=after.get(n))
                    for n in set(before) | set(after) if before.get(n) != after.get(n)}
                if row['source_changes_after_restore']:
                    raise ValueError('correct source was not completely restored')
                row['restored_source_sha256'] = sha(file)
                row['target_final_state'] = 'correct-restored-build-verified'
                row['rejected_by_unchanged_assertion'] = True
                mutated = False
                save()
            finally:
                restore_error = None
                try:
                    file.write_bytes(original)
                    row['restored_source_sha256'] = sha(file)
                except BaseException as error:
                    restore_error = error
                    row['source_restore_error'] = repr(error)
                if mutated:
                    # Any partial outputs already copied above remain evidence;
                    # the shared allocated target must not retain the bad package.
                    try:
                        clean(folder, 'cleanup_failed_variant', source, row)
                        row['target_final_state'] = 'failed-variant-package-cleared'
                    except BaseException as cleanup:
                        row['cleanup_error'] = repr(cleanup)
                        row['target_final_state'] = 'cleanup-unconfirmed'
                save()
                if restore_error is not None:
                    raise restore_error
        result['success'] = True
    except BaseException as error:
        result.update(success=False, error=repr(error), traceback=traceback.format_exc())
    result['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(result, indent=2)+'\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-archive', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--target', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.target.absolute(), args.source_archive.absolute())
    print(json.dumps({k: result.get(k) for k in ['kind', 'success', 'seconds', 'error']}))
    raise SystemExit(0 if result['success'] else 1)
