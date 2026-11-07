"""Real one-expression Rust mutations of the original retained-order check."""
from hashlib import sha256
from pathlib import Path

HERE = Path('verification/experiments/stage1-protocol/inbox-retained')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
RENEWAL = 'controller-service/src/controller/lifecycle_inbox/renewal_confirmation.rs'
MUTATIONS = {
    'reverse-compaction-bound': ('|through| expected <= through', '|through| expected >= through'),
    'reverse-confirmation-bound': (
        'intent.accepted_version <= confirmed.request.accepted_version',
        'intent.accepted_version >= confirmed.request.accepted_version'),
    'zero-version-adjacency': ('if expected == previous => previous > 0', 'if expected == previous => true'),
}
VARIANTS = ('original', *MUTATIONS, 'restored')


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown retained-order variant')
    path = project/RENEWAL
    text = path.read_text()
    start = text.index('pub(super) fn follows_retained(')
    end = text.index('\n}\n', start)+2
    body = text[start:end]
    before = sha(path)
    mutation = None
    if variant in MUTATIONS:
        old, new = MUTATIONS[variant]
        if body.count(old) != 1:
            raise ValueError('retained-order mutation is not unique')
        path.write_text(text[:start]+body.replace(old, new, 1)+text[end:])
        mutation = dict(path=RENEWAL, old=old, new=new, before=before, after=sha(path),
                        scope='only the original renewal_confirmation::follows_retained body')
    model = project/MODEL
    if 'fn proof_retained_history(' in model.read_text():
        raise ValueError('retained-order root already exists')
    model.write_text(model.read_text()+'\n'+(project/HERE/'roots.rs.in').read_text())
    return {MODEL: sha(model), RENEWAL: sha(path)}, mutation
