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
UNLOAD = 'controller-service/src/controller/lifecycle_inbox/unload_resolution.rs'
HERE = Path('verification/experiments/stage1-protocol/inbox-entry-validation')
MUTATIONS = {
    'duplicate-cap': ('count += 1;', 'count = 1;'),
    'key-sequence-only': ('entry.intent.request.binding.key == *key',
                          'entry.intent.request.binding.key.sequence == key.sequence'),
    'requested-deadline-only': ('''(if previous > *end_at_ms {
                                    previous
                                } else {
                                    *end_at_ms
                                })''', '''{
                                    let _ = previous;
                                    *end_at_ms
                                }'''),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')



def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown actual source variant: ' + variant)
    before = views.inputs(project, HERE)
    mutation = None
    if variant in MUTATIONS:
        scope = ('fn count_entry_key_from(' if variant in ('duplicate-cap', 'key-sequence-only')
                 else 'fn entry_kind_using<')
        mutation = views.edit(project, MODEL, [MUTATIONS[variant]], scope)
    views.append(project, MODEL, (project/HERE/'roots.rs.in').read_text())
    if variant not in ('original', 'restored'):
        views.append(project, MODEL, (project/HERE/'generic-root.rs.in').read_text())
    return views.finish(project, before, mutation)
