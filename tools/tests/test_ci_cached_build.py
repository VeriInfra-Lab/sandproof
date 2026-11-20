import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import ci_cached_build as cache


class CachedBuildTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root=Path(self.tmp.name).resolve()
        self.source=self.root/'source';self.source.mkdir()
        (self.source/'Cargo.lock').write_text('version = 4\n')
        self.cache=self.root/'cache';self.cache.mkdir(mode=0o700)
        self.evidence=self.root/'evidence';self.evidence.mkdir(mode=0o700)

    def run_build(self, code=0, compiler='rustc frozen'):
        def cargo(argv, **kwargs):
            self.assertIn('--locked',argv)
            self.assertIn('--offline',argv)
            self.assertEqual(kwargs['env']['CARGO_NET_OFFLINE'],'true')
            self.assertEqual(kwargs['cwd'],self.source)
            kwargs['stdout'].write(json.dumps({'reason':'compiler-artifact','fresh':True})+'\n')
            from types import SimpleNamespace
            return SimpleNamespace(returncode=code)
        with patch.object(cache.subprocess,'check_output',return_value=compiler), \
                patch.object(cache.subprocess,'run',side_effect=cargo):
            return cache.build(self.source,self.cache,'native',self.evidence,['build'])

    def test_same_configuration_reuses_target_and_keeps_independent_reports(self):
        first=self.run_build()
        original=self.evidence
        self.evidence=self.root/'second';self.evidence.mkdir(mode=0o700)
        second=self.run_build()
        self.assertEqual(first['cargo_target'],second['cargo_target'])
        self.assertEqual(second['artifacts'],{'fresh':1,'compiled':0})
        self.assertTrue((original/'result.json').exists())

    def test_compiler_change_uses_separate_cache(self):
        first=self.run_build()
        self.evidence=self.root/'second';self.evidence.mkdir(mode=0o700)
        self.assertNotEqual(first['cargo_target'],self.run_build(compiler='different')['cargo_target'])

    def test_failed_build_keeps_original_failure_without_retry(self):
        self.assertEqual(self.run_build(code=101)['exit_code'],101)
        self.assertFalse(json.loads((self.evidence/'result.json').read_text())['test_or_proof_acceptance_claimed'])

    def test_source_target_override_or_nonbuild_command_is_rejected(self):
        for args in (['clean'],['test'],['build','--target-dir=/tmp/elsewhere'],
                     ['build','--manifest-path','other'],['build','--config=x'],['build','--','x']):
            with self.subTest(args=args), self.assertRaises(ValueError):
                cache.build(self.source,self.cache,'native',self.evidence,args)

    def test_evidence_cannot_be_inside_reusable_cache(self):
        p=self.cache/'evidence';p.mkdir(mode=0o700)
        with self.assertRaises(ValueError):
            cache.build(self.source,self.cache,'native',p,['build'])

    def test_symlink_target_is_rejected(self):
        self.run_build()
        result=json.loads((self.evidence/'result.json').read_text())
        target=Path(result['cargo_target']);target.rmdir()
        target.symlink_to(self.source,target_is_directory=True)
        self.evidence=self.root/'second';self.evidence.mkdir(mode=0o700)
        with self.assertRaises(ValueError):self.run_build()

    def test_same_lane_lock_refuses_concurrent_writer(self):
        import fcntl
        self.run_build()
        self.evidence=self.root/'second';self.evidence.mkdir(mode=0o700)
        with next(self.cache.glob('*.lock')).open('a') as stream:
            fcntl.flock(stream,fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):self.run_build()
