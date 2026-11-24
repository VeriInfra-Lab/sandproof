"""Compare independently captured #931 artifacts on the two reviewed source trees."""
import argparse
import json
from pathlib import Path

from probe_prepare_iterators import BASELINES, digest, prepare


CURRENT = '6fd0852025468e2583c436d37af18383bf2e71ea'
CANDIDATE = '4b433e2be3337db97e9fb7e8d9b868d159c3e2cb8c894a2a0ec78ea4867489fd'


def load_capture(path, commit, wrapper):
    r = json.loads((path / 'probe-result.json').read_bytes())
    tree, archive = BASELINES[commit]
    if ((r['source_commit'], r['source_tree'], r['variant'], r['wrapper_ir']) !=
            (commit, tree, 'json-boundary', wrapper)
            or r['candidate_source_sha256'] != CANDIDATE
            or r['archive_sha256'] != archive or digest(path / 'source.tar') != archive
            or digest(path / 'source' / prepare.SOURCE) != CANDIDATE
            or 'error' in r):
        raise ValueError('capture is not the reviewed candidate on its exact source tree')
    labels = ['charon'] if wrapper else ['charon', 'aeneas']
    if ([step['label'] for step in r['commands']] != labels
            or any(type(step['exit_code']) is not int or step['exit_code'] != 0
                   or step['bounded_stop'] is not False
                   or digest(path / (step['label'] + '.log')) != step['log_sha256']
                   for step in r['commands'])):
        raise ValueError('original tool results are missing or incomplete')
    ir = path / 'actual.llbc'
    if digest(ir) != r['ir_sha256']:
        raise ValueError('captured IR changed')
    if wrapper:
        if r['generated'] is not False or type(r.get('serde_to_vec_id')) is not int:
            raise ValueError('the independent raw encoder admission is missing')
    else:
        root = path / 'generated'
        generated = {str(p.relative_to(root)): digest(p) for p in root.rglob('*') if p.is_file()}
        if (r['generated'] is not True or not generated
                or r['generated_sha256'] != generated or r['lean_name'] != prepare.LEAN_ROOT):
            raise ValueError('the admitted original outer Lean files are required')
    return r, ir


def canonical_json(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False)


def comparison_ir(ir):
    value = json.loads(ir.read_bytes())
    translated = value['translated']
    options = translated['options']
    if options['dest_file'] != str(ir.resolve()):
        raise ValueError('IR destination does not point to the original local capture')
    options['dest_file'] = ir.name
    names = translated['short_names']
    keys = {canonical_json(row['key']) for row in names}
    if len(keys) != len(names):
        raise ValueError('short-name table is not a map')
    translated['short_names'] = sorted(names, key=lambda row: canonical_json(row['key']))
    return canonical_json(value)


def compare(old, current, source_commit, wrapper):
    before, old_ir = load_capture(old, prepare.SOURCE_COMMIT, wrapper)
    after, current_ir = load_capture(current, source_commit, wrapper)
    if comparison_ir(old_ir) != comparison_ir(current_ir):
        raise ValueError('fresh capture differs beyond destination or short-name ordering')
    keys = ('encoder_id', 'serde_to_vec_id') if wrapper else ('encoder_id', 'candidate_calls', 'generated_sha256')
    if any(canonical_json(before[key]) != canonical_json(after[key]) for key in keys):
        raise ValueError('admitted generated files or exact calls differ')
    return {
        'original_ir': before['ir_sha256'], 'current_ir': after['ir_sha256'],
        **{key: after[key] for key in keys},
    }


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-commit', choices=[commit for commit in BASELINES if commit != prepare.SOURCE_COMMIT],
                        default=CURRENT)
    for name in ('old-parent', 'current-parent', 'old-wrapper', 'current-wrapper'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps({
        'source': args.source_commit, 'historical_source': prepare.SOURCE_COMMIT, 'candidate_source_sha256': CANDIDATE,
        'parent': compare(args.old_parent, args.current_parent, args.source_commit, wrapper=False),
        'wrapper': compare(args.old_wrapper, args.current_wrapper, args.source_commit, wrapper=True),
        'only_ir_differences_allowed': ['options.dest_file', 'short_names ordering'],
        'public_acceptance': False,
    }, indent=2))
