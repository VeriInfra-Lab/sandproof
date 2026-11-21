"""Recapture the same complete retirement environment with one settled-byte mutation.

The theorem selects material_charge from the original transparent dependency
closure. The capture root remains the original per-record clear; no business
helper, field, library call or error is replaced by an opaque assumption.
"""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-remove-retired'
sys.path.insert(0, str(PARENT))
_spec = importlib.util.spec_from_file_location('source_retire_capture_parent', PARENT / 'extract.py')
retire = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(retire)
VARIANTS = ('original', 'ignore-settlement-bytes')
SELECTED = retire.ports.WORK + '::retirement::material_charge'
EXPECTED = '\n                || settlement.released_material_bytes != capture.reserved_material_bytes'


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown complete material-charge view')
    start = text.index('pub(in crate::controller) fn material_charge(')
    stop = text.index('\npub(super) fn has_closed_material_use(', start)
    body = text[start:stop]
    if body.count(EXPECTED) != 1:
        raise ValueError('one original settled-byte check is required')
    if variant == 'original':
        return text
    return text[:start] + body.replace(EXPECTED, '', 1) + text[stop:]


def capture(output, tools, lock, variant):
    original_stage = retire.reconstruct

    def stage(archive, source, selected_variant):
        original_stage(archive, source, 'original')
        path = source / retire.ports.RETIREMENT
        path.write_text(mutate(path.read_text(), selected_variant))

    retire.reconstruct = stage
    try:
        retire.capture(output, tools, lock, variant)
    finally:
        retire.reconstruct = original_stage
        result = output / 'result.json'
        if result.is_file():
            report = json.loads(result.read_bytes())
            report['base_driver'] = report['driver']
            report['driver'] = digest(Path(__file__))
            report['capture_root'] = retire.ROOT
            report['selected_business'] = SELECTED
            report['mutation'] = None if variant == 'original' else 'omit settled released-material byte equality only'
            result.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
