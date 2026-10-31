"""Admission regressions on complete actual LLBC, not Rust counterexamples."""
import copy
import json
import os
from pathlib import Path
import unittest

from check import check, AUTHORITY


def visit(value):
    yield value
    if isinstance(value, dict):
        for child in value.values():
            yield from visit(child)
    elif isinstance(value, list):
        for child in value:
            yield from visit(child)


class ActualBinding(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = Path(os.environ['SANDPROOF_SERDE_BINDING_LLBC'])
        cls.raw = json.loads(path.read_text())
        root = Path(__file__).resolve().parents[4]
        cls.schema = json.loads((root /
            'verification/experiments/stage1-protocol/envelope-schema/inventory.json').read_text())
        cls.good = check(cls.raw, cls.schema)
        cls.record = cls.good['bodies'][AUTHORITY + 'RecordedLifecycleIntent']

    def mutated(self):
        raw = copy.deepcopy(self.raw)
        function = next(f for f in raw['translated']['fun_decls']
                        if f and f['def_id'] == self.record['function_id'])
        return raw, function

    @staticmethod
    def call(function, statement):
        return next(v['kind']['Call']['call'] for v in visit(function)
                    if isinstance(v, dict) and v.get('id') == statement and
                    isinstance(v.get('kind'), dict) and 'Call' in v['kind'])

    def field(self, name):
        return next(c for c in self.record['calls'] if c['callee'].endswith('::serialize_field')
                    and c['arguments'][1] == {'constant': name})

    def refused(self, raw):
        with self.assertRaises((ValueError, KeyError)):
            check(raw, self.schema)

    def test_complete_actual_bodies(self):
        self.assertEqual(len(self.good['bodies']), 26)
        self.assertEqual(len(self.good['checked']), 31)
        self.assertFalse(self.good['library_refinement_proved'])

    def test_same_typed_field_substitution(self):
        raw, function = self.mutated()
        projections = [v for v in visit(function) if isinstance(v, dict) and v.get('Field') == [None, 3]]
        self.assertEqual(len(projections), 1)
        projections[0]['Field'] = [None, 2]
        self.refused(raw)

    def test_complete_field_omission(self):
        raw, function = self.mutated()
        call = self.call(function, self.field('terminal')['statement'])
        call['func'] = copy.deepcopy(self.call(function,
            next(c['statement'] for c in self.record['calls'] if c['callee'].endswith('::end')))['func'])
        self.refused(raw)

    def test_original_key_not_other_key(self):
        raw, function = self.mutated()
        call = self.call(function, self.field('terminal')['statement'])
        other = self.call(function, self.field('current')['statement'])
        call['args'][1] = copy.deepcopy(other['args'][1])
        self.refused(raw)

    def test_field_dictionary_not_other_dictionary(self):
        raw, function = self.mutated()
        call = self.call(function, self.field('terminal')['statement'])
        other = self.call(function, self.field('current_session_version')['statement'])
        call['func']['Regular']['generics']['trait_refs'] = copy.deepcopy(
            other['func']['Regular']['generics']['trait_refs'])
        self.refused(raw)

    def test_original_error_not_discarded(self):
        raw, function = self.mutated()
        error = next(c for c in self.record['calls'] if c['callee'].endswith('::from_residual'))
        self.call(function, error['statement'])['dest']['kind'] = {'Local': 300}
        self.refused(raw)

    def test_error_branch_not_success_branch(self):
        raw, function = self.mutated()
        # Select the ordinary main body rather than a nested unwind cleanup.
        switch = next(s['kind']['Switch'] for s in function['body']['Structured']['body']['statements']
                      if isinstance(s['kind'], dict) and 'Switch' in s['kind'])
        switch['data']['branches'][0][1], switch['data']['branches'][1][1] = (
            switch['data']['branches'][1][1], switch['data']['branches'][0][1])
        self.refused(raw)

    def test_incomplete_extraction_refused(self):
        raw, _ = self.mutated()
        raw['has_errors'] = True
        self.refused(raw)


if __name__ == '__main__':
    unittest.main()
