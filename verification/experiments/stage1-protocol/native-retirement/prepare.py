"""Prepare the actual isolated saved-retirement validator and real Rust mistakes."""
from pathlib import Path
from hashlib import sha256
import difflib
import json

HERE = Path("verification/experiments/stage1-protocol/native-retirement")

SOURCE = 'controller-service/src/controller/lifecycle_inbox/native_retirement.rs'
INTERFACE = '''pub(super) trait RetirementEncoding {
    fn encode_intent(
        value: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    ) -> Result<Vec<u8>, serde_json::Error>;
    fn digest(bytes: &[u8]) -> ContentDigest;
    fn encode_receipt(value: &NativeRetirementReceipt) -> Result<Vec<u8>, serde_json::Error>;
}

struct StandardRetirementEncoding;
impl RetirementEncoding for StandardRetirementEncoding {
    fn encode_intent(
        value: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    ) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(value)
    }
    fn digest(bytes: &[u8]) -> ContentDigest {
        source_work::digest(bytes)
    }
    fn encode_receipt(value: &NativeRetirementReceipt) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(value)
    }
}

'''


HELPER = '''
fn last_saved_entry(session: &Session) -> Option<&super::model::Entry> {
    if session.entries.is_empty() {
        None
    } else {
        Some(&session.entries[session.entries.len() - 1])
    }
}

fn is_destroy_operation(operation: &LifecycleOperation) -> bool {
    matches!(operation, LifecycleOperation::Destroy { .. })
}
'''


def candidate(before):
    signature = 'pub(super) fn validate_saved(session: &Session) -> Result<(), JournalError> {'
    start = before.index(signature)
    end = before.index('pub(super) fn cleaned(', start)
    body = before[start:end]
    changes = [
        (signature, signature+'''
    validate_saved_using::<StandardRetirementEncoding>(session)
}

fn validate_saved_using<E: RetirementEncoding>(session: &Session) -> Result<(), JournalError> {'''),
        ('source_work::digest(&serde_json::to_vec(&last.intent)?)', 'E::digest(&E::encode_intent(&last.intent)?)'),
        ('serde_json::to_vec(saved)?.len()', 'E::encode_receipt(saved)?.len()'),
    ]
    changes.append(('matches!(\n            last.intent.request.operation,\n            LifecycleOperation::Destroy { .. }\n        )', 'is_destroy_operation(&last.intent.request.operation)'))
    changes.append(('session.entries.last()', 'last_saved_entry(session)'))
    for old, new in changes:
        assert body.count(old) == 1
        body = body.replace(old, new, 1)
    return before[:start]+INTERFACE+body+before[end:]+HELPER, changes


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


VARIANTS = ('original', 'candidate', *FAILURES, 'restored')


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown saved-retirement variant')
    inventory = json.loads((project/HERE/'inventory.json').read_text())
    if sha(project/SOURCE) != inventory['production_inputs'][SOURCE]:
        raise ValueError('fixed original saved-retirement source differs')
    before = (project/SOURCE).read_text()
    correct, changes = candidate(before)
    actual = before
    mutation = None
    if variant not in ('original', 'restored'):
        actual, edits = mutate(correct, variant)
        if edits:
            mutation = dict(path=SOURCE, edits=[list(edit) for edit in edits],
                before=sha256(correct.encode()).hexdigest(), after=sha256(actual.encode()).hexdigest())
        actual += '\n'+(project/HERE/'generic-root.rs.in').read_text()
    (project/SOURCE).write_text(actual)
    return {SOURCE: sha(project/SOURCE)}, dict(mutation=mutation,
        source_before=sha256(before.encode()).hexdigest(), source_after=sha(project/SOURCE),
        changes=changes, production_modified=False,
        private_refactor=variant not in ('original', 'restored'),
        original_equivalence_proved=False, standard_encoding_and_digest_adopted=False)


def patch(project):
    before = (project/SOURCE).read_text()
    after, _ = candidate(before)
    return ''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
        fromfile='a/'+SOURCE, tofile='b/'+SOURCE, n=3)).replace('\n \n', '\n\n')
