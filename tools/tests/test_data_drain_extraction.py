import copy
import json
from pathlib import Path
import unittest

from tools.check_data_drain_extraction import check, METHOD, CRATE, LLBC, DEST, nodes

ROOT = Path(__file__).resolve().parents[2]


class DataDrainExtractionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.llbc = json.loads((ROOT / LLBC).read_text())
        cls.inventory = json.loads((ROOT / DEST / 'translation.json').read_text())

    def test_original_read_surface(self):
        self.assertEqual(check(ROOT, self.llbc, self.inventory), 4)

    def test_every_owned_root_and_provider_is_required(self):
        for item in self.inventory['functions']:
            inventory = copy.deepcopy(self.inventory)
            inventory['functions'] = [f for f in inventory['functions']
                                      if f['rust_name'] != item['rust_name']]
            with self.subTest(name=item['rust_name']), self.assertRaisesRegex(ValueError, 'surface'):
                check(ROOT, self.llbc, inventory)

    def test_duplicate_inventory_is_rejected(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['functions'].append(inventory['functions'][0])
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            check(ROOT, self.llbc, inventory)

    def test_owned_body_cannot_be_opaque(self):
        inventory = copy.deepcopy(self.inventory)
        next(f for f in inventory['functions'] if f['rust_name'] == METHOD + 'drained')['is_opaque'] = True
        with self.assertRaisesRegex(ValueError, 'opaque'):
            check(ROOT, self.llbc, inventory)

    def test_missing_structured_body_is_rejected(self):
        llbc = copy.deepcopy(self.llbc)
        target = next(f['def_id'] for f in self.inventory['functions']
                      if f['rust_name'] == CRATE + '::read_drained')
        next(f for f in llbc['translated']['fun_decls'] if f and f['def_id'] == target)['body'] = 'Opaque'
        with self.assertRaisesRegex(ValueError, 'body missing'):
            check(ROOT, llbc, self.inventory)

    def test_wrong_source_and_generated_file_are_rejected(self):
        for field, value, message in (('source', {'file': 'copied-business.rs'}, 'original source'),
                                       ('lean_file', 'unreviewed.lean', 'generated file')):
            inventory = copy.deepcopy(self.inventory)
            next(f for f in inventory['functions'] if f['rust_name'] == METHOD + 'drained')[field] = value
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, message):
                check(ROOT, self.llbc, inventory)

    def test_unordered_owned_function_is_rejected(self):
        llbc = copy.deepcopy(self.llbc)
        llbc['translated']['ordered_decls'] = []
        with self.assertRaisesRegex(ValueError, 'declaration order'):
            check(ROOT, llbc, self.inventory)

    def test_errors_target_and_options_fail_closed(self):
        llbc = copy.deepcopy(self.llbc)
        llbc['has_errors'] = True
        with self.assertRaisesRegex(ValueError, 'Charon reported errors'):
            check(ROOT, llbc, self.inventory)
        llbc = copy.deepcopy(self.llbc)
        llbc['translated']['target_information'][0]['key'] = 'aarch64-apple-darwin'
        with self.assertRaisesRegex(ValueError, 'target'):
            check(ROOT, llbc, self.inventory)
        llbc = copy.deepcopy(self.llbc)
        llbc['translated']['options']['skip_borrowck'] = True
        with self.assertRaisesRegex(ValueError, 'options'):
            check(ROOT, llbc, self.inventory)

    def test_external_type_and_global_expansion_are_rejected(self):
        inventory = copy.deepcopy(self.inventory)
        inventory['types'] = inventory['types'][:-1]
        with self.assertRaisesRegex(ValueError, 'type surface'):
            check(ROOT, self.llbc, inventory)
        inventory = copy.deepcopy(self.inventory)
        inventory['globals'] = [{'rust_name': 'new_global'}]
        with self.assertRaisesRegex(ValueError, 'global or trait'):
            check(ROOT, self.llbc, inventory)

    def test_non_acquire_load_is_rejected(self):
        llbc = copy.deepcopy(self.llbc)
        target = next(f['def_id'] for f in self.inventory['functions']
                      if f['rust_name'] == METHOD + 'drained')
        body = next(f for f in llbc['translated']['fun_decls'] if f and f['def_id'] == target)['body']
        aggregate = next(n['Aggregate'] for n in nodes(body) if 'Aggregate' in n)
        aggregate[0]['Adt'][1] = 0
        with self.assertRaisesRegex(ValueError, 'Acquire'):
            check(ROOT, llbc, self.inventory)

    def test_unreviewed_drop_or_error_is_rejected(self):
        for operation in ('Drop', 'Error', 'Loop'):
            llbc = copy.deepcopy(self.llbc)
            target = next(f['def_id'] for f in self.inventory['functions']
                          if f['rust_name'] == METHOD + 'routes_dropped')
            body = next(f for f in llbc['translated']['fun_decls'] if f and f['def_id'] == target)['body']
            body['Structured']['unexpected'] = {operation: {}}
            with self.subTest(operation=operation), self.assertRaisesRegex(ValueError, 'error/drop/loop'):
                check(ROOT, llbc, self.inventory)


if __name__ == '__main__':
    unittest.main()
