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

HERE = Path('verification/experiments/stage1-protocol/inbox-origin-selection')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
MUTATIONS = {
    'skip-resume': (
        'LifecycleOperation::Create { .. } | LifecycleOperation::Resume { .. }',
        'LifecycleOperation::Create { .. }'),
    'require-local-operation': (
        '            ) {\n                return Some(entry);',
        '            ) && entry.local_operation.is_some()\n            {\n                return Some(entry);'),
    'return-first-slot': ('return Some(entry);', 'return Some(&self.entries[0]);'),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')



def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown actual source variant: ' + variant)
    before = views.inputs(project, HERE)
    mutation = None
    if variant in MUTATIONS:
        mutation = views.edit(project, MODEL, [MUTATIONS[variant]], 'fn current_execution_origin(')
    views.append(project, MODEL, (project/HERE/'roots.rs.in').read_text())
    return views.finish(project, before, mutation)
