"""Forward to the adopted lifetime checker; mutate copies of its actual body."""
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
    for symbol in ('pub(super) fn validate_lifetimes(', 'fn validate_applied_lifetimes(',
                   '    validate_applied_lifetimes(session, lifetime, start, initial)'):
        if source.count(symbol) != 1:
            raise ValueError('adopted lifetime checker missing or duplicated: ' + symbol)
    return source, dict(source_rewritten=False, original_equivalence_proved=False,
                        adopted=True)


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
