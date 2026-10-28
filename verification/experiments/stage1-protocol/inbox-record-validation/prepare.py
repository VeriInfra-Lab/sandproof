"""Prepare thin roots and real mutations of the adopted production implementation."""
from hashlib import sha256
from pathlib import Path
import importlib.util
import json


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


views = module('adopted_source_views', Path(__file__).parents[1] / 'source_views.py')
sha = views.sha

HERE = Path('verification/experiments/stage1-protocol/inbox-record-validation')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
SHIM = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'

parent = module('record_validation_actual_save', Path(__file__).parents[1]/'inbox-save/prepare.py')
FAILURES = {variant:set(tests) for variant,tests in parent.FAILURES.items()}
FAILURES['wrong-validation'].add('capture_frontier_checks_the_whole_value_and_actual_save')
VARIANTS = parent.VARIANTS


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown actual source variant: ' + variant)
    before = views.inputs(project, HERE)
    mutation = None
    _, inherited = parent.prepare(project, variant)
    mutation = inherited['mutation']
    if variant not in ('original', 'restored'):
        views.append(project, 'controller-service/src/controller/lifecycle_inbox/model.rs',
            (project/HERE.parent/'inbox-record/roots.rs.in').read_text())
    if variant == 'candidate':
        for target, package, fragment in (
            (SOURCE, 'inbox-load', 'generic-root.rs.in'),
            ('controller-service/src/controller/lifecycle_inbox/model.rs', 'inbox-admission', 'roots.rs.in'),
            ('controller-service/src/controller/lifecycle_inbox/model.rs', 'inbox-state', 'roots.rs.in'),
        ):
            views.append(project, target, (project/HERE.parent/package/fragment).read_text())
    changes, metadata = views.finish(project, before, mutation)
    metadata.update(record_validation_preservation_proved=False, transaction_correctness_proved=False)
    return changes, metadata
