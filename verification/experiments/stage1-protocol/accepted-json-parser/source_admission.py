"""Reuse canonical decoder assembly admission for the current production probe.

All struct/enum/leaf checks belong to decode-assembly. Only the transparent
current callback binding is added here. Neither part proves JSON semantics.
"""
import argparse
import contextlib
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent
PROVIDER = PARENT / 'decode-assembly'
ROOT = HERE.parents[3]
ROOT_NAME = 'crate::server::lifecycle::proof_current_accepted_parser'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


binding = load('accepted_canonical_decode_binding', PROVIDER / 'binding.py')
require, digest = binding.require, binding.digest


@contextlib.contextmanager
def provider_modules():
    names = ('binding', 'expected', 'leaves', 'operation')
    previous = {name: sys.modules.get(name) for name in names}
    sys.modules['binding'] = binding
    try:
        for name in names[1:]:
            sys.modules[name] = load('accepted_canonical_' + name, PROVIDER / (name + '.py'))
        yield
    finally:
        for name, value in previous.items():
            if value is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = value


with provider_modules():
    assembly = load('accepted_canonical_decode_check', PROVIDER / 'check.py')
calls = load('accepted_current_callback_binding', HERE / 'call_binding.py')


def report(raw, schema):
    with provider_modules():
        result = assembly.check(raw, schema, root=ROOT_NAME)
    original = binding.Original(raw)
    accepted = [original.functions[fid] for fid, observation in result['functions'].items()
                if observation['method'] == 'deserialize' and observation['success_type'] ==
                'sandproof_protocol::lifecycle::authority::AcceptedLifecycleIntent' and
                len(original.functions[fid]['signature']['inputs']) == 1]
    require(len(accepted) == 1, 'current Accepted Deserialize body is ambiguous')
    result['parser_call_binding'] = calls.report(original, accepted[0], digest)
    result.update(kind='current-parser-canonical-assembly-and-callback-source-binding',
                  canonical_provider='verification/experiments/stage1-protocol/decode-assembly',
                  production_slice_contract=False, native_acceptance=False, shared_acceptance=False,
                  new_Lean_proof=False, full_v3=False)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--llbc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    require(not args.output.exists(), 'preserve existing source admission records')
    schema_path = PARENT / 'envelope-schema/inventory.json'
    result = report(json.loads(args.llbc.read_text()), json.loads(schema_path.read_text()))
    providers = [schema_path, *sorted(PROVIDER.glob('*.py')), Path(__file__), HERE / 'call_binding.py']
    result['providers'] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                           for path in providers}
    result['original_llbc_file_sha256'] = hashlib.sha256(args.llbc.read_bytes()).hexdigest()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')


if __name__ == '__main__':
    main()
