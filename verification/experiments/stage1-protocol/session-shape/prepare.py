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

HERE = Path('verification/experiments/stage1-protocol/session-shape')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/model.rs'
SHIM = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'
MUTATIONS = {'wrong-first-lane': ('        if lane.slot == session.retention.slot {\n            return lane.current_window == Some(session.retention.window);\n        }', '        if lane.slot == session.retention.slot\n            && lane.current_window == Some(session.retention.window)\n        {\n            return true;\n        }'), 'wrong-physical': ('            || session.local.is_some_and(|local| {\n                old.local\n                    .is_some_and(|prior| prior.sandbox == local.sandbox)\n            })', ''), 'wrong-refusal-field': ('            || entry.previous_effective_end_at_ms.is_some()\n', ''), 'wrong-last-session': ('.is_none_or(|entry| entry.intent != confirmed.request)', '.is_none_or(|entry| entry.intent.request.binding.session != confirmed.request.request.binding.session)')}
FAILURES = {'wrong-first-lane': ['first_matching_lane_controls_window'], 'wrong-physical': ['prior_physical_sandbox_conflicts_despite_distinct_owner_and_operation'], 'wrong-refusal-field': ['refusal_checks_initial_shape_and_every_native_field'], 'wrong-last-session': ['terminal_binds_complete_last_intent_and_cleaned_status']}
VARIANTS = ('original', 'candidate', *FAILURES, 'restored')



def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown actual source variant: ' + variant)
    before = views.inputs(project, HERE)
    mutation = None
    if variant in MUTATIONS:
        scope = {
            'wrong-first-lane': 'fn first_lane_window_matches(',
            'wrong-physical': 'fn prior_session_conflict(',
            'wrong-refusal-field': 'fn refusal_entries_invalid(',
            'wrong-last-session': 'fn validate_session_shape_using<',
        }[variant]
        mutation = views.edit(project, SOURCE, [MUTATIONS[variant]], scope)
    if variant not in ('original', 'restored'):
        views.append(project, SOURCE, (project/HERE/'generic-root.rs.in').read_text())
        views.append(project, SHIM, (project/HERE.parent/'terminal-binding/roots.rs.in').read_text())
    call = ('inbox.validate_session_shape_using::<'
            'super::native_retirement::StandardRetirementEncoding, StandardTerminalEncoding>(index, session)')
    views.append(project, SOURCE,
        '#[cfg(test)]\npub(super) fn proof_inbox_session_shape_original('
        'inbox: &Inbox, index: usize, session: &Session) -> Result<(), JournalError> {\n    '+call+'\n}\n')
    return views.finish(project, before, mutation)
