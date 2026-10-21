"""Fail closed on changed component inputs or an incomplete actual audit inventory."""
import hashlib
import json
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
MODEL = Path('SandboxModel/Boundaries/ProtectedDirectory')
LIBRARY = Path('verification/aeneas/protected-directory')
NAMESPACE = 'SandboxModel.Boundaries.ProtectedDirectory.'
CORRECT = ('allow_iff', 'private_owner_nonempty', 'root_sticky_nonempty',
    'group_writable_is_not_allowed', 'nonroot_sticky_is_not_allowed', 'source_exact',
    'source_ok_iff', 'source_untrusted_iff', 'source_acceptance_safe',
    'source_rejects_non_directory', 'source_rejects_other_owner',
    'source_rejects_nonroot_writes', 'source_private_owner_accepted',
    'source_root_sticky_accepted')
NEGATIVE = ('mutant_accepts_forbidden', 'mutant_not_acceptance_safe')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check_inputs(root=ROOT):
    environment = json.loads((root / HERE.relative_to(ROOT) / 'environment.json').read_text())
    frozen = json.loads((root / HERE.relative_to(ROOT) / 'frozen.json').read_text())
    if frozen.get('schema') != 1 or not frozen.get('files'):
        raise ValueError('component input inventory missing')
    for name, expected in {**environment['base_inputs'], **frozen['files']}.items():
        path = Path(name)
        if path.is_absolute() or '..' in path.parts or sha(root / path) != expected:
            raise ValueError('frozen input differs: ' + name)
    actual = {str(p.relative_to(root)) for folder in (root / MODEL, root / LIBRARY, root / HERE.relative_to(ROOT))
              for p in folder.rglob('*') if p.is_file() and '__pycache__' not in p.parts
              and p.name != 'frozen.json'}
    if actual != set(frozen['files']):
        raise ValueError('component files missing or unregistered')
    expected = [NAMESPACE + name for name in CORRECT + NEGATIVE]
    audits = []
    for path in sorted((root / MODEL).glob('*.lean')):
        text = path.read_text()
        if re.search(r'\b(?:sorry|admit|axiom)\b', text):
            raise ValueError('unapproved proof placeholder or assumption')
        audits += re.findall(r'^#audit_controller_proof (\S+)', text, re.M)
    if len(audits) != len(set(audits)) or sorted(audits) != sorted(expected):
        raise ValueError('exact sixteen-theorem audit inventory differs')
    return frozen


if __name__ == '__main__':
    inputs = check_inputs()
    print(json.dumps({'success': True, 'files': len(inputs['files']), 'audits': 16}))
