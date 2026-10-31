"""Check the adopted production entry and add roots or scoped mutants to copies."""
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
    before = (project/SOURCE).read_text()
    for symbol in ('pub(super) trait RenewalEncoding', 'fn validate_session_using<',
                   'pub(super) struct StandardRenewalEncoding;',
                   'fn checkpoint_entry_invalid(', 'fn contains_accepted_version(',
                   'fn has_unapplied_sdk_through('):
        if before.count(symbol) != 1:
            raise ValueError('adopted renewal checker missing or duplicated: ' + symbol)
    return before, before, []


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
