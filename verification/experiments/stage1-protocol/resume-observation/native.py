"""Ordinary-host correct/mutant/restored assertions; not a shared native seal."""
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

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/resume-observation')
TESTS = ('legal_original_and_successors', 'full_original_is_not_session_only', 'current_retention_is_not_omitted', 'lost_unload_is_rejected', 'renewal_incarnation_is_not_omitted', 'state_lost_value_is_allowed_for_original_and_renewal', 'negative_renewal_pending_is_allowed', 'negative_renewal_observed_is_rejected', 'not_current_needs_a_successor', 'expired_observation_is_rejected')
MUTANT_TESTS = {'resume-session-only': ('full_original_is_not_session_only', 'same session does not identify the complete original'), 'resume-ignore-retention': ('current_retention_is_not_omitted', 'resume must preserve current retention'), 'resume-lost-unload': ('lost_unload_is_rejected', 'StateLost does not match continued Unload'), 'resume-wrong-renew-incarnation': ('renewal_incarnation_is_not_omitted', 'renew must target original resumed incarnation')}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def compiler_artifacts(source, target, log):
    """Reject stale, duplicate, foreign, or missing own library/test binaries."""
    expected = {
        'sandproof_protocol': ('lib', Path('sandbox-protocol/Cargo.toml'), Path('sandbox-protocol/src/lib.rs')),
        'sandproof_stage1_resume_observation_contract': ('lib', HERE / 'Cargo.toml', HERE / 'src/lib.rs'),
        'contract': ('test', HERE / 'Cargo.toml', HERE / 'tests/contract.rs'),
    }
    found = {}
    for line in log.read_text().splitlines():
        if not line.startswith('{'):
            continue
        row = json.loads(line)
        if row.get('reason') != 'compiler-artifact' or row['target']['name'] not in expected:
            continue
        name = row['target']['name']
        kind, manifest, path = expected[name]
        if (name in found or kind not in row['target']['kind'] or row['fresh'] is not False
                or Path(row['manifest_path']).resolve() != source / manifest
                or Path(row['target']['src_path']).resolve() != source / path
                or (kind == 'test' and (not row['profile']['test'] or not row['executable']))):
            raise ValueError('actual own artifact is missing, duplicate, stale or foreign')
        for filename in row['filenames']:
            p = Path(filename)
            if not p.is_file() or not p.resolve().is_relative_to(target.resolve()):
                raise ValueError('compiler output escaped allocated target')
        found[name] = row
    if set(found) != set(expected):
        raise ValueError('missing freshly compiled actual own packages or test program')
    return found


