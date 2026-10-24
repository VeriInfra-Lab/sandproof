"""Actual quota output parser guards only; no filesystem quota or VM changes."""
import importlib.util
from pathlib import Path
import unittest

PATH=Path(__file__).resolve().parents[2]/'tests/system/lifecycle_unload_quota.py'
SPEC=importlib.util.spec_from_file_location('lifecycle_unload_quota_readback',PATH)
quota=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(quota)


class QuotaReadbackTests(unittest.TestCase):
    def test_exact_numeric_project_hard_limit_without_human_units(self):
        output='Project ID Used Soft Hard Warn/Grace\n#0 9 0 0 00 [------]\n#63801 12 0 16777216 00 [------]\n'
        self.assertEqual(quota.project_hard_limit(output),16777216)
        self.assertEqual(quota.project_hard_limit('63801 3 0 65536 00 [------]\n'),65536)

    def test_missing_duplicate_zero_and_partial_limits_cannot_publish_receipt(self):
        for text in ('','#63802 0 0 16777216 00\n','#63801 0 0 0 00\n',
                '#63801 0 0\n','#63801 0 0 16G 00\n',
                '#63801 0 0 16 00\n#63801 0 0 16 00\n',
                '#63801 0 4 16777216 00\n'):
            with self.subTest(output=text),self.assertRaises(ValueError):
                quota.project_hard_limit(text)


if __name__=='__main__':
    unittest.main()
