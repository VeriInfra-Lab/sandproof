"""Prepare the actual isolated session-shape validator and four wrong Rust bodies."""
from pathlib import Path
from hashlib import sha256
import difflib
import importlib.util
import json
import subprocess

HERE = Path('verification/experiments/stage1-protocol/session-shape')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/model.rs'
SAVED = 'controller-service/src/controller/lifecycle_inbox/native_retirement.rs'
SHIM = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'

HELPERS = '''
fn first_lane_window_matches(inbox: &Inbox, session: &Session) -> bool {
    let mut position = 0;
    while position < inbox.lanes.len() {
        let lane = &inbox.lanes[position];
        if lane.slot == session.retention.slot {
            return lane.current_window == Some(session.retention.window);
        }
        position += 1;
    }
    false
}

fn prior_session_conflict(inbox: &Inbox, index: usize, session: &Session) -> bool {
    let previous = &inbox.sessions[..index];
    let mut position = 0;
    while position < previous.len() {
        let old = &previous[position];
        if old.id == session.id
            || old.retention.slot == session.retention.slot
            || session.local.is_some_and(|local| {
                old.local.is_some_and(|prior| prior.sandbox == local.sandbox)
            })
        {
            return true;
        }
        position += 1;
    }
    false
}

fn refusal_entries_invalid(session: &Session) -> bool {
    let mut position = 0;
    while position < session.entries.len() {
        let entry = &session.entries[position];
        if entry.local_operation.is_some()
            || entry.applied_deadline.is_some()
            || entry.previous_effective_end_at_ms.is_some()
            || !matches!(
                entry.intent.request.operation,
                LifecycleOperation::Create { .. } | LifecycleOperation::Destroy {}
            )
        {
            return true;
        }
        position += 1;
    }
    false
}
'''


def shape_candidate(before):
    start = before.index('    fn validate_session_shape(')
    end = before.index('    fn entry_kind(', start)
    body = before[start:end]
    edits = [
        ('''self
                .lanes
                .iter()
                .find(|lane| lane.slot == session.retention.slot)
                .is_none_or(|lane| lane.current_window != Some(session.retention.window))''',
         '!first_lane_window_matches(self, session)'),
        ('''self.sessions[..index].iter().any(|old| {
                old.id == session.id
                    || old.retention.slot == session.retention.slot
                    || session.local.is_some_and(|local| {
                        old.local
                            .is_some_and(|prior| prior.sandbox == local.sandbox)
                    })
            })''', 'prior_session_conflict(self, index, session)'),
        ('''session.entries.iter().any(|entry| {
                    entry.local_operation.is_some()
                        || entry.applied_deadline.is_some()
                        || entry.previous_effective_end_at_ms.is_some()
                        || !matches!(
                            entry.intent.request.operation,
                            LifecycleOperation::Create { .. } | LifecycleOperation::Destroy {}
                        )
                })''', 'refusal_entries_invalid(session)'),
    ]
    for old, new in edits:
        assert body.count(old) == 1
        body = body.replace(old, new, 1)
    return before[:start]+body+before[end:]+HELPERS, edits

FIRST = '\nfn first_session_entry(session: &Session) -> Option<&Entry> {\n    if session.entries.is_empty() {\n        None\n    } else {\n        Some(&session.entries[0])\n    }\n}\n'

