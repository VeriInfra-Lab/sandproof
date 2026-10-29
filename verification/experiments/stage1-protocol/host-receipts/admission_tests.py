"""Reject omissions/opacity in actual HostObservation/EffectId extraction records."""
import argparse
import copy
import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('host_receipt_development_admission', HERE/'development.py')
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class ActualAdmission(unittest.TestCase):
    def admitted(self, raw=None, generated=None):
        driver.admit(raw or self.raw, generated or self.generated, self.inventory, self.variant)

    def actual_type(self, raw, parts):
        name = [{'Ident': [s, 0]} for s in parts]
        return next(t for t in raw['translated']['type_decls'] if t and t['item_meta']['name'] == name)

    def test_actual_complete_input(self):
        self.admitted()

    def test_host_and_effect_field_omissions(self):
        for parts in [['sandproof_node', 'linux', 'HostObservation'],
                      ['sandbox_controller_core', 'model', 'EffectId']]:
            with self.subTest(parts=parts):
                raw = copy.deepcopy(self.raw)
                self.actual_type(raw, parts)['kind']['Struct'].pop()
                with self.assertRaises(ValueError): self.admitted(raw=raw)

    def test_prepared_boot_field_cannot_be_erased(self):
        raw = copy.deepcopy(self.raw)
        result = self.actual_type(raw, ['sandproof_node', 'linux', 'ObservedResult'])
        prepared = next(v for v in result['kind']['Enum'] if v['name'] == 'Prepared')
        prepared['fields'].pop()
        with self.assertRaises(ValueError): self.admitted(raw=raw)

    def test_work_outcome_payload_cannot_be_erased(self):
        raw = copy.deepcopy(self.raw)
        result = self.actual_type(raw, ['sandproof_protocol', 'WorkOutcome'])
        next(v for v in result['kind']['Enum'] if v['name'] == 'Unknown')['fields'].pop()
        with self.assertRaises(ValueError): self.admitted(raw=raw)

    def test_opaque_production_getter_rejected(self):
        raw = copy.deepcopy(self.raw)
        method = next(f for f in raw['translated']['fun_decls'] if f and
            f['item_meta']['name'][:2] == [{'Ident': ['sandproof_node', 0]}, {'Ident': ['linux', 0]}]
            and f['item_meta']['name'][-1] == {'Ident': ['resource', 0]})
        method['item_meta']['opacity'] = 'Opaque'
        with self.assertRaises(ValueError): self.admitted(raw=raw)

    def test_unsafe_capture_options_rejected(self):
        for key in ['skip_borrowck', 'no_typecheck', 'monomorphize', 'remove_unused_clauses']:
            with self.subTest(key=key):
                raw = copy.deepcopy(self.raw)
                raw['translated']['options'][key] = True
                with self.assertRaises(ValueError): self.admitted(raw=raw)

    def test_missing_or_extra_generated_method_rejected(self):
        generated = copy.deepcopy(self.generated)
        generated['functions'].pop()
        with self.assertRaises(ValueError): self.admitted(generated=generated)
        generated = copy.deepcopy(self.generated)
        extra = copy.deepcopy(generated['functions'][0]); extra['lean_name'] = 'trusted_entire_host'
        generated['functions'].append(extra)
        with self.assertRaises(ValueError): self.admitted(generated=generated)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--llbc', type=Path, required=True)
    parser.add_argument('--generated', type=Path, required=True)
    parser.add_argument('--variant', choices=('correct', 'owner-only'), required=True)
    args = parser.parse_args()
    ActualAdmission.raw = json.loads(args.llbc.read_text())
    ActualAdmission.generated = json.loads((args.generated/'translation.json').read_text())
    ActualAdmission.inventory = json.loads((HERE/'inventory.json').read_text())
    ActualAdmission.variant = args.variant
    unittest.main(argv=['admission_tests'], verbosity=2)
