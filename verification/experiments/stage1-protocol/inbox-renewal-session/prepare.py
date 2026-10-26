"""Private source factorization; original equivalence and adoption remain open."""
from hashlib import sha256
from pathlib import Path
import difflib
import json

HERE = Path('verification/experiments/stage1-protocol/inbox-renewal-session')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/renewal_confirmation.rs'
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
LIBRARY = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'
MUTATIONS = {
    'key-only-checkpoint': (
        'if entry.intent == confirmed.request {',
        'if entry.intent.request.binding.key == confirmed.request.request.binding.key {'),
    'ignore-retained-frontier': (
        '|| contains_accepted_version(session, through)',
        '|| { let _ = contains_accepted_version(session, through); false }'),
    'ignore-unapplied-sdk': (
        '&& entry.applied_deadline.is_none()',
        '&& entry.applied_deadline.is_some_and(|_| false)'),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def candidate(project):
    parts = json.loads((project/HERE/'fragments.json').read_text())
    before = (project/SOURCE).read_text()
    signature = 'pub(super) fn validate_session(session: &Session) -> Result<(), JournalError> {'
    changes = [
        (parts['first_match_old'], 'checkpoint_entry_invalid(session, confirmed)'),
        (signature, parts['encoding_interface']),
        ('serde_json::to_vec(confirmed)?', 'E::encode(confirmed)?'),
        (parts['frontier_old'], 'contains_accepted_version(session, through)'),
        (parts['unapplied_old'], 'has_unapplied_sdk_through(session, confirmed.request.accepted_version)'),
    ]
    after = before
    for old, new in changes:
        if after.count(old) != 1:
            raise ValueError('source factorization is not unique')
        after = after.replace(old, new, 1)
    return before, after+parts['first_match_helper']+parts['any_helpers'], changes


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown renewal-session variant')
    before, correct, changes = candidate(project)
    mutation = None
    if variant not in ('original', 'restored'):
        actual = correct
        if variant in MUTATIONS:
            old, new = MUTATIONS[variant]
            # The last predicate also exists in the original validate_session;
            # after factorization it occurs once, in the named helper.
            if actual.count(old) != 1:
                raise ValueError('renewal-session mutation is not unique')
            actual = actual.replace(old, new, 1)
            mutation = dict(path=SOURCE, old=old, new=new,
                before=sha256(correct.encode()).hexdigest(), after=sha256(actual.encode()).hexdigest())
        (project/SOURCE).write_text(actual)
    root = project/MODEL
    if 'fn proof_renewal_session(' in root.read_text():
        raise ValueError('forwarding root already exists')
    with root.open('a') as stream:
        stream.write('\n'+(project/HERE/'roots.rs.in').read_text())
        if variant not in ('original', 'restored'):
            stream.write('\n'+(project/HERE/'generic-root.rs.in').read_text())
    lib = project/LIBRARY
    if 'pub fn renewal_valid(' in lib.read_text():
        raise ValueError('reused #696 roots already exist')
    with lib.open('a') as stream:
        stream.write(json.loads((project/HERE/'fragments.json').read_text())['renewal_roots'])
    return {n:sha(project/n) for n in (SOURCE, MODEL, LIBRARY)}, dict(
        source_before=sha256(before.encode()).hexdigest(), source_after=sha(project/SOURCE),
        mutation=mutation, changes=changes, production_modified=False,
        original_equivalence_proved=False, standard_serde_instance_adopted=False)


def patch(project):
    before, after, _ = candidate(project)
    return ''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
        fromfile='a/'+SOURCE, tofile='b/'+SOURCE))
