"""Validate capture provenance and apply one complete source field contract."""
import argparse
import json
from pathlib import Path
from tempfile import TemporaryDirectory

import capture
import review
from prepare_world_begin import read_exact


def admit(directory, output):
    result = json.loads((directory / 'result.json').read_bytes())
    variant = result['variant']
    if (variant not in capture.VARIANTS or result['source'] != capture.metadata.SOURCE
            or result['tree'] != capture.metadata.TREE or result['options'] != capture.OPTIONS
            or result['driver'] != capture.metadata.sql.digest(Path(capture.__file__))
            or result['source_capture_complete'] is not True
            or result['library_refinement_proved'] is not False):
        raise ValueError('the complete current Configuration source capture is required')
    fixed = json.loads((capture.ROOT / 'verification/foundations-environment.json').read_bytes())
    if result['tools'] != {name: fixed[name]['binary_sha256'] for name in ('charon', 'aeneas')}:
        raise ValueError('the exact fixed tools are required')
    statuses = [(c['label'], c['exit_code'], c['bounded_stop']) for c in result['commands']]
    if (len(statuses) != 2 or statuses[0] != ('charon', 0, False)
            or statuses[1][0] != 'aeneas' or statuses[1][2] is not False
            or not isinstance(statuses[1][1], int) or statuses[1][1] < 0):
        raise ValueError('the successful capture and completed translation attempt are required')
    for command in result['commands']:
        read_exact(directory / (command['label'] + '.log'), command['log_sha256'])
    generated = directory / 'generated'
    actual_files = {str(path.relative_to(generated)) for path in generated.rglob('*') if path.is_file()}
    if actual_files != set(result['generated']):
        raise ValueError('the complete attempted translation inventory differs')
    for name, expected in result['generated'].items():
        read_exact(generated / name, expected)
    archive = directory / 'source.tar'
    read_exact(archive, capture.metadata.ARCHIVE)
    with TemporaryDirectory(prefix='source-metadata-json-') as fresh:
        source = Path(fresh)
        codec = capture.metadata.reconstruct_codec(source, archive, 'original')
        if read_exact(directory / 'source' / capture.CODEC, result['staged'][str(capture.CODEC)]) != codec:
            raise ValueError('the exact parent Configuration wrappers must reproduce')
        limits = (source / capture.CONFIG).read_text()
        if variant == 'swapped-limit-names':
            limits = capture.swap_names(limits)
        if read_exact(directory / 'source' / capture.CONFIG, result['staged'][str(capture.CONFIG)]) != limits.encode():
            raise ValueError('the original six fields or exact name-swap mutation differs')
    raw = json.loads(read_exact(directory / 'actual.llbc', result['ir']))
    translated = raw['translated']
    if (raw['has_errors'] is not False
            or {row['key'] for row in translated['target_information']} != {fixed['target']}
            or any(translated['options'][flag] != capture.OPTIONS[group] for flag, group in
                   (('start_from', 'roots'), ('include', 'include'), ('opaque', 'opaque')))
            or any(translated['options'][flag] for flag in
                   ('skip_borrowck', 'no_typecheck', 'no_normalize', 'no_reorder_decls', 'exclude'))):
        raise ValueError('the actual complete LLBC selectors or target differs')
    rejection = None
    try:
        fields = review.check(raw)
    except ValueError as error:
        if variant != 'swapped-limit-names' or str(error) != (
                'original field/container reference changed: SourceObservationConfig.max_disk_bytes'):
            raise
        rejection = str(error)
        fields = None
    if (fields is None) != (variant == 'swapped-limit-names'):
        raise ValueError('the unchanged field contract must reject the actual swapped names')
    rules = [Path(__file__), Path(capture.__file__), Path(review.__file__),
             review.PROTOCOL / 'serde-binding/binding.py',
             review.PROTOCOL / 'serde-binding/check.py',
             review.PROTOCOL / 'parser-binding/binding.py']
    report = dict(source=result['source'], variant=variant, source_admission_complete=True,
                  original_contract_matches=rejection is None, rejection=rejection, fields=fields,
                  capture_sha256=capture.metadata.sql.digest(directory / 'result.json'),
                  ir_sha256=result['ir'], checker_sha256=capture.metadata.sql.digest(Path(review.__file__)),
                  driver_sha256=capture.metadata.sql.digest(Path(__file__)),
                  rules_sha256={str(path.relative_to(capture.ROOT)): capture.metadata.sql.digest(path)
                                for path in rules},
                  aeneas_exit_code=statuses[1][1], new_lean_theorems=0, library_refinement_proved=False,
                  ning_acceptance=False, shared_acceptance=False)
    with output.open('x') as stream:
        stream.write(json.dumps(report, indent=2) + '\n')
    print(variant, 'source contract matches', report['original_contract_matches'])


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--capture', required=True, type=Path)
    cli.add_argument('--output', required=True, type=Path)
    args = cli.parse_args()
    admit(args.capture.resolve(strict=True), args.output.resolve())
