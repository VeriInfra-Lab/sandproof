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

HERE = Path('verification/experiments/stage1-protocol/terminal-binding')
TERMINAL = 'sandbox-protocol/src/lifecycle/terminal.rs'
SHIM = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'
MUTATIONS = {
    'key-only-observation': ('        self.request == *request\n',
        '        self.request.request.binding.key == request.request.binding.key\n'),
    'allow-unload': ('            && matches!(request.request.operation, LifecycleOperation::Destroy {})\n',
        '            && matches!(\n                request.request.operation,\n                LifecycleOperation::Destroy {} | LifecycleOperation::Unload { .. }\n            )\n'),
    'omit-retention-shape': ('            && request.retention.valid_for(4096)\n', ''),
    'omit-version-relation': ('            && request.accepted_version_for_request() == Some(request.accepted_version)\n', ''),
}
VARIANTS = ('original', *MUTATIONS, 'restored')



def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown actual source variant: ' + variant)
    before = views.inputs(project, HERE)
    mutation = None
    if variant in MUTATIONS:
        mutation = views.edit(project, TERMINAL, [MUTATIONS[variant]])
    views.append(project, SHIM, (project/HERE/'roots.rs.in').read_text())
    return views.finish(project, before, mutation)
