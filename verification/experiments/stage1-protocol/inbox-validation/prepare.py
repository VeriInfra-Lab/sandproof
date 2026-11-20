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

HERE = Path('verification/experiments/stage1-protocol/inbox-validation')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/model.rs'
SHIM = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'

mutation_module = module('actual_inbox_validation_mutations', Path(__file__).with_name('mutations.py'))
FAILURES = mutation_module.FAILURES
VARIANTS = ('original', 'candidate', *FAILURES, 'restored')


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown actual source variant: ' + variant)
    before = views.inputs(project, HERE)
    mutation = None
    if variant not in ('original', 'restored'):
        if variant in FAILURES:
            mutation = mutation_module.mutate(project, variant)
        views.append(project, SOURCE, (project/HERE/'generic-root.rs.in').read_text())
        views.append(project, 'controller-service/src/capture.rs', (project/HERE/'primitive-roots.rs.in').read_text())
        views.append(project, SHIM, (project/HERE.parent/'terminal-binding/roots.rs.in').read_text())
        renewal = json.loads((project/HERE.parent/'inbox-renewal-session/fragments.json').read_text())
        views.append(project, SHIM, renewal['renewal_roots'])
    return views.finish(project, before, mutation)
