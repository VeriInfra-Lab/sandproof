"""Admission of the actual original-file mutation, without Cargo or a VM."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('material_file_identity_native',
    ROOT / 'verification/experiments/material-file-identity/native.py')
native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native)


class MaterialFileMutationTests(unittest.TestCase):
    def source(self, root):
        for name in (native.SOURCE, native.FIXTURE):
            path = root/name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((ROOT/name).read_bytes())
        return root/native.SOURCE

    def test_only_actual_saved_identity_guard_changes_and_whole_original_test_file_stays_exact(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); path = self.source(root)
            original = path.read_text(); fixture = (root/native.FIXTURE).read_bytes()
            line = native.rejection_line(root)
            self.assertEqual(fixture.splitlines()[line-1].strip(),
                b'assert!(fixture.store.retire_capture_material(&plan).is_err());')
            native.mutate(root)
            self.assertEqual(path.read_text(), original.replace(native.BEFORE, native.AFTER, 1))
            self.assertEqual((root/native.FIXTURE).read_bytes(), fixture)
            self.assertEqual(native.fixture_digest(root), hashlib.sha256(fixture).hexdigest())
            self.assertEqual(native.rejection_line(root), line)
            with self.assertRaises(ValueError): native.mutate(root)

    def test_missing_duplicate_or_outside_guard_is_rejected_without_a_write(self):
        for failure in ('missing', 'duplicate', 'outside'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); path = self.source(root); text = path.read_text()
                if failure == 'missing': text = text.replace(native.BEFORE, '')
                if failure == 'duplicate': text = text.replace(native.BEFORE, native.BEFORE*2)
                if failure == 'outside': text = text.replace(native.BEFORE, '') + native.BEFORE
                path.write_text(text)
                with self.assertRaises(ValueError): native.mutate(root)
                self.assertEqual(path.read_text(), text)

    def test_wrong_property_or_repeated_assertion_cannot_change_the_rejection_location(self):
        for failure in ('property', 'assertion', 'duplicate'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); self.source(root); path = root/native.FIXTURE
                text = path.read_text()
                marker = '        assert!(fixture.store.retire_capture_material(&plan).is_err());'
                if failure == 'property': text = text.replace(native.PROPERTY.rsplit('::', 1)[1], 'other_test')
                if failure == 'assertion': text = text.replace(marker, '')
                if failure == 'duplicate': text = text.replace(marker, marker+'\n'+marker)
                path.write_text(text)
                with self.assertRaises(ValueError): native.rejection_line(root)


class MaterialFileExecutionTests(unittest.TestCase):
    def output(self, rejected, line=93):
        text = '\nrunning 1 test\ntest '+native.PROPERTY+(' ... FAILED\n' if rejected else ' ... ok\n')
        if rejected:
            text += (f"thread '{native.PROPERTY}' panicked at {native.FIXTURE}:{line}:9:\n"
                     'assertion failed: fixture.store.retire_capture_material(&plan).is_err()\n')
        text += (f'test result: {"FAILED" if rejected else "ok"}. '
                 f'{0 if rejected else 1} passed; {1 if rejected else 0} failed; '
                 '0 ignored; 0 measured; 100 filtered out; finished in 0.1s\n')
        return text.encode()

    def test_same_original_assertion_requires_real_success_or_its_exact_rejection(self):
        native.checked_execution(self.output(False), b'', 0, native.PROPERTY, False, 93)
        native.checked_execution(self.output(True), b'', 101, native.PROPERTY, True, 93)
        for name in native.TESTS[1:]:
            native.checked_execution(self.output(False).replace(native.PROPERTY.encode(), name.encode()),
                b'', 0, name, False, 93)
        self.assertEqual(len(set(native.TESTS)), 3)

    def test_compile_error_timeout_skip_wrong_location_test_or_assertion_is_not_a_negative(self):
        bad = self.output(True)
        for output, code in ((b'', 101), (bad, 124),
                (bad.replace(b'running 1 test', b'running 0 tests'), 101),
                (bad.replace(b'0 ignored;', b'1 ignored;'), 101),
                (bad.replace(b':93:9:', b':94:9:'), 101),
                (bad.replace(str(native.FIXTURE).encode(), b'another.rs'), 101),
                (bad.replace(b'assertion failed:', b'called Result::unwrap()'), 101),
                (bad.replace(b'fixture.store.retire_capture_material', b'other_check'), 101),
                (bad.replace(native.PROPERTY.encode(), b'another_test'), 101),
                (self.output(False), 0)):
            with self.subTest(output=output, code=code), self.assertRaises(ValueError):
                native.checked_execution(output, b'', code, native.PROPERTY, True, 93)
        with self.assertRaises(ValueError):
            native.checked_execution(bad, b'', 101, native.PROPERTY, False, 93)


if __name__ == '__main__':
    unittest.main()
