"""Strict source/mutation registration for the incomplete Create proof candidate."""
from pathlib import Path
import hashlib
import importlib.util
import json
import re
ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-observation')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
NEW_PROOFS = {'CreateObservationModel', 'CreateObservationEquality', 'CreateObservationValues'}
CASES = {'create-response-key-only', 'create-current-retention-omitted',
         'create-refused-after-success', 'create-renewal-negative-wrap'}


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def inventory(root):
    data = json.loads((root / HERE / 'inventory.json').read_text())
    if data.get('schema') != 1 or set(data['mutations']) != CASES:
        raise ValueError('incomplete actual Rust mutation inventory')
    if data.get('full_guard_refinement') != 'pending exact TryFrom i64/u64 interface':
        raise ValueError('value support must not claim the full Create guard')
    parent = module('create_original_inventory', root / HERE.parent / 'formal.py')
    resume = module('create_resume_inventory', root / HERE.parent / 'resume-observation/formal.py')
    source_inventory = module('unified_protocol_source_inventory', root / HERE.parent / 'source_inventory.py')
    source_inventory.retained(root, set(parent.POSITIVE) | set(resume.INHERITED) | resume.NEW_PROOFS | NEW_PROOFS)
    if set(data['audits']) != NEW_PROOFS:
        raise ValueError('incomplete value proof audit inventory')
    audits = []
    for name, expected in data['audits'].items():
        found = re.findall(r'^#audit_controller_proof (\S+)$', (root / PROOFS / (name + '.lean')).read_text(), re.M)
        if found != expected:
            raise ValueError('value proof audit declaration differs: ' + name)
        audits += found
    if len(audits) != len(set(audits)):
        raise ValueError('duplicate value proof audit declaration')
    for name, expected in data['base_sources'].items():
        if hashlib.sha256((root / name).read_bytes()).hexdigest() != expected:
            raise ValueError('fixed production source changed: ' + name)
    for mutation in data['mutations'].values():
        if (root / mutation['source']).read_text().count(mutation['old']) != 1:
            raise ValueError('actual source mutation is no longer unique')
    return data
