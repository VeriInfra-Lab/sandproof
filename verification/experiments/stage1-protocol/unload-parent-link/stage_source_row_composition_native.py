"""Join the combined source candidate and the original exact #950 tests."""
import argparse
from pathlib import Path
import subprocess

from source_row_sql_composition import CODEC, MODES, digest, stage


HERE = Path(__file__).resolve().parent / 'row-sql-composition'
FIXTURE = '56e28c2859ccdd1e5264bef03e9e35752d062fda'
TESTS = CODEC.with_name('tests.rs')
ROWS = TESTS.with_suffix('') / 'codec_rows.rs'
ROWS_SHA = '8a6a56465e27039acdd45db670b78ff86d26a4c9eee9fcf9476ba026c49c35a8'


def install(source, fixture, mode):
    if subprocess.check_output(['git', '-C', fixture, 'rev-parse', 'HEAD'], text=True).strip() != FIXTURE:
        raise ValueError('the exact published #950 test checkout is required')
    source_fixture = fixture / ROWS
    if source_fixture.is_symlink() or digest(source_fixture) != ROWS_SHA:
        raise ValueError('the untouched combined #949/#950 original fixture is required')
    target = source / ROWS
    tests = source / TESTS
    anchor = 'mod continue_application;\n'
    original_tests = tests.read_text()
    if target.exists() or target.is_symlink() or original_tests.count(anchor) != 1:
        raise ValueError('the original fixed source test layout must be used')
    prepared = stage(source, mode)
    with target.open('xb') as new:
        new.write(source_fixture.read_bytes())
    tests.write_text(original_tests.replace(anchor, 'mod codec_rows;\n' + anchor, 1))
    with (source / CODEC).open('ab') as new:
        new.write(b'\n' + (HERE / 'native-writer.rs.in').read_bytes())
    return prepared


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--source', type=Path, required=True)
    cli.add_argument('--fixture', type=Path, required=True)
    cli.add_argument('--mode', choices=MODES, required=True)
    args = cli.parse_args()
    print(args.mode, install(args.source.resolve(), args.fixture.resolve(), args.mode))
