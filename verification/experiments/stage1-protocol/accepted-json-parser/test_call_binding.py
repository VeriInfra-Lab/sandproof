"""Actual-probe source admission regressions, not compiled Rust counterexamples."""
import copy
import json
import os
from pathlib import Path
import unittest

import source_admission as source


def nodes(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from nodes(child)
    elif isinstance(value, list):
        for child in value:
            yield from nodes(child)


class CurrentCallback(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw = json.loads(Path(os.environ['SANDPROOF_CURRENT_ACCEPTED_LLBC']).read_text())
        cls.schema = json.loads((source.PARENT / 'envelope-schema/inventory.json').read_text())
        cls.normal = source.report(cls.raw, cls.schema)

    def callback(self, raw):
        original = source.binding.Original(raw)
        root = original.functions[self.normal['current_capture_root']['function_id']]
        report = original.links(root)
        call, = [row for row in report['rows'] if 'callee' in row]
        reference, = call['original_call']['func']['Regular']['generics']['trait_refs']
        implementation, _ = source.calls.dictionary(original, reference, 'core::ops::function::FnOnce')
        return original, source.calls.method(original, implementation['id'], 'call_once')

    def test_current_root_reuses_all_canonical_assembly_checks(self):
        self.assertEqual(len(self.normal['checked']), 28)
        self.assertTrue(self.normal['byte_key_matching_checked'])
        self.assertTrue(self.normal['expected_session']['version_uses_plain_u64'])
        self.assertTrue(self.normal['parser_call_binding']['complete_slice_and_context_source_checked'])
        self.assertFalse(self.normal['new_Lean_proof'])

    def test_old_wrapper_identity_cannot_be_inferred_for_current_root(self):
        raw = copy.deepcopy(self.raw)
        raw['translated']['options']['start_from'] = ['crate::accepted_decode']
        with self.assertRaisesRegex(ValueError, 'unregistered current decoder root'):
            source.report(raw, self.schema)

    def test_probe_cannot_swap_input_and_context(self):
        raw = copy.deepcopy(self.raw)
        original = source.binding.Original(raw)
        root = original.functions[self.normal['current_capture_root']['function_id']]
        calls = [node['Call']['call'] for node in nodes(root['body']) if 'Call' in node]
        self.assertEqual(len(calls), 1)
        calls[0]['args'][0], calls[0]['args'][1] = calls[0]['args'][1], calls[0]['args'][0]
        with self.assertRaisesRegex(ValueError, 'probe changes the complete input slice or context'):
            source.report(raw, self.schema)

    def test_callback_cannot_call_parser_twice(self):
        raw = copy.deepcopy(self.raw)
        _, callback = self.callback(raw)
        statements = callback['body']['Structured']['body']['statements']
        call, = [node for node in statements if isinstance(node['kind'], dict) and 'Call' in node['kind']]
        duplicate = copy.deepcopy(call)
        duplicate['id'] = 1 + max(node['id'] for node in nodes(raw) if isinstance(node.get('id'), int))
        statements.insert(statements.index(call) + 1, duplicate)
        with self.assertRaisesRegex(ValueError, 'parser callback has extra calls'):
            source.report(raw, self.schema)

    def test_callback_cannot_substitute_decoded_type(self):
        raw = copy.deepcopy(self.raw)
        original, callback = self.callback(raw)
        call, = [node['Call']['call'] for node in nodes(callback['body']) if 'Call' in node]
        accepted = next(reference for reference in original.types.values()
                        if isinstance(reference, dict) and 'Adt' in reference and
                        original.declaration_name(reference['Adt']['id']) ==
                        'sandproof_protocol::lifecycle::request::LifecycleRequest')
        index = next(index for index, reference in original.types.items() if reference == accepted)
        call['func']['Regular']['generics']['types'] = [{'Deduplicated': index}]
        with self.assertRaisesRegex(ValueError, 'parser decodes a different protocol value'):
            source.report(raw, self.schema)


if __name__ == '__main__':
    unittest.main()
