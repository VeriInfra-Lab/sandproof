"""Stage a clean exact observation first-error fixture for native SQLite checks."""
import argparse
from pathlib import Path
import subprocess

import observation_prefix_capture as capture
from stage_observation_slot_native import TESTS, TESTS_SHA256


def stage(source, mode):
    def git(*args):
        return subprocess.check_output(['git', *args], cwd=source, text=True).strip()

    finish = capture.slot.finish
    if (git('rev-parse', '--show-toplevel') != str(source)
            or git('rev-parse', 'HEAD') != finish.SOURCE
            or git('rev-parse', 'HEAD^{tree}') != finish.TREE or git('status', '--porcelain')):
        raise ValueError('a clean checkout of the exact integration tree is required')
    path = source / TESTS
    if capture.slot.digest(path) != TESTS_SHA256:
        raise ValueError('original test registration differs')
    marker = b'mod original_continue;\n'
    text = path.read_bytes()
    fixtures = {
        'observation_prefix': capture.HERE / 'native-observation-prefix.rs.in',
        'observation_slot': capture.slot.HERE / 'native-observation-slot.rs.in',
    }
    destination = source / TESTS.with_suffix('')
    if text.count(marker) != 1 or any((destination / (name + '.rs')).exists()
                                    or (destination / (name + '.rs')).is_symlink() for name in fixtures):
        raise ValueError('unique original registration and fresh fixture paths required')
    capture.stage(source, mode)
    for name, fixture in fixtures.items():
        (destination / (name + '.rs')).write_bytes(fixture.read_bytes())
    registration = b''.join(f'mod {name};\n'.encode() for name in fixtures)
    path.write_bytes(text.replace(marker, marker + registration))


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', choices=('core', 'begin-first'), required=True)
    cli.add_argument('--source', type=Path, required=True)
    args = cli.parse_args()
    stage(args.source.resolve(strict=True), args.mode)
