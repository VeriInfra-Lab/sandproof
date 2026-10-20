"""Synthetic source-materialization admission tests; not SQLite semantic proofs."""
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
ASSETS=ROOT/'verification/experiments/sqlite-statements'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class SqliteStatementMaterializationTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(prefix='sqlite-statement-admission-')
        self.addCleanup(temporary.cleanup)
        self.base=Path(temporary.name)
        self.source=self.base/'source'
        self.source.mkdir()
        self.assets=self.base/'assets'
        self.assets.mkdir()
        self.profile=json.loads((ASSETS/'candidate.json').read_text())
        for name in self.profile['source_files']:
            path=self.source/name
            path.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(ROOT/name,path)
        subprocess.run(['git','init','-q'],cwd=self.source,check=True)
        subprocess.run(['git','add','.'],cwd=self.source,check=True)
        subprocess.run(['git','-c','user.name=fixture','-c','user.email=fixture@example.invalid',
                        'commit','-qm','synthetic source fixture'],cwd=self.source,check=True)
        self.profile['base']=subprocess.check_output(['git','rev-parse','HEAD'],cwd=self.source,text=True).strip()
        for name in ('candidate-core.patch',):
            shutil.copyfile(ASSETS/name,self.assets/name)
        self.write_profile()
        spec=importlib.util.spec_from_file_location('sqlite_statements_materialization_test',ASSETS/'prepare.py')
        self.module=importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)
        self.module.HERE=self.assets

    def write_profile(self):
        (self.assets/'candidate.json').write_text(json.dumps(self.profile))

    def test_exact_candidate_and_base_stays_untouched(self):
        before={n:sha(self.source/n) for n in self.profile['source_files']}
        destination=self.base/'candidate'
        result=self.module.prepare(self.source,destination)
        self.assertEqual(set(result['changed']),set(self.profile['candidate_files']))
        self.assertEqual({n:sha(self.source/n) for n in before},before)
        self.assertEqual(subprocess.check_output(['git','status','--porcelain'],cwd=self.source,text=True),'')
        self.assertEqual((destination/'controller-service/src/journal.rs').read_text().count(
            'statement_core::update_existing(transaction, write, next_revision_sql, expected)?;'),1)
        self.assertIn('not a build/test/proof',result['scope'])

    def fixture_archive(self):
        raw=subprocess.check_output(['git','archive','--format=tar',self.profile['base']],cwd=self.source)
        archive=self.base/'source.tar'
        archive.write_bytes(raw)
        self.profile['source_archive_sha256']=hashlib.sha256(raw).hexdigest()
        self.write_profile()
        return archive

    def test_frozen_archive_materializes_the_same_candidate(self):
        archive=self.fixture_archive()
        result=self.module.prepare_archive(archive,self.base/'candidate')
        self.assertEqual(result['source_archive_sha256'],self.profile['source_archive_sha256'])
        self.assertEqual(set(result['changed']),set(self.profile['candidate_files']))
        for name,expected in self.profile['candidate_files'].items():
            self.assertEqual(sha(self.base/'candidate'/name),expected)

    def test_changed_archive_is_rejected_before_materialization(self):
        archive=self.fixture_archive()
        archive.write_bytes(archive.read_bytes()+b'changed')
        with self.assertRaisesRegex(ValueError,'frozen source archive changed'):
            self.module.prepare_archive(archive,self.base/'candidate')
        self.assertFalse((self.base/'candidate').exists())

    def test_input_inventory_keeps_production_src_artifacts(self):
        source=self.base/'inventory-source'
        for name in ('sandbox-center/src/artifacts/mod.rs','sandbox-center/Cargo.toml',
                     'artifacts/private/source.rs','verification/extract-sqlite-statements/target/cache.rs'):
            path=source/name
            path.parent.mkdir(parents=True,exist_ok=True)
            path.write_text('fixture')
        spec=importlib.util.spec_from_file_location('sqlite_statements_preflight_inventory',ASSETS/'preflight.py')
        module=importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        inputs=module.production_inputs(source)
        self.assertEqual(set(inputs),{'sandbox-center/src/artifacts/mod.rs','sandbox-center/Cargo.toml'})

    def test_existing_destination_is_preserved(self):
        destination=self.base/'candidate'
        destination.mkdir()
        marker=destination/'keep'
        marker.write_text('untouched')
        with self.assertRaisesRegex(ValueError,'destination already exists'):
            self.module.prepare(self.source,destination)
        self.assertEqual(marker.read_text(),'untouched')

    def test_existing_symlink_destination_is_preserved(self):
        destination=self.base/'candidate'
        destination.symlink_to(self.source,target_is_directory=True)
        with self.assertRaisesRegex(ValueError,'destination already exists'):
            self.module.prepare(self.source,destination)
        self.assertTrue(destination.is_symlink())

    def test_patch_content_mismatch_rejected_before_materialization(self):
        patch=self.assets/'candidate-core.patch'
        patch.write_text(patch.read_text()+'\n')
        destination=self.base/'candidate'
        with self.assertRaisesRegex(ValueError,'candidate patch changed'):
            self.module.prepare(self.source,destination)
        self.assertFalse(destination.exists())

    def test_frozen_production_mismatch_rejected(self):
        self.profile['source_files']['controller-service/src/journal.rs']='0'*64
        self.write_profile()
        destination=self.base/'candidate'
        with self.assertRaisesRegex(ValueError,'frozen production source changed'):
            self.module.prepare(self.source,destination)
        self.assertFalse(destination.exists())

    def test_profile_cannot_widen_to_production_cargo_or_other_files(self):
        self.profile['candidate_files']['Cargo.toml']='0'*64
        self.write_profile()
        with self.assertRaisesRegex(ValueError,'fixed SQL candidate file scope'):
            self.module.prepare(self.source,self.base/'candidate')

    def test_patch_path_cannot_escape_asset_directory(self):
        self.profile['patch']='../candidate-core.patch'
        self.write_profile()
        with self.assertRaisesRegex(ValueError,'registered local candidate-core.patch'):
            self.module.prepare(self.source,self.base/'candidate')

    def test_unregistered_patch_file_rejected_even_after_rehashing(self):
        patch=self.assets/'candidate-core.patch'
        patch.write_text(patch.read_text()+
            '--- /dev/null\n+++ b/unregistered.txt\n@@ -0,0 +1 @@\n+unexpected\n')
        self.profile['patch_sha256']=sha(patch)
        self.write_profile()
        with self.assertRaisesRegex(ValueError,'outside registered scope'):
            self.module.prepare(self.source,self.base/'candidate')
        self.assertFalse((self.source/'unregistered.txt').exists())


if __name__=='__main__':
    unittest.main()
