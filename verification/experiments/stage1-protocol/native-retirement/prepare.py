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

HERE = Path("verification/experiments/stage1-protocol/native-retirement")
SOURCE = 'controller-service/src/controller/lifecycle_inbox/native_retirement.rs'
MUTATIONS = {
    'local-presence-only': (
        'session.local != Some(saved.local)',
        '{\n            let _ = session.local != Some(saved.local);\n            session.local.is_none()\n        }'),
    'omit-intent-binding': (
        'saved.request_binding != E::digest(&E::encode_intent(&last.intent)?)',
        '{\n            let _ = saved.request_binding != E::digest(&E::encode_intent(&last.intent)?);\n            false\n        }'),
    'omit-worker-status': (
        '        || qc.worker_drained != QuiescenceStatus::Confirmed\n', ''),
    'omit-quiescence-epoch': ('        || qc.epoch != request.epoch\n', ''),
    'oversize-encoding': ('E::encode_receipt(saved)?.len() > 8192',
                          'E::encode_receipt(saved)?.len() > 8193'),
}
FAILURES = {
    'local-presence-only': ['terminal_local_version_operation_guards',
                           'absent_and_prefix_refusals_skip_callbacks'],
    'omit-intent-binding': ['full_intent_digest_binding_is_observed',
                            'binding_and_fields_skip_receipt_encoding'],
    'omit-worker-status': ['all_four_status_tags_are_observed',
                           'binding_and_fields_skip_receipt_encoding'],
    'omit-quiescence-epoch': ['receipt_identity_fields_are_observed',
                              'binding_and_fields_skip_receipt_encoding'],
    'early-intent-encoding': ['absent_and_prefix_refusals_skip_callbacks'],
    'oversize-encoding': ['receipt_size_accepts_boundary_and_rejects_successor'],
}
VARIANTS = ('original', 'candidate', *FAILURES, 'restored')



def mutate(source, variant):
    if variant == 'candidate':
        return source, []
    if variant == 'early-intent-encoding':
        edits = [
            ('    let request = saved.retirement;',
             '    let encoded_intent = E::encode_intent(&last.intent)?;\n    let request = saved.retirement;'),
            ('E::digest(&E::encode_intent(&last.intent)?)', 'E::digest(&encoded_intent)'),
        ]
    else:
        edits = [MUTATIONS[variant]]
    start = source.index('fn validate_saved_using<')
    end = source.index('pub(super) fn cleaned(', start)
    body = source[start:end]
    for old, new in edits:
        assert body.count(old) == 1, (variant, old)
        body = body.replace(old, new, 1)
    return source[:start]+body+source[end:], edits

def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown actual source variant: ' + variant)
    before = views.inputs(project, HERE)
    mutation = None
    if variant not in ('original', 'restored'):
        source = project/SOURCE
        actual, edits = mutate(source.read_text(), variant)
        if edits:
            mutation = views.edit(project, SOURCE, edits, 'fn validate_saved_using<')
            if source.read_text() != actual:
                raise ValueError('saved-retirement mutation does not match actual function')
        views.append(project, SOURCE, (project/HERE/'generic-root.rs.in').read_text())
    return views.finish(project, before, mutation)
