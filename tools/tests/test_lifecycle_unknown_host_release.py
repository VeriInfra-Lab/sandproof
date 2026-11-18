"""Admission checks for the real Rust mutation runner; these tests execute no VM."""
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('unknown_host_release_native',
    ROOT / 'verification/experiments/unknown-host-release/native.py')
native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native)


class UnknownReleaseMutationTests(unittest.TestCase):
    def source(self, root, body=None):
        path = root / native.SOURCE
        path.parent.mkdir(parents=True)
        path.write_text('    pub fn release_host_slot() {\n'
                        + (native.BEFORE if body is None else body)
                        + '    }\n    // Same two original transactions\n')
        return path

    def test_mutation_changes_only_actual_original_error_propagation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = self.source(root)
            before = path.read_text()
            native.mutate(root)
            self.assertEqual(path.read_text(), before.replace(native.BEFORE, native.AFTER, 1))
            with self.assertRaises(ValueError):
                native.mutate(root)

    def test_missing_duplicate_or_outside_function_guard_is_rejected(self):
        for body in ('        different();\n', native.BEFORE * 2,
                     '    }\n    // Same two original transactions\n' + native.BEFORE):
            with self.subTest(body=body), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                path = self.source(root, body)
                before = path.read_bytes()
                with self.assertRaises(ValueError):
                    native.mutate(root)
                self.assertEqual(path.read_bytes(), before)


class UnknownReleaseExecutionTests(unittest.TestCase):
    def output(self, rejected):
        name = native.PROPERTY
        text = f'\nrunning 1 test\ntest {name} ... {"FAILED" if rejected else "ok"}\n'
        if rejected:
            text += (f"thread '{name}' panicked at {native.FIXTURE}:170:5:\n"
                     'assertion failed: journal.release_host_slot(moved, callback, crashes).is_err()\n')
        text += (f'test result: {"FAILED" if rejected else "ok"}. '
                 f'{0 if rejected else 1} passed; {1 if rejected else 0} failed; '
                 '0 ignored; 0 measured; 100 filtered out; finished in 0.1s\n')
        return text.encode()

    def test_exact_positive_and_original_assertion_rejection_are_accepted(self):
        native.checked_execution(self.output(False), b'', 0, native.PROPERTY, False, 170)
        native.checked_execution(self.output(True), b'', 101, native.PROPERTY, True, 170)

    def test_rejection_binds_one_panic_thread_location_and_assertion(self):
        original = self.output(True)
        header, message = original.split(b'\nassertion failed: ', 1)
        assertion, summary = message.split(b'\ntest result: ', 1)
        expected_thread = ("thread '" + native.PROPERTY + "' panicked").encode()
        tid_thread = ("thread '" + native.PROPERTY + "' (361) panicked").encode()
        native.checked_execution(original.replace(expected_thread, tid_thread), b'', 101,
                                 native.PROPERTY, True, 170)

        summary = b'\ntest result: ' + summary
        other_panic = b"thread 'other_test' (362) panicked at another.rs:10:5:\nnot the target assertion"
        location = (str(native.FIXTURE) + ':170:5:').encode()
        for stdout, stderr in (
                (original.replace(expected_thread, b"thread 'other_test' panicked"), b''),
                (header.replace(location, b'another.rs:10:5:') +
                 b'\nassertion failed: ' + assertion + b'\n\nlocation: ' + location + summary, b''),
                (header + b'\ncalled Result::unwrap()\n\nassertion failed: ' + assertion + summary, b''),
                (header + summary, b'assertion failed: ' + assertion),
                (header + b'\nassertion failed: ' + assertion + b'\n\n' + other_panic + summary, b''),
                (original, other_panic),
                (original + summary, b'')):
            with self.subTest(stdout=stdout, stderr=stderr), self.assertRaises(ValueError):
                native.checked_execution(stdout, stderr, 101, native.PROPERTY, True, 170)

    def test_compile_timeout_skip_zero_tests_and_wrong_panic_are_not_counterexamples(self):
        original = self.output(True)
        for stdout, stderr, code in (
                (original, b'', 1), (b'', b'compilation failed', 101),
                (original.replace(b'running 1 test', b'running 0 tests'), b'', 101),
                (original.replace(b'0 ignored;', b'1 ignored;'), b'', 101),
                (original.replace(b':170:5:', b':171:5:'), b'', 101),
                (original.replace(str(native.FIXTURE).encode(), b'other.rs'), b'', 101),
                (original.replace(b'assertion failed:', b'called Result::unwrap()'), b'', 101),
                (original.replace(b'release_host_slot(moved,', b'other_call(moved,'), b'', 101),
                (original.replace(native.PROPERTY.encode(), b'other_test'), b'', 101)):
            with self.subTest(code=code, stdout=stdout), self.assertRaises(ValueError):
                native.checked_execution(stdout, stderr, code, native.PROPERTY, True, 170)

    def test_failure_cannot_be_used_as_positive_or_success_as_negative(self):
        for rejected in (False, True):
            with self.subTest(rejected=rejected), self.assertRaises(ValueError):
                native.checked_execution(self.output(not rejected), b'',
                    0 if rejected else 101, native.PROPERTY, rejected, 170)


