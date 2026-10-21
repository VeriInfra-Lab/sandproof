"""Finite admission regressions against preserved real outputs, Ning only."""
from __future__ import annotations

import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('carrier_preflight', Path(__file__).with_name('run.py'))
driver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(driver)


class OriginalOutputInspection(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.original = Path(os.environ['SANDPROOF_CARRIER_ORIGINAL'])
        cls.types = (cls.original / 'generated/SandproofPlainFieldCarriers/Types.lean').read_text()
        cls.funs = (cls.original / 'generated/SandproofPlainFieldCarriers/Funs.lean').read_text()
        cls.profile = driver.read(driver.ROOT / driver.DIRECTORY / 'source-inputs.json')

    def test_actual_qualified_scalars_and_all_ten_bodies(self):
        record = {}
        driver.inspect_generated(self.original, self.profile, record)
        self.assertEqual(len(record['original_and_root_functions']), 10)
        self.assertEqual(record['generated_signatures']['version_fields'][-1], ('host_store', 'Std.I64'))

    def test_host_store_signedness_cannot_be_erased(self):
        for replacement in ('Std.U64', 'Nat', 'I64'):
            with self.subTest(replacement=replacement), self.assertRaisesRegex(ValueError, 'version fields'):
                driver.generated_signature(self.types.replace('host_store : Std.I64', 'host_store : ' + replacement), self.funs)

    def test_other_field_widths_cannot_be_swapped(self):
        for old, new in [('internal_protocol : Std.U16', 'internal_protocol : Std.U32'),
                         ('center_store : Std.U32', 'center_store : Std.U16')]:
            with self.subTest(field=old), self.assertRaisesRegex(ValueError, 'version fields'):
                driver.generated_signature(self.types.replace(old, new), self.funs)

    def test_lost_or_extra_version_field_rejected(self):
        for text in (self.types.replace('  host_store : Std.I64\n', ''),
                     self.types.replace('  host_store : Std.I64\n', '  host_store : Std.I64\n  extra : Std.U64\n')):
            with self.assertRaisesRegex(ValueError, 'version fields'):
                driver.generated_signature(text, self.funs)

    def test_exact_constant_value_and_width_required(self):
        for old, new in [('2#u16', '1#u16'), ('6#u32', '6#u16'), ('16#i64', '16#u64')]:
            with self.subTest(value=old), self.assertRaisesRegex(ValueError, 'typed version constant'):
                driver.generated_signature(self.types, self.funs.replace(old, new))

    def test_original_alias_is_not_relabelled_as_nominal_isolation(self):
        for name in ('TenantId', 'SandboxId'):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'scalar carrier'):
                driver.generated_signature(self.types.replace('def sandbox_controller_core.ids.' + name + ' := Std.U64',
                                                              'def sandbox_controller_core.ids.' + name + ' := Nat'), self.funs)

    def test_opaque_original_body_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp)
            (output / 'original.llbc').write_bytes((self.original / 'original.llbc').read_bytes())
            (output / 'generated').mkdir()
            inventory = copy.deepcopy(driver.read(self.original / 'generated/translation.json'))
            inventory['functions'][0]['is_opaque'] = True
            (output / 'generated/translation.json').write_text(json.dumps(inventory))
            with self.assertRaisesRegex(ValueError, 'missing or opaque'):
                driver.inspect_generated(output, self.profile, {})


if __name__ == '__main__':
    unittest.main(verbosity=2)
