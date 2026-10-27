"""Prepare the actual candidate or one exact Rust composition mutation in isolation."""
from hashlib import sha256
from pathlib import Path
import difflib
import importlib.util
import json
import subprocess

HERE = Path('verification/experiments/stage1-protocol/inbox-validation')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/model.rs'
SHIM = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


mutation_module = module('inbox_composition_mutations', Path(__file__).with_name('mutations.py'))
FAILURES = mutation_module.FAILURES
VARIANTS = ('original', 'candidate', *FAILURES, 'restored')


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown Inbox composition variant')
    inventory = json.loads((project/HERE/'inventory.json').read_text())
    for name, digest in inventory['production_inputs'].items():
        if sha(project/name) != digest:
            raise ValueError('fixed production input differs: '+name)
    before = {n:sha(project/n) for n in [*inventory['production_inputs'], SHIM]}
    metadata = dict(mutation=None, production_modified=False,
        private_refactor=variant not in ('original', 'restored'),
        original_equivalence_proved=False, standard_encoding_and_digest_adopted=False)
    if variant not in ('original', 'restored'):
        candidate = module('inbox_composition_candidate', project/HERE/'candidate.py')
        metadata['candidate'] = candidate.candidate(project)
        changed = [n for n,h in before.items() if sha(project/n) != h]
        subprocess.run(['rustfmt', '--edition', '2024', *changed], cwd=project, check=True)
        if variant in FAILURES:
            metadata['mutation'] = mutation_module.mutate(project, variant)
        with (project/SOURCE).open('a') as stream:
            stream.write((project/HERE/'generic-root.rs.in').read_text())
        with (project/'controller-service/src/capture.rs').open('a') as stream:
            stream.write((project/HERE/'primitive-roots.rs.in').read_text())
        with (project/SHIM).open('a') as stream:
            stream.write((project/HERE.parent/'terminal-binding/roots.rs.in').read_text())
            stream.write(json.loads((project/HERE.parent/'inbox-renewal-session/fragments.json').read_text())['renewal_roots'])
        changed = [n for n,h in before.items() if sha(project/n) != h]
        subprocess.run(['rustfmt', '--edition', '2024', *changed], cwd=project, check=True)
    changes = {n:sha(project/n) for n,h in before.items() if sha(project/n) != h}
    return changes, metadata


def candidate_patch(original, prepared):
    """Production review only: omit proof forwarders and extraction shim."""
    inventory = json.loads((original/HERE/'inventory.json').read_text())
    # Format each proof-only fragment once before removing it from the review diff.
    fragments = [subprocess.run(['rustfmt', '--edition', '2024', '--emit', 'stdout'],
        input=(original/HERE/name).read_text(), text=True,
        capture_output=True, check=True).stdout.strip()
        for name in ('generic-root.rs.in', 'primitive-roots.rs.in')]
    diffs = []
    for name in inventory['production_inputs']:
        before, after = (original/name).read_text(), (prepared/name).read_text()
        for fragment in fragments:
            if fragment in after:
                if after.count(fragment) != 1:
                    raise ValueError('nonunique proof forwarder in production review patch')
                after = after.replace(fragment, '').rstrip()+'\n'
        if any(name in after for name in ('fn proof_inbox_validate_using<',
                'fn proof_standard_graphic_bytes(', 'fn proof_result_is_error<')):
            raise ValueError('proof forwarder leaked into production review patch')
        if before != after:
            diffs.append(''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                fromfile='a/'+name, tofile='b/'+name, n=3)))
    return ''.join(diffs).replace('\n \n', '\n\n')
