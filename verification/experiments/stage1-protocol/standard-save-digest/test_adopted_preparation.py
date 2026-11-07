"""Fail-closed source admission and isolated real-Rust mutation regressions."""
from pathlib import Path
import hashlib
import importlib.util
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[4]
PARENT = ROOT/'verification/experiments/stage1-protocol'
SOURCE = Path('controller-service/src/controller/lifecycle_inbox/codec.rs')
LEAF = Path('controller-service/src/controller/lifecycle_inbox/sha256.rs')


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), PARENT/name/'preparation.py')
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


BLOB, DIGEST, SAVE = (load(n) for n in ('standard-blob','standard-digest','standard-save-digest'))


class AdoptedSourcePreparation(unittest.TestCase):
    def project(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        for name in (SOURCE, LEAF):
            path = root/name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((ROOT/name).read_bytes())
        return root

    def test_correct_views_preserve_every_production_byte(self):
        for provider in (BLOB,DIGEST,SAVE):
            with self.subTest(provider=provider.__name__):
                project = self.project()
                provider.candidate(project, 'actual')
                self.assertEqual((project/SOURCE).read_bytes(), (ROOT/SOURCE).read_bytes())
                self.assertEqual((project/LEAF).read_bytes(), (ROOT/LEAF).read_bytes())

    def test_unreviewed_source_is_rejected_before_mutation(self):
        for provider in (BLOB,DIGEST,SAVE):
            with self.subTest(provider=provider.__name__):
                project = self.project()
                path = project/SOURCE
                changed = path.read_bytes()+b'\n// unreviewed source\n'
                path.write_bytes(changed)
                with self.assertRaises(ValueError):
                    provider.candidate(project, 'actual')
                self.assertEqual(path.read_bytes(), changed)

    def test_changed_sha_leaf_is_rejected_by_both_digest_views(self):
        for provider in (DIGEST,SAVE):
            project = self.project()
            (project/LEAF).write_text('fn digest() {}\n')
            with self.assertRaises(ValueError):
                provider.candidate(project, 'actual')
            self.assertEqual((project/SOURCE).read_bytes(), (ROOT/SOURCE).read_bytes())

    def test_mutations_change_only_the_selected_real_method(self):
        original = (ROOT/SOURCE).read_text()
        for provider,variants,start,end in [
            (BLOB,('repeat-first','second-error-first'),'fn read_blob_pair(',None),
            (DIGEST,('input-empty','nonempty-tail-zero'),'impl InboxLoadEffects for ','pub(in crate::controller) fn load('),
            (SAVE,('save-input-empty','save-tail-zero'),"impl InboxSaveEffects for StandardInboxSave<'_>",'// The existing connection carrier'),
            (SAVE,('delegate-input-empty',),'// The existing connection carrier','pub(super) fn save('),
        ]:
            begin = original.index(start)
            finish = original.index(end, begin) if end else len(original)
            prefix, suffix = original[:begin],original[finish:]
            for variant in variants:
                with self.subTest(variant=variant):
                    project = self.project()
                    provider.candidate(project,variant)
                    changed = (project/SOURCE).read_text()
                    self.assertNotEqual(changed,original)
                    self.assertTrue(changed.startswith(prefix))
                    self.assertTrue(changed.endswith(suffix))
                    self.assertEqual((project/LEAF).read_bytes(),(ROOT/LEAF).read_bytes())
                    self.assertNotIn('fn sha256_bytes(',changed)

    def test_restoration_records_a_different_real_wrong_source(self):
        original = (ROOT/SOURCE).read_bytes()
        digest = hashlib.sha256(original).hexdigest()
        for provider in (BLOB,DIGEST,SAVE):
            with self.subTest(provider=provider.__name__):
                project = self.project()
                row,_ = provider.candidate(project,'restored')
                self.assertEqual((project/SOURCE).read_bytes(),original)
                restoration = row['restoration']
                wrong = restoration.get('actual_wrong_sha256',restoration.get('wrong_source_sha256'))
                self.assertEqual(len(wrong),64)
                self.assertNotEqual(wrong,digest)


if __name__ == '__main__':
    unittest.main()
