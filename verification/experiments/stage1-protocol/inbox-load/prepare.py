"""Prepare the fixed load candidate and actual Rust mutations in private copies."""
from hashlib import sha256
from pathlib import Path
import difflib
import importlib.util
import json
import subprocess

HERE = Path('verification/experiments/stage1-protocol/inbox-load')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
SHIM = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


mutations = module('load_actual_mutations', Path(__file__).with_name('mutations.py'))
FAILURES = mutations.FAILURES
VARIANTS = ('original', 'candidate', *FAILURES, 'restored')


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown load variant')
    inventory = json.loads((project/HERE/'inventory.json').read_text())
    before = {name: sha(project/name) for name in [*inventory['production_inputs'], SHIM]}
    for name, digest in inventory['production_inputs'].items():
        if before[name] != digest:
            raise ValueError('fixed production input differs: '+name)
    metadata = dict(mutation=None, production_modified=False,
        private_refactor=False, original_equivalence_proved=False,
        standard_load_provider_adopted=False, same_transaction_read_proved=False)
    if variant not in ('original', 'restored'):
        _, metadata = mutations.prepare_candidate(project, variant)
        source = project/SOURCE
        if 'fn proof_inbox_load_using<' in source.read_text():
            raise ValueError('preexisting load forwarding root')
        with source.open('a') as stream:
            stream.write((project/HERE/'generic-root.rs.in').read_text())
    return {name: sha(project/name) for name, digest in before.items()
            if sha(project/name) != digest}, metadata


def candidate_patch(original, prepared):
    """Review business changes relative to the fixed source; exclude proof roots."""
    inventory = json.loads((original/HERE/'inventory.json').read_text())
    fragments = []
    for package, name in [('inbox-validation', 'generic-root.rs.in'),
                          ('inbox-validation', 'primitive-roots.rs.in'),
                          ('inbox-load', 'generic-root.rs.in')]:
        result = subprocess.run(['rustfmt', '--edition', '2024', '--emit', 'stdout'],
            input=(original/HERE.parent/package/name).read_text(), text=True,
            capture_output=True, check=True)
        fragments.append(result.stdout.strip())
    diffs = []
    for name in inventory['production_inputs']:
        before, after = (original/name).read_text(), (prepared/name).read_text()
        for fragment in fragments:
            if fragment in after:
                if after.count(fragment) != 1:
                    raise ValueError('nonunique proof forwarder in review patch')
                after = after.replace(fragment, '').rstrip()+'\n'
        if any(symbol in after for symbol in ('fn proof_inbox_validate_using<',
                'fn proof_inbox_load_using<', 'fn proof_standard_graphic_bytes(',
                'fn proof_result_is_error<')):
            raise ValueError('proof forwarder leaked into production review patch')
        if before != after:
            diffs.append(''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                fromfile='a/'+name, tofile='b/'+name, n=3)))
    return ''.join(diffs).replace('\n \n', '\n\n')