def run(archive, output, target):
    output.mkdir(mode=0o700, parents=True)
    target.mkdir(mode=0o700, parents=True, exist_ok=True)
    if target.is_symlink():
        raise ValueError('allocated target must be a real directory')
    started = time.monotonic()
    record = dict(kind='resume-observation-native-component', archive_sha256=sha(archive),
                  full_acceptance=False, variants={}, commands=[])
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE='true',
        CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='4', RUST_TEST_THREADS='1', CARGO_TERM_COLOR='never')
    for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'LD_PRELOAD', 'LD_LIBRARY_PATH'):
        env.pop(name, None)

    def save():
        (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')

    def command(source, folder, name, argv):
        log = folder / (name + '.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            process = subprocess.Popen(argv, cwd=source, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = process.wait(timeout=1200)
            except BaseException:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
                raise
        record['commands'].append(dict(argv=argv, cwd=str(source), exit_code=code,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        save()
        return code, log

    def checked(source, folder, name, argv):
        code, log = command(source, folder, name, argv)
        if code:
            raise ValueError('command failed before assertion: ' + str(log))
        return log

    def snapshot(source):
        files = {}
        for path in sorted(source.rglob('*')):
            if path.is_symlink() or not (path.is_file() or path.is_dir()):
                raise ValueError('source contains an unsupported file type')
            if path.is_file():
                files[str(path.relative_to(source))] = dict(sha256=sha(path), mode=path.stat().st_mode & 0o777)
        return files

    try:
        record['versions'] = {n: subprocess.check_output(argv, env=env, text=True) for n, argv in
            [('rustc', ['rustc', '-vV']), ('cargo', ['cargo', '-vV'])]}
        mutations = json.loads((ROOT / HERE / 'inventory.json').read_text())['mutations']
        if set(mutations) != set(MUTANT_TESTS):
            raise ValueError('unregistered native mutant')
        for case in ('correct', *MUTANT_TESTS):
            folder = output / case
            source = folder / 'source'
            source.mkdir(mode=0o700, parents=True)
            with tarfile.open(archive) as tar:
                names = set()
                for member in tar.getmembers():
                    p = Path(member.name)
                    if (member.name in names or p.is_absolute() or '..' in p.parts
                            or not member.isfile()):
                        raise ValueError('archive must contain unique regular source files')
                    names.add(member.name)
                tar.extractall(source, filter='data')
            if sha(source / HERE / 'inventory.json') != sha(ROOT / HERE / 'inventory.json'):
                raise ValueError('archive has a different mutant registration')
            before = snapshot(source)
            row = dict(stages={})
            record['variants'][case] = row
            (folder / 'source-before.json').write_text(json.dumps(before, indent=2)+'\n')
            mutation = mutations.get(case)
            path = source / mutation['source'] if mutation else None
            original = path.read_bytes() if path else None
            stages = ('correct',) if case == 'correct' else ('baseline', 'mutant', 'restored')
            try:
                for stage in stages:
                    if path:
                        if original.decode().count(mutation['old']) != 1:
                            raise ValueError('mutation must alter one exact original expression')
                        content = (original.decode().replace(mutation['old'], mutation['new'], 1).encode()
                                   if stage == 'mutant' else original)
                        path.write_bytes(content)
                        shutil.copyfile(path, folder / (stage + '.rs'))
                    manifest = str(HERE / 'Cargo.toml')
                    checked(source, folder, stage + '-clean', ['cargo', 'clean', '--manifest-path', manifest,
                        '-p', 'sandproof-protocol', '-p', 'sandproof-stage1-resume-observation-contract'])
                    log = checked(source, folder, stage + '-build', ['cargo', 'test', '--manifest-path', manifest,
                        '--test', 'contract', '--locked', '--offline', '--no-run', '--message-format=json'])
                    found = compiler_artifacts(source, target, log)
                    compiled = folder / (stage + '-compiled')
                    compiled.mkdir(mode=0o700)
                    for item in found.values():
                        for name in item['filenames']:
                            shutil.copy2(name, compiled / Path(name).name)
                    program = found['contract']['executable']
                    test = MUTANT_TESTS[case][0] if mutation else None
                    args = [program, *([test, '--exact'] if test else []), '--nocapture', '--test-threads=1']
                    code, log = command(source, folder, stage + '-assertion', args)
                    text = log.read_text()
                    if stage == 'mutant':
                        valid = (code == 101 and MUTANT_TESTS[case][1] in text
                            and 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' in text
                            and 'running 1 test' in text
                            and re.search(r'^\s*' + re.escape(test) + r'\s*$', text, re.M))
                    else:
                        passed = re.findall(r'^test ([a-z0-9_]+) \.\.\. ok$', text, re.M)
                        expected = [test] if test else list(TESTS)
                        valid = (code == 0 and sorted(passed) == sorted(expected)
                            and f'test result: ok. {len(expected)} passed; 0 failed; 0 ignored;' in text)
                    if not valid:
                        raise ValueError('unchanged actual assertion did not give the required result')
                    expected_inputs = dict(before)
                    if path:
                        expected_inputs[mutation['source']] = dict(before[mutation['source']], sha256=sha(path))
                    if snapshot(source) != expected_inputs:
                        raise ValueError('test changed source or mutation altered extra inputs')
                    row['stages'][stage] = dict(complete=True, artifacts=found,
                        compiled_sha256={p.name: sha(p) for p in compiled.iterdir()},
                        production_sha256=sha(path) if path else None)
                    save()
                if case == 'correct':
                    checked(source, folder, 'driver-tests', ['bash', 'scripts/python-entry.sh', str(HERE / 'test_drivers.py')])
                    checked(source, folder, 'format', ['cargo', 'fmt', '--manifest-path', manifest, '--check'])
                    checked(source, folder, 'clippy', ['cargo', 'clippy', '--manifest-path', manifest,
                        '--all-targets', '--locked', '--offline', '--', '-D', 'warnings'])
            finally:
                if path:
                    path.write_bytes(original)
                    if 'restored' not in row['stages']:
                        checked(source, folder, 'failed-own-clean', ['cargo', 'clean', '--manifest-path',
                            str(HERE / 'Cargo.toml'), '-p', 'sandproof-protocol',
                            '-p', 'sandproof-stage1-resume-observation-contract'])
            if snapshot(source) != before:
                raise ValueError('source not fully restored')
            row['complete'] = True
        record['success'] = True
    except BaseException as error:
        record.update(success=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-archive', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--target', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.source_archive.absolute(), args.output.absolute(), args.target.absolute())
    print(json.dumps({k: result.get(k) for k in ['kind', 'success', 'seconds', 'error']}))
    raise SystemExit(0 if result['success'] else 1)
