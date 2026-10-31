import copy
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from tools.check_foundations_extraction import check, llbc_digest

from tools.tests.proof_input_fixture import reference_root

ROOT = reference_root()


class FoundationsExtractionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.llbc = json.loads((ROOT / 'verification/aeneas/llbc/sandproof_foundations.llbc').read_text())
        cls.inventory = json.loads((ROOT / 'verification/aeneas/foundations/translation.json').read_text())

    def test_real_checked_roots_are_present(self):
        # Preserve every old root; add the real lifecycle guard. Two explicit B4
        # byte call-throughs are admitted separately and are not Lean roots.
        self.assertEqual(check(ROOT, self.llbc, self.inventory), 75)

    def test_value_conversion_roots_cannot_disappear(self):
        for name in ('sandproof_foundations_extract::counter_into_u64',
                     'sandproof_foundations_extract::internal_error_text',
                     'sandproof_foundations_extract::resource_values'):
            with self.subTest(name=name):
                inventory = copy.deepcopy(self.inventory)
                inventory['functions'] = [f for f in inventory['functions'] if f['rust_name'] != name]
                with self.assertRaisesRegex(ValueError, 'root was not generated'):
                    check(ROOT, self.llbc, inventory)

    def test_actual_value_conversions_cannot_disappear_or_become_opaque(self):
        for name in (
            'sandproof_protocol::internal::{impl core::fmt::Display for sandproof_protocol::internal::InternalError}::fmt',
            'sandproof_protocol::internal::{impl core::convert::From<sandproof_protocol::internal::PositiveCounter> for u64}::from',
            'sandproof_protocol::internal::lifecycle::{sandproof_protocol::internal::lifecycle::Resources}::values',
        ):
            with self.subTest(name=name):
                inventory = copy.deepcopy(self.inventory)
                function = next(f for f in inventory['functions'] if f['rust_name'] == name)
                inventory['functions'].remove(function)
                with self.assertRaisesRegex(ValueError, 'root was not generated'):
                    check(ROOT, self.llbc, inventory)
                inventory = copy.deepcopy(self.inventory)
                next(f for f in inventory['functions'] if f['rust_name'] == name)['is_opaque'] = True
                with self.assertRaisesRegex(ValueError, 'owned body missing'):
                    check(ROOT, self.llbc, inventory)
                llbc = copy.deepcopy(self.llbc)
                next(f for f in llbc['translated']['fun_decls']
                     if f and f['def_id'] == function['def_id'])['body'] = 'Opaque'
                with self.assertRaisesRegex(ValueError, 'owned body missing'):
                    check(ROOT, llbc, self.inventory)

    def test_value_conversion_roots_cannot_silently_reduce_coverage(self):
        read_text = Path.read_text
        for function in ('counter_into_u64', 'resource_values', 'internal_error_text'):
            def without_root(path, *args, **kwargs):
                value = read_text(path, *args, **kwargs)
                if path == ROOT / 'scripts/regenerate-foundations-lean.sh':
                    return value.replace(f'  --start-from crate::{function} \\\n', '')
                if path == ROOT / 'verification/extract-foundations/src/lib.rs':
                    return value.replace(f'pub fn {function}', f'fn {function}')
                return value

            with patch.object(Path, 'read_text', without_root):
                with self.assertRaisesRegex(ValueError, 'roots and script disagree'):
                    check(ROOT, self.llbc, self.inventory)

    def test_destination_cannot_depend_on_the_checkout_or_another_output(self):
        for destination in ('/old-checkout/verification/aeneas/llbc/sandproof_foundations.llbc',
                            'verification/aeneas/llbc/other.llbc'):
            llbc = copy.deepcopy(self.llbc)
            llbc['translated']['options']['dest_file'] = destination
            with self.assertRaisesRegex(ValueError, 'project-relative path'):
                check(ROOT, llbc, self.inventory)

    def test_only_unordered_name_table_order_is_normalized(self):
        reordered = copy.deepcopy(self.llbc)
        reordered['translated']['short_names'].reverse()
        self.assertEqual(llbc_digest(self.llbc), llbc_digest(reordered))
        changed = copy.deepcopy(self.llbc)
        changed['translated']['short_names'][0]['value'] = ['changed-name']
        self.assertNotEqual(llbc_digest(self.llbc), llbc_digest(changed))
        changed = copy.deepcopy(self.llbc)
        function = next(f for f in changed['translated']['fun_decls'] if f and isinstance(f['body'], dict))
        function['body'] = 'Opaque'
        self.assertNotEqual(llbc_digest(self.llbc), llbc_digest(changed))
        duplicate = copy.deepcopy(self.llbc)
        duplicate['translated']['short_names'].append(duplicate['translated']['short_names'][0])
        with self.assertRaisesRegex(ValueError, 'duplicate short-name key'):
            llbc_digest(duplicate)

    def test_missing_root_or_empty_generation_cannot_pass(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['functions'] = [f for f in inventory['functions']
                                  if f['rust_name'] != 'sandproof_foundations_extract::counter_next']
        with self.assertRaisesRegex(ValueError, 'root was not generated'):
            check(ROOT, self.llbc, inventory)
        inventory = copy.deepcopy(self.inventory)
        next(f for f in inventory['functions'] if f['rust_name'].endswith('::counter_next'))['lean_file'] = 'missing.lean'
        with self.assertRaisesRegex(ValueError, 'unexpected generated file'):
            check(ROOT, self.llbc, inventory)

    def test_write_root_cannot_disappear_from_inventory(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['functions'] = [f for f in inventory['functions']
                                  if f['rust_name'] != 'sandproof_foundations_extract::store_put']
        with self.assertRaisesRegex(ValueError, 'root was not generated'):
            check(ROOT, self.llbc, inventory)

    def test_retain_root_and_callback_cannot_become_opaque_or_disappear(self):
        root_name = 'sandproof_foundations_extract::store_retain_above'
        inventory = copy.deepcopy(self.inventory)
        inventory['functions'] = [f for f in inventory['functions'] if f['rust_name'] != root_name]
        with self.assertRaisesRegex(ValueError, 'root was not generated'):
            check(ROOT, self.llbc, inventory)

        inventory = copy.deepcopy(self.inventory)
        callback = next(f for f in inventory['functions']
                        if f['rust_name'].startswith(root_name + '::')
                        and f['rust_name'].endswith('::call_mut'))
        callback['is_opaque'] = True
        with self.assertRaisesRegex(ValueError, 'owned body missing'):
            check(ROOT, self.llbc, inventory)

    def test_standard_library_sources_cannot_be_omitted_or_malformed(self):
        read_text = Path.read_text
        for role, relative, change in (
            ('production', 'core/src/option.rs', 'missing'),
            ('extraction', 'alloc/src/vec/mod.rs', 'malformed'),
            ('production', 'core/src/result.rs', 'missing'),
            ('extraction', 'core/src/result.rs', 'missing'),
        ):
            def changed_environment(path, *args, **kwargs):
                value = read_text(path, *args, **kwargs)
                if path == ROOT / 'verification/foundations-environment.json':
                    environment = json.loads(value)
                    if change == 'missing':
                        environment['library_sources'][role].pop(relative)
                    else:
                        environment['library_sources'][role][relative] = 'unchecked'
                    return json.dumps(environment)
                return value

            with patch.object(Path, 'read_text', changed_environment):
                with self.assertRaisesRegex(ValueError, 'source registry'):
                    check(ROOT, self.llbc, self.inventory)

    def test_simultaneous_write_root_removal_cannot_reduce_coverage(self):
        read_text = Path.read_text

        def without_write_root(path, *args, **kwargs):
            value = read_text(path, *args, **kwargs)
            if path == ROOT / 'scripts/regenerate-foundations-lean.sh':
                return value.replace('  --start-from crate::store_put \\\n', '')
            if path == ROOT / 'verification/extract-foundations/src/lib.rs':
                return value.replace('pub fn store_put(', 'fn store_put(')
            return value

        with patch.object(Path, 'read_text', without_write_root):
            with self.assertRaisesRegex(ValueError, 'roots and script disagree'):
                check(ROOT, self.llbc, self.inventory)

    def test_owned_opacity_or_missing_body_is_rejected(self):
        for name in (
            'sandproof_foundations_extract::counter_next',
            'sandproof_center_core::model::{sandproof_center_core::model::RetentionLane}::current',
            'sandbox_controller_core::store::{sandbox_controller_core::store::Store<K, V>}::get_copy',
            'sandbox_controller_core::store::{sandbox_controller_core::store::Store<K, V>}::put',
            'sandbox_controller_core::store::{sandbox_controller_core::store::Store<K, V>}::retain',
        ):
            with self.subTest(name=name):
                inventory = copy.deepcopy(self.inventory)
                function = next(f for f in inventory['functions']
                                if f['rust_name'] == name and 'loop' not in f)
                function['is_opaque'] = True
                with self.assertRaisesRegex(ValueError, 'owned body missing|malformed loop inventory'):
                    check(ROOT, self.llbc, inventory)
                llbc = copy.deepcopy(self.llbc)
                next(f for f in llbc['translated']['fun_decls']
                     if f and f['def_id'] == function['def_id'])['body'] = 'Opaque'
                with self.assertRaisesRegex(ValueError, 'owned body missing'):
                    check(ROOT, llbc, self.inventory)

    def test_loop_aliases_require_exact_parent_body_and_wrapper(self):
        inventory = copy.deepcopy(self.inventory)
        loops = [f for f in inventory['functions'] if 'loop' in f]
        self.assertEqual(len(loops), 4)
        inventory['functions'].remove(loops[0])
        with self.assertRaisesRegex(ValueError, 'wrapper or body missing'):
            check(ROOT, self.llbc, inventory)
        inventory = copy.deepcopy(self.inventory)
        next(f for f in inventory['functions'] if 'loop' in f)['parent_lean_name'] = 'wrong.parent'
        with self.assertRaisesRegex(ValueError, 'unbound or malformed loop'):
            check(ROOT, self.llbc, inventory)
        inventory = copy.deepcopy(self.inventory)
        inventory['functions'].append(copy.deepcopy(loops[0]))
        with self.assertRaisesRegex(ValueError, 'duplicate Lean function'):
            check(ROOT, self.llbc, inventory)
        inventory = copy.deepcopy(self.inventory)
        next(f for f in inventory['functions'] if 'loop' in f)['lean_name'] += '.missing'
        with self.assertRaisesRegex(ValueError, 'no generated definition'):
            check(ROOT, self.llbc, inventory)

    def test_unordered_body_and_new_external_are_rejected(self):
        llbc = copy.deepcopy(self.llbc)
        llbc['translated']['ordered_decls'] = []
        with self.assertRaisesRegex(ValueError, 'omitted from order'):
            check(ROOT, llbc, self.inventory)
        inventory = copy.deepcopy(self.inventory)
        inventory['functions'].append({'rust_name': 'foreign::unreviewed', 'is_opaque': True})
        with self.assertRaisesRegex(ValueError, 'unreviewed external surface'):
            check(ROOT, self.llbc, inventory)

    def test_extraction_errors_fail_closed(self):
        llbc = copy.deepcopy(self.llbc)
        llbc['has_errors'] = True
        with self.assertRaisesRegex(ValueError, 'Charon reported errors'):
            check(ROOT, llbc, self.inventory)
        llbc = copy.deepcopy(self.llbc)
        llbc['translated']['target_information'][0]['key'] = 'unreviewed-target'
        with self.assertRaisesRegex(ValueError, 'extraction target'):
            check(ROOT, llbc, self.inventory)

    def test_identity_constructor_and_recovery_parser_must_be_real(self):
        for name in (
            'sandproof_foundations_extract::tenant_from_str',
            'sandproof_foundations_extract::node_from_string',
            'sandproof_foundations_extract::node_into_string',
            'sandproof_protocol::valid_identifier',
            'sandproof_protocol::identifiers::parse_allocated_number',
            'sandproof_center_core::validation::number',
        ):
            with self.subTest(name=name):
                inventory = copy.deepcopy(self.inventory)
                next(f for f in inventory['functions'] if f['rust_name'] == name)['is_opaque'] = True
                with self.assertRaisesRegex(ValueError, 'owned body missing'):
                    check(ROOT, self.llbc, inventory)

    def test_new_constructor_and_direct_root_cannot_disappear(self):
        for name in ('sandproof_foundations_extract::deployment_from_string',
                     'sandproof_center_core::validation::number'):
            inventory = copy.deepcopy(self.inventory)
            inventory['functions'] = [f for f in inventory['functions'] if f['rust_name'] != name]
            with self.assertRaisesRegex(ValueError, 'root was not generated'):
                check(ROOT, self.llbc, inventory)

    def test_numeric_and_receipt_modules_cannot_silently_reduce_coverage(self):
        read_text = Path.read_text
        for module, function in (
            ('numeric', 'bytes_to_mebibytes'),
            ('receipts', 'receipt_validate'),
        ):
            def without_root(path, *args, **kwargs):
                value = read_text(path, *args, **kwargs)
                if path == ROOT / 'scripts/regenerate-foundations-lean.sh':
                    return value.replace(f'  --start-from crate::{module}::{function} \\\n', '')
                if path == ROOT / f'verification/extract-foundations/src/{module}.rs':
                    return value.replace(f'pub fn {function}', f'fn {function}')
                return value

            with patch.object(Path, 'read_text', without_root):
                with self.assertRaisesRegex(ValueError, 'roots and script disagree'):
                    check(ROOT, self.llbc, self.inventory)
            inventory = copy.deepcopy(self.inventory)
            name = f'sandproof_foundations_extract::{module}::{function}'
            inventory['functions'] = [f for f in inventory['functions'] if f['rust_name'] != name]
            with self.assertRaisesRegex(ValueError, 'root was not generated'):
                check(ROOT, self.llbc, inventory)

    def test_numeric_production_helpers_must_remain_transparent(self):
        for name in (
            'sandproof_protocol::numeric::seconds_to_milliseconds',
            'sandproof_protocol::numeric::bytes_to_mebibytes',
            'sandproof_protocol::numeric::available_resource',
        ):
            inventory = copy.deepcopy(self.inventory)
            next(f for f in inventory['functions'] if f['rust_name'] == name)['is_opaque'] = True
            with self.assertRaisesRegex(ValueError, 'owned body missing'):
                check(ROOT, self.llbc, inventory)

    def test_actual_receipt_guards_and_resource_comparison_must_remain_transparent(self):
        for name in (
            'sandproof_protocol::{sandproof_protocol::ActionIdentity}::validate_version',
            'sandproof_protocol::{sandproof_protocol::WorkReceipt<E>}::validate_for',
            'sandproof_protocol::{impl core::cmp::PartialEq<sandproof_protocol::ResourceRef> for sandproof_protocol::ResourceRef}::eq',
        ):
            inventory = copy.deepcopy(self.inventory)
            next(f for f in inventory['functions'] if f['rust_name'] == name)['is_opaque'] = True
            with self.assertRaisesRegex(ValueError, 'owned body missing'):
                check(ROOT, self.llbc, inventory)


if __name__ == '__main__':
    unittest.main()
