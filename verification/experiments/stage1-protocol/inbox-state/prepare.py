"""Thin state root and scoped mutants in private original-source copies only."""
from hashlib import sha256
from pathlib import Path

HERE = Path('verification/experiments/stage1-protocol/inbox-state')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
VARIANTS = ('original', 'always-current', 'drop-operation', 'restored')
MUTATIONS = {
    'always-current': ('current: entry.intent.accepted_version == session.current_version,',
                       'current: true,'),
    'drop-operation': ('local_operation: entry.local_operation,', 'local_operation: None,'),
}


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(project, variant, forwarder=False):
    if variant not in VARIANTS:
        raise ValueError('unknown state variant')
    path = project/MODEL
    text = path.read_text()
    start = text.index('    pub(super) fn state(')
    end = text.index('\n    pub(super) fn ', start + 1)
    original = text[start:end]
    body = original
    mutation = None
    if variant in MUTATIONS:
        old, new = MUTATIONS[variant]
        if body.count(old) != 1:
            raise ValueError('state mutation is not unique')
        body = body.replace(old, new, 1)
        before = sha(path)
        text = text[:start] + body + text[end:]
        path.write_text(text)
        mutation = dict(path=MODEL, old=old, new=new, before=before, after=sha(path),
                        scope='only the original Inbox::state body')
    if forwarder:
        addition = (project/HERE/'roots.rs.in').read_text()
        if 'fn proof_state(' in text:
            raise ValueError('state root already exists')
        path.write_text(text + '\n' + addition)
    return {MODEL: sha(path)}, mutation
