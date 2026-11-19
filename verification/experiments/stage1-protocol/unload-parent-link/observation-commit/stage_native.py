"""Stage the SQLite fixture and the single hook-order control in a clean source checkout."""

import argparse
from hashlib import sha256
from pathlib import Path
import subprocess


SOURCE_COMMIT = 'b0d791ccc4d4e058b710dbf8f46766652b2853df'
SOURCE_TREE = '7ca91afd9cd79601d562f3c636066d192fc25fc5'
WORK = Path('controller-service/src/controller/lifecycle_inbox/source_work.rs')
WORK_SHA256 = '61191bb746157427e97ba10b518a7afbb61f52e4515e5e53a20e832bb79a916a'
TESTS = WORK.with_suffix('') / 'tests.rs'
TESTS_SHA256 = '77c06c23da0d136e07be206ea2df71a2182697490805a2b2b19cbef6cc28d0bb'
FIXTURE = Path(__file__).with_name('native-observation-commit.rs.in')


def git(source, *arguments):
    return subprocess.check_output(['git', *arguments], cwd=source, text=True).strip()


def checked_file(path, expected):
    contents = path.read_bytes()
    if sha256(contents).hexdigest() != expected:
        raise ValueError(f'fixed original input differs: {path}')
    return contents


def stage(source, mode):
    if (git(source, 'rev-parse', '--show-toplevel') != str(source)
            or git(source, 'rev-parse', 'HEAD') != SOURCE_COMMIT
            or git(source, 'rev-parse', 'HEAD^{tree}') != SOURCE_TREE
            or git(source, 'status', '--porcelain')):
        raise ValueError('a clean checkout of the fixed integration commit is required')
    work = checked_file(source / WORK, WORK_SHA256)
    tests = checked_file(source / TESTS, TESTS_SHA256)
    fixture = source / TESTS.with_suffix('') / 'observation_commit.rs'
    if fixture.exists() or fixture.is_symlink():
        raise ValueError('the original checkout must have no staged fixture')
    registration = b'mod original_continue;\n'
    if tests.count(registration) != 1:
        raise ValueError('the unique original test registration is required')
    native = FIXTURE.read_bytes()
    if mode == 'early-after-commit':
        method = b'    pub(crate) fn save_source_observation<'
        before, body = work.split(method)
        commit = b'        tx.commit()?;\n'
        after = b'        crashes.hit(CrashPoint::AfterCommit)?;\n'
        if body.count(commit + after) != 1:
            raise ValueError('the unique original observation commit and hook are required')
        (source / WORK).write_bytes(before + method + body.replace(commit + after, after + commit, 1))
    fixture.write_bytes(native)
    (source / TESTS).write_bytes(tests.replace(registration, b'mod observation_commit;\n' + registration, 1))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', required=True, type=Path)
    parser.add_argument('--mode', required=True, choices=('core', 'early-after-commit'))
    args = parser.parse_args()
    stage(args.source.resolve(strict=True), args.mode)
