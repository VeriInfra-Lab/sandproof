"""Keep source admission separate from the shared registry content comparison."""
from hashlib import sha256
import importlib.util
from pathlib import Path
import tomllib

_path = Path(__file__).resolve().parents[1] / 'registry_identity.py'
_spec = importlib.util.spec_from_file_location('inbox_shared_registry_identity', _path)
registry_identity = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(registry_identity)


def admit(record, native_result, inputs, registry, root, lock_name):
    record.update(inputs=inputs, registry=registry)
    if native_result is None:
        return
    # Keep both original physical records, including on a rejected admission.
    record['native_registry'] = native_result['registry']
    if native_result['inputs'] != inputs:
        raise ValueError('native and formal source inputs differ')
    record['native_inputs_match'] = True
    lock_bytes = (root / lock_name).read_bytes()
    if sha256(lock_bytes).hexdigest() != inputs[str(lock_name)]:
        raise ValueError('registry Cargo.lock differs from admitted source input')
    registry_identity.require_same_content(native_result['registry'], registry,
                                          tomllib.loads(lock_bytes.decode()))
    record['registry_content_match'] = True
