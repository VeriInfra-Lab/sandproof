"""Ning-only SQLite checks of the original and composed source-save views."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

import inputs as v

GROUP = 'controller::lifecycle_inbox::source_work::tests::'
CASE = GROUP + 'proof_source_save_rejects_same_binding_with_different_complete_work'
ORDER_CASE = GROUP + 'proof_source_save_validation_precedes_any_row_update'
TESTS = Path('controller-service/src/controller/lifecycle_inbox/source_work/tests.rs')
REJECTION = 'the same binding must not replace different complete SourceWork: Ok(()), candidate persisted: true'


def candidate():
    if subprocess.check_output(['git', 'status', '--porcelain'], cwd=v.ROOT):
        raise ValueError('the exact proof candidate must be clean')
    commit, tree = subprocess.check_output(['git', 'rev-parse', 'HEAD', 'HEAD^{tree}'], cwd=v.ROOT, text=True).splitlines()
    changed = subprocess.check_output(
        ['git', 'diff', '--name-only', v.SOURCE, commit, '--', 'Cargo.lock', 'Cargo.toml', 'rust-toolchain.toml',
         ':(glob)**/src/**', ':(glob)**/Cargo.toml'], cwd=v.ROOT)
    if changed:
        raise ValueError('compiled production inputs differ from the source anchor')
    return dict(commit=commit, tree=tree, source=v.SOURCE,
                rules={p.name: v.sha(p) for p in v.HERE.iterdir() if p.is_file()})


def inventory(source):
    return {str(p.relative_to(source)): v.sha(p) for p in sorted(source.rglob('*')) if p.is_file()}


def command(argv, source, log, env):
    with log.open('xb') as stream:
        process = subprocess.run(list(map(str, argv)), cwd=source, env=env,
                                 stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
    return dict(exit_code=process.returncode, log_sha256=v.sha(log))


def prepare_view(output, mode):
    output.mkdir(mode=0o700)
    source = v.snapshot(output)
    staged = v.stage(source, mode) if mode != 'original' else {str(v.CODEC): v.sha(source / v.CODEC)}
    with (source / TESTS).open('ab') as stream:
        stream.write(b'\n' + v.pinned(v.SAVE_PARENT, v.SAVE_PACKAGE / 'save-codec/native-save-order.rs.in'))
        stream.write(b'\n' + v.provider(v.OLD_PACKAGE / 'first-match/native-first-match.rs.in'))
    return source, staged


def view(output, mode):
    source, staged = prepare_view(output, mode)
    before = inventory(source)
    target = output / 'target'
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(target), CARGO_TERM_COLOR='never')
    for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(name, None)
    compile_log = output / 'compile.log'
    compiled = command(['cargo', 'test', '--locked', '--offline', '-p', 'sandproof-controller-service',
                        '--features', 'rusqlite/hooks', '--lib', '--no-run', '--message-format=json'], source, compile_log, env)
    if compiled['exit_code'] != 0:
        raise ValueError('the actual native view failed to compile: ' + mode)
    artifacts = [json.loads(line) for line in compile_log.read_text().splitlines() if line.startswith('{')]
    item, = [row for row in artifacts if row.get('reason') == 'compiler-artifact'
             and row['target']['name'] == 'sandproof_controller_service'
             and row['profile']['test'] and row.get('executable')]
    binary = Path(item['executable'])
    if (item['fresh'] is not False or binary.is_symlink()
            or item['target']['src_path'] != str(source / v.LIB)
            or item['manifest_path'] != str(source / 'controller-service/Cargo.toml')
            or not binary.resolve(strict=True).is_relative_to(target.resolve(strict=True))):
        raise ValueError('a fresh test executable containing the actual same-crate business source is required')
    binary_sha = v.sha(binary)
    listed = command([binary, GROUP, '--list'], source, output / 'list.log', env)
    tests = re.findall(r'^(' + re.escape(GROUP) + r'\S+): test$', (output / 'list.log').read_text(), re.M)
    if listed['exit_code'] != 0 or not {CASE, ORDER_CASE}.issubset(tests) or len(set(tests)) != len(tests):
        raise ValueError('both original assertions and the complete original test group are required')
    selected = ['--exact', CASE] if mode == 'binding-only' else [GROUP]
    asserted = command([binary, *selected, '--nocapture'], source, output / 'assertions.log', env)
    text = (output / 'assertions.log').read_text()
    if mode == 'binding-only':
        valid = (asserted['exit_code'] == 101 and REJECTION in text
                 and 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' in text and CASE in text)
    else:
        valid = (asserted['exit_code'] == 0
                 and f'test result: ok. {len(tests)} passed; 0 failed; 0 ignored;' in text
                 and all('test ' + case + ' ... ok' in text for case in (CASE, ORDER_CASE)))
    if not valid:
        raise ValueError('the same original SQLite assertion did not produce its required result: ' + mode)
    if before != inventory(source) or v.sha(binary) != binary_sha:
        raise ValueError('the actual compiled source or executable changed during native checks')
    return dict(staged=staged, compilation=compiled, assertions=asserted, listed_tests=len(tests),
                sources=before, actual_test_executable_sha256=binary_sha,
                business_source_in_same_crate_test=True, property_rejected=mode == 'binding-only')


def prepare_only(output):
    os.umask(0o077)
    output.mkdir(mode=0o700)
    result = dict(source=v.SOURCE, preparation_complete=False, native_compiled=False, native_executed=False, views={})
    try:
        for mode in ('original', 'correct', 'binding-only'):
            source, staged = prepare_view(output / mode, mode)
            result['views'][mode] = dict(staged=staged, sources=inventory(source),
                                        fixture_sha256=v.sha(source / TESTS))
        result['preparation_complete'] = True
    finally:
        v.write_json(output / 'result.json', result)
    print('Three exact native source views prepared; no native compilation or tests executed')


def native(output):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'native' or sys.platform != 'linux':
        raise ValueError('the Linux Ning native role is required')
    os.umask(0o077)
    before = candidate()
    fixed = json.loads((v.ROOT / 'verification/foundations-environment.json').read_bytes())
    rustc = subprocess.check_output(['rustc', '-vV'], cwd=v.ROOT, text=True)
    if rustc.splitlines()[0] != fixed['production_compiler']:
        raise ValueError('the original registered production compiler is required')
    output.mkdir(mode=0o700)
    result = dict(kind='source-save-retention-native', complete=False, candidate=before, rustc=rustc, views={})
    try:
        for mode in ('original', 'correct', 'binding-only'):
            result['views'][mode] = view(output / mode, mode)
        if before != candidate():
            raise ValueError('the exact candidate changed during native checks')
        result['complete'] = True
    finally:
        v.write_json(output / 'result.json', result)
    print('Original and composed SQLite groups passed; actual binding-only assertion rejected')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    (prepare_only if args.prepare_only else native)(args.output.resolve())
