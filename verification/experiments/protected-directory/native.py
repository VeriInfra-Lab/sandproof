"""Ordinary-machine real Metadata tests; the mutant must compile then fail the same policy."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
from urllib.parse import unquote, urlsplit

from prepare import prepare, add_tests, sha, VARIANTS

HERE = Path(__file__).resolve().parent


def artifact(log, source, target):
    rows = []
    for line in log.read_text().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if (row.get('reason') == 'compiler-artifact'
                and row.get('target', {}).get('name') == 'sandproof_internal'
                and row.get('target', {}).get('kind') == ['lib']
                and row.get('profile', {}).get('test') is True):
            rows.append(row)
    if len(rows) != 1 or rows[0].get('fresh') is not False:
        raise ValueError('fresh real business unit harness required')
    row = rows[0]
    package = (source / 'sandbox-internal').resolve()
    identity = urlsplit(row.get('package_id', '').split('#', 1)[0])
    if (Path(row['manifest_path']).resolve() != package / 'Cargo.toml'
            or Path(row['target']['src_path']).resolve() != package / 'src/lib.rs'
            or identity.scheme != 'path+file' or identity.netloc
            or Path(unquote(identity.path)).resolve() != package):
        raise ValueError('compiled business package/source differs')
    binary = Path(row.get('executable', ''))
    if not binary.is_file() or not binary.resolve().is_relative_to(target):
        raise ValueError('real executable missing or outside the private target')
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'linux' or os.geteuid() == 0 or not os.environ.get('CARGO_TARGET_DIR'):
        raise ValueError('ordinary non-root Linux environment and private target required')
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False, mode=0o700)
    target = Path(os.environ['CARGO_TARGET_DIR']).resolve()
    record = {'kind': 'original-directory-real-Metadata-native-component', 'variants': {}}
    start = time.monotonic()
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0', RUST_TEST_THREADS='1')
    record['rustc'] = subprocess.check_output(['rustc', '--version'], env=env, text=True).strip()
    if record['rustc'] != 'rustc 1.97.0 (2d8144b78 2026-07-07)':
        raise ValueError('production compiler differs')
    source = None

    def run(case, label, command, expected=0):
        log = case / (label + '.log')
        began = time.monotonic()
        with log.open('xb') as stream:
            code = subprocess.run(command, cwd=source, env=env, stdout=stream,
                                  stderr=subprocess.STDOUT, timeout=900).returncode
        result = {'label': label, 'command': command, 'exit_code': code,
                  'expected_exit': expected, 'seconds': time.monotonic() - began,
                  'log_sha256': sha(log)}
        (case / (label + '.json')).write_text(json.dumps(result, indent=2) + '\n')
        if code != expected:
            raise ValueError(label + ' failed; preserve original record')
        return log, result

    try:
        for variant in VARIANTS:
            case = out / variant
            case.mkdir()
            source = case / 'source'
            entry = prepare(args.archive.resolve(), source, variant)
            entry['tests'] = add_tests(source)
            entry['commands'] = []
            for label, command in [
                ('format', ['cargo', 'fmt', '--all', '--check']),
                ('clean-own-package', ['cargo', 'clean', '-p', 'sandproof-internal']),
                ('compile-original-body', ['cargo', 'test', '-p', 'sandproof-internal', '--lib',
                  '--no-run', '--locked', '--offline', '--message-format=json'])]:
                log, item = run(case, label, command)
                entry['commands'].append(item)
            entry['actual_artifact'] = artifact(log, source, target)
            binary = case / 'actual-tests'
            shutil.copy2(entry['actual_artifact']['executable'], binary)
            entry['binary_sha256'] = sha(binary)
            if variant == 'correct':
                log, item = run(case, 'real-Metadata-cases',
                    [str(binary), 'protected_files::protected_directory_tests::', '--nocapture'])
                if 'test result: ok. 7 passed; 0 failed; 0 ignored;' not in log.read_text():
                    raise ValueError('every required real Metadata case must run')
                entry['commands'].append(item)
                _, item = run(case, 'strict-clippy', ['cargo', 'clippy', '-p',
                    'sandproof-internal', '--lib', '--tests', '--locked', '--offline',
                    '--', '-D', 'warnings'])
                entry['commands'].append(item)
            else:
                log, item = run(case, 'same-property-rejects', [str(binary),
                    'protected_files::protected_directory_tests::writable_directory_cannot_be_accepted',
                    '--exact', '--nocapture'], expected=101)
                text = log.read_text()
                if ('directory accepted a snapshot that violates the shared acceptance property' not in text
                        or 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' not in text):
                    raise ValueError('mutant failed for another reason')
                entry['commands'].append(item)
            record['variants'][variant] = entry
            (out / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
    finally:
        if source is not None:
            _, record['last_mutant_cache_cleanup'] = run(out, 'remove-own-mutant-cache',
                ['cargo', 'clean', '-p', 'sandproof-internal'])
    record.update(success=True, seconds=time.monotonic() - start)
    (out / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps({'success': True, 'seconds': record['seconds']}))


if __name__ == '__main__':
    main()
