import copy
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from tools.check_foundations_extraction import check, llbc_digest

ROOT = Path(__file__).resolve().parents[2]


class FoundationsExtractionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.llbc = json.loads((ROOT / 'verification/aeneas/llbc/sandproof_foundations.llbc').read_text())
        cls.inventory = json.loads((ROOT / 'verification/aeneas/foundations/translation.json').read_text())

    def test_real_checked_roots_are_present(self):
        self.assertEqual(check(ROOT, self.llbc, self.inventory), 13)

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


if __name__ == '__main__':
    unittest.main()
