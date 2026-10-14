import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.check_network_extraction import (ROOTS, STANDARD_SOURCES, check, check_standard_sources,
                                           drop_glue_name, llbc_digest)

ROOT = Path(__file__).resolve().parents[2]
FUNCTION = 'sandbox_controller_core::network::ipv4_list_contains'


class NetworkExtractionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.llbc = json.loads((ROOT / 'verification/aeneas/llbc/sandproof_network.llbc').read_text())
        cls.inventory = json.loads((ROOT / 'verification/aeneas/network/translation.json').read_text())

    def test_real_roots_bodies_loops_globals_and_models_are_present(self):
        self.assertEqual(check(ROOT, self.llbc, self.inventory), len(ROOTS))

    def test_destination_cannot_depend_on_the_checkout_or_another_output(self):
        for destination in ('/old-checkout/verification/aeneas/llbc/sandproof_network.llbc',
                            'verification/aeneas/llbc/other.llbc'):
            llbc = copy.deepcopy(self.llbc)
            llbc['translated']['options']['dest_file'] = destination
            with self.assertRaisesRegex(ValueError, 'project-relative path'):
                check(ROOT, llbc, self.inventory)

    def test_only_unordered_short_name_order_is_normalized(self):
        reordered = copy.deepcopy(self.llbc)
        reordered['translated']['short_names'].reverse()
        self.assertEqual(llbc_digest(self.llbc), llbc_digest(reordered))
        changed = copy.deepcopy(self.llbc)
        changed['translated']['ordered_decls'].reverse()
        self.assertNotEqual(llbc_digest(self.llbc), llbc_digest(changed))
        duplicate = copy.deepcopy(self.llbc)
        duplicate['translated']['short_names'].append(duplicate['translated']['short_names'][0])
        with self.assertRaisesRegex(ValueError, 'duplicate short-name key'):
            llbc_digest(duplicate)

    def test_extraction_error_and_unreviewed_target_are_rejected(self):
        llbc = copy.deepcopy(self.llbc)
        llbc['has_errors'] = True
        with self.assertRaisesRegex(ValueError, 'Charon reported errors'):
            check(ROOT, llbc, self.inventory)
        llbc = copy.deepcopy(self.llbc)
        llbc['translated']['target_information'][0]['key'] = 'unreviewed-target'
        with self.assertRaisesRegex(ValueError, 'extraction target'):
            check(ROOT, llbc, self.inventory)

    def test_removed_root_is_not_metadata_acceptance(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['functions'] = [function for function in inventory['functions']
                                  if function['rust_name'] != 'sandproof_network_extract::cidr_contains']
        with self.assertRaisesRegex(ValueError, 'root was not generated'):
            check(ROOT, self.llbc, inventory)

    def test_current_state_and_input_roots_cannot_disappear(self):
        for name in ('runtime_crash', 'runtime_recover', 'slot_transition', 'runtime', 'controller_input', 'create'):
            with self.subTest(root=name):
                inventory = copy.deepcopy(self.inventory)
                inventory['functions'] = [function for function in inventory['functions']
                    if function['rust_name'] != f'sandproof_network_extract::{name}']
                with self.assertRaisesRegex(ValueError, 'root was not generated'):
                    check(ROOT, self.llbc, inventory)

    def test_creation_scalar_closure_cannot_hide_a_different_capture(self):
        llbc = copy.deepcopy(self.llbc)
        closure = next(entry for entry in self.inventory['types']
                       if entry['rust_name'] == 'sandbox_controller_core::maintenance::fresh_identity::{closure}')
        declaration = next(entry for entry in llbc['translated']['type_decls']
                           if entry and entry['def_id'] == closure['def_id'])
        declaration['kind']['Struct'][0]['ty'] = {'Deduplicated': -1}
        with self.assertRaisesRegex(ValueError, 'closure capture alias'):
            check(ROOT, llbc, self.inventory)

    def test_owned_body_missing_and_opacity_are_rejected(self):
        inventory = copy.deepcopy(self.inventory)
        function = next(function for function in inventory['functions']
                        if function['rust_name'] == FUNCTION and 'loop' not in function)
        llbc = copy.deepcopy(self.llbc)
        next(declaration for declaration in llbc['translated']['fun_decls']
             if declaration and declaration['def_id'] == function['def_id'])['body'] = 'Opaque'
        with self.assertRaisesRegex(ValueError, 'project-owned body missing'):
            check(ROOT, llbc, inventory)
        function['is_opaque'] = True
        with self.assertRaisesRegex(ValueError, 'unreviewed external function surface'):
            check(ROOT, self.llbc, inventory)

    def test_owned_llbc_body_cannot_disappear_from_inventory(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['functions'] = [function for function in inventory['functions']
                                  if function['rust_name'] != FUNCTION]
        with self.assertRaisesRegex(ValueError, 'loop identity or position|owned LLBC body omitted'):
            check(ROOT, self.llbc, inventory)

    def test_loop_requires_body_and_wrapper(self):
        inventory = copy.deepcopy(self.inventory)
        loop = next(function for function in inventory['functions']
                    if function['rust_name'] == FUNCTION and 'loop' in function)
        inventory['functions'].remove(loop)
        with self.assertRaisesRegex(ValueError, 'wrapper or body missing'):
            check(ROOT, self.llbc, inventory)

    def test_deleting_both_loop_aliases_is_still_rejected(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['functions'] = [function for function in inventory['functions']
                                  if not (function['rust_name'] == FUNCTION and 'loop' in function)]
        with self.assertRaisesRegex(ValueError, 'loop identity or position|LLBC loop omitted or invented'):
            check(ROOT, self.llbc, inventory)

    def test_loop_parent_and_role_cannot_be_rebound(self):
        inventory = copy.deepcopy(self.inventory)
        loop = next(function for function in inventory['functions'] if 'loop' in function)
        loop['parent_lean_name'] = 'wrong.parent'
        with self.assertRaisesRegex(ValueError, 'unbound or malformed loop'):
            check(ROOT, self.llbc, inventory)
        inventory = copy.deepcopy(self.inventory)
        loop = next(function for function in inventory['functions'] if 'loop' in function)
        loop['loop']['id'] = False
        with self.assertRaisesRegex(ValueError, 'unbound or malformed loop'):
            check(ROOT, self.llbc, inventory)

    def test_loop_inventory_requires_actual_generated_definition(self):
        inventory = copy.deepcopy(self.inventory)
        next(function for function in inventory['functions']
             if 'loop' in function)['lean_name'] += '.missing'
        with self.assertRaisesRegex(ValueError, 'no generated definition'):
            check(ROOT, self.llbc, inventory)

    def test_loop_positive_identity_and_position_cannot_be_invented(self):
        for field, value in (('id', 99), ('pos', [99])):
            with self.subTest(field=field):
                inventory = copy.deepcopy(self.inventory)
                for function in inventory['functions']:
                    if function['rust_name'] == FUNCTION and 'loop' in function:
                        function['loop'][field] = value
                with self.assertRaisesRegex(ValueError, 'loop identity or position'):
                    check(ROOT, self.llbc, inventory)

    def test_duplicate_function_parent_and_lean_alias_are_rejected(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['functions'].append(copy.deepcopy(inventory['functions'][-1]))
        with self.assertRaisesRegex(ValueError, 'duplicate parent function'):
            check(ROOT, self.llbc, inventory)
        inventory = copy.deepcopy(self.inventory)
        loop = next(function for function in inventory['functions'] if 'loop' in function)
        inventory['functions'].append(copy.deepcopy(loop))
        with self.assertRaisesRegex(ValueError, 'duplicate Lean function'):
            check(ROOT, self.llbc, inventory)

    def test_owned_order_removal_is_rejected(self):
        llbc = copy.deepcopy(self.llbc)
        llbc['translated']['ordered_decls'] = []
        with self.assertRaisesRegex(ValueError, 'body omitted from order'):
            check(ROOT, llbc, self.inventory)

    def test_duplicate_llbc_declaration_id_is_rejected(self):
        llbc = copy.deepcopy(self.llbc)
        declaration = next(declaration for declaration in llbc['translated']['fun_decls']
                           if declaration is not None)
        llbc['translated']['fun_decls'].append(copy.deepcopy(declaration))
        with self.assertRaisesRegex(ValueError, 'duplicate LLBC declaration ID'):
            check(ROOT, llbc, self.inventory)

    def test_unknown_external_and_removed_model_are_rejected(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['functions'][0]['rust_name'] = 'foreign::unreviewed'
        with self.assertRaisesRegex(ValueError, 'unreviewed external function surface'):
            check(ROOT, self.llbc, inventory)
        with patch('tools.check_network_extraction.has_definition',
                   side_effect=lambda _root, file, _name, *_args:
                   file != 'SandproofNetwork/FunsExternal.lean'):
            with self.assertRaisesRegex(ValueError, 'external model definition missing'):
                check(ROOT, self.llbc, self.inventory)

    def test_external_type_is_exactly_bound_and_defined(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['types'][0]['rust_name'] = 'foreign::unreviewed'
        with self.assertRaisesRegex(ValueError, 'unreviewed external type surface'):
            check(ROOT, self.llbc, inventory)
        llbc = copy.deepcopy(self.llbc)
        def_id = self.inventory['types'][0]['def_id']
        next(declaration for declaration in llbc['translated']['type_decls']
             if declaration and declaration['def_id'] == def_id)['kind'] = {'Struct': []}
        with self.assertRaisesRegex(ValueError, 'external type model missing or incorrectly bound'):
            check(ROOT, llbc, self.inventory)

    def test_owned_type_cannot_be_omitted(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['types'] = [entry for entry in inventory['types']
                              if entry['rust_name'] != 'sandbox_controller_core::model::Ipv4Cidr']
        with self.assertRaisesRegex(ValueError, 'owned LLBC type omitted'):
            check(ROOT, self.llbc, inventory)

    def test_owned_global_cannot_be_omitted(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['globals'] = []
        with self.assertRaisesRegex(ValueError, 'owned global omitted'):
            check(ROOT, self.llbc, inventory)

    def test_platform_global_initializer_must_have_real_body(self):
        llbc = copy.deepcopy(self.llbc)
        from tools.check_network_extraction import called_functions, owned_declaration
        global_decl = next(declaration for declaration in llbc['translated']['global_decls']
                           if declaration is not None and owned_declaration(declaration))
        initializer_id, = called_functions(global_decl['value'])
        initializer = next(declaration for declaration in llbc['translated']['fun_decls']
                           if declaration and declaration['def_id'] == initializer_id)
        initializer['body'] = 'Opaque'
        with self.assertRaisesRegex(ValueError, 'global initializer body missing'):
            check(ROOT, llbc, self.inventory)

    def test_standard_source_digest_is_checked_not_merely_recorded(self):
        with tempfile.TemporaryDirectory() as directory:
            sysroot = Path(directory)
            library = sysroot / 'lib/rustlib/src/rust/library'
            expected = {}
            for path in STANDARD_SOURCES:
                source = library / path
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_bytes(b'fixed reviewed source')
                expected[path] = hashlib.sha256(source.read_bytes()).hexdigest()
            check_standard_sources(sysroot, expected)
            parser = library / 'core/src/net/parser.rs'
            parser.write_bytes(b'changed parser')
            with self.assertRaisesRegex(ValueError, 'standard-library source changed'):
                check_standard_sources(sysroot, expected)

    def test_standard_source_inventory_cannot_be_removed(self):
        with self.assertRaisesRegex(ValueError, 'source inventory differs'):
            check_standard_sources(Path('/unused'), {})

    def test_compiler_glue_cannot_hide_calls_or_state_mutation(self):
        from tools.check_network_extraction import drop_glue_name
        llbc = copy.deepcopy(self.llbc)
        glue = next(d for d in llbc['translated']['fun_decls']
                    if d and drop_glue_name(d) == 'sandbox_controller_core::model::Endpoint')
        statements = glue['body']['Structured']['body']['statements']
        statements[0]['kind'] = {'Call': {}}
        with self.assertRaisesRegex(ValueError, 'custom operation inside compiler drop glue'):
            check(ROOT, llbc, self.inventory)

    def test_u64_newtype_alias_must_remain_a_single_u64(self):
        llbc = copy.deepcopy(self.llbc)
        entry = next(t for t in self.inventory['types']
                     if t['rust_name'] == 'sandbox_controller_core::ids::TenantId')
        declaration = next(d for d in llbc['translated']['type_decls']
                           if d and d['def_id'] == entry['def_id'])
        declaration['kind']['Struct'].append(copy.deepcopy(declaration['kind']['Struct'][0]))
        with self.assertRaisesRegex(ValueError, 'u64 newtype alias shape differs'):
            check(ROOT, llbc, self.inventory)

    def test_compiler_glue_cannot_write_through_argument(self):
        llbc = copy.deepcopy(self.llbc)
        glue = next(d for d in llbc['translated']['fun_decls']
                    if d and drop_glue_name(d) == 'sandbox_controller_core::model::Endpoint')
        statements = glue['body']['Structured']['body']['statements']
        assignment = next(s for s in statements if isinstance(s['kind'], dict)
                          and 'Assign' in s['kind'])
        assignment['kind']['Assign'][0]['kind'] = {'Local': 1}
        with self.assertRaisesRegex(ValueError, 'non-unit assignment inside compiler drop glue'):
            check(ROOT, llbc, self.inventory)


if __name__ == '__main__':
    unittest.main()
