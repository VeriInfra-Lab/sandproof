"""Reject incomplete actual command/observation/process IR at the development boundary."""
import argparse
import copy
import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('host_evidence_development_admission', HERE/'development.py')
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class ActualAdmission(unittest.TestCase):
    def admitted(self, raw=None, generated=None):
        driver.admit(self.raw if raw is None else raw,
            self.generated if generated is None else generated, self.inventory, self.variant)

    def actual_type(self, raw, path):
        name = [{'Ident': [s, 0]} for s in path]
        return next(t for t in raw['translated']['type_decls'] if t and t['item_meta']['name'] == name)

    def test_actual_complete_input(self):
        self.admitted()

    def test_observation_process_and_resource_fields_cannot_be_erased(self):
        for path in [('sandproof_node', 'linux', 'HostObservation'),
                     ('sandbox_runtime', 'linux', 'helper_gate', 'RuntimeProcessIdentity'),
                     ('sandproof_controller_service', 'capture', 'SourceProcessIdentity'),
                     ('sandproof_protocol', 'ResourceRef')]:
            with self.subTest(path=path):
                raw = copy.deepcopy(self.raw)
                self.actual_type(raw, path)['kind']['Struct'].pop()
                with self.assertRaises(ValueError): self.admitted(raw=raw)

    def test_every_command_and_result_payload_is_required(self):
        for path in [('sandbox_controller_core', 'model', 'Command'),
                     ('sandproof_node', 'linux', 'ObservedResult')]:
            actual = self.actual_type(self.raw, path)['kind']['Enum']
            for index, variant in enumerate(actual):
                for field in range(len(variant['fields'])):
                    with self.subTest(path=path, variant=variant['name'], field=field):
                        raw = copy.deepcopy(self.raw)
                        self.actual_type(raw, path)['kind']['Enum'][index]['fields'].pop(field)
                        with self.assertRaises(ValueError): self.admitted(raw=raw)

    def test_command_variant_and_tag_cannot_be_changed(self):
        path = ('sandbox_controller_core', 'model', 'Command')
        raw = copy.deepcopy(self.raw)
        self.actual_type(raw, path)['kind']['Enum'].pop()
        with self.assertRaises(ValueError): self.admitted(raw=raw)
        raw = copy.deepcopy(self.raw)
        self.actual_type(raw, path)['kind']['Enum'][0]['discriminant'] = 'unreviewed-tag'
        with self.assertRaises(ValueError): self.admitted(raw=raw)

    def test_nested_restore_endpoint_and_policy_fields_are_required(self):
        for name in ('RestoreSource', 'Endpoint', 'Ipv4Cidr', 'SnapshotManifestRef', 'FilePolicyRef'):
            with self.subTest(name=name):
                raw = copy.deepcopy(self.raw)
                self.actual_type(raw, ('sandbox_controller_core', 'model', name))['kind']['Struct'].pop()
                with self.assertRaises(ValueError): self.admitted(raw=raw)

    def test_business_types_cannot_be_opaque(self):
        raw = copy.deepcopy(self.raw)
        self.actual_type(raw, ('sandproof_node', 'linux', 'HostObservation'))['kind'] = 'Opaque'
        with self.assertRaises(ValueError): self.admitted(raw=raw)

    def test_actual_business_methods_cannot_be_opaque(self):
        for name in self.inventory['transparent_functions']:
            if name[0] != {'Ident': ['sandproof_node', 0]}:
                continue
            with self.subTest(name=name):
                raw = copy.deepcopy(self.raw)
                method = next(f for f in raw['translated']['fun_decls'] if f and f['item_meta']['name'] == name)
                method['item_meta']['opacity'] = 'Opaque'
                with self.assertRaises(ValueError): self.admitted(raw=raw)

    def test_unsafe_capture_and_extra_roots_rejected(self):
        for key in ('skip_borrowck', 'no_typecheck', 'monomorphize', 'remove_unused_clauses'):
            with self.subTest(key=key):
                raw = copy.deepcopy(self.raw); raw['translated']['options'][key] = True
                with self.assertRaises(ValueError): self.admitted(raw=raw)
        raw = copy.deepcopy(self.raw); raw['translated']['options']['start_from_if_exists'] = ['trusted_entire_host']
        with self.assertRaises(ValueError): self.admitted(raw=raw)

    def test_missing_extra_or_opaque_generated_methods_rejected(self):
        generated = copy.deepcopy(self.generated); generated['functions'].pop()
        with self.assertRaises(ValueError): self.admitted(generated=generated)
        generated = copy.deepcopy(self.generated)
        extra = copy.deepcopy(generated['functions'][0]); extra['lean_name'] = 'trusted_entire_host'
        generated['functions'].append(extra)
        with self.assertRaises(ValueError): self.admitted(generated=generated)
        generated = copy.deepcopy(self.generated)
        next(f for f in generated['functions'] if not f['is_opaque'])['is_opaque'] = True
        with self.assertRaises(ValueError): self.admitted(generated=generated)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--llbc', type=Path, required=True)
    parser.add_argument('--generated', type=Path, required=True)
    parser.add_argument('--variant', choices=('correct', 'skip-command', 'skip-resource'), required=True)
    args = parser.parse_args()
    ActualAdmission.raw = json.loads(args.llbc.read_text())
    ActualAdmission.generated = json.loads((args.generated/'translation.json').read_text())
    ActualAdmission.inventory = json.loads((HERE/'inventory.json').read_text())
    ActualAdmission.variant = args.variant
    unittest.main(argv=['admission_tests'], verbosity=2)
