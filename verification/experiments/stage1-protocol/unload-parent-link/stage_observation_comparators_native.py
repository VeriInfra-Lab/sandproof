"""Stage the actual comparator ports and the unchanged #963 SQLite fixture."""
import argparse
from pathlib import Path
import subprocess

import observation_comparators_capture as capture
from observation_slot_capture import HERE, finish
from stage_observation_slot_native import TESTS, TESTS_SHA256


def stage(source, mode):
    def git(*args):
        return subprocess.check_output(['git', *args], cwd=source, text=True).strip()

    if (git('rev-parse', '--show-toplevel') != str(source)
            or git('rev-parse', 'HEAD') != finish.SOURCE
            or git('rev-parse', 'HEAD^{tree}') != finish.TREE or git('status', '--porcelain')):
        raise ValueError('a clean checkout of the exact integration tree is required')
    test_source = source / TESTS
    if capture.slot.digest(test_source) != TESTS_SHA256:
        raise ValueError('the fixed original fixture registration differs')
    registration = b'mod original_continue;\n'
    text = test_source.read_bytes()
    fixture = source / TESTS.with_suffix('') / 'observation_slot.rs'
    if text.count(registration) != 1 or fixture.exists() or fixture.is_symlink():
        raise ValueError('unique original registration and new fixture path required')
    capture.stage(source, mode)
    fixture.write_bytes((HERE / 'native-observation-slot.rs.in').read_bytes())
    test_source.write_bytes(text.replace(registration, registration + b'mod observation_slot;\n'))


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', choices=capture.MODES, required=True)
    cli.add_argument('--source', type=Path, required=True)
    args = cli.parse_args()
    stage(args.source.resolve(strict=True), args.mode)
