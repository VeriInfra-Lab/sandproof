"""Prepare the same seven SQLite assertions in four views; never run tests."""
import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile

import extract

VARIANTS = ('original', 'adapted-original', 'allow-zero-update', 'adapted-allow-zero-update')
TESTS = Path('controller-service/src/controller/lifecycle_inbox/source_work/tests/end_pipeline.rs')
EXPECTED_TESTS = (
    'source_retire_requires_transaction_even_for_empty_expected',
    'source_retire_empty_transaction_does_not_load_missing_schema',
    'source_retire_clears_only_complete_original_slot_and_keeps_reservation',
    'source_retire_same_key_with_changed_complete_record_is_rejected',
    'source_retire_unsettled_material_keeps_complete_source_row',
    'source_retire_zero_row_compare_and_set_is_rejected',
    'source_retire_duplicate_preserves_first_update_until_explicit_rollback',
)


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def inventory(source):
    result = {}
    for path in sorted(source.rglob('*')):
        if path.is_symlink():
            raise ValueError('source inventory cannot contain symlinks')
        if path.is_file():
            result[str(path.relative_to(source))] = dict(
                sha256=digest(path), executable=bool(path.stat().st_mode & 0o111))
    return result


def prepare(output, variant):
    os.umask(0o077)
    if variant not in VARIANTS:
        raise ValueError('unknown native retirement view')
    fixture = (extract.HERE / 'native.rs.in').read_bytes()
    names = tuple(re.findall(rb'#\[test\]\s+fn\s+(\w+)\(', fixture))
    if names != tuple(name.encode() for name in EXPECTED_TESTS):
        raise ValueError('all seven identical retirement assertions are required')
    if b'retired source clearing must reject a zero-row compare-and-set' not in fixture:
        raise ValueError('the specific zero-row rejection assertion is required')
    output.mkdir(mode=0o700)
    archive = output / 'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git', 'archive', extract.SOURCE], cwd=extract.REPO, stdout=stream, check=True)
    if digest(archive) != extract.ARCHIVE:
        raise ValueError('the exact production source archive is required')
    source = output / 'source'
    source.mkdir(mode=0o700)
    with tarfile.open(archive) as inputs:
        inputs.extractall(source, filter='data')
    original = inventory(source)
    actual = variant.removeprefix('adapted-')
    if variant.startswith('adapted-'):
        extract.reconstruct(archive, source, actual)
        with tarfile.open(archive) as inputs:
            (source / extract.LIB).write_bytes(inputs.extractfile(str(extract.LIB)).read())
        targets = extract.STAGED[:-1]
    else:
        path = source / extract.ports.CODEC
        path.write_text(extract.mutate(path.read_text(), actual))
        targets = (extract.ports.CODEC,)
    pre_format = {str(path): digest(source / path) for path in targets}
    for path in targets:
        subprocess.run(['rustfmt', '--edition', '2024', str(source / path)], cwd=source, check=True)
    formatted = {str(path): digest(source / path) for path in targets}
    tests = source / TESTS
    tests.write_bytes(tests.read_bytes() + b'\n' + fixture)
    subprocess.run(['rustfmt', '--edition', '2024', str(tests)], cwd=source, check=True)
    installed = inventory(source)
    if original.keys() != installed.keys():
        raise ValueError('native preparation changed the full production file inventory')
    changed = {name for name in original if original[name] != installed[name]}
    allowed = {str(path) for path in (*targets, TESTS)}
    required = {str(TESTS)}
    if variant != 'original':
        required.update(str(path) for path in targets)
    if not required <= changed <= allowed:
        raise ValueError('only exact staged library calls and the identical fixture may change')
    if installed[str(extract.LIB)] != original[str(extract.LIB)]:
        raise ValueError('native views must retain the unmodified original library root')
    report = dict(prepared=True, compiled=False, tests_executed=False,
        source=extract.SOURCE, tree=extract.TREE, archive=extract.ARCHIVE, variant=variant,
        fixture=sha256(fixture).hexdigest(), tests=list(EXPECTED_TESTS),
        pre_format=pre_format, formatted=formatted, changed=sorted(changed),
        original=original, installed=installed)
    (output / 'input.json').write_text(json.dumps(report, indent=2) + '\n')
    print(variant, 'seven complete retirement assertions prepared; compilation and execution pending')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    cli.add_argument('--output', type=Path, required=True)
    args = cli.parse_args()
    prepare(args.output.resolve(), args.variant)
