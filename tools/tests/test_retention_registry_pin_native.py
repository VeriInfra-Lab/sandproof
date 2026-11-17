"""Checker regression; synthetic output is not native execution evidence."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('retention_registry_pin_native',
    ROOT / 'verification/experiments/retention-registry-pin/native.py')
native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native)


class RegistryPinMutationTests(unittest.TestCase):
    def source(self, root):
        for name in (native.SOURCE, native.FIXTURE):
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((ROOT / name).read_bytes())
        return root / native.SOURCE

    def test_only_ignores_positive_pin_and_preserves_original_query_and_full_tests(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = self.source(root)
            original = path.read_text()
            fixture = (root / native.FIXTURE).read_bytes()
            line = native.rejection_line(root)
            self.assertEqual(fixture.splitlines()[line - 1].strip(), b'assert!(matches!(')
            native.mutate(root)
            expected = original.replace(native.BEFORE, native.AFTER, 1)
            self.assertEqual(path.read_text(), expected)
            self.assertEqual((root / native.FIXTURE).read_bytes(), fixture)
            self.assertEqual(native.rejection_line(root), line)
            self.assertEqual(original.count('self.host_slot_release_pinned(local.sandbox)?'), 1)
            self.assertEqual(expected.count('self.host_slot_release_pinned(local.sandbox)?'), 1)
            with self.assertRaises(ValueError):
                native.mutate(root)
            self.assertEqual(path.read_text(), expected)

    def test_missing_duplicate_or_moved_guard_refuses_before_writing(self):
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

    def test_requires_original_assertion_inside_original_property(self):
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


class RegistryPinExecutionTests(unittest.TestCase):
    def output(self, rejected, name=None, line=208):
        name = native.PROPERTY if name is None else name
        text = '\nrunning 1 test\ntest ' + name + (' ... FAILED\n' if rejected else ' ... ok\n')
        if rejected:
            text += (f"thread '{name}' panicked at {native.FIXTURE}:{line}:5:\n"
                f'assertion failed: matches!({native.ASSERTION.decode()},\n'
                '    Err(JournalError::UnresolvedAction))\n')
        text += (f'test result: {"FAILED" if rejected else "ok"}. '
            f'{0 if rejected else 1} passed; {1 if rejected else 0} failed; '
            '0 ignored; 0 measured; 100 filtered out; finished in 0.1s\n')
        return text.encode()

    def test_same_original_contract_and_three_legal_controls(self):
        native.checked_execution(self.output(False), b'', 0, native.PROPERTY, False, 208)
        native.checked_execution(self.output(True), b'', 101, native.PROPERTY, True, 208)
        self.assertEqual(len(set(native.TESTS)), 4)
        for name in native.TESTS[1:]:
            native.checked_execution(self.output(False, name), b'', 0, name, False, 208)
            with self.assertRaises(ValueError):
                native.checked_execution(self.output(True, name), b'', 101, name, True, 208)

    def test_compile_errors_wrong_locations_other_contracts_and_timeouts_are_not_rejection(self):
        good = self.output(True)
        for out, code in (
            (b'', 101), (good, 124), (good, 0),
            (good.replace(b'running 1 test', b'running 0 tests'), 101),
            (good.replace(b'0 ignored;', b'1 ignored;'), 101),
            (good.replace(b':208:5:', b':207:5:'), 101),
            (good.replace(str(native.FIXTURE).encode(), b'other.rs'), 101),
            (good.replace(b'assertion failed:', b'called Result::unwrap(): CorruptStore'), 101),
            (good.replace(native.ASSERTION, b'inbox(&journal).sessions.is_empty()'), 101),
            (good.replace(b'Err(JournalError::UnresolvedAction)', b'Err(JournalError::CorruptStore)'), 101),
            (good.replace(native.PROPERTY.encode(), b'other_test'), 101),
            (self.output(False), 0),
        ):
            with self.subTest(output=out, code=code), self.assertRaises(ValueError):
                native.checked_execution(out, b'', code, native.PROPERTY, True, 208)
        with self.assertRaises(ValueError):
            native.checked_execution(good, b'', 101, native.PROPERTY, False, 208)


if __name__ == '__main__':
    unittest.main()
