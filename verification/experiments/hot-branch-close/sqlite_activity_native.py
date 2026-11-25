"""Witness the original CloseWindow SQL and protected reopen on real SQLite."""

import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile

from native_witness import BRANCHING, BRANCH_TEST, COMPONENT_SOURCE, EARLY_COMMIT, MUTATION_ANCHOR
from source_profile import linux_admission


HERE = Path(__file__).resolve().parent
TEST = ('controller::lifecycle_inbox::recording::branching_tests::sqlite_activity::'
        'original_retirement_sqlite_activity_and_protected_reopen_are_joint')
LOAD_FIELDS = b'let (key, parent, execution, record, actual_digest) = row;'
KEY_GUARD = b'if key != digest(&decoded.key.operation)?\n        || parent != digest(&decoded.parent)?'
REJECTIONS = {
    'split-commit': 'retirement published a partial image before commit',
    'skip-operation-key-check':
        'retirement attempted DELETE or UPDATE before rejecting the mismatched operation key',
}


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def require(value, reason):
    if not value:
        raise ValueError(reason)


def git(checkout, *arguments):
    return subprocess.check_output(['git', '-C', checkout, *arguments], text=True).strip()


def run(args):
    checkout = args.checkout.resolve(strict=True)
    output = args.output.resolve()
    admission = linux_admission(output)
    require(re.fullmatch(r'[0-9a-f]{40}', args.candidate)
            and git(checkout, 'rev-parse', 'HEAD') == args.candidate
            and not git(checkout, 'status', '--porcelain'),
            'requires a clean checkout at the exact reviewed source commit')
    tree = git(checkout, 'rev-parse', 'HEAD^{tree}')
    output.mkdir(mode=0o700)
    archive = output / 'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git', '-C', checkout, 'archive', args.candidate], stdout=stream,
                       timeout=60, check=True)
    report = dict(candidate=args.candidate, tree=tree, complete=False, linux_admission=admission,
                  archive_sha256=digest(archive), variants={})
    try:
        for name in ('original', *REJECTIONS, 'restored'):
            source = output / name
            source.mkdir(mode=0o700)
            target = output / (name + '-target')
            with tarfile.open(archive) as stream:
                stream.extractall(source, filter='data')
            archived_fixture = source / HERE.relative_to(checkout) / 'sqlite_activity.rs.in'
            require(digest(archived_fixture) == digest(HERE / 'sqlite_activity.rs.in'),
                    'executed fixture differs from the clean candidate archive')
            branch_tests = source / BRANCH_TEST
            branch_tests.write_bytes(branch_tests.read_bytes() + archived_fixture.read_bytes())
            product = source / BRANCHING
            original = product.read_bytes()
            require(original.count(MUTATION_ANCHOR) == 1, 'split-commit mutation is ambiguous')
            if name == 'split-commit':
                product.write_bytes(original.replace(MUTATION_ANCHOR, EARLY_COMMIT + MUTATION_ANCHOR))
            elif name == 'skip-operation-key-check':
                require(original.count(LOAD_FIELDS) == original.count(KEY_GUARD) == 1,
                        'operation-key guard mutation is ambiguous')
                product.write_bytes(
                    original.replace(LOAD_FIELDS, LOAD_FIELDS.replace(b'(key,', b'(_key,'))
                    .replace(KEY_GUARD, b'if parent != digest(&decoded.parent)?')
                )
            environment = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE='true',
                               CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='4', CARGO_TERM_COLOR='never')
            for variable in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
                environment.pop(variable, None)
            compile_log = output / (name + '-compile.jsonl')
            compile_error = output / (name + '-compile.stderr')
            cargo = ['cargo', 'test', '--locked', '--offline', '--no-run', '--message-format=json',
                     '-p', 'sandproof-controller-service', '--features',
                     'rusqlite/trace,rusqlite/hooks', '--lib']
            with compile_log.open('xb') as stdout, compile_error.open('xb') as stderr:
                subprocess.run(cargo, cwd=source, env=environment, stdout=stdout, stderr=stderr,
                               timeout=600, check=True)
            rows = [json.loads(line) for line in compile_log.read_text().splitlines()]
            artifacts = {}
            for crate, entry in COMPONENT_SOURCE.items():
                (artifact,) = (
                    row for row in rows if row.get('reason') == 'compiler-artifact'
                    and row['target']['name'] == crate and row['target']['src_path'] == str(source / entry)
                    and row['profile']['test'] is (crate == 'sandproof_controller_service')
                )
                require(not artifact['fresh'] and all(Path(path).is_relative_to(target)
                                                     for path in artifact['filenames']),
                        'workspace crate did not compile from the original isolated source')
                artifacts[crate] = artifact
            executable = Path(artifacts['sandproof_controller_service']['executable'])
            require(executable.is_relative_to(target) and executable.is_file() and not executable.is_symlink(),
                    'candidate executable is not the fresh isolated test binary')
            binary_sha = digest(executable)
            test_log = output / (name + '-test.log')
            argv = [executable, TEST, '--exact', '--nocapture', '--test-threads=1']
            with test_log.open('xb') as stream:
                result = subprocess.run(argv, cwd=source, env=environment, stdout=stream,
                                        stderr=subprocess.STDOUT, timeout=120, check=False)
            text = test_log.read_text()
            if name in REJECTIONS:
                require(result.returncode == 101
                        and REJECTIONS[name] in text
                        and 'test result: FAILED. 0 passed; 1 failed;' in text,
                        'native variant did not reach its original bound assertion: ' + name)
            else:
                require(result.returncode == 0 and f'test {TEST} ... ok' in text
                        and 'test result: ok. 1 passed; 0 failed;' in text,
                        'original protected SQLite witness did not pass exactly once')
            require(digest(executable) == binary_sha, 'observed test binary changed')
            report['variants'][name] = dict(
                exit_code=result.returncode, source_sha256=digest(product),
                fixture_sha256=digest(branch_tests), executable_sha256=binary_sha,
                cargo=cargo, test=list(map(str, argv)), logs={file.name: digest(file)
                    for file in (compile_log, compile_error, test_log)},
            )
        require(digest(archive) == report['archive_sha256'], 'archived source changed during witness')
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--checkout', required=True, type=Path)
    parser.add_argument('--candidate', required=True)
    parser.add_argument('--output', required=True, type=Path)
    run(parser.parse_args())
