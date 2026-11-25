"""Execute the original, split-commit, and restored branch quota assertion."""

import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import tarfile

from source_profile import linux_admission, source_profile


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
BRANCHING = 'controller-service/src/controller/branching.rs'
BRANCH_TEST = 'controller-service/src/controller/lifecycle_inbox/branching_tests.rs'
TEST = ('controller::lifecycle_inbox::recording::branching_tests::'
        'host_branch_quota_retires_with_exact_terminal_parent_window_and_rejects_old_replay')
MUTATION_ANCHOR = b'    Ok(())\n}\n\npub(super) fn check_parent_unload('
EARLY_COMMIT = b'    tx.execute_batch("COMMIT; BEGIN IMMEDIATE;")?;\n'
COMPONENT_SOURCE = {
    'sandbox_controller_core': 'controller-core/src/lib.rs',
    'sandproof_protocol': 'sandbox-protocol/src/lib.rs',
    'sandproof_controller_service': 'controller-service/src/lib.rs',
}


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def require(condition, explanation):
    if not condition:
        raise ValueError(explanation)


def rebuild(args):
    inventory = json.loads((HERE / 'inventory.json').read_bytes())
    checkout = args.checkout.resolve(strict=True)
    output = args.output.resolve()
    admission = linux_admission(output) if args.approved_linux else {}
    archived, tree, _, identity = source_profile(
        checkout, inventory, args.approved_linux, args.candidate,
    )
    commit = identity.get('combined_commit', inventory['source_commit'])
    output.mkdir(mode=0o700)
    source_archive = output / 'source.tar'
    report = dict(kind='hot-branch-close-local-native-development', complete=False,
                  commit=commit, tree=tree, platform=platform.platform(), variants={}, **identity)
    if admission:
        report['linux_admission'] = admission
    try:
        with source_archive.open('xb') as stream:
            subprocess.run(['git', '-C', checkout, 'archive', archived], stdout=stream, check=True)
        report['archive_sha256'] = sha(source_archive)
        for variant in ('original', 'split-commit', 'restored'):
            source = output / variant
            target = output / (variant + '-target')
            source.mkdir(mode=0o700)
            with tarfile.open(source_archive) as stream:
                stream.extractall(source, filter='data')
            for name, expected in inventory['source_inputs'].items():
                require(sha(source / name) == expected, 'archived source differs: ' + name)
            test_source = (source / BRANCH_TEST).read_text().splitlines()
            require(test_source[529].strip() == 'must_retain_quota(&mut journal);'
                    and 'fixture::refresh_epoch' in test_source[528],
                    'fixed post-restart capacity assertion moved')
            branch = source / BRANCHING
            original = branch.read_bytes()
            require(original.count(MUTATION_ANCHOR) == 1, 'split-commit mutation is ambiguous')
            body = (original.replace(MUTATION_ANCHOR, EARLY_COMMIT + MUTATION_ANCHOR)
                    if variant == 'split-commit' else original)
            branch.write_bytes(body)
            environment = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE='true',
                               CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='4', CARGO_TERM_COLOR='never',
                               RUST_BACKTRACE='1')
            for variable in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
                environment.pop(variable, None)
            compile_log = output / (variant + '-compile.jsonl')
            compile_error = output / (variant + '-compile.stderr')
            cargo = ['cargo', 'test', '--locked', '--offline', '--no-run',
                     '--message-format=json', '-p', 'sandproof-controller-service', '--lib']
            with compile_log.open('xb') as stdout, compile_error.open('xb') as stderr:
                subprocess.run(cargo, cwd=source, env=environment,
                               stdout=stdout, stderr=stderr, timeout=600, check=True)
            rows = [json.loads(line) for line in compile_log.read_text().splitlines()]
            compiled = []
            for package, entry in COMPONENT_SOURCE.items():
                (actual,) = (
                    row for row in rows if row.get('reason') == 'compiler-artifact'
                    and row['target']['name'] == package and row['target']['src_path'] == str(source / entry)
                    and row['profile']['test'] is (package == 'sandproof_controller_service')
                )
                require(actual['fresh'] is False and all(Path(name).is_relative_to(target)
                                                        for name in actual['filenames']),
                        'workspace crate was not freshly built in the original target: ' + package)
                compiled.append(actual)
            executable = Path(compiled[-1]['executable'])
            require(executable.is_relative_to(target) and executable.is_file() and not executable.is_symlink(),
                    'executed test is not the freshly compiled candidate binary')
            executable_sha = sha(executable)
            test_log = output / (variant + '-test.log')
            with test_log.open('xb') as stream:
                test = subprocess.run([executable, TEST, '--exact', '--nocapture', '--test-threads=1'],
                                      cwd=source, env=environment, stdout=stream,
                                      stderr=subprocess.STDOUT, timeout=60, check=False)
            text = test_log.read_text()
            if variant == 'split-commit':
                require(test.returncode == 101
                        and 'Err(JournalError::MaintenanceCapacity)' in text
                        and re.search(r'recording::branching_tests::'
                                      r'host_branch_quota_retires_with_exact_terminal_parent_window'
                                      r'_and_rejects_old_replay\s*\n\s*at .*branching_tests\.rs:530:',
                                      text)
                        and 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' in text,
                        'split-commit case did not hit the original capacity assertion after reopening')
                outcome = 'rejected at original capacity assertion after reopening'
            else:
                require(test.returncode == 0 and f'test {TEST} ... ok' in text
                        and 'test result: ok. 1 passed; 0 failed; 0 ignored;' in text,
                        'original quota assertion did not pass exactly once')
                outcome = 'one original quota assertion passed'
            require(executable_sha == sha(executable) and branch.read_bytes() == body,
                    'candidate executable or branch source changed during assertion')
            report['variants'][variant] = dict(
                outcome=outcome, exit_code=test.returncode,
                branch_source_sha256=sha(branch), executable_sha256=executable_sha,
                executable=str(executable.relative_to(output)), cargo=cargo, test=TEST,
                compiler_artifacts={package: row['target']['src_path']
                                    for package, row in zip(COMPONENT_SOURCE, compiled, strict=True)},
                logs={file.name: sha(file) for file in (compile_log, compile_error, test_log)},
            )
        require(sha(source_archive) == report['archive_sha256'], 'original source archive changed')
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(f'original, split-commit rejection, restored: {output}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--checkout', type=Path, default=ROOT)
    parser.add_argument('--approved-linux', action='store_true')
    parser.add_argument('--candidate', help='the pinned combined Linux commit')
    parser.add_argument('--output', type=Path, required=True)
    rebuild(parser.parse_args())
