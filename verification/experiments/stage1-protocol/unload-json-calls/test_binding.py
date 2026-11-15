"""Admission-checker regressions on complete LLBC, not Rust/Lean counterexamples."""
import copy
import json
import os
from pathlib import Path
import unittest

from source_binding import ANCHOR, HERE, OPTIONS, SNAPSHOT, Original, check, inspect


def visit(value):
    yield value
    if isinstance(value, dict):
        for child in value.values():
            yield from visit(child)
    elif isinstance(value, list):
        for child in value:
            yield from visit(child)


class ActualJsonCalls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw = json.loads(Path(os.environ['SANDPROOF_UNLOAD_JSON_LLBC']).read_text())
        cls.profile = json.loads((HERE / 'profile.json').read_text())
        cls.good = check(cls.raw, cls.profile)

    def mutated(self, name='encode_source_anchor'):
        raw = copy.deepcopy(self.raw)
        identifier = self.good['roots'][SNAPSHOT + name]['function_id']
        function = next(row for row in raw['translated']['fun_decls']
                        if row and row['def_id'] == identifier)
        return raw, function

    def refused(self, raw):
        with self.assertRaises((ValueError, KeyError)):
            inspect(raw)

    def test_complete_original_calls(self):
        self.assertEqual(len(self.good['roots']), 3)
        self.assertEqual(self.good['derives'][ANCHOR]['field_calls'], 12)
        self.assertEqual(len(self.good['derives'][OPTIONS]['original_fields']), 4)
        self.assertTrue(self.good['derives'][ANCHOR]['ordered_error_paths_checked'])
        self.assertFalse(self.good['actual_wrapper_lean_proved'])

    def test_wrapper_result_not_discarded(self):
        raw, function = self.mutated()
        function['body']['Structured']['body']['statements'][3]['kind']['Call']['call']['dest']['kind'] = {'Local': 2}
        self.refused(raw)

    def test_extra_effect_or_guard_is_not_admitted(self):
        raw, function = self.mutated()
        statements = function['body']['Structured']['body']['statements']
        statements.insert(4, copy.deepcopy(statements[3]))
        self.refused(raw)

    def test_wrapper_must_forward_complete_original_argument(self):
        raw, function = self.mutated()
        function['body']['Structured']['body']['statements'][3]['kind']['Call']['call']['args'][0]['Move']['kind'] = {'Local': 1}
        self.refused(raw)

    def test_unwind_path_must_propagate_original(self):
        raw, function = self.mutated()
        function['body']['Structured']['body']['statements'][3]['kind']['Call']['on_unwind']['statements'][-1]['kind'] = 'Return'
        self.refused(raw)

    def test_dictionary_not_from_other_actual_type(self):
        raw, function = self.mutated()
        identifier = self.good['roots'][SNAPSHOT + 'encode_source_options']['function_id']
        other = next(row for row in raw['translated']['fun_decls'] if row and row['def_id'] == identifier)
        dest = function['body']['Structured']['body']['statements'][3]['kind']['Call']['call']
        source = other['body']['Structured']['body']['statements'][3]['kind']['Call']['call']
        dest['func']['Regular']['generics']['trait_refs'] = copy.deepcopy(source['func']['Regular']['generics']['trait_refs'])
        self.refused(raw)

    def test_same_typed_request_cannot_replace_execution_origin(self):
        raw = copy.deepcopy(self.raw)
        identifier = self.good['derives'][ANCHOR]['function_id']
        function = next(row for row in raw['translated']['fun_decls'] if row and row['def_id'] == identifier)
        projections = [row for row in visit(function) if isinstance(row, dict) and row.get('Field') == [None, 1]]
        self.assertEqual(len(projections), 1)
        projections[0]['Field'] = [None, 0]
        self.refused(raw)

    def test_complete_field_declaration_cannot_be_truncated(self):
        raw = copy.deepcopy(self.raw)
        original = Original(raw)
        declaration = next(row for row in original.type_decls.values()
                           if row['item_meta']['name'][-1] == {'Ident': ['SourceAnchor', 0]})
        declaration['kind']['Struct'].pop()
        self.refused(raw)

    def test_changed_complete_body_is_not_only_a_matching_field_summary(self):
        raw, function = self.mutated()
        function['body']['Structured']['comments'].append('changed original metadata')
        with self.assertRaisesRegex(ValueError, 'complete reviewed'):
            check(raw, self.profile)

    def test_incomplete_extraction_is_not_a_counterexample(self):
        raw = copy.deepcopy(self.raw)
        raw['has_errors'] = True
        self.refused(raw)

    def test_target_and_selectors_must_match_actual_capture(self):
        raw = copy.deepcopy(self.raw)
        raw['translated']['target_information'][0]['key'] = 'aarch64-unknown-linux-gnu'
        self.refused(raw)
        raw = copy.deepcopy(self.raw)
        raw['translated']['options']['include'].pop()
        with self.assertRaisesRegex(ValueError, 'selectors'):
            check(raw, self.profile)


if __name__ == '__main__':
    unittest.main()
