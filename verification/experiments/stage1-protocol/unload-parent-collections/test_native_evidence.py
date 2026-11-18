"""Regressions for collection-native evidence admission on controlled records."""
import io
import json
import tarfile
import unittest

from native_evidence import (CONTRACT_ASSERTION, Evidence, IDENTITY, RECEIPTS,
                             compile_artifact, source_for_case, verify_assertions)


class NativeCollectionEvidence(unittest.TestCase):
    source_path = 'controller-service/src/controller/lifecycle_inbox/source_work/snapshot.rs'
    source = b'fn test() {\n    ' + CONTRACT_ASSERTION + b'\n}\n'

    def correct(self):
        return (f'test {IDENTITY} ... ok\n'
                f'test {RECEIPTS} ... ok\n'
                'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured;\n').encode()

    def mutant(self):
        return (f'test {IDENTITY} ... FAILED\n'
                f'test {RECEIPTS} ... ok\n'
                f"thread '{IDENTITY}' (730) panicked at {self.source_path}:2:5:\n"
                'assertion `left == right` failed\n'
                'test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured;\n').encode()

    def test_expected_correct_mutant_and_restored_results(self):
        verify_assertions('correct', 0, self.correct(), self.source_path, self.source)
        verify_assertions('attempt-zero', 101, self.mutant(), self.source_path, self.source)
        verify_assertions('restored', 0, self.correct(), self.source_path, self.source)

    def test_unrelated_assertion_or_invalid_execution_cannot_count(self):
        mutant = self.mutant()
        for label, code, log in (
            ('other line', 101, mutant.replace(b':2:5:', b':3:9:')),
            ('other thread', 101, mutant.replace(b"full_identity_contract'", b"unrelated'")),
            ('other test failed', 101, mutant.replace(b'full_receipt_contract ... ok',
                                                      b'full_receipt_contract ... FAILED')),
            ('timeout', 124, mutant),
            ('not restored', 101, self.correct()),
        ):
            with self.subTest(label=label), self.assertRaises(ValueError):
                verify_assertions('attempt-zero', code, log, self.source_path, self.source)

    def test_source_replacement_is_unique_and_restored_retains_original(self):
        profile = dict(source=self.source_path, mutation=dict(before='old_field', after='changed_field'))
        production, fixture = b'mod old_field {}', b'mod tests {}'
        original, _ = source_for_case('correct', production, fixture, profile)
        mutant, mutation = source_for_case('attempt-zero', production, fixture, profile)
        restored, _ = source_for_case('restored', production, fixture, profile)
        self.assertEqual(restored, original)
        self.assertEqual(mutant, b'mod changed_field {}\nmod tests {}')
        self.assertEqual(mutation['variant'], 'attempt-zero')
        for wrong in (b'missing', b'old_field old_field'):
            with self.subTest(wrong=wrong), self.assertRaisesRegex(ValueError, 'missing or ambiguous'):
                source_for_case('attempt-zero', wrong, fixture, profile)

    def test_compile_requires_exact_fresh_artifact_and_complete_build(self):
        artifact = dict(reason='compiler-artifact', executable='/candidate/app',
                        target={'name':'sandproof_controller_service'}, profile={'test':True},
                        fresh=False)

        def log(row, success=True):
            return (json.dumps(row) + '\n' + json.dumps(dict(reason='build-finished', success=success))).encode()

        compile_artifact(log(artifact), artifact)
        for raw, expected in ((log(artifact, False), artifact),
                              (log(dict(artifact, fresh=True)), dict(artifact, fresh=True)),
                              (log(dict(artifact, executable='/other/app')), artifact)):
            with self.assertRaisesRegex(ValueError, 'complete fresh'):
                compile_artifact(raw, expected)

    def test_preserved_bundle_refuses_duplicate_or_linked_members(self):
        for duplicate in (False, True):
            with self.subTest(duplicate=duplicate):
                raw = io.BytesIO()
                with tarfile.open(fileobj=raw, mode='w') as bundle:
                    member = tarfile.TarInfo('result.json')
                    if duplicate:
                        bundle.addfile(member)
                        bundle.addfile(member)
                    else:
                        member.type = tarfile.SYMTYPE
                        member.linkname = 'another.json'
                        bundle.addfile(member)
                raw.seek(0)
                with tarfile.open(fileobj=raw, mode='r') as bundle:
                    with self.assertRaisesRegex(ValueError, 'duplicate' if duplicate else 'linked'):
                        Evidence(None, bundle).read('result.json')


if __name__ == '__main__':
    unittest.main()
