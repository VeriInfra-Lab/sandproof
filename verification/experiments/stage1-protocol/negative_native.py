"""Run unchanged protocol assertions against isolated, actually compiled wrong Rust."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tarfile
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
MUTATIONS = [
    dict(name='skip-disk-id', source='sandbox-protocol/src/lifecycle/manifest.rs',
         old='if *drive_id != drive.drive_id {',
         new='if *drive_id != drive.drive_id && false {', profile='manifest',
         test='file_names_kinds_and_all_profile_drives_are_checked'),
    dict(name='wrapping-manifest-length', source='sandbox-protocol/src/lifecycle/manifest.rs',
         old='current\n                    .checked_add(length)\n                    .ok_or(ManifestError::LengthOverflow)',
         new='Ok(current.wrapping_add(length))', profile='manifest',
         test='lengths_are_positive_and_sum_never_wraps'),
    dict(name='same-incarnation', source='sandbox-protocol/src/lifecycle/request.rs',
         old='target_incarnation.get() <= source_incarnation.get()',
         new='target_incarnation.get() < source_incarnation.get()', profile='request',
         test='resume_allows_number_gaps_but_never_old_or_same_incarnation'),
    dict(name='key-only-replay', source='sandbox-protocol/src/lifecycle/request.rs',
         old='        self == original\n',
         new='        self.binding.key == original.binding.key\n', profile='request',
         test='replay_compares_complete_original_input'),
    dict(name='key-only-confirmation', source='sandbox-protocol/src/lifecycle/renewal.rs',
         old='self.request == *request && self.is_sdk_renewal()',
         new='self.request.request.binding.key == request.request.binding.key && self.is_sdk_renewal()',
         profile='request', test='renewal_confirmation_compares_complete_accepted_intent'),
    dict(name='skip-capture-binding', source='sandbox-protocol/src/lifecycle/manifest.rs',
         old='if self.binding != *expected {',
         new='if self.binding != *expected && false {', profile='manifest',
         test='full_manifest_has_exact_original_binding_and_complete_files'),
]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(output, target, archive):
    output.mkdir(mode=0o700)
    target.mkdir(mode=0o700)
    started = time.monotonic()
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE='true',
               CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='4', RUST_TEST_THREADS='1')
    for name in ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'LD_PRELOAD', 'LD_LIBRARY_PATH']:
        env.pop(name, None)
    result = dict(kind='stage1-isolated-native-negatives-not-formal-proof',
                  archive_sha256=sha(archive), variants=[])

    def command(folder, name, argv, cwd):
        log = folder / (name + '.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            proc = subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                  stdout=stream, stderr=subprocess.STDOUT, timeout=1200)
        return dict(argv=argv, exit_code=proc.returncode,
                    seconds=time.monotonic()-begin, log_sha256=sha(log))

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
            file = source / mutation['source']
            original = file.read_text()
            if original.count(mutation['old']) != 1:
                raise ValueError('mutation must match one exact actual production expression')
            file.write_text(original.replace(mutation['old'], mutation['new'], 1))
            shutil.copy2(file, folder / 'mutation.rs')
            row = dict(mutation, original_source_sha256=hashlib.sha256(original.encode()).hexdigest(),
                       wrong_source_sha256=sha(file))
            result['variants'].append(row)
            args = ['cargo', 'test', '-p', 'sandproof-protocol', '--test', 'lifecycle_contracts',
                    '--locked', '--offline', '--no-run', '--message-format=json']
            row['build'] = command(folder, 'build', args, source)
            if row['build']['exit_code']:
                raise ValueError('wrong Rust must compile before its assertion is tested')
            library, test = None, None
            for line in (folder / 'build.log').read_text().splitlines():
                if not line.startswith('{'):
                    continue
                item = json.loads(line)
                if item.get('reason') != 'compiler-artifact':
                    continue
                t = item['target']
                if t['name'] == 'sandproof_protocol' and 'lib' in t['kind']:
                    library = item
                    expected = source / 'sandbox-protocol/src/lib.rs'
                elif t['name'] == 'lifecycle_contracts':
                    test = item
                    expected = source / 'sandbox-protocol/tests/lifecycle_contracts.rs'
                else:
                    continue
                if Path(t['src_path']).resolve() != expected.resolve() or item['fresh'] is not False:
                    raise ValueError('actual library/test is stale or from another source')
            if library is None or test is None or not test['executable']:
                raise ValueError('missing actual own library or test program')
            compiled = folder / 'compiled'
            compiled.mkdir(mode=0o700)
            for item in [library, test]:
                for name in item['filenames']:
                    p = Path(name)
                    if p.is_file():
                        shutil.copy2(p, compiled / p.name)
            row['compiler_artifacts'] = [library, test]
            row['compiled_sha256'] = {p.name: sha(p) for p in compiled.iterdir()}
            args = [test['executable'], mutation['test'], '--exact', '--nocapture', '--test-threads=1']
            row['assertion'] = command(folder, 'assertions', args, source)
            text = (folder / 'assertions.log').read_text()
            if (row['assertion']['exit_code'] != 101 or 'assertion' not in text
                    or '1 failed' not in text or mutation['test'] not in text):
                raise ValueError('unchanged assertion did not reject the compiled wrong implementation')
            row['rejected_by_unchanged_assertion'] = True
            (output / 'progress.json').write_text(json.dumps(result, indent=2)+'\n')
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