class UnknownReleaseBinaryTests(unittest.TestCase):
    def test_rebuild_marker_preserves_actual_source_inode_and_bytes_without_cache_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'controller-service/src/lib.rs'
            source.parent.mkdir(parents=True)
            source.write_bytes(b'original actual unit source')
            os.utime(source, (1, 1))
            before = source.stat()
            marker = native.mark_actual_lib_for_rebuild(root)
            self.assertEqual(source.read_bytes(), b'original actual unit source')
            self.assertEqual(source.stat().st_ino, before.st_ino)
            self.assertGreater(marker['after_mtime_ns'], marker['before_mtime_ns'])
            self.assertEqual(marker['sha256'], native.sha(source))

    def test_only_fresh_original_service_unit_binary_in_selected_target_is_accepted(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            project, target = root / 'source', root / 'target'
            target.mkdir()
            binary = target / 'service-tests'
            binary.write_bytes(b'controlled artifact, never executed')
            row = dict(reason='compiler-artifact', executable=str(binary),
                package_id=(project / 'controller-service').as_uri().replace('file:', 'path+file:')
                           + '#sandproof-controller-service@0.1.0',
                manifest_path=str(project / 'controller-service/Cargo.toml'),
                target=dict(src_path=str(project / 'controller-service/src/lib.rs'),
                            name='sandproof_controller_service', kind=['lib']),
                profile=dict(test=True, debug_assertions=True, overflow_checks=True), fresh=False)
            found = native.checked_binary(json.dumps(row).encode(), project, target)
            self.assertEqual(found['sha256'], native.sha(binary))
            for field, value in (('fresh', True), ('manifest_path', '/other/Cargo.toml'),
                                  ('package_id', 'registry+https://example.invalid#service@1'),
                                  ('executable', str(root / 'not-built'))):
                changed = copy.deepcopy(row)
                changed[field] = value
                with self.subTest(field=field), self.assertRaises(ValueError):
                    native.checked_binary(json.dumps(changed).encode(), project, target)
            for field, value in (('src_path', '/other/lib.rs'), ('name', 'other'), ('kind', ['bin'])):
                changed = copy.deepcopy(row)
                changed['target'][field] = value
                with self.subTest(field=field), self.assertRaises(ValueError):
                    native.checked_binary(json.dumps(changed).encode(), project, target)
            duplicate = (json.dumps(row) + '\n' + json.dumps(row)).encode()
            with self.assertRaises(ValueError):
                native.checked_binary(duplicate, project, target)
            link = target / 'linked-tests'
            link.symlink_to(binary)
            row['executable'] = str(link)
            with self.assertRaises(ValueError):
                native.checked_binary(json.dumps(row).encode(), project, target)

    def test_actual_test_profile_requires_true_debug_assertions_and_overflow_checks_for_both_packages(self):
        for package, crate in (('controller-service', 'sandproof_controller_service'),
                               ('sandbox-runtime', 'sandbox_runtime')):
            with self.subTest(package=package), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary); project = root/'source'; target = root/'target'; target.mkdir()
                binary = target/'actual-tests'; binary.write_bytes(b'controlled compiler metadata, never executed')
                row = dict(reason='compiler-artifact', executable=str(binary),
                    package_id=(project/package).as_uri().replace('file:', 'path+file:')+'#'+crate+'@0.1.0',
                    manifest_path=str(project/package/'Cargo.toml'),
                    target=dict(src_path=str(project/package/'src/lib.rs'), name=crate, kind=['lib']),
                    profile=dict(test=True, debug_assertions=True, overflow_checks=True), fresh=False)
                def admit(value):
                    return native.checked_binary(json.dumps(value).encode(), project, target,
                        package_dir=Path(package), crate=crate)
                self.assertEqual(admit(row)['sha256'], native.sha(binary))
                for flag in ('debug_assertions', 'overflow_checks'):
                    for value in (False, 0, 1, None, 'true', 'missing'):
                        with self.subTest(flag=flag, value=value):
                            changed = copy.deepcopy(row)
                            if value == 'missing': changed['profile'].pop(flag)
                            else: changed['profile'][flag] = value
                            with self.assertRaises(ValueError): admit(changed)


