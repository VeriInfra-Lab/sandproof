"""Prepare four exact native views and fifteen shared assertions; never test."""
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
PARENT = HERE.parent / 'source-material-charge'
sys.path.insert(0, str(HERE.parent / 'source-remove-retired'))


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


capture = module('source_retirement_question_native_capture', HERE / 'capture.py')
parent_native = module('source_retirement_question_parent_native', PARENT / 'native.py')
VARIANTS = ('original', 'adapted-original', 'sandbox-only-request', 'adapted-sandbox-only-request')
OWN_TESTS = (
    'source_question_checks_all_seven_original_request_fields',
    'source_question_missing_record_precedes_request_errors',
    'source_question_keeps_the_first_matching_record_error',
    'source_question_selects_complete_work_instead_of_only_its_binding',
)
EXPECTED_TESTS = (*parent_native.EXPECTED_TESTS, *OWN_TESTS)
REJECTION = 'retirement question must match all seven original request fields'


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(output, variant):
    os.umask(0o077)
    if variant not in VARIANTS:
        raise ValueError('unknown original retirement-question native view')
    parent_fixture = ((parent_native.PARENT / 'native.rs.in').read_bytes()
                      + b'\n' + (PARENT / 'native.rs.in').read_bytes())
    own_fixture = (HERE / 'native.rs.in').read_bytes()
    bridge = (HERE / 'native-bridge.rs.in').read_bytes()
    fixture = parent_fixture + b'\n' + own_fixture
    if tuple(re.findall(rb'#\[test\]\s+fn\s+(\w+)\(', fixture)) != tuple(n.encode() for n in EXPECTED_TESTS):
        raise ValueError('all eleven parent and four current assertions are mandatory')
    if REJECTION.encode() not in own_fixture or bridge.count(b'    find(sources, question)\n') != 1:
        raise ValueError('the original lookup bridge and precise property rejection are required')
    output.mkdir(mode=0o700)
    archive = output / 'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git', 'archive', capture.retire.SOURCE], cwd=capture.retire.REPO, stdout=stream, check=True)
    if digest(archive) != capture.retire.ARCHIVE:
        raise ValueError('the exact complete fixed production archive is required')
    source = output / 'source'
    source.mkdir(mode=0o700)
    with tarfile.open(archive) as inputs:
        inputs.extractall(source, filter='data')
    original = parent_native.parent_native.inventory(source)
    actual = variant.removeprefix('adapted-')
    if variant.startswith('adapted-'):
        capture.reconstruct(archive, source, actual)
        with tarfile.open(archive) as inputs:
            (source / capture.retire.LIB).write_bytes(inputs.extractfile(str(capture.retire.LIB)).read())
        targets = capture.retire.STAGED[:-1]
    else:
        path = source / capture.retire.ports.RETIREMENT
        path.write_text(capture.mutate(path.read_text(), actual))
        targets = (capture.retire.ports.RETIREMENT,)
    path = source / capture.retire.ports.RETIREMENT
    path.write_bytes(path.read_bytes() + b'\n' + bridge)
    pre_format = {str(path): digest(source / path) for path in targets}
    for path in targets:
        subprocess.run(['rustfmt', '--edition', '2024', str(source / path)], cwd=source, check=True)
    formatted = {str(path): digest(source / path) for path in targets}
    tests = source / parent_native.parent_native.TESTS
    tests.write_bytes(tests.read_bytes() + b'\n' + fixture)
    subprocess.run(['rustfmt', '--edition', '2024', str(tests)], cwd=source, check=True)
    installed = parent_native.parent_native.inventory(source)
    changed = {name for name in original if original[name] != installed[name]}
    allowed = {str(path) for path in (*targets, parent_native.parent_native.TESTS)}
    if (original.keys() != installed.keys() or changed != allowed
            or installed[str(capture.retire.LIB)] != original[str(capture.retire.LIB)]):
        raise ValueError('only exact staging/mutation, test bridge and shared assertions may change')
    result = dict(prepared=True, compiled=False, tests_executed=False,
        source=capture.retire.SOURCE, tree=capture.retire.TREE, archive=capture.retire.ARCHIVE, variant=variant,
        driver=digest(Path(__file__)), capture_driver=digest(HERE / 'capture.py'),
        parent_driver=digest(PARENT / 'native.py'), adapter_driver=digest(capture.PARENT / 'extract.py'),
        ports_driver=digest(capture.PARENT / 'ports.py'),
        parent_fixture=sha256(parent_fixture).hexdigest(), own_fixture=sha256(own_fixture).hexdigest(),
        bridge=sha256(bridge).hexdigest(), fixture=sha256(fixture).hexdigest(),
        tests=list(EXPECTED_TESTS), pre_format=pre_format, formatted=formatted,
        changed=sorted(changed), original=original, installed=installed)
    (output / 'input.json').write_text(json.dumps(result, indent=2) + '\n')
    print(variant, 'fifteen identical original assertions prepared; compilation/tests pending')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    cli.add_argument('--output', type=Path, required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(), args.variant)
