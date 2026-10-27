"""Only add a transparent root; variants mutate the actual protocol method."""
from pathlib import Path
import hashlib
import json

HERE = Path('verification/experiments/stage1-protocol/terminal-binding')
PARENT = HERE.parent / 'inbox-record'
TERMINAL = 'sandbox-protocol/src/lifecycle/terminal.rs'
SHIM = str(PARENT / 'lib.rs')
MUTATIONS = {
    'key-only-observation': ('        self.request == *request\n',
        '        self.request.request.binding.key == request.request.binding.key\n'),
    'allow-unload': ('            && matches!(request.request.operation, LifecycleOperation::Destroy {})\n',
        '            && matches!(\n                request.request.operation,\n                LifecycleOperation::Destroy {} | LifecycleOperation::Unload { .. }\n            )\n'),
    'omit-retention-shape': ('            && request.retention.valid_for(4096)\n', ''),
    'omit-version-relation': ('            && request.accepted_version_for_request() == Some(request.accepted_version)\n', ''),
}
VARIANTS = ('original', *MUTATIONS, 'restored')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown terminal matching variant')
    fixed = json.loads((project/HERE/'inventory.json').read_text())
    if sha(project/TERMINAL) != fixed['production_inputs'][TERMINAL]:
        raise ValueError('actual original terminal implementation differs')
    original = {name: sha(project/name) for name in [TERMINAL, SHIM]}
    mutation = None
    if variant in MUTATIONS:
        old, new = MUTATIONS[variant]
        text = (project/TERMINAL).read_text()
        if text.count(old) != 1:
            raise ValueError('nonunique terminal mutation')
        (project/TERMINAL).write_text(text.replace(old, new, 1))
        mutation = dict(path=TERMINAL, old=old, new=new,
            before=original[TERMINAL], after=sha(project/TERMINAL))
    shim = (project/SHIM).read_text()
    if 'fn terminal_matches(' in shim:
        raise ValueError('existing terminal root')
    (project/SHIM).write_text(shim+(project/HERE/'roots.rs.in').read_text())
    return {name: sha(project/name) for name in original}, dict(original_inputs=original,
        mutation=mutation, production_modified=False, private_refactor=False,
        original_body_unchanged=variant in ('original', 'restored'))
