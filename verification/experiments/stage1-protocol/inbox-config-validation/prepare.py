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

HERE = Path('verification/experiments/stage1-protocol/inbox-config-validation')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
CAPTURE = 'controller-service/src/capture.rs'
MUTATIONS = {
    'host-version-range': (CAPTURE, 'self.version == 1\n            && !self.boot_id.is_empty()',
                           'self.version <= 2\n            && !self.boot_id.is_empty()'),
    'omit-profile-bound': (MODEL, '            && (1..=65_536).contains(&self.max_profile_bytes)\n', ''),
    'double-budget-limit': (MODEL, '&& self.required_bytes() <= 16 * 1024 * 1024',
                          '&& self.required_bytes() <= 32 * 1024 * 1024'),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')



def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown actual source variant: ' + variant)
    before = views.inputs(project, HERE)
    mutation = None
    if variant in MUTATIONS:
        name, old, new = MUTATIONS[variant]
        mutation = views.edit(project, name, [(old, new)])
    views.append(project, MODEL, (project/HERE/'roots.rs.in').read_text())
    return views.finish(project, before, mutation)
