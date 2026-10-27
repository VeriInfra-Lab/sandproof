"""Join the reviewed candidates in a private copy; never mutate production checkout."""
from pathlib import Path
import difflib
import importlib.util
import json
import shutil
import subprocess

HERE = Path('verification/experiments/stage1-protocol/inbox-record-validation')
PARENT = HERE.parent/'inbox-save'
SOURCE = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
SHIM = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


parent = module('combined_save_prepare', Path(__file__).parents[1]/'inbox-save/prepare.py')
combine = module('combined_record_prepare', Path(__file__).with_name('combine.py'))
sha = parent.sha
FAILURES = {variant:set(tests) for variant,tests in parent.FAILURES.items()}
FAILURES['wrong-validation'].add('capture_frontier_checks_the_whole_value_and_actual_save')
VARIANTS = parent.VARIANTS


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown combined variant')
    inventory = json.loads((project/HERE/'inventory.json').read_text())
    before = {n:sha(project/n) for n in [*inventory['production_inputs'],SHIM]}
    for n,digest in inventory['production_inputs'].items():
        if before[n] != digest:
            raise ValueError('fixed production input differs: '+n)
    # Preserve the pristine merge base and failed merge output as review evidence.
    review = project/'artifacts/record-validation-candidate'
    review.mkdir(parents=True)
    original = review/'original'
    patch = HERE.parent/'inbox-record/production-candidate.patch'
    for name in [*combine.FILES,str(patch)]:
        target=original/name
        target.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(project/name,target)
    _, metadata = parent.prepare(project,variant)
    if variant not in ('original','restored'):
        metadata['record_merge'] = combine.apply(original,project,review)
        model=project/combine.FILES[0]
        if 'fn proof_record(' in model.read_text():
            raise ValueError('preexisting record forwarder')
        with model.open('a') as stream:
            stream.write('\n'+(project/HERE.parent/'inbox-record/roots.rs.in').read_text())
        subprocess.run(['rustfmt','--edition','2024',str(model)],check=True)
    metadata['record_validation_preservation_proved'] = False
    metadata['transaction_correctness_proved'] = False
    return {n:sha(project/n) for n,d in before.items() if sha(project/n)!=d},metadata


def candidate_patch(original, prepared):
    """Review actual business source, excluding every proof-only forwarder."""
    inventory = json.loads((original/HERE/'inventory.json').read_text())
    fragments = []
    for package, name in [('inbox-validation','generic-root.rs.in'),
                          ('inbox-validation','primitive-roots.rs.in'),
                          ('inbox-save','generic-root.rs.in'),
                          ('inbox-record','roots.rs.in')]:
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
                'fn proof_result_is_error<', 'fn proof_record(')):
            raise ValueError('proof forwarder leaked into production review patch')
        if before != after:
            diffs.append(''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                fromfile='a/'+name, tofile='b/'+name, n=3)))
    return ''.join(diffs).replace('\n \n', '\n\n')
