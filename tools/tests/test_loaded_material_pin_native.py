"""Check native mutation/admission, without treating fixtures as Rust execution."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('loaded_material_pin_native',
    ROOT / 'verification/experiments/loaded-material-pin/native.py')
native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native)


class LoadedPinMutationTests(unittest.TestCase):
    def source(self, root):
        for name in (native.SOURCE, native.FIXTURE):
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((ROOT / name).read_bytes())
        return root / native.SOURCE

    def test_changes_one_lifetime_policy_and_keeps_original_identity_and_retirement_checks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = self.source(root)
            original = path.read_text()
            fixture = (root / native.FIXTURE).read_bytes()
            line = native.rejection_line(root)
            self.assertEqual(fixture.splitlines()[line - 1].strip(),
                b'assert!(first_before.restore.as_ref().unwrap().material_use);')
            native.mutate(root)
            expected = original
            for before, after in zip(native.BEFORE, native.AFTER, strict=True):
                expected = expected.replace(before, after, 1)
            self.assertEqual(path.read_text(), expected)
            self.assertEqual((root / native.FIXTURE).read_bytes(), fixture)
            self.assertEqual(native.fixture_digest(root), hashlib.sha256(fixture).hexdigest())
            self.assertEqual(native.rejection_line(root), line)
            # Admission, source identity, exact marker replay and actual closure
            # continue to use the original implementation after the mutation.
            for first, last in (
                ('pub(super) fn close_material_use(', '\nfn work('),
                ('    let capture = record.capture.as_ref().ok_or(JournalError::CorruptStore)?;\n    let LifecycleOperation::Resume',
                 '\n/// Runs inside the original ready-to-dispatched transaction.'),
                ('\nfn admitted_source_generation(', None),
            ):
                old = original[original.index(first):]
                changed = expected[expected.index(first):]
                if last is not None:
                    old = old[:old.index(last)]
                    changed = changed[:changed.index(last)]
                self.assertEqual(changed, old)
            self.assertIn('return Ok(original == action);', expected)
            with self.assertRaises(ValueError):
                native.mutate(root)
            self.assertEqual(path.read_text(), expected)

    def test_any_missing_duplicate_or_moved_policy_site_fails_before_writing(self):
        for index, before in enumerate(native.BEFORE):
            for fault in ('missing', 'duplicate', 'moved'):
                with self.subTest(index=index, fault=fault), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    path = self.source(root)
                    original = path.read_text()
                    if fault == 'missing':
                        changed = original.replace(before, '', 1)
                    elif fault == 'duplicate':
                        changed = original.replace(before, before * 2, 1)
                    else:
                        changed = original.replace(before, '', 1) + before
                    path.write_text(changed)
                    with self.assertRaises(ValueError):
                        native.mutate(root)
                    self.assertEqual(path.read_text(), changed)

    def test_only_the_original_running_backing_pin_assertion_is_admitted(self):
        marker = '    assert!(first_before.restore.as_ref().unwrap().material_use);'
        for fault in ('property', 'missing', 'duplicate', 'moved'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.source(root)
                path = root / native.FIXTURE
                original = path.read_text()
                if fault == 'property':
                    changed = original.replace(native.PROPERTY.rsplit('::', 1)[1], 'other_test')
                elif fault == 'missing':
                    changed = original.replace(marker, '', 1)
                elif fault == 'duplicate':
                    changed = original.replace(marker, marker + '\n' + marker, 1)
                else:
                    changed = original.replace(marker, '', 1) + marker
                path.write_text(changed)
                with self.assertRaises(ValueError):
                    native.rejection_line(root)


class LoadedPinExecutionTests(unittest.TestCase):
    def output(self, rejected, name=None, line=2192):
        name = native.PROPERTY if name is None else name
        text = '\nrunning 1 test\ntest ' + name + (' ... FAILED\n' if rejected else ' ... ok\n')
        if rejected:
            text += (f"thread '{name}' panicked at {native.FIXTURE}:{line}:5:\n"
                'assertion failed: first_before.restore.as_ref().unwrap().material_use\n')
        text += (f'test result: {"FAILED" if rejected else "ok"}. '
            f'{0 if rejected else 1} passed; {1 if rejected else 0} failed; '
            '0 ignored; 0 measured; 100 filtered out; finished in 0.1s\n')
        return text.encode()

    def test_original_pin_assertion_and_both_legal_controls_use_the_same_contract(self):
        native.checked_execution(self.output(False), b'', 0, native.PROPERTY, False, 2192)
        native.checked_execution(self.output(True), b'', 101, native.PROPERTY, True, 2192)
        for name in native.TESTS[1:]:
            native.checked_execution(self.output(False, name), b'', 0, name, False, 2192)
            with self.assertRaises(ValueError):
                native.checked_execution(self.output(True, name), b'', 101, name, True, 2192)
        self.assertEqual(len(set(native.TESTS)), 3)

    def test_early_corrupt_store_other_panic_skip_or_timeout_is_never_a_pin_counterexample(self):
        good = self.output(True)
        for out, code in (
            (b'', 101), (good, 124), (good, 0),
            (good.replace(b'running 1 test', b'running 0 tests'), 101),
            (good.replace(b'0 ignored;', b'1 ignored;'), 101),
            (good.replace(b':2192:5:', b':2191:5:'), 101),
            (good.replace(str(native.FIXTURE).encode(), b'other.rs'), 101),
            (good.replace(b'assertion failed:', b'called Result::unwrap(): CorruptStore'), 101),
            (good.replace(native.ASSERTION, b'first_before.capture.is_none()'), 101),
            (good.replace(native.PROPERTY.encode(), b'other_test'), 101),
            (self.output(False), 0),
        ):
            with self.subTest(output=out, code=code), self.assertRaises(ValueError):
                native.checked_execution(out, b'', code, native.PROPERTY, True, 2192)
        with self.assertRaises(ValueError):
            native.checked_execution(good, b'', 101, native.PROPERTY, False, 2192)

    def test_uses_original_strict_service_package_and_actual_profile_admission(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            project = root / 'source'
            target = root / 'cache'
            target.mkdir()
            binary = target / 'service-tests'
            binary.write_bytes(b'controlled Cargo metadata; this is not a Rust binary')
            row = dict(reason='compiler-artifact', executable=str(binary),
                package_id='path+file://' + str(project / native.PACKAGE_DIR) + '#sandproof-controller-service@0.1.0',
                manifest_path=str(project / native.PACKAGE_DIR / 'Cargo.toml'),
                target=dict(src_path=str(project / native.PACKAGE_DIR / 'src/lib.rs'), name=native.CRATE, kind=['lib']),
                profile=dict(test=True, debug_assertions=True, overflow_checks=True), fresh=False)
            def admit(record):
                return native.runner.checked_binary(json.dumps(record).encode(), project, target,
                    package_dir=native.PACKAGE_DIR, crate=native.CRATE)
            self.assertEqual(admit(row)['path'], str(binary))
            for field in ('test', 'debug_assertions', 'overflow_checks'):
                for value in (False, 1, 'true', None):
                    changed = dict(row, profile=dict(row['profile'], **{field: value}))
                    with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                        admit(changed)
            with self.assertRaises(ValueError):
                admit(dict(row, fresh=True))
            with self.assertRaises(ValueError):
                admit(dict(row, target=dict(row['target'], name='sandbox_controller_core')))


if __name__ == '__main__':
    unittest.main()
