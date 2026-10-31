"""Fail-closed checks for the narrow LLBC field relation under review."""
from pathlib import Path
import copy
import importlib.util
import json
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('conversion_candidate', Path(__file__).with_name('conversion_candidate.py'))
CANDIDATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CANDIDATE)


def fixture():
    def declaration(name, index, kind):
        return dict(def_id=index, item_meta=dict(name=[{'Ident': [n, 0]} for n in name.split('::')],
            opacity='Transparent'), generics=dict(types=[], regions=[], const_generics=[]), kind=kind)
    field = dict(name='_0', is_positional=True, ty={'Value': [400, {'Adt': dict(id=71,
        generics=dict(types=[], regions=[], const_generics=[], trait_refs=[]), builtin=None)}]})
    variants = [dict(name=n, fields=[]) for n in
        ['Empty', 'InvalidDigit', 'PosOverflow', 'NegOverflow', 'Zero', 'NotAPowerOfTwo']]
    return dict(has_errors=False, translated=dict(type_decls=[
        declaration('core::num::error::TryFromIntError', 24, {'Struct': [field]}),
        declaration('core::num::error::IntErrorKind', 71, {'Enum': variants})]))


class TypeRelationAdmission(unittest.TestCase):
    def check(self, value):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'candidate.llbc'
            path.write_text(json.dumps(value))
            return CANDIDATE.type_shape(path)

    def test_definition_ids_are_resolved_by_name(self):
        self.assertEqual(self.check(fixture())['kind_def_id'], 71)

    def test_extra_error_field_is_rejected(self):
        value = fixture()
        fields = value['translated']['type_decls'][0]['kind']['Struct']
        fields.append(copy.deepcopy(fields[0]))
        with self.assertRaises(ValueError):
            self.check(value)

    def test_wrong_field_type_is_rejected(self):
        value = fixture()
        value['translated']['type_decls'][0]['kind']['Struct'][0]['ty']['Value'][1]['Adt']['id'] = 99
        with self.assertRaises(ValueError):
            self.check(value)

    def test_new_variant_payload_is_rejected(self):
        value = fixture()
        value['translated']['type_decls'][1]['kind']['Enum'][3]['fields'] = [{}]
        with self.assertRaises(ValueError):
            self.check(value)

    def test_missing_variant_is_rejected(self):
        value = fixture()
        value['translated']['type_decls'][1]['kind']['Enum'].pop()
        with self.assertRaises(ValueError):
            self.check(value)

    def test_opaque_type_is_rejected(self):
        value = fixture()
        value['translated']['type_decls'][0]['item_meta']['opacity'] = 'Opaque'
        with self.assertRaises(ValueError):
            self.check(value)

    def test_unknown_interning_is_rejected(self):
        value = fixture()
        value['translated']['type_decls'][0]['kind']['Struct'][0]['ty'] = {'Deduplicated': 400}
        with self.assertRaises(ValueError):
            self.check(value)


if __name__ == '__main__':
    unittest.main()
