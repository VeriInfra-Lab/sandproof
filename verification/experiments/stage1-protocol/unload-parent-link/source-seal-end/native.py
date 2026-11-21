"""Prepare four complete native views and eighteen shared assertions; never test."""
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
PARENT = HERE.parent / 'source-retirement-question'
sys.path.insert(0, str(HERE.parent / 'source-remove-retired'))


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


capture = module('source_seal_end_native_capture', HERE / 'capture.py')
parent_native = module('source_seal_end_parent_native', PARENT / 'native.py')
base_native = parent_native.parent_native.parent_native
VARIANTS = ('original', 'adapted-original', 'ignore-source-generation', 'adapted-ignore-source-generation')
OWN_TESTS = (
    'source_seal_end_keeps_complete_output_without_assuming_capture_presence',
    'source_seal_end_checks_all_six_original_identity_fields',
    'source_seal_end_rejects_non_seal_commands',
)
EXPECTED_TESTS = (*parent_native.EXPECTED_TESTS, *OWN_TESTS)
REJECTION = 'seal end must retain every original sandbox, source generation, VM and action binding'
FULL = capture.END.parent.with_suffix('.rs')


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(output, variant):
    os.umask(0o077)
    if variant not in VARIANTS:
        raise ValueError('unknown original seal-end native view')
    fixture_paths = [HERE.parent / name / 'native.rs.in' for name in
                     ('source-remove-retired', 'source-material-charge', 'source-retirement-question')]
    parent_fixture = b'\n'.join(path.read_bytes() for path in fixture_paths)
    own_fixture = (HERE / 'native.rs.in').read_bytes()
    fixture = parent_fixture + b'\n' + own_fixture
    bridges = {capture.retire.ports.RETIREMENT: (PARENT / 'native-bridge.rs.in').read_bytes(),
               FULL: (HERE / 'native-bridge.rs.in').read_bytes()}
    if tuple(re.findall(rb'#\[test\]\s+fn\s+(\w+)\(', fixture)) != tuple(n.encode() for n in EXPECTED_TESTS):
        raise ValueError('all fifteen parent and three current assertions are mandatory')
    if (REJECTION.encode() not in own_fixture or bridges[FULL].count(b'    end::seal_end(record, action)\n') != 1
            or bridges[capture.retire.ports.RETIREMENT].count(b'    find(sources, question)\n') != 1):
        raise ValueError('both actual original bridges and the specific property rejection are required')
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
    original = base_native.inventory(source)
    actual = variant.removeprefix('adapted-')
    if variant.startswith('adapted-'):
        capture.reconstruct(archive, source, actual)
        with tarfile.open(archive) as inputs:
            (source / capture.retire.LIB).write_bytes(inputs.extractfile(str(capture.retire.LIB)).read())
        targets = tuple(path for path in capture.STAGED if path != capture.retire.LIB)
    else:
        path = source / capture.END
        path.write_text(capture.mutate(path.read_text(), actual))
        targets = (capture.END,)
    for path, bridge in bridges.items():
        target = source / path
        target.write_bytes(target.read_bytes() + b'\n' + bridge)
    targets = tuple(dict.fromkeys((*targets, *bridges)))
    pre_format = {str(path): digest(source / path) for path in targets}
    for path in targets:
        subprocess.run(['rustfmt', '--edition', '2024', str(source / path)], cwd=source, check=True)
    formatted = {str(path): digest(source / path) for path in targets}
    tests = source / base_native.TESTS
    tests.write_bytes(tests.read_bytes() + b'\n' + fixture)
    subprocess.run(['rustfmt', '--edition', '2024', str(tests)], cwd=source, check=True)
    installed = base_native.inventory(source)
    changed = {name for name in original if original[name] != installed[name]}
    required = {str(base_native.TESTS), *(str(path) for path in bridges)}
    if variant.startswith('adapted-'):
        required.update(str(path) for path in capture.BASE_STAGED if path != capture.retire.LIB)
    if actual != 'original':
        required.add(str(capture.END))
    allowed = {str(path) for path in (*targets, base_native.TESTS)}
    if (original.keys() != installed.keys() or not required <= changed <= allowed
            or installed[str(capture.retire.LIB)] != original[str(capture.retire.LIB)]):
        raise ValueError('only exact original staging/mutation, two test bridges and shared fixtures may change')
    result = dict(prepared=True, compiled=False, tests_executed=False,
        source=capture.retire.SOURCE, tree=capture.retire.TREE, archive=capture.retire.ARCHIVE, variant=variant,
        driver=digest(Path(__file__)), capture_driver=digest(HERE / 'capture.py'),
        parent_driver=digest(PARENT / 'native.py'), adapter_driver=digest(capture.PARENT / 'extract.py'),
        ports_driver=digest(capture.PARENT / 'ports.py'),
        parent_fixture=sha256(parent_fixture).hexdigest(), own_fixture=sha256(own_fixture).hexdigest(),
        bridges={str(path): sha256(body).hexdigest() for path, body in bridges.items()},
        fixture=sha256(fixture).hexdigest(), tests=list(EXPECTED_TESTS),
        pre_format=pre_format, formatted=formatted, changed=sorted(changed), original=original, installed=installed)
    (output / 'input.json').write_text(json.dumps(result, indent=2) + '\n')
    print(variant, 'eighteen identical original assertions prepared; compilation/tests pending')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    cli.add_argument('--output', type=Path, required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(), args.variant)
