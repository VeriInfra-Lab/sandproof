"""Fail-closed checks for the actual JSON counter extraction admission gate."""
import copy
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from tools import check_json_counter_extraction as audit

ROOT = Path(__file__).resolve().parents[2]


class JsonCounterExtractionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        for relative in (audit.ENVIRONMENT, audit.SOURCE, audit.ENTRY,
                         'verification/foundations-environment.json',
                         'scripts/regenerate-foundations-lean.sh',
                         'verification/extract-foundations/src/lib.rs',
                         'verification/extract-foundations/src/numeric.rs',
                         'verification/extract-foundations/src/receipts.rs',
                         'verification/extract-foundations/src/json_ids.rs',
                         audit.DEST + '/SandproofFoundations/Funs.lean'):
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)
        self.llbc = json.loads((ROOT / audit.LLBC).read_text())
        self.inventory = json.loads((ROOT / audit.DEST / 'translation.json').read_text())

    def check(self):
        return audit.check(self.root, self.llbc, self.inventory)

    def test_real_snapshot_has_ten_transparent_functions(self):
        self.assertEqual(self.check(), 10)

    def test_cannot_hide_owned_deserialize(self):
        function = next(f for f in self.inventory['functions']
                        if f['rust_name'].startswith('sandproof_protocol::')
                        and f['rust_name'].endswith('::deserialize'))
        function['is_opaque'] = True
        with self.assertRaisesRegex(ValueError, 'opaque surface|owned body missing'):
            self.check()

    def test_cannot_omit_call_through_root(self):
        self.inventory['functions'] = [f for f in self.inventory['functions']
                                       if f['rust_name'] != audit.CRATE + '::json::counter_decode']
        with self.assertRaisesRegex(ValueError, 'missing function|root was not generated'):
            self.check()

    def test_cannot_keep_inventory_without_body(self):
        function = next(f for f in self.inventory['functions']
                        if f['rust_name'] == audit.CRATE + '::json::counter_decode')
        declaration = next(f for f in self.llbc['translated']['fun_decls']
                           if f is not None and f['def_id'] == function['def_id'])
        declaration['body'] = None
        with self.assertRaisesRegex(ValueError, 'body missing'):
            self.check()

    def test_cannot_rebind_source(self):
        function = next(f for f in self.inventory['functions']
                        if f['rust_name'] in audit.FOCUSED
                        and f['rust_name'].startswith('sandproof_protocol::'))
        function['source']['file'] = 'verification/unrelated.rs'
        with self.assertRaisesRegex(ValueError, 'wrong production'):
            self.check()

    def test_cannot_skip_borrow_checking(self):
        self.llbc['translated']['options']['skip_borrowck'] = True
        with self.assertRaisesRegex(ValueError, 'extraction options'):
            self.check()

    def test_cannot_duplicate_function_identity(self):
        function = next(f for f in self.inventory['functions']
                        if f['rust_name'] == audit.CRATE + '::json::counter_decode')
        self.inventory['functions'].append(copy.deepcopy(function))
        with self.assertRaisesRegex(ValueError, 'duplicate function'):
            self.check()

    def test_cannot_drop_generated_definition(self):
        path = self.root / audit.DEST / 'SandproofFoundations/Funs.lean'
        path.write_text(path.read_text().replace('def json.counter_decode', 'def json.hidden_counter_decode'))
        with self.assertRaisesRegex(ValueError, 'generated definition'):
            self.check()

    def test_only_short_name_map_order_is_irrelevant(self):
        self.llbc['translated']['short_names'].reverse()
        self.assertEqual(self.check(), 10)

    def test_other_ast_array_order_remains_frozen(self):
        self.llbc['translated']['fun_decls'].reverse()
        with self.assertRaisesRegex(ValueError, 'full LLBC changed'):
            self.check()

    def test_cannot_change_cleanup_destructor(self):
        function = next(f for f in self.inventory['functions']
                        if f['rust_name'] == audit.CRATE + '::json::counter_decode')
        declaration = next(f for f in self.llbc['translated']['fun_decls']
                           if f is not None and f['def_id'] == function['def_id'])
        drop = next(n['Drop'] for n in audit.nodes(declaration['body']) if 'Drop' in n)
        drop['fn_ptr']['kind'] = {'Fun': 0}
        with self.assertRaisesRegex(ValueError, 'cleanup location or type'):
            self.check()


if __name__ == '__main__':
    unittest.main()
