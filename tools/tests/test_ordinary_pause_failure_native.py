"""Strict mutation/output admission; controlled output is not Rust evidence."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('ordinary_pause_failure_native',
    ROOT / 'verification/experiments/ordinary-pause-failure/native.py')
native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native)


class OrdinaryPauseMutationTests(unittest.TestCase):
    def source(self, root):
        for name in (native.SOURCE, native.FIXTURE):
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((ROOT / name).read_bytes())
        return root / native.SOURCE

    def test_reuses_the_existing_policy_and_preserves_the_complete_original_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = self.source(root)
            original = path.read_text()
            fixture = (root / native.FIXTURE).read_bytes()
            line = native.rejection_line(root)
            self.assertEqual(fixture.splitlines()[line - 1].strip(), native.MARKER.strip().encode())
            self.assertEqual(native.PROPERTY, 'capture::tests::' + native.CASE[4])
            with patch.object(native.legacy, 'mutate', wraps=native.legacy.mutate) as reused:
                native.mutate(root)
                reused.assert_called_once_with(root, native.CASE)
            expected = original.replace(native.BEFORE, native.AFTER, 1)
            self.assertEqual(path.read_text(), expected)
            self.assertEqual((root / native.FIXTURE).read_bytes(), fixture)
            self.assertEqual(native.fixture_digest(root), hashlib.sha256(fixture).hexdigest())
            self.assertEqual(native.rejection_line(root), line)
            first, last = original.index(native.FIRST), original.index(native.LAST)
            self.assertEqual(expected[:first], original[:first])
            self.assertEqual(expected[expected.index(native.LAST):], original[last:])
            with self.assertRaises(ValueError):
                native.mutate(root)
            self.assertEqual(path.read_text(), expected)

    def test_missing_duplicate_or_moved_policy_refuses_before_writing(self):
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

    def test_requires_both_original_helper_assertion_and_the_property_call(self):
        for fault in ('property', 'missing', 'duplicate', 'moved', 'call'):
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
                elif fault == 'moved':
                    changed = text.replace(native.MARKER, '', 1) + native.MARKER
                else:
                    first = text.index('\nfn ' + native.PROPERTY.rsplit('::', 1)[1] + '() {')
                    changed = text[:first] + text[first:].replace(native.CALL, '', 1)
                path.write_text(changed)
                with self.assertRaises(ValueError):
                    native.rejection_line(root)


class OrdinaryPauseExecutionTests(unittest.TestCase):
    def output(self, rejected, name=None, line=191):
        name = native.PROPERTY if name is None else name
        text = '\nrunning 1 test\ntest ' + name + (' ... FAILED\n' if rejected else ' ... ok\n')
        if rejected:
            text += (f"thread '{name}' panicked at {native.FIXTURE}:{line}:5:\n"
                'assertion `left == right` failed\n  left: Revoking\n right: Pausing\n')
        text += (f'test result: {"FAILED" if rejected else "ok"}. '
            f'{0 if rejected else 1} passed; {1 if rejected else 0} failed; '
            '0 ignored; 0 measured; 100 filtered out; finished in 0.1s\n')
        return text.encode()

    def test_same_original_property_and_three_legal_controls(self):
        native.checked_execution(self.output(False), b'', 0, native.PROPERTY, False, 191)
        native.checked_execution(self.output(True), b'', 101, native.PROPERTY, True, 191)
        self.assertEqual(len(set(native.TESTS)), 4)
        for name in native.TESTS[1:]:
            native.checked_execution(self.output(False, name), b'', 0, name, False, 191)
            with self.assertRaises(ValueError):
                native.checked_execution(self.output(True, name), b'', 101, name, True, 191)

    def test_rejects_other_phases_early_errors_skips_and_timeouts(self):
        good = self.output(True)
        for out, code in (
            (b'', 101), (good, 124), (good, 0),
            (good.replace(b'running 1 test', b'running 0 tests'), 101),
            (good.replace(b'0 ignored;', b'1 ignored;'), 101),
            (good.replace(b':191:5:', b':190:5:'), 101),
            (good.replace(str(native.FIXTURE).encode(), b'other.rs'), 101),
            (good.replace(b'assertion `left == right` failed', b'called Result::unwrap(): CorruptStore'), 101),
            (good.replace(b'left: Revoking', b'left: Paused'), 101),
            (good.replace(b'right: Pausing', b'right: Destroyed'), 101),
            (good.replace(b'right: Pausing', b'right: Destroyed') + b'\nright: Pausing\n', 101),
            (good.replace(native.PROPERTY.encode(), b'other_test'), 101),
            (self.output(False), 0),
        ):
            with self.subTest(output=out, code=code), self.assertRaises(ValueError):
                native.checked_execution(out, b'', code, native.PROPERTY, True, 191)
        with self.assertRaises(ValueError):
            native.checked_execution(good, b'', 101, native.PROPERTY, False, 191)


if __name__ == '__main__':
    unittest.main()
