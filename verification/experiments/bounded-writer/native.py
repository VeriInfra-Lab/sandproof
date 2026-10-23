"""Native-only reproduction; caller owns ning's existing native-heavy lock."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import signal
import shutil
import subprocess
import sys
import tarfile
import time
import tomllib
import traceback

from prepare import ARCHIVE_SHA256, prepare
from native_artifacts import OWN, TESTS, build_artifacts, package_identities, preserve_file, quarantine_owned_cache, sha

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.check_bounded_writer import ENVIRONMENT, check_crate_archive, check_frozen, check_sources
from replay import crate_archive as pack_crate


def run(archive, crate, output, review_root=ROOT):
    check_frozen(review_root)
    environment = check_sources(review_root)
    if sha(archive) != ARCHIVE_SHA256 or output.exists():
        raise ValueError('frozen production archive and a fresh evidence directory required')
    target = Path(os.environ['CARGO_TARGET_DIR']).resolve(strict=True)
    output.mkdir(parents=True, mode=0o700)
    if crate is None:
        crate = output / 'crate.tar.gz'
        pack_crate(review_root, crate)
    check_crate_archive(review_root, crate)
    start = time.monotonic()
    record = dict(kind='bounded-writer-native-component-not-full-suite', steps=[], variants={})
    record['production_rustc'] = subprocess.check_output(['rustc', '-vV'], cwd=review_root, text=True)
    record['cargo_version'] = subprocess.check_output(['cargo', '--version'], cwd=review_root, text=True).strip()
    if record['production_rustc'] != environment['production_rustc']:
        raise ValueError('native compiler differs from the frozen production compiler')
    manifest = 'verification/extract-bounded-writer/Cargo.toml'
    last_root = None

    def command(name, args, root, expected=0):
        log = output / (name + '.log')
        began = time.monotonic()
        with log.open('xb') as stream:
            process = subprocess.Popen(args, cwd=root, stdin=subprocess.DEVNULL, stdout=stream,
                stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = process.wait(timeout=900)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                code = 124
        record['steps'].append(dict(name=name, command=args, exit_code=code,
            seconds=time.monotonic() - began, log_sha256=sha(log)))
        (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
        if code != expected:
            raise ValueError('unexpected command result: ' + name)
        return log

    def clean(name, root):
        args = ['cargo', 'clean', '--manifest-path', manifest]
        for package in OWN:
            args += ['--package', package]
        command(name, args, root)
        return quarantine_owned_cache(target, output / (name + '-quarantine'))

    def copy_profile(root, variant):
        shutil.copyfile(review_root / ENVIRONMENT, root / ENVIRONMENT)
        check_sources(root, variant)

    try:
        original = output / 'original-source'
        original.mkdir(mode=0o700)
        for package in (archive, crate):
            with tarfile.open(package) as tf:
                for member in tf.getmembers():
                    path = Path(member.name)
                    if path.is_absolute() or '..' in path.parts or not (member.isfile() or member.isdir()):
                        raise ValueError('unsafe source archive member')
                tf.extractall(original, filter='data')
        frozen = {(p['name'], p['version'], p.get('source'), p.get('checksum'))
                  for p in tomllib.loads((original / 'Cargo.lock').read_text())['package']}
        packages = tomllib.loads((original / 'verification/extract-bounded-writer/Cargo.lock').read_text())['package']
        if any((p['name'], p['version'], p.get('source'), p.get('checksum')) not in frozen
               for p in packages if p.get('source')):
            raise ValueError('registry dependencies changed')
        record['registry_dependencies_match'] = True
        roots = {'original': original}
        for variant in ('correct', 'no-limit'):
            roots[variant] = output / (variant + '-source')
            prepare(archive, roots[variant], crate, variant)
        for variant, root in roots.items():
            last_root = root
            copy_profile(root, None if variant == 'original' else variant)
            row = {'cache_isolation_before': clean(variant + '-clean-before', root)}
            record['variants'][variant] = row
            features = [] if variant == 'original' else ['--features', 'candidate']
            meta_log = command(variant + '-metadata', ['cargo', 'metadata', '--manifest-path', manifest,
                '--locked', '--offline', '--format-version=1'] + features, root)
            metadata = [json.loads(line) for line in meta_log.read_text().splitlines()
                        if line.startswith('{') and '"packages"' in line]
            if len(metadata) != 1:
                raise ValueError('one complete Cargo metadata record required')
            identities = package_identities(root, metadata[0])
            selection = ['--test', 'original_writer'] if variant == 'no-limit' else ['--tests']
            log = command(variant + '-build', ['cargo', 'test', '--manifest-path', manifest,
                '--locked', '--offline', '--no-run', '--message-format=json'] + features + selection, root)
            required = ['original_writer'] if variant == 'no-limit' else list(TESTS)
            artifacts = build_artifacts(root, target, log, identities, required)
            row['artifacts'] = artifacts
            # Keep the exact owned rlibs/rmeta and binaries, then clear only these
            # packages. Dependency objects and the one stable target are retained.
            for package, artifact in artifacts['libraries'].items():
                for item in artifact['filenames']:
                    source = Path(item['path'])
                    preserve_file(source, output / 'compiled' / variant / package / source.name)
            for name, artifact in artifacts['tests'].items():
                binary = preserve_file(Path(artifact['executable']), output / 'compiled' / variant / name)
                if variant == 'no-limit':
                    failed = command('wrong-rejection', [str(binary), '--exact',
                        'successful_status_encoding_never_exceeds_the_original_limit', '--nocapture'], root, 101)
                    if 'bounded encoder published too many bytes' not in failed.read_text() or '1 failed' not in failed.read_text():
                        raise ValueError('wrong rejection is not the original bound assertion')
                else:
                    log = command(variant + '-test-' + name, [str(binary), '--test-threads=1'], root)
                    summary = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', log.read_text())
                    source = root / TESTS[name]
                    expected = len(re.findall(r'^#\[test\]\nfn ', source.read_text(), re.M))
                    if variant == 'original' and name == 'candidate_writer':
                        expected = 0
                    if summary != [(str(expected), '0', '0', '0', '0')]:
                        raise ValueError('missing/skipped/filtered native cases: ' + name)
            check_sources(root, None if variant == 'original' else variant)
            if variant == 'correct':
                command('fmt', ['cargo', 'fmt', '--manifest-path', manifest, '--check'], root)
                command('strict-clippy', ['cargo', 'clippy', '--manifest-path', manifest, '--tests',
                    '--locked', '--offline', '--features', 'candidate', '--', '-D', 'warnings'], root)
        record.update(success=True, source_archive_sha256=sha(archive), crate_archive_sha256=sha(crate))
    except BaseException as error:
        record.update(success=False, error=repr(error), traceback=traceback.format_exc())
    finally:
        if last_root is not None:
            try:
                record['cache_isolation_after'] = clean('final-own-package-clean', last_root)
                for variant in record['variants'].values():
                    for row in variant.get('artifacts', {}).get('libraries', {}).values():
                        if any(Path(item['path']).exists() for item in row['filenames']):
                            raise ValueError('owned variant library still present in reusable target')
            except BaseException as error:
                record.update(success=False, cleanup_error=repr(error))
    record['seconds'] = time.monotonic() - start
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-archive', type=Path, required=True)
    parser.add_argument('--crate-archive', type=Path)
    parser.add_argument('--review-root', type=Path, default=ROOT)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = run(args.source_archive.resolve(strict=True),
                 args.crate_archive.resolve(strict=True) if args.crate_archive else None,
                 args.output.absolute(), args.review_root.resolve(strict=True))
    print(json.dumps(result))
    raise SystemExit(0 if result['success'] else 1)
