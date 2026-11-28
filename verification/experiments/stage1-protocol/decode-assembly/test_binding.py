"""Admission regressions on complete real LLBC, separate from Rust negatives."""
import copy
import json
import os
from pathlib import Path
import unittest
from binding import Original, same
from check import check


def dictionaries(node):
    if isinstance(node, dict):
        yield node
        for value in node.values():
            yield from dictionaries(value)
    elif isinstance(node, list):
        for value in node:
            yield from dictionaries(value)


class DecoderAssembly(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw = json.loads(Path(os.environ['SANDPROOF_DECODE_ASSEMBLY_LLBC']).read_text())
        root = Path(__file__).resolve().parents[4]
        cls.schema = json.loads((root / 'verification/experiments/stage1-protocol/envelope-schema/inventory.json').read_text())
        cls.normal = check(cls.raw, cls.schema)

    def function(self, raw, mode):
        fid = self.normal['checked']['RecordedLifecycleIntent::None::' + mode]['function_id']
        return Original(raw).functions[fid]

    def swap_returned_booleans(self, raw, mode):
        original = Original(raw)
        target, = [d['def_id'] for d in original.type_decls.values()
                   if d['item_meta']['name'][-1] == {'Ident': ['RecordedLifecycleIntent', 0]}]
        count = 0
        for node in dictionaries(self.function(raw, mode)['body']):
            if 'Aggregate' not in node:
                continue
            kind, fields = node['Aggregate']
            if 'Adt' in kind and kind['Adt'][0]['id'] == target:
                self.assertEqual(len(fields), 4)
                fields[2], fields[3] = fields[3], fields[2]
                count += 1
        self.assertGreater(count, 0)

    def test_complete_original_struct_and_operation_assembly(self):
        self.assertEqual(len(self.normal['checked']), 30)
        self.assertFalse(self.normal['library_refinement_proved'])
        self.assertTrue(self.normal['byte_key_matching_checked'])
        self.assertTrue(self.normal['expected_session_dispatch_checked'])
        self.assertEqual(len(self.normal['leaf_forwarding']['original_bodies']), 14)

    def test_boolean_and_integer_branch_constants_are_distinct(self):
        self.assertFalse(same({'constant': True}, {'constant': 1}))
        self.assertFalse(same({'constant': False}, {'constant': 0}))

    def test_byte_field_name_requires_exact_length(self):
        raw = copy.deepcopy(self.raw)
        original = Original(raw)
        prefix = original.functions[self.normal['checked']['RecordedLifecycleIntent::None::visit_map'][
            'function_id']]['item_meta']['name'][:-2]
        function, = [f for f in original.functions.values() if f['item_meta']['name'][:-2] == prefix and
                     f['item_meta']['name'][-1] == {'Ident': ['visit_bytes', 0]}]
        changed = False
        for node in dictionaries(function['body']):
            if set(node) == {'Const'} and original.constant(node['Const']) == {'constant': 6}:
                value, ty = copy.deepcopy(original.resolve(node['Const'], original.constants))
                self.assertEqual(value['Integer']['Unsigned'], ['Usize', '6'])
                value['Integer']['Unsigned'][1] = '7'
                if set(node['Const']) == {'Value'}:
                    # Preserve the original interning definition for every
                    # remaining use; a dangling reference is not a rejection.
                    index = node['Const']['Value'][0]
                    other = next(n for n in dictionaries(raw) if set(n) == {'Const'} and
                                 n['Const'] == {'Deduplicated': index})
                    other['Const'] = copy.deepcopy(node['Const'])
                node['Const'] = {'Value': [max(original.constants) + 1, [value, ty]]}
                changed = True
                break
        self.assertTrue(changed)
        # Length 7 collides with the later "current" selector. The exact
        # selector-count check rejects that lost branch before byte matching.
        with self.assertRaisesRegex(ValueError, 'missing/extra byte field selector'):
            check(raw, self.schema)

    def test_operation_tag_cannot_select_another_branch(self):
        raw = copy.deepcopy(self.raw)
        original = Original(raw)
        prefix = original.functions[self.normal['operation_dispatch']['root']]['item_meta']['name']
        candidates = [r for r in self.normal['functions'].values() if r['method'] == 'visit_str' and
                      isinstance(r['success_type'], dict) and r['success_type']['name'] ==
                      prefix + [{'Ident': ['__Field', 0]}]]
        selector, = candidates
        enum = selector['success_type']['original_type']
        function = original.functions[selector['function_id']]
        count = 0
        for node in dictionaries(function['body']):
            if 'Aggregate' in node:
                kind, _ = node['Aggregate']
                if 'Adt' in kind and kind['Adt'][0]['id'] == enum and kind['Adt'][1] in (0, 1):
                    kind['Adt'][1] = 1 - kind['Adt'][1]
                    count += 1
        self.assertEqual(count, 2)
        with self.assertRaisesRegex(ValueError, 'original input field name routed to another field'):
            check(raw, self.schema)

    def test_version_constructor_cannot_replace_value_with_zero(self):
        raw = copy.deepcopy(self.raw)
        original = Original(raw)
        seed_id = self.normal['expected_session']['seed']
        mapped, = [r for r in self.normal['functions'][seed_id]['rows']
                   if r.get('callee') == 'core::result::<impl>::map']
        ctor = original.functions[mapped['arguments'][1]['original_constant']['FnDef']['kind']['Fun']]
        changed = False
        for node in dictionaries(ctor['body']):
            if 'Aggregate' in node:
                _, fields = node['Aggregate']
                self.assertEqual(len(fields), 1)
                ty = next(iter(fields[0].values()))['ty']
                fields[0] = {'Const': {'Value': [max(original.constants) + 1,
                    [{'Integer': {'Unsigned': ['U64', '0']}}, ty]]}}
                changed = True
                break
        self.assertTrue(changed)
        with self.assertRaisesRegex(ValueError, 'Version constructor changes its original value'):
            check(raw, self.schema)

    def test_same_type_seq_field_swap(self):
        raw = copy.deepcopy(self.raw)
        self.swap_returned_booleans(raw, 'visit_seq')
        with self.assertRaisesRegex(ValueError, 'sequence missing value has wrong rejection index'):
            check(raw, self.schema)

    def test_same_type_map_field_swap(self):
        raw = copy.deepcopy(self.raw)
        self.swap_returned_booleans(raw, 'visit_map')
        with self.assertRaisesRegex(ValueError, 'original input field name routed to another field'):
            check(raw, self.schema)

    def test_swallowed_original_error(self):
        raw = copy.deepcopy(self.raw)
        original = Original(raw)
        changed = False
        for node in dictionaries(self.function(raw, 'visit_seq')['body']):
            if 'Call' in node:
                call = node['Call']['call']
                if original.callee(call['func']) == 'core::result::<impl>::from_residual':
                    call['dest']['kind']['Local'] = 1
                    changed = True
                    break
        self.assertTrue(changed)
        with self.assertRaisesRegex(ValueError, 'original field error not returned'):
            check(raw, self.schema)

    def test_missing_original_body(self):
        raw = copy.deepcopy(self.raw)
        self.function(raw, 'visit_map')['body'] = None
        self.function(raw, 'visit_map')['item_meta']['opacity'] = 'Opaque'
        with self.assertRaisesRegex(ValueError, 'original visitor body missing'):
            check(raw, self.schema)

    def test_incomplete_original_capture(self):
        raw = copy.deepcopy(self.raw)
        raw['has_errors'] = True
        with self.assertRaisesRegex(ValueError, 'incomplete Charon input'):
            check(raw, self.schema)


class NativeReceipt(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.receipt = json.loads((Path(os.environ['SANDPROOF_DECODE_ASSEMBLY_NATIVE']) / 'result.json').read_text())

    def test_complete_original_receipt(self):
        from negative import validate_native_commands
        validate_native_commands(self.receipt)

    def test_pruned_commands_are_rejected(self):
        from negative import validate_native_commands
        receipt = copy.deepcopy(self.receipt)
        receipt['commands'].pop()
        with self.assertRaisesRegex(ValueError, 'all six'):
            validate_native_commands(receipt)

    def test_duplicate_commands_are_rejected(self):
        from negative import validate_native_commands
        receipt = copy.deepcopy(self.receipt)
        receipt['commands'][2] = copy.deepcopy(receipt['commands'][0])
        with self.assertRaisesRegex(ValueError, 'all six'):
            validate_native_commands(receipt)


if __name__ == '__main__':
    unittest.main()
