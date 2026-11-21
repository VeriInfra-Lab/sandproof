"""Prepare four exact native views and the same eleven assertions; never run tests."""
import argparse
from hashlib import sha256
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-remove-retired'
sys.path.insert(0, str(PARENT))


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


capture = module('source_material_charge_native_capture', HERE / 'capture.py')
parent_native = module('source_material_charge_parent_native', PARENT / 'native.py')
VARIANTS = ('original', 'adapted-original', 'ignore-settlement-bytes', 'adapted-ignore-settlement-bytes')
OWN_TESTS = (
    'source_material_charge_no_capture_requires_empty_retirement_and_release',
    'source_material_charge_retiring_zero_does_not_mean_settled',
    'source_material_charge_settlement_matches_every_original_byte',
    'source_material_charge_state_size_is_rejected_before_plan_shape',
)
EXPECTED_TESTS = (*parent_native.EXPECTED_TESTS, *OWN_TESTS)


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(output, variant):
    os.umask(0o077)
    if variant not in VARIANTS:
        raise ValueError('unknown current native material-charge view')
    parent_fixture = (PARENT / 'native.rs.in').read_bytes()
    own_fixture = (HERE / 'native.rs.in').read_bytes()
    fixture = parent_fixture + b'\n' + own_fixture
    if tuple(re.findall(rb'#\[test\]\s+fn\s+(\w+)\(', fixture)) != tuple(n.encode() for n in EXPECTED_TESTS):
        raise ValueError('all seven parent and four current assertions are mandatory')
    if b'material settlement must match every released byte of the original capture' not in own_fixture:
        raise ValueError('the specific wrong-byte rejection assertion is required')
    output.mkdir(mode=0o700)
    archive = output / 'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git', 'archive', capture.retire.SOURCE], cwd=capture.retire.REPO, stdout=stream, check=True)
    if digest(archive) != capture.retire.ARCHIVE:
        raise ValueError('the exact complete original production archive is required')
    source = output / 'source'
    source.mkdir(mode=0o700)
    with tarfile.open(archive) as inputs:
        inputs.extractall(source, filter='data')
    original = parent_native.inventory(source)
    actual = variant.removeprefix('adapted-')
    if variant.startswith('adapted-'):
        capture.retire.reconstruct(archive, source, 'original')
        with tarfile.open(archive) as inputs:
            (source / capture.retire.LIB).write_bytes(inputs.extractfile(str(capture.retire.LIB)).read())
        targets = capture.retire.STAGED[:-1]
    else:
        targets = (capture.retire.ports.RETIREMENT,)
    path = source / capture.retire.ports.RETIREMENT
    path.write_text(capture.mutate(path.read_text(), actual))
    pre_format = {str(path): digest(source / path) for path in targets}
    for path in targets:
        subprocess.run(['rustfmt', '--edition', '2024', str(source / path)], cwd=source, check=True)
    formatted = {str(path): digest(source / path) for path in targets}
    tests = source / parent_native.TESTS
    tests.write_bytes(tests.read_bytes() + b'\n' + fixture)
    subprocess.run(['rustfmt', '--edition', '2024', str(tests)], cwd=source, check=True)
    installed = parent_native.inventory(source)
    changed = {name for name in original if original[name] != installed[name]}
    allowed = {str(path) for path in (*targets, parent_native.TESTS)}
    required = {str(parent_native.TESTS)}
    if variant != 'original':
        required.update(str(path) for path in targets)
    if (original.keys() != installed.keys() or not required <= changed <= allowed
            or installed[str(capture.retire.LIB)] != original[str(capture.retire.LIB)]):
        raise ValueError('only exact staging/mutation and identical complete fixtures may change')
    result = dict(prepared=True, compiled=False, tests_executed=False,
        source=capture.retire.SOURCE, tree=capture.retire.TREE, archive=capture.retire.ARCHIVE, variant=variant,
        driver=digest(Path(__file__)), capture_driver=digest(HERE / 'capture.py'),
        parent_driver=digest(PARENT / 'native.py'), parent_fixture=sha256(parent_fixture).hexdigest(),
        own_fixture=sha256(own_fixture).hexdigest(), fixture=sha256(fixture).hexdigest(),
        tests=list(EXPECTED_TESTS), pre_format=pre_format, formatted=formatted,
        changed=sorted(changed), original=original, installed=installed)
    (output / 'input.json').write_text(json.dumps(result, indent=2) + '\n')
    print(variant, 'eleven exact parent/current assertions prepared; compilation and tests pending')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    cli.add_argument('--output', type=Path, required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(), args.variant)
