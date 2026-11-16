"""Checker regression; generated output here is not Rust execution evidence."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('retirement_permit_commit_native',
    ROOT / 'verification/experiments/retirement-permit-commit/native.py')
native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native)


class RetirementPermitMutationTests(unittest.TestCase):
    def source(self, root):
        for name in (native.SOURCE, native.FIXTURE):
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((ROOT / name).read_bytes())
        return root / native.SOURCE

    def test_omits_only_source_save_before_the_original_commit_and_permit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = self.source(root)
            original = path.read_text()
            fixture = (root / native.FIXTURE).read_bytes()
            line = native.rejection_line(root)
            self.assertEqual(fixture.splitlines()[line - 1].strip(), native.MARKER.strip().encode())
            native.mutate(root)
            expected = original.replace(native.BEFORE, native.AFTER, 1)
            self.assertEqual(path.read_text(), expected)
            self.assertEqual((root / native.FIXTURE).read_bytes(), fixture)
            self.assertEqual(native.fixture_digest(root), hashlib.sha256(fixture).hexdigest())
            self.assertEqual(native.rejection_line(root), line)
            first, last = original.index(native.FIRST), original.index(native.LAST)
            self.assertEqual(expected[:first], original[:first])
            self.assertEqual(expected[expected.index(native.LAST):], original[last:])
            self.assertEqual(original[first:last].count('codec::save('), 1)
            self.assertEqual(expected[first:expected.index(native.LAST)].count('codec::save('), 0)
            with self.assertRaises(ValueError):
                native.mutate(root)
            self.assertEqual(path.read_text(), expected)

    def test_missing_duplicate_or_moved_boundary_refuses_before_writing(self):
        for fault in ('missing', 'duplicate', 'moved'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                path = self.source(root)
                text = path.read_text()
                if fault == 'missing':
                    changed = text.replace(native.BEFORE, '', 1)
                elif fault == 'duplicate':
                    changed = text.replace(native.BEFORE, native.BEFORE * 2, 1)
                else:
                    changed = text.replace(native.BEFORE, '', 1) + native.BEFORE
                path.write_text(changed)
                with self.assertRaises(ValueError):
                    native.mutate(root)
                self.assertEqual(path.read_text(), changed)

    def test_requires_the_original_assertion_inside_the_original_property(self):
        for fault in ('property', 'missing', 'duplicate', 'moved'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.source(root)
                path = root / native.FIXTURE
                text = path.read_text()
                if fault == 'property':
                    changed = text.replace(native.PROPERTY.rsplit('::', 1)[1], 'other_test')
                elif fault == 'missing':
                    changed = text.replace(native.MARKER, '', 1)
                elif fault == 'duplicate':
                    changed = text.replace(native.MARKER, native.MARKER * 2, 1)
                else:
                    changed = text.replace(native.MARKER, '', 1) + native.MARKER
                path.write_text(changed)
                with self.assertRaises(ValueError):
                    native.rejection_line(root)


class RetirementPermitExecutionTests(unittest.TestCase):
    def output(self, rejected, name=None, line=224):
        name = native.PROPERTY if name is None else name
        text = '\nrunning 1 test\ntest ' + name + (' ... FAILED\n' if rejected else ' ... ok\n')
        if rejected:
            text += (f"thread '{name}' panicked at {native.FIXTURE}:{line}:5:\n"
                'assertion failed: !retiring.restore.as_ref().unwrap().material_use\n')
        text += (f'test result: {"FAILED" if rejected else "ok"}. '
            f'{0 if rejected else 1} passed; {1 if rejected else 0} failed; '
            '0 ignored; 0 measured; 100 filtered out; finished in 0.1s\n')
        return text.encode()

    def test_uses_the_same_original_property_and_three_legal_controls(self):
        native.checked_execution(self.output(False), b'', 0, native.PROPERTY, False, 224)
        native.checked_execution(self.output(True), b'', 101, native.PROPERTY, True, 224)
        self.assertEqual(len(set(native.TESTS)), 4)
        for name in native.TESTS[1:]:
            native.checked_execution(self.output(False, name), b'', 0, name, False, 224)
            with self.assertRaises(ValueError):
                native.checked_execution(self.output(True, name), b'', 101, name, True, 224)

    def test_rejects_early_errors_other_assertions_skips_and_timeouts(self):
        good = self.output(True)
        for out, code in (
            (b'', 101), (good, 124), (good, 0),
            (good.replace(b'running 1 test', b'running 0 tests'), 101),
            (good.replace(b'0 ignored;', b'1 ignored;'), 101),
            (good.replace(b':224:5:', b':223:5:'), 101),
            (good.replace(str(native.FIXTURE).encode(), b'other.rs'), 101),
            (good.replace(b'assertion failed:', b'called Result::unwrap(): CorruptStore'), 101),
            (good.replace(native.ASSERTION, b'retiring.retirement.is_none()'), 101),
            (good.replace(native.PROPERTY.encode(), b'other_test'), 101),
            (self.output(False), 0),
        ):
            with self.subTest(output=out, code=code), self.assertRaises(ValueError):
                native.checked_execution(out, b'', code, native.PROPERTY, True, 224)
        with self.assertRaises(ValueError):
            native.checked_execution(good, b'', 101, native.PROPERTY, False, 224)


if __name__ == '__main__':
    unittest.main()