MUTATIONS = {'wrong-first-lane': ('        if lane.slot == session.retention.slot {\n            return lane.current_window == Some(session.retention.window);\n        }', '        if lane.slot == session.retention.slot\n            && lane.current_window == Some(session.retention.window)\n        {\n            return true;\n        }'), 'wrong-physical': ('            || session.local.is_some_and(|local| {\n                old.local.is_some_and(|prior| prior.sandbox == local.sandbox)\n            })', ''), 'wrong-refusal-field': ('            || entry.previous_effective_end_at_ms.is_some()\n', ''), 'wrong-last-session': ('.is_none_or(|entry| entry.intent != confirmed.request)', '.is_none_or(|entry| entry.intent.request.binding.session != confirmed.request.request.binding.session)')}
FAILURES = {'wrong-first-lane': ['first_matching_lane_controls_window'], 'wrong-physical': ['prior_physical_sandbox_conflicts_despite_distinct_owner_and_operation'], 'wrong-refusal-field': ['refusal_checks_initial_shape_and_every_native_field'], 'wrong-last-session': ['terminal_binds_complete_last_intent_and_cleaned_status']}
VARIANTS = ('original', 'candidate', *FAILURES, 'restored')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def candidate(project):
    saved = module('shape_saved_candidate', project/HERE.parent/'native-retirement/prepare.py')
    terminal = module('shape_terminal_interface', project/HERE.parent/'retention-lane/prepare.py')
    body, edits = shape_candidate((project/SOURCE).read_text())
    signature = '    fn validate_session_shape(&self, index: usize, session: &Session) -> Result<(), JournalError> {'
    wrapper = signature+"""
        self.validate_session_shape_using::<
            super::native_retirement::StandardRetirementEncoding,
            StandardTerminalEncoding,
        >(index, session)
    }

    fn validate_session_shape_using<
        R: super::native_retirement::RetirementEncoding,
        E: TerminalEncoding,
    >(&self, index: usize, session: &Session) -> Result<(), JournalError> {"""
    changes = [
        (signature, wrapper),
        ('super::native_retirement::validate_saved(session)?;',
         'super::native_retirement::validate_saved_using::<R>(session)?;'),
        ('serde_json::to_vec(confirmed)?.len() > 8192', 'E::encode(confirmed)?.len() > 8192'),
        ('session.entries.first()', 'first_session_entry(session)'),
        ('session\n                    .entries\n                    .last()',
         'super::native_retirement::last_saved_entry(session)'),
    ]
    for old, new in changes:
        assert body.count(old) == 1
        body = body.replace(old, new, 1)
    saved_body, saved_edits = saved.candidate((project/SAVED).read_text())
    visibility = [
        ('struct StandardRetirementEncoding;', 'pub(super) struct StandardRetirementEncoding;'),
        ('fn validate_saved_using<E:', 'pub(super) fn validate_saved_using<E:'),
        ('fn last_saved_entry(', 'pub(super) fn last_saved_entry('),
    ]
    for old, new in visibility:
        assert saved_body.count(old) == 1
        saved_body = saved_body.replace(old, new, 1)
    return body+'\n'+terminal.INTERFACE+FIRST, saved_body, dict(
        iterator_edits=edits, encoding_and_entry_edits=changes,
        reused_saved_edits=saved_edits, visibility_edits=visibility,
        terminal_interface_sha256=sha256(terminal.INTERFACE.encode()).hexdigest())


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown session-shape variant')
    inventory = json.loads((project/HERE/'inventory.json').read_text())
    for name in (SOURCE, SAVED, SHIM):
        if sha(project/name) != inventory['original_inputs'][name]:
            raise ValueError('fixed original input differs: '+name)
    before = (project/SOURCE).read_text()
    correct, saved, metadata = candidate(project)
    actual = before
    mutation = None
    names = [SOURCE]
    call = 'inbox.validate_session_shape(index, session)'
    if variant not in ('original', 'restored'):
        actual = correct
        if variant in MUTATIONS:
            old, new = MUTATIONS[variant]
            assert actual.count(old) == 1, (variant, actual.count(old))
            actual = actual.replace(old, new, 1)
            mutation = dict(path=SOURCE, edits=[[old,new]],
                before=sha256(correct.encode()).hexdigest(), after=sha256(actual.encode()).hexdigest())
        actual += '\n'+(project/HERE/'generic-root.rs.in').read_text()
        (project/SAVED).write_text(saved)
        (project/SHIM).write_text((project/SHIM).read_text()+
            (project/HERE.parent/'terminal-binding/roots.rs.in').read_text())
        names += [SAVED, SHIM]
        call = 'proof_inbox_session_shape::<super::native_retirement::StandardRetirementEncoding, StandardTerminalEncoding>(inbox, index, session)'
    actual += '\n#[cfg(test)]\npub(super) fn proof_inbox_session_shape_original(inbox: &Inbox, index: usize, session: &Session) -> Result<(), JournalError> {\n    '+call+'\n}\n'
    (project/SOURCE).write_text(actual)
    subprocess.run(['rustfmt', '--edition', '2024', SOURCE, SAVED], cwd=project, check=True)
    metadata.update(mutation=mutation, source_before=sha256(before.encode()).hexdigest(),
        production_modified=False, private_refactor=variant not in ('original','restored'),
        original_equivalence_proved=False, standard_encoding_and_digest_adopted=False)
    return {name:sha(project/name) for name in names}, metadata


def patch(project):
    body, saved, _ = candidate(project)
    diffs = []
    for name, after in [(SOURCE, body), (SAVED, saved)]:
        before = (project/name).read_text()
        diffs.append(''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
            fromfile='a/'+name, tofile='b/'+name, n=3)).replace('\n \n', '\n\n'))
    return ''.join(diffs)
