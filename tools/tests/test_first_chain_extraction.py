import copy
import json
from pathlib import Path
import unittest

from tools.check_first_chain_extraction import check, OPTION_EQ

ROOT = Path(__file__).resolve().parents[2]


class FirstChainExtractionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.llbc = json.loads((ROOT / 'verification/aeneas/llbc/sandproof_first_chain.llbc').read_text())
        cls.inventory = json.loads((ROOT / 'verification/aeneas/first-chain/translation.json').read_text())

    def test_real_linux_roots_and_option_body(self):
        self.assertEqual(check(ROOT, self.llbc, self.inventory), 3)

    def test_unfrozen_output_destination_is_rejected(self):
        for destination in ('/different-checkout/output.llbc', 'unreviewed.llbc'):
            llbc = copy.deepcopy(self.llbc)
            llbc['translated']['options']['dest_file'] = destination
            with self.assertRaisesRegex(ValueError, 'artifact destination'):
                check(ROOT, llbc, self.inventory)

    def test_missing_root_or_option_cannot_pass(self):
        for name in ('sandproof_first_chain_extract::lifecycle_response',
                     'sandproof_first_chain_extract::operation_response', OPTION_EQ):
            inventory = copy.deepcopy(self.inventory)
            inventory['functions'] = [f for f in inventory['functions'] if f['rust_name'] != name]
            with self.assertRaisesRegex(ValueError, 'missing from inventory'):
                check(ROOT, self.llbc, inventory)

    def test_duplicate_inventory_is_rejected(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['functions'].append(inventory['functions'][1])
        with self.assertRaisesRegex(ValueError, 'duplicate function'):
            check(ROOT, self.llbc, inventory)

    def test_nonroot_owned_function_cannot_disappear(self):
        for name in ('sandproof_protocol::internal::validate_version',
                     'sandproof_center_core::decision::fact_rank'):
            inventory = copy.deepcopy(self.inventory)
            inventory['functions'] = [f for f in inventory['functions'] if f['rust_name'] != name]
            with self.assertRaisesRegex(ValueError, 'owned LLBC function omitted'):
                check(ROOT, self.llbc, inventory)

    def test_center_classifier_cannot_become_an_opaque_assumption(self):
        inventory = copy.deepcopy(self.inventory)
        function = next(f for f in inventory['functions']
                        if f['rust_name'].startswith('sandproof_center_core::')
                        and f['rust_name'].endswith('::classify_response'))
        function['is_opaque'] = True
        with self.assertRaisesRegex(ValueError, 'unreviewed external surface'):
            check(ROOT, self.llbc, inventory)

    def test_protocol_constant_and_initializer_cannot_disappear(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['globals'] = []
        with self.assertRaisesRegex(ValueError, 'protocol version global missing'):
            check(ROOT, self.llbc, inventory)
        llbc = copy.deepcopy(self.llbc)
        initializer = next(f for f in llbc['translated']['fun_decls']
                           if f and isinstance(f['src'], dict) and 'GlobalInitializer' in f['src'])
        initializer['body'] = 'Opaque'
        with self.assertRaisesRegex(ValueError, 'global initializer body missing'):
            check(ROOT, llbc, self.inventory)

    def test_owned_body_or_generation_cannot_be_lost(self):
        inventory = copy.deepcopy(self.inventory)
        function = next(f for f in inventory['functions'] if f['rust_name'].endswith('::lifecycle_response'))
        llbc = copy.deepcopy(self.llbc)
        next(f for f in llbc['translated']['fun_decls']
             if f and f['def_id'] == function['def_id'])['body'] = 'Opaque'
        with self.assertRaisesRegex(ValueError, 'body missing'):
            check(ROOT, llbc, inventory)
        function['lean_file'] = 'missing.lean'
        with self.assertRaisesRegex(ValueError, 'unexpected generated file'):
            check(ROOT, self.llbc, inventory)

    def test_new_external_or_opaque_option_is_rejected(self):
        inventory = copy.deepcopy(self.inventory)
        next(f for f in inventory['functions'] if f['rust_name'] == OPTION_EQ)['is_opaque'] = True
        with self.assertRaisesRegex(ValueError, 'unreviewed external surface'):
            check(ROOT, self.llbc, inventory)

    def test_unordered_functions_are_rejected(self):
        llbc = copy.deepcopy(self.llbc)
        llbc['translated']['ordered_decls'] = []
        with self.assertRaisesRegex(ValueError, 'ordered constant'):
            check(ROOT, llbc, self.inventory)

    def test_errors_and_wrong_target_fail_closed(self):
        llbc = copy.deepcopy(self.llbc)
        llbc['has_errors'] = True
        with self.assertRaisesRegex(ValueError, 'Charon reported errors'):
            check(ROOT, llbc, self.inventory)
        llbc = copy.deepcopy(self.llbc)
        llbc['translated']['target_information'][0]['key'] = 'aarch64-apple-darwin'
        with self.assertRaisesRegex(ValueError, 'extraction target'):
            check(ROOT, llbc, self.inventory)


if __name__ == '__main__':
    unittest.main()
