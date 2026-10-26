"""Private, exact suffix extraction; production files are never changed.

Aeneas cannot translate the original early-return loop under the Option
prelude. Moving the original suffix into a helper is a candidate refactor,
not a claimed general equivalence proof.
"""
from hashlib import sha256
from pathlib import Path

HERE = Path('verification/experiments/stage1-protocol/inbox-lifetimes')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
SOURCE = 'controller-service/src/controller/lifecycle_inbox/renewal_confirmation.rs'
MUTATIONS = {
    'skip-predecessor': (
        'prior_end.is_some_and(|prior| prior != before)',
        'prior_end.is_some_and(|_prior| false)'),
    'keep-gap-predecessor': (
        'entry.intent.request.binding.expected != ExpectedSession::Version(prior_version)',
        'entry.intent.request.binding.expected != ExpectedSession::Version(prior_version)'
        '\n            && entry.applied_deadline.is_none()'),
    'skip-final-deadline': (
        'last_applied.is_some_and(|end| lifetime.deadline_ms != end)',
        'last_applied.is_some_and(|_end| false)'),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def candidate(source):
    marker = '    let mut prior_end = Some(initial);'
    begin = source.index(marker, source.index('pub(super) fn validate_lifetimes('))
    end = source.index('\n}\n', begin)
    suffix = source[begin:end]
    helper = '''fn validate_applied_lifetimes(
    session: &Session,
    lifetime: crate::Lifetime,
    start: i64,
    initial: i64,
) -> Result<(), JournalError> {
'''+suffix+'\n}\n'
    updated = source[:begin] + '    validate_applied_lifetimes(session, lifetime, start, initial)'
    updated += source[end:end+3]+'\n'+helper+source[end+3:]
    return updated, dict(moved_suffix=suffix, helper=helper,
                        original_equivalence_proved=False, adopted=False)


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown lifetime variant')
    path = project/SOURCE
    original = path.read_text()
    before = sha(path)
    mutation = None
    refactor = None
    if variant not in ('original', 'restored'):
        text, refactor = candidate(original)
        if variant in MUTATIONS:
            old, new = MUTATIONS[variant]
            start = text.index('fn validate_applied_lifetimes(')
            prefix, body = text[:start], text[start:]
            if body.count(old) != 1:
                raise ValueError('lifetime mutation is not unique inside the candidate helper')
            body = body.replace(old, new, 1)
            text = prefix + body
            mutation = dict(path=SOURCE, old=old, new=new, scope='original moved suffix only')
        path.write_text(text)
    if mutation is not None:
        mutation.update(before=sha256(candidate(original)[0].encode()).hexdigest(), after=sha(path))
    root = project/MODEL
    text = root.read_text()
    if 'fn proof_renewal_lifetimes(' in text:
        raise ValueError('lifetime forwarding root already exists')
    root.write_text(text+'\n'+(project/HERE/'roots.rs.in').read_text())
    return {SOURCE: sha(path), MODEL: sha(root)}, dict(
        production_modified=False, source_before=before, source_after=sha(path),
        refactor=refactor, mutation=mutation)
