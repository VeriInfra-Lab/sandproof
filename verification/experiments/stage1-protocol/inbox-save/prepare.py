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

HERE = Path('verification/experiments/stage1-protocol/inbox-save')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
SHIM = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'

mutation_module = module('actual_inbox_save_mutations', Path(__file__).with_name('mutations.py'))
FAILURES = mutation_module.FAILURES
VARIANTS = ('original', 'candidate', *FAILURES, 'restored')
parent = module('save_actual_validation', Path(__file__).parents[1]/'inbox-validation/prepare.py')


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown actual source variant: ' + variant)
    before = views.inputs(project, HERE)
    mutation = None
    if variant not in ('original', 'restored'):
        parent.prepare(project, 'candidate')
        if variant in FAILURES:
            mutation = mutation_module.mutate(project, variant)
        views.append(project, SOURCE, (project/HERE/'generic-root.rs.in').read_text())
    return views.finish(project, before, mutation)
