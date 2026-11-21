"""Prepare four complete native views and fifty-two shared assertions; never test."""
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
PARENT = HERE.parent / 'source-saved-question'
sys.path.insert(0, str(HERE.parent / 'source-remove-retired'))


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


capture = module('source_drain_validation_native_capture', HERE / 'inputs.py')
parent_native = module('source_drain_validation_parent_native', PARENT / 'native.py')
base_native = parent_native.base_native
VARIANTS = ('original', 'adapted-original', 'allow-mismatched-prior', 'adapted-allow-mismatched-prior')
OWN_TESTS = (
    'source_drain_validation_accepts_exact_saved_observation_and_prior_controls',
    'source_drain_validation_preserves_original_question_errors_before_matching',
    'source_drain_validation_checks_actual_observation_before_writer_success',
    'source_drain_validation_rejects_mismatched_saved_prior',
)

EXPECTED_TESTS = (*parent_native.EXPECTED_TESTS, *OWN_TESTS)
REJECTION = 'drain validation must reject mismatched saved prior'


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(output, variant):
    os.umask(0o077)
    if variant not in VARIANTS:
        raise ValueError('unknown request-end native view')
    fixture_paths = [HERE.parent / name / 'native.rs.in' for name in
                     ('source-remove-retired', 'source-material-charge', 'source-retirement-question', 'source-seal-end', 'source-continuation-gate', 'source-terminal-end', 'source-full-work-guard', 'source-worker-guard', 'source-request-end-match', 'source-prior-end-match', 'source-terminal-match', 'source-full-binding', 'source-saved-question')]
    parent_fixture = b'\n'.join(path.read_bytes() for path in fixture_paths)
    own_fixture = (HERE / 'native.rs.in').read_bytes()
    fixture = parent_fixture + b'\n' + own_fixture
    full = Path('controller-service/src/controller/lifecycle_inbox/source_work/full.rs')
    drain = full.parent / 'full/drain.rs'
    bridges = {
        capture.retire.ports.RETIREMENT: (HERE.parent / 'source-retirement-question/native-bridge.rs.in').read_bytes(),
        full: b'\n'.join((HERE.parent / name / 'native-bridge.rs.in').read_bytes()
                           for name in ('source-seal-end', 'source-continuation-gate', 'source-terminal-end', 'source-full-binding')),
        drain: b'\n'.join((HERE.parent / name / 'native-drain-bridge.rs.in').read_bytes()
                           for name in ('source-terminal-end', 'source-prior-end-match', 'source-terminal-match', 'source-drain-validation')),
    }
    if tuple(re.findall(rb'#\[test\]\s+fn\s+(\w+)\(', fixture)) != tuple(n.encode() for n in EXPECTED_TESTS):
        raise ValueError('all48 parent and four current assertions are mandatory')
    if (bridges[full].count(b'    validate::full_binding(record, full)\n') != 1
            or bridges[drain].count(b'    question::from_saved(record, stop, observation)\n') != 1
            or bridges[drain].count(b'    prior_matches(record, &prior)\n') != 1
            or bridges[drain].count(b'    validate_record(record, stop, observation)\n') != 1
            or bridges[drain].count(b'    writer_closed(record)\n') != 1
            or REJECTION.encode() not in own_fixture
            or bridges[drain].count(b'    terminal_end(old, stop, observation)\n') != 1
            or bridges[full].count(b'    end::seal_end(record, action)\n') != 1
            or bridges[full].count(b'    end::continuation_allowed(record)\n') != 1
            or bridges[capture.retire.ports.RETIREMENT].count(b'    find(sources, question)\n') != 1):
        raise ValueError('all original parent bridges and current saved-prior rejection are mandatory')
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
        required.update(str(path) for path in capture.parent.parent.parent.BASE_STAGED if path != capture.retire.LIB)
    if actual != 'original':
        required.add(str(capture.END))
    allowed = {str(path) for path in (*targets, base_native.TESTS)}
    if (original.keys() != installed.keys() or not required <= changed <= allowed
            or installed[str(capture.retire.LIB)] != original[str(capture.retire.LIB)]):
        raise ValueError('only exact original staging/mutation, three inherited test bridge files and shared fixtures may change')
    result = dict(prepared=True, compiled=False, tests_executed=False,
        source=capture.retire.SOURCE, tree=capture.retire.TREE, archive=capture.retire.ARCHIVE, variant=variant,
        driver=digest(Path(__file__)), capture_driver=digest(HERE / 'inputs.py'),
        parent_driver=digest(PARENT / 'native.py'), adapter_driver=digest(Path(capture.retire.__file__)),
        ports_driver=digest(Path(capture.retire.ports.__file__)),
        parent_fixture=sha256(parent_fixture).hexdigest(), own_fixture=sha256(own_fixture).hexdigest(),
        bridges={str(path): sha256(body).hexdigest() for path, body in bridges.items()},
        fixture=sha256(fixture).hexdigest(), tests=list(EXPECTED_TESTS),
        pre_format=pre_format, formatted=formatted, changed=sorted(changed), original=original, installed=installed)
    (output / 'input.json').write_text(json.dumps(result, indent=2) + '\n')
    print(variant, 'fifty-two identical original assertions prepared; compilation/tests pending')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    cli.add_argument('--output', type=Path, required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(), args.variant)
