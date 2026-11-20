"""Add a forwarding root and exact caller mutations only to a private source copy."""
from hashlib import sha256
from pathlib import Path

HERE = Path('verification/experiments/stage1-protocol/inbox-admission')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
CALLER = 'controller-service/src/controller/lifecycle_inbox.rs'
VARIANTS = ('original', 'skip-node', 'skip-envelope', 'restored')
MUTATIONS = {
    'skip-node': (MODEL,
        'if self.config.center != *context.center() || self.config.node != *context.expected_node() {',
        'if self.config.center != *context.center() {'),
    'skip-envelope': (CALLER,
        'intent.validate(context).map_err(|_| JournalError::Conflict)',
        'let _ = intent;\n    Ok(())'),
}


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    """Mutate a verified original copy; never factor or replace a business body."""
    if variant not in VARIANTS:
        raise ValueError('unknown admission variant')
    changes = {}
    mutation = None
    if variant in MUTATIONS:
        name, old, new = MUTATIONS[variant]
        path = project / name
        text = path.read_text()
        if text.count(old) != 1:
            raise ValueError('actual guard is not unique: ' + name)
        before = sha(path)
        path.write_text(text.replace(old, new))
        mutation = dict(path=name, old=old, new=new, before=before, after=sha(path))
        changes[name] = sha(path)
    path = project / MODEL
    if 'fn proof_authenticated(' in path.read_text():
        raise ValueError('forwarding root already exists')
    with path.open('a') as source:
        source.write('\n' + (project / HERE / 'roots.rs.in').read_text())
    changes[MODEL] = sha(path)
    # Both actual business files are bound even when only one changes.
    changes[CALLER] = sha(project / CALLER)
    return changes, mutation