class UnknownReleaseArchiveTests(unittest.TestCase):
    def test_archive_must_match_complete_committed_contents_and_modes(self):
        data = b'original bytes'
        blob = hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
        for fault in ('none', 'contents', 'mode', 'missing', 'extra', 'duplicate', 'link'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                archive = root / 'source.tar'
                tree = {'source.txt': ('100644', blob)}
                if fault == 'missing':
                    tree['omitted.txt'] = ('100644', blob)
                with tarfile.open(archive, 'w') as stream:
                    for name in ['source.txt'] + (['extra.txt'] if fault == 'extra' else
                                                   ['source.txt'] if fault == 'duplicate' else []):
                        item = tarfile.TarInfo(name)
                        body = b'changed' if fault == 'contents' else data
                        item.mode = 0o755 if fault == 'mode' else 0o644
                        item.size = len(body)
                        if fault == 'link':
                            item.type = tarfile.SYMTYPE
                            item.linkname = 'target'
                            item.size = 0
                        stream.addfile(item, io.BytesIO(body))
                output = root / 'source'
                output.mkdir()
                if fault == 'none':
                    result = native.unpack(archive, output, tree)
                    self.assertEqual((output / 'source.txt').read_bytes(), data)
                    self.assertEqual(result['source.txt']['sha256'], hashlib.sha256(data).hexdigest())
                else:
                    with self.assertRaises(ValueError):
                        native.unpack(archive, output, tree)

    def test_extracted_tree_rejects_unregistered_entries_missing_files_and_links(self):
        for fault, message in (
                ('extra-file', 'Unexpected input'),
                ('extra-directory', 'Unexpected directory'),
                ('file-link', 'Source inputs changed'),
                ('directory-link', 'Unexpected input'),
                ('hard-link', 'Source inputs changed'),
                ('missing-file', 'Missing input')):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                project = root / 'source'
                unit = project / 'src' / 'lib.rs'
                unit.parent.mkdir(parents=True)
                unit.write_bytes(b'approved original source')
                unit.chmod(0o644)
                inputs = {'src/lib.rs': dict(sha256=native.sha(unit), mode=0o644)}
                native.check_inputs(project, inputs)

                if fault == 'extra-file':
                    (unit.parent / 'unused.rs').write_bytes(unit.read_bytes())
                elif fault == 'extra-directory':
                    (unit.parent / 'extra').mkdir()
                elif fault == 'directory-link':
                    actual = unit.parent.rename(root / 'other-src')
                    unit.parent.symlink_to(actual, target_is_directory=True)
                elif fault == 'file-link':
                    unit.symlink_to(unit.rename(root / 'other.rs'))
                elif fault == 'hard-link':
                    os.link(unit, root / 'linked.rs')
                else:
                    unit.unlink()
                with self.assertRaisesRegex(ValueError, message):
                    native.check_inputs(project, inputs)

    def test_mutation_digest_must_be_different_and_registered(self):
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary)
            mutated = project / 'lib.rs'
            mutated.write_bytes(b'original')
            mutated.chmod(0o644)
            original = native.sha(mutated)
            inputs = {'lib.rs': dict(sha256=original, mode=0o644)}
            with self.assertRaisesRegex(ValueError, 'registered mutation source'):
                native.check_inputs(project, inputs, original, source=Path('lib.rs'))
            with self.assertRaisesRegex(ValueError, 'registered mutation source'):
                native.check_inputs(project, inputs, original, source=Path('unregistered.rs'))
            mutated.write_bytes(b'isolated negative variant')
            native.check_inputs(project, inputs, native.sha(mutated), source=Path('lib.rs'))


if __name__ == '__main__':
    unittest.main()
