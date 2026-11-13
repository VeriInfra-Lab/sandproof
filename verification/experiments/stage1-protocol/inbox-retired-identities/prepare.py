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

MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
HERE = Path('verification/experiments/stage1-protocol/inbox-retired-identities')
MUTATIONS = {
    'key-sequence-only': ('entry.intent.request.binding.key == *key',
        'entry.intent.request.binding.key.sequence == key.sequence'),
    'ignore-active-conflicts': (
        "self.prior_retired_identity_conflict(index, &binding.session, &binding.key)\n"
        "                || self.current_identity_conflict(&binding.session, &binding.key)\n            {",
        "self.prior_retired_identity_conflict(index, &binding.session, &binding.key) || {\n"
        "                let _ = self.current_identity_conflict(&binding.session, &binding.key);\n"
        "                false\n            } {"),
    'ignore-prior-session': (
        "last.request.request.binding.session == *id\n                    || last.request.request.binding.key == *key",
        "{\n                    let _ = last.request.request.binding.session == *id;\n"
        "                    false\n                } || last.request.request.binding.key == *key"),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')



def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown actual source variant: ' + variant)
    before = views.inputs(project, HERE)
    mutation = None
    if variant in MUTATIONS:
        scope = 'fn contains_entry_key(' if variant == 'key-sequence-only' else None
        mutation = views.edit(project, MODEL, [MUTATIONS[variant]], scope)
    views.append(project, MODEL, (project/HERE/'roots.rs.in').read_text())
    return views.finish(project, before, mutation)
