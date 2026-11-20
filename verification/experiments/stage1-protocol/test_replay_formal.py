"""Ordinary driver regressions; these do not run or certify Lean/Rust proofs."""
import importlib.util
from pathlib import Path
import re
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


DRIVER = load('stage1_replay_driver_tests', HERE / 'replay_formal.py')
FORMAL = load('stage1_replay_checker_tests', HERE / 'formal.py')


def audits(paths):
    return [name for path in paths for name in re.findall(
        r'^#audit_controller_proof (\S+)$', path.read_text(), re.M)]


class ReplayDriverTests(unittest.TestCase):
    def test_each_case_has_a_private_target_before_extraction(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            correct = DRIVER.prepare_extract_target(root / 'correct/source')
            wrong = DRIVER.prepare_extract_target(root / 'wrong/source')
            self.assertTrue(correct.is_dir())
            self.assertTrue(wrong.is_dir())
            self.assertNotEqual(correct.stat().st_ino, wrong.stat().st_ino)
            self.assertEqual(correct.stat().st_mode & 0o777, 0o700)
            marker = correct / 'preserve-cache'
            marker.write_text('original')
            with self.assertRaises(FileExistsError):
                DRIVER.prepare_extract_target(root / 'correct/source')
            self.assertEqual(marker.read_text(), 'original')

    def test_existing_target_symlink_is_not_followed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            existing = root / 'existing'
            existing.mkdir()
            (root / 'source/artifacts').mkdir(parents=True)
            (root / 'source/artifacts/extract-target').symlink_to(existing, target_is_directory=True)
            with self.assertRaises(FileExistsError):
                DRIVER.prepare_extract_target(root / 'source')
            self.assertEqual(list(existing.iterdir()), [])

    def test_each_property_registers_the_full_physical_audit_inventory(self):
        for name, config in DRIVER.PROPERTIES.items():
            for case in ('positive', 'negative'):
                with self.subTest(property=name, case=case):
                    files = FORMAL.component_files(ROOT, config[case], 'request')
                    observed = audits([ROOT / path for path in files])
                    expected = config['audits'] if case == 'positive' else config['negative_audits']
                    if case == 'negative':
                        observed += audits([HERE / (config['template'] + '.lean.in')])
                    self.assertCountEqual(observed, expected)
                    self.assertEqual(len(observed), len(set(observed)))

    def test_unregistered_or_missing_audit_is_rejected_by_original_checker(self):
        config = DRIVER.PROPERTIES['envelope']
        expected = config['audits']
        for observed in (expected[:-1], [*expected, 'Unexpected.audit'], [*expected, expected[0]]):
            with self.subTest(observed=observed):
                with patch.object(FORMAL, 'actual_files', return_value={'audits': observed}):
                    with self.assertRaisesRegex(ValueError, 'inventory differs'):
                        FORMAL.check_actual(ROOT, config['positive'], 'request', None, {}, None, expected)


if __name__ == '__main__':
    unittest.main()
