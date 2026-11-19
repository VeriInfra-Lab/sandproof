"""Stage one exact observation slot experiment in a clean integration worktree."""
import argparse
from pathlib import Path
import subprocess

import observation_slot_capture as slot


WORK = Path(slot.finish.prepare.SOURCE)
TESTS = WORK.with_suffix('') / 'tests.rs'
TESTS_SHA256 = '77c06c23da0d136e07be206ea2df71a2182697490805a2b2b19cbef6cc28d0bb'


def stage(source, mode):
    def git(*args):
        return subprocess.check_output(['git', *args], cwd=source, text=True).strip()

    if (git('rev-parse', '--show-toplevel') != str(source)
            or git('rev-parse', 'HEAD') != slot.finish.SOURCE
            or git('rev-parse', 'HEAD^{tree}') != slot.finish.TREE
            or git('status', '--porcelain')):
        raise ValueError('a clean checkout of the fixed integration tree is required')
    for name, expected in {**slot.finish.prepare.SOURCE_INPUTS, str(TESTS): TESTS_SHA256}.items():
        if slot.digest(source / name) != expected:
            raise ValueError('the fixed integration input differs: ' + name)

    registration = b'mod original_continue;\n'
    tests = (source / TESTS).read_bytes()
    fixture = source / TESTS.with_suffix('') / 'observation_slot.rs'
    if tests.count(registration) != 1 or fixture.exists() or fixture.is_symlink():
        raise ValueError('the unique source test registration and a fresh fixture path are required')
    work = source / WORK
    source_text = slot.factor(work.read_text(), mode)
    fixture.write_bytes((slot.HERE / 'native-observation-slot.rs.in').read_bytes())
    work.write_text(source_text)
    (source / TESTS).write_bytes(tests.replace(registration, registration + b'mod observation_slot;\n'))


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--source', required=True, type=Path)
    cli.add_argument('--mode', required=True, choices=('core', 'binding-only'))
    args = cli.parse_args()
    stage(args.source.resolve(strict=True), args.mode)
