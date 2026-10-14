"""Regression: an extractor's successful exit can still omit selected bodies."""
import copy
import unittest

from tools.check_guest_extraction import check_inventory


ROOT = 'crate::command::CommandStream::outcome'
NAME = 'sandproof_guest_core::command::{sandproof_guest_core::command::CommandStream}::outcome'


class GuestExtractionInventoryTests(unittest.TestCase):
    def setUp(self):
        self.inventory = {'crate': 'sandproof_guest_core', 'functions': [
            {'rust_name': NAME, 'is_local': True, 'is_opaque': False},
        ]}

    def test_empty_translation_is_not_success(self):
        self.inventory['functions'] = []
        with self.assertRaisesRegex(ValueError, 'missing or opaque'):
            check_inventory([ROOT], self.inventory)

    def test_wrong_owner_duplicate_or_opaque_body_is_rejected(self):
        variants = [
            {'rust_name': NAME.replace('CommandStream', 'WrongType')},
            {'is_local': False},
            {'is_opaque': True},
        ]
        for change in variants:
            with self.subTest(change=change):
                inventory = copy.deepcopy(self.inventory)
                inventory['functions'][0].update(change)
                with self.assertRaises(ValueError):
                    check_inventory([ROOT], inventory)
        self.inventory['functions'] *= 2
        with self.assertRaises(ValueError):
            check_inventory([ROOT], self.inventory)

    def test_opaque_dependency_cannot_hide_local_logic(self):
        self.inventory['functions'].append({
            'rust_name': 'some_dependency', 'is_local': True, 'is_opaque': True,
        })
        with self.assertRaisesRegex(ValueError, 'opaque local dependency'):
            check_inventory([ROOT], self.inventory)
        self.inventory['functions'][-1]['is_local'] = False
        self.assertEqual(check_inventory([ROOT], self.inventory), ['some_dependency'])

    def test_roots_and_crate_must_identify_actual_work(self):
        for roots in ([], [ROOT, ROOT], ['crate::elsewhere::Type::outcome']):
            with self.subTest(roots=roots), self.assertRaises(ValueError):
                check_inventory(roots, self.inventory)
        self.assertEqual(check_inventory([ROOT], self.inventory), [])
        self.inventory['crate'] = 'another_crate'
        with self.assertRaisesRegex(ValueError, 'wrong translated crate'):
            check_inventory([ROOT], self.inventory)


if __name__ == '__main__':
    unittest.main()
