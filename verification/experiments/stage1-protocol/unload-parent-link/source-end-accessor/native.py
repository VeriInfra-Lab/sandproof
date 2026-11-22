"""Prepare six full native views, retaining all60 parent assertions; never test."""
import argparse
from hashlib import sha256
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from types import ModuleType

HERE = Path(__file__).resolve().parent

def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value

sys.path.insert(0, str(HERE.parent / 'source-remove-retired'))
capture = module('source_end_accessor_native_capture', HERE / 'capture.py')
parent_native = module('source_end_accessor_parent_native', HERE.parent / 'source-end-receipt/native.py')
base_native = parent_native.base_native
VARIANTS = tuple(name for variant in capture.VARIANTS for name in (variant, 'adapted-' + variant))
OWN_TESTS = ('source_end_accessor_preserves_complete_continuation_question',)
EXPECTED_TESTS = (*parent_native.EXPECTED_TESTS, *OWN_TESTS)
REJECTION = {
    'receipt-no-request-as-returned': parent_native.REJECTION,
    'swap-receipt-bindings': 'continuation request must preserve all six complete fields',
}


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def rules():
    # Keep every imported repository Python module and each source fixture in
    # these inherited native packages. No host metadata substitutes for bytes.
    pending, visited, paths = [capture, parent_native, *tuple(sys.modules.values())], set(), {Path(__file__).resolve()}
    while pending:
        value = pending.pop()
        if not isinstance(value, ModuleType) or id(value) in visited:
            continue
        visited.add(id(value))
        filename = getattr(value, '__file__', None)
        if not filename or str(filename).startswith('<'):
            continue
        path = Path(filename).resolve()
        if not path.is_relative_to(capture.retire.REPO):
            continue
        if not path.is_file() or path.suffix != '.py':
            raise ValueError('complete native repository Python inputs required')
        paths.add(path)
        if path.parent.is_relative_to(HERE.parent):
            paths.update(path.parent.glob('*.rs.in'))
        pending.extend(x for x in vars(value).values() if isinstance(x, ModuleType))
    paths.add(HERE / 'native-candidate.json')
    paths.add(HERE / 'registration-candidate.json')
    return {str(p.relative_to(capture.retire.REPO)): digest(p) for p in sorted(paths)}


def prepare(output, variant):
    os.umask(0o077)
    if variant not in VARIANTS:
        raise ValueError('all six exact original/adapted accessor views required')
    fixture = (HERE / 'native.rs.in').read_bytes()
    if tuple(re.findall(rb'#\[test\]\s+fn\s+(\w+)\(', fixture)) != tuple(n.encode() for n in OWN_TESTS):
        raise ValueError('one complete six-field assertion must be appended to all60 parent assertions')
    if (len(parent_native.EXPECTED_TESTS) != 60 or len(set(EXPECTED_TESTS)) != 61
            or REJECTION['swap-receipt-bindings'].encode() not in fixture):
        raise ValueError('all61 identical assertions and the exact rejection are required')
    rule_inputs = rules()
    actual = variant.removeprefix('adapted-')
    legacy = 'no-request-as-returned' if actual == 'receipt-no-request-as-returned' else 'original'
    if variant.startswith('adapted-'):
        legacy = 'adapted-' + legacy
    # The unchanged strict parent installs the exact archive, all60 fixtures,
    # all actual call bridges and only its reviewed production/staging delta.
    parent_native.prepare(output, legacy)
    parent_path = output / 'input.json'
    parent = json.loads(parent_path.read_bytes())
    source = output / 'source'
    before = base_native.inventory(source)
    if (parent['source'] != capture.retire.SOURCE or parent['tree'] != capture.retire.TREE
            or parent['archive'] != capture.retire.ARCHIVE or parent['variant'] != legacy
            or parent['prepared'] is not True or parent['compiled'] is not False
            or parent['tests_executed'] is not False
            or parent['tests'] != list(parent_native.EXPECTED_TESTS)
            or before != parent['installed'] or len(before) != 4906
            or parent['driver'] != digest(Path(parent_native.__file__))):
        raise ValueError('the complete unchanged parent native preparation is mandatory')
    parent_path.rename(output / 'parent-input.json')
    changed_here = {str(base_native.TESTS)}
    if actual == 'swap-receipt-bindings':
        path = source / capture.CONTINUATION_SOURCE
        path.write_text(capture.mutate_constructor(path.read_text(), actual))
        subprocess.run(['rustfmt', '--edition', '2024', str(path)], cwd=source, check=True)
        changed_here.add(str(capture.CONTINUATION_SOURCE))
    tests = source / base_native.TESTS
    tests.write_bytes(tests.read_bytes() + b'\n' + fixture)
    subprocess.run(['rustfmt', '--edition', '2024', str(tests)], cwd=source, check=True)
    installed = base_native.inventory(source)
    added = {name for name in before if before[name] != installed[name]}
    if (before.keys() != installed.keys() or added != changed_here
            or any(before[n]['executable'] != installed[n]['executable'] for n in before)
            or installed[str(capture.retire.LIB)] != parent['original'][str(capture.retire.LIB)]):
        raise ValueError('only the exact constructor mutation and shared extra fixture may extend parent preparation')
    if any(len(re.findall(r'#\[test\]\s+fn\s+' + re.escape(n) + r'\(', tests.read_text())) != 1
            for n in EXPECTED_TESTS):
        raise ValueError('all61 actual test functions must be uniquely present')
    if rules() != rule_inputs:
        raise ValueError('native rules or complete fixture inputs changed during preparation')
    changed = sorted(name for name in parent['original'] if parent['original'][name] != installed[name])
    result = dict(prepared=True, compiled=False, tests_executed=False,
        source=capture.retire.SOURCE, tree=capture.retire.TREE, archive=capture.retire.ARCHIVE,
        variant=variant, parent_variant=legacy, parent_input=digest(output / 'parent-input.json'),
        driver=digest(Path(__file__)), capture_driver=digest(HERE / 'capture.py'),
        parent_driver=digest(Path(parent_native.__file__)), rules=rule_inputs,
        own_fixture=digest(HERE / 'native.rs.in'), tests=list(EXPECTED_TESTS),
        changed=changed, extended_parent_files=sorted(added),
        original=parent['original'], installed=installed,
        expected=dict(passed=61 if actual == 'original' else 60, failed=0 if actual == 'original' else 1),
        rejection=REJECTION.get(actual), restore='independent original source/target; all61 after each wrong')
    (output / 'input.json').write_text(json.dumps(result, indent=2) + '\n')
    print(variant, 'all61 original assertions prepared; compilation/execution/restore pending')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    cli.add_argument('--output', type=Path, required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(), args.variant)
