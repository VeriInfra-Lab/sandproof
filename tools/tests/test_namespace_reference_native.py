"""Admission of the actual Rust FD-only mutation; no Cargo or VM in this suite."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('namespace_reference_native',
    ROOT / 'verification/experiments/namespace-reference/native.py')
native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native)


class NamespaceMutationTests(unittest.TestCase):
    def source(self, project):
        path = project / native.SOURCE
        path.parent.mkdir(parents=True)
        path.write_bytes((ROOT / native.SOURCE).read_bytes())
        return path

    def test_one_real_descriptor_check_changes_and_full_original_rust_test_module_is_identical(self):
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary); path = self.source(project)
            before = path.read_text(); tests = native.test_module(project)
            line = native.rejection_line(project)
            self.assertEqual(before.splitlines()[line - 1].strip(),
                'assert!(require_namespace_unheld(temp.path(), &namespace).is_err());')
            native.mutate(project)
            self.assertEqual(path.read_text(), before.replace(native.BEFORE, native.AFTER, 1))
            self.assertEqual(native.test_module(project), tests)
            self.assertEqual(native.rejection_line(project), line)
            with self.assertRaises(ValueError): native.mutate(project)

    def test_missing_repeated_or_wrong_function_mutation_is_rejected_without_writing(self):
        for failure in ('missing', 'repeated', 'outside', 'test-module'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temporary:
                project = Path(temporary); path = self.source(project); text = path.read_text()
                if failure == 'missing': text = text.replace(native.BEFORE, '')
                if failure == 'repeated': text = text.replace(native.BEFORE, native.BEFORE * 2)
                if failure == 'outside': text = text.replace(native.BEFORE, '') + native.BEFORE
                if failure == 'test-module': text += '\n#[cfg(test)]\nmod tests {}\n'
                path.write_text(text)
                with self.assertRaises(ValueError): native.mutate(project)
                self.assertEqual(path.read_text(), text)


class NamespaceExecutionTests(unittest.TestCase):
    def output(self, rejected):
        name = native.PROPERTY
        text = '\nrunning 1 test\ntest ' + name + (' ... FAILED\n' if rejected else ' ... ok\n')
        if rejected:
            text += (f"thread '{name}' panicked at {native.FIXTURE}:888:9:\n"
                     'assertion failed: require_namespace_unheld(temp.path(), &namespace).is_err()\n')
        text += (f'test result: {"FAILED" if rejected else "ok"}. '
                 f'{0 if rejected else 1} passed; {1 if rejected else 0} failed; '
                 '0 ignored; 0 measured; 100 filtered out; finished in 0.1s\n')
        return text.encode()

    def test_same_original_assertion_accepts_success_and_only_its_actual_negative(self):
        native.checked_execution(self.output(False), b'', 0, native.PROPERTY, False, 888)
        native.checked_execution(self.output(True), b'', 101, native.PROPERTY, True, 888)

    def test_compile_failure_timeout_skip_wrong_test_location_or_assertion_cannot_count(self):
        bad = self.output(True)
        for output, code in ((b'', 101), (bad, 124),
                (bad.replace(b'running 1 test', b'running 0 tests'), 101),
                (bad.replace(b'0 ignored;', b'1 ignored;'), 101),
                (bad.replace(b':888:9:', b':889:9:'), 101),
                (bad.replace(str(native.FIXTURE).encode(), b'another.rs'), 101),
                (bad.replace(b'assertion failed:', b'called Result::unwrap()'), 101),
                (bad.replace(b'require_namespace_unheld(temp.path(),', b'another_check(temp.path(),'), 101),
                (bad.replace(native.PROPERTY.encode(), b'another_test'), 101),
                (self.output(False), 0)):
            with self.subTest(output=output, code=code), self.assertRaises(ValueError):
                native.checked_execution(output, b'', code, native.PROPERTY, True, 888)

    def test_real_failure_is_never_a_positive_and_both_original_controls_stay_positive(self):
        with self.assertRaises(ValueError):
            native.checked_execution(self.output(True), b'', 101, native.PROPERTY, False, 888)
        for name in native.TESTS[1:]:
            output = self.output(False).replace(native.PROPERTY.encode(), name.encode())
            native.checked_execution(output, b'', 0, name, False, 888)
        self.assertEqual(len(set(native.TESTS)), 3)


class NamespaceInputTests(unittest.TestCase):
    def test_runtime_libtest_must_be_fresh_and_bound_to_its_actual_package_target_and_cache(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); project = root / 'source'; target = root / 'target'; target.mkdir()
            binary = target / 'runtime-tests'; binary.write_bytes(b'controlled artifact, never executed')
            row = dict(reason='compiler-artifact', executable=str(binary),
                package_id=(project / native.PACKAGE_DIR).as_uri().replace('file:', 'path+file:') + '#sandbox-runtime@0.1.0',
                manifest_path=str(project / native.PACKAGE_DIR / 'Cargo.toml'),
                target=dict(src_path=str(project / native.PACKAGE_DIR / 'src/lib.rs'), name=native.CRATE, kind=['lib']),
                profile=dict(test=True), fresh=False)
            def admit(value):
                return native.runner.checked_binary(json.dumps(value).encode(), project, target,
                    package_dir=native.PACKAGE_DIR, crate=native.CRATE)
            self.assertEqual(admit(row)['sha256'], native.runner.sha(binary))
            changes = [lambda r: r.update(fresh=True),
                lambda r: r.update(package_id='registry+https://example.invalid#runtime@1'),
                lambda r: r.update(manifest_path=str(project / 'controller-service/Cargo.toml')),
                lambda r: r['target'].update(name='sandproof_controller_service'),
                lambda r: r['target'].update(src_path=str(project / 'other/lib.rs')),
                lambda r: r['profile'].update(test=False)]
            for index, change in enumerate(changes):
                changed = copy.deepcopy(row); change(changed)
                with self.subTest(index=index), self.assertRaises(ValueError): admit(changed)
            with self.assertRaises(ValueError):
                native.runner.checked_binary(json.dumps(row).encode(), project, target)

    def test_only_registered_runtime_file_may_differ_and_all_other_input_modes_and_bytes_remain_exact(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); source = root / native.SOURCE; source.parent.mkdir(parents=True)
            source.write_bytes(b'original'); os.chmod(source, 0o644)
            other = root / 'other.rs'; other.write_bytes(b'unchanged'); os.chmod(other, 0o644)
            inputs = {str(path.relative_to(root)): dict(sha256=native.runner.sha(path), mode=0o644)
                      for path in (source, other)}
            source.write_bytes(b'one allowed actual mutation')
            digest = native.runner.sha(source)
            native.runner.check_inputs(root, inputs, digest, source=native.SOURCE)
            with self.assertRaises(ValueError): native.runner.check_inputs(root, inputs, digest)
            other.write_bytes(b'unexpected')
            with self.assertRaises(ValueError): native.runner.check_inputs(root, inputs, digest, source=native.SOURCE)
            other.write_bytes(b'unchanged'); os.chmod(source, 0o755)
            with self.assertRaises(ValueError): native.runner.check_inputs(root, inputs, digest, source=native.SOURCE)

    def test_runtime_rebuild_preserves_original_unit_inode_bytes_and_mode_without_cache_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary); path = project / native.PACKAGE_DIR / 'src/lib.rs'
            path.parent.mkdir(parents=True); path.write_bytes(b'actual original runtime unit'); os.chmod(path, 0o644)
            os.utime(path, (1, 1)); before = path.stat()
            result = native.runner.mark_actual_lib_for_rebuild(project, package_dir=native.PACKAGE_DIR)
            self.assertEqual(path.stat().st_ino, before.st_ino)
            self.assertEqual(path.stat().st_mode, before.st_mode)
            self.assertEqual(result['sha256'], hashlib.sha256(path.read_bytes()).hexdigest())
            self.assertGreater(result['after_mtime_ns'], result['before_mtime_ns'])


if __name__ == '__main__':
    unittest.main()
