"""Prepare the original or actual save candidate in a private source copy."""
from hashlib import sha256
from pathlib import Path
import difflib
import importlib.util
import json
import subprocess

HERE = Path('verification/experiments/stage1-protocol/inbox-save')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
SHIM = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


mutation_module = module('inbox_save_mutations', Path(__file__).with_name('mutations.py'))
FAILURES = mutation_module.FAILURES
VARIANTS = ('original', 'candidate', *FAILURES, 'restored')


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown save variant')
    inventory = json.loads((project/HERE/'inventory.json').read_text())
    before = {n:sha(project/n) for n in [*inventory['production_inputs'], SHIM]}
    for name, digest in inventory['production_inputs'].items():
        if before[name] != digest:
            raise ValueError('fixed production input differs: '+name)
    metadata = dict(mutation=None, production_modified=False,
        private_refactor=variant not in ('original', 'restored'),
        original_equivalence_proved=False, standard_encoding_and_digest_adopted=False,
        transaction_correctness_proved=False)
    if variant not in ('original', 'restored'):
        candidate = module('inbox_save_candidate', project/HERE/'candidate.py')
        _, inherited = candidate.candidate(project)
        metadata['candidate'] = inherited
        if variant in FAILURES:
            metadata['mutation'] = mutation_module.mutate(project, variant)
        with (project/SOURCE).open('a') as stream:
            stream.write('\n'+(project/HERE/'generic-root.rs.in').read_text())
        changed = [n for n,h in before.items() if sha(project/n) != h]
        subprocess.run(['rustfmt', '--edition', '2024', *changed], cwd=project, check=True)
    changes = {n:sha(project/n) for n,h in before.items() if sha(project/n) != h}
    return changes, metadata


def candidate_patch(original, prepared):
    """Review actual business source, excluding every proof-only forwarder."""
    inventory = json.loads((original/HERE/'inventory.json').read_text())
    fragments = []
    for package, name in [('inbox-validation','generic-root.rs.in'),
                          ('inbox-validation','primitive-roots.rs.in'),
                          ('inbox-save','generic-root.rs.in')]:
        result = subprocess.run(['rustfmt','--edition','2024','--emit','stdout'],
            input=(original/HERE.parent/package/name).read_text(), text=True,
            capture_output=True, check=True)
        fragments.append(result.stdout.strip())
    diffs = []
    for name in inventory['production_inputs']:
        before, after = (original/name).read_text(), (prepared/name).read_text()
        for fragment in fragments:
            if fragment in after:
                if after.count(fragment) != 1:
                    raise ValueError('nonunique proof forwarder in production review patch')
                after = after.replace(fragment, '').rstrip()+'\n'
        if any(symbol in after for symbol in ('fn proof_inbox_validate_using<',
                'fn proof_inbox_save_using<', 'fn proof_standard_graphic_bytes(',
                'fn proof_result_is_error<')):
            raise ValueError('proof forwarder leaked into production review patch')
        if before != after:
            diffs.append(''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                fromfile='a/'+name, tofile='b/'+name, n=3)))
    return ''.join(diffs).replace('\n \n', '\n\n')
