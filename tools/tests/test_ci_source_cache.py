"""Exercise lock lifetime and exact checkout matching with small local fixtures."""
from concurrent.futures import ThreadPoolExecutor
from dataclasses import asdict
import json
import os
from pathlib import Path
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from tools import ci_source_cache as cache, ci_vm


SOURCE = cache.PullSource(1040, 'a'*40, 'b'*40)
MERGE = 'c'*40
TREE = 'd'*40


class CacheTests(unittest.TestCase):
    def setUp(self):
        directory=self.enterContext(tempfile.TemporaryDirectory())
        self.cache=cache.SourceCache(Path(directory))
        self.prepares=[]

    def prepare(self,_settings,source,work):
        self.prepares.append(source)
        path=work/'publication'
        path.mkdir()
        path.chmod(0o711)
        image=path/'source.iso'
        image.write_bytes(b'immutable test disk')
        image.chmod(0o444)
        merge=MERGE if source==SOURCE else 'e'*40
        record=dict(source=asdict(source),merge=merge,tree=TREE,bytes=image.stat().st_size,sha256=cache.digest(image))
        (path/'manifest.json').write_text(json.dumps(record)+'\n')
        return path,record

    def test_publication_is_traversable_under_private_launcher_umask(self):
        work=self.cache.root
        pack=work/'source.pack'
        pack.write_bytes(b'pack fixture')

        def make_iso(argv,**_kwargs):
            Path(argv[argv.index('-o')+1]).write_bytes(b'immutable test disk')

        previous_umask=os.umask(0o077)
        try:
            with (patch.object(cache,'download',return_value=(MERGE,pack)), patch.object(cache,'git'),
                  patch.object(cache,'verify_repo',return_value=TREE),
                  patch.object(cache.subprocess,'run',side_effect=make_iso)):
                published,_=cache.prepare_disk({},SOURCE,work)
        finally:
            os.umask(previous_umask)
        self.assertEqual(published.stat().st_mode & 0o777,0o711)
        self.assertEqual((published/'source.iso').stat().st_mode & 0o777,0o444)
        self.assertEqual((published/'manifest.json').stat().st_mode & 0o777,0o600)

    def test_concurrent_workers_share_one_publication_and_keep_it_leased(self):
        gate=threading.Barrier(3)
        done=threading.Event()
        def worker():
            with self.cache.acquire(SOURCE,{}) as disk:
                gate.wait(timeout=10)
                done.wait(timeout=10)
                return disk.merge
        with patch.object(cache,'prepare_disk',side_effect=self.prepare), ThreadPoolExecutor(max_workers=2) as pool:
            workers=[pool.submit(worker) for _ in range(2)]
            try:
                gate.wait(timeout=10)
                self.assertEqual(self.prepares,[SOURCE])
                current=self.cache.root/MERGE
                expired=time.time()-cache.MAX_IDLE-1
                os.utime(current/'manifest.json',(expired,expired))
                self.cache.collect()
                self.assertTrue((current/'source.iso').is_file())
            finally:
                done.set()
            self.assertEqual([job.result() for job in workers],[MERGE,MERGE])
            self.cache.collect()
            self.assertFalse(current.exists())

    def test_capacity_allows_fallback_then_reclaims_released_disk(self):
        other=cache.PullSource(1041,'e'*40,'f'*40)
        with patch.object(cache,'prepare_disk',side_effect=self.prepare), patch.object(cache,'MAX_ENTRIES',1):
            with self.cache.acquire(SOURCE,{}) as first:
                with self.cache.acquire(other,{}) as refused:
                    self.assertIsNone(refused)
                self.assertTrue(first.image.is_file())
            with self.cache.acquire(other,{}) as admitted:
                self.assertTrue(admitted.image.is_file())
                self.assertFalse(first.image.exists())
            self.assertEqual(self.prepares,[SOURCE,other])

    def test_failed_publication_falls_back_without_four_repeated_fetches(self):
        with patch.object(cache,'prepare_disk',side_effect=OSError('transfer failed')) as prepare:
            with self.assertRaises(OSError), self.cache.acquire(SOURCE,{}):
                self.fail('failed disk must not be published')
            with self.cache.acquire(SOURCE,{}) as disk:
                self.assertIsNone(disk)
            self.assertEqual(prepare.call_count,1)
            self.assertEqual(self.cache.entries(),[])

    def test_inaccessible_existing_cache_is_not_attached(self):
        with patch.object(cache,'prepare_disk',side_effect=self.prepare):
            with self.cache.acquire(SOURCE,{}):
                pass
            (self.cache.root/MERGE).chmod(0o700)
            with self.assertRaisesRegex(ValueError,'inaccessible to the VM account'):
                with self.cache.acquire(SOURCE,{}):
                    self.fail('an unreadable disk must not be attached')


class IdentityTests(unittest.TestCase):
    def test_only_exact_checkout_action_and_assigned_merge_match(self):
        env=dict(GITHUB_REPOSITORY=cache.REPOSITORY,GITHUB_EVENT_NAME='pull_request',
                 GITHUB_REF='refs/pull/1040/merge',GITHUB_SHA=MERGE)
        argv=['-c','protocol.version=2','fetch','--no-tags','--prune','--no-recurse-submodules','--progress',
              '--depth=1','origin',f'+{MERGE}:refs/remotes/pull/1040/merge']
        self.assertTrue(cache.translated_argv(argv,env,SOURCE,MERGE))
        self.assertFalse(cache.translated_argv(argv,dict(env,GITHUB_SHA=SOURCE.head),SOURCE,MERGE))
        self.assertFalse(cache.translated_argv(argv,dict(env,GITHUB_REF='refs/pull/1041/merge'),SOURCE,MERGE))
        self.assertFalse(cache.translated_argv(argv+['--tags'],env,SOURCE,MERGE))

    def test_actions_inventory_and_guest_disk_preserve_the_original_boundary(self):
        run=dict(event='pull_request', head_sha=SOURCE.head,
                 pull_requests=[dict(number=SOURCE.number,head={'sha':SOURCE.head},base={'sha':SOURCE.base})])
        self.assertEqual(cache.from_run(run),SOURCE)
        self.assertIsNone(cache.from_run(dict(run,head_sha=SOURCE.base)))
        self.assertIsNone(cache.from_run(dict(run,event='push')))
        disk=cache.SourceDisk(Path('/private/source.iso'),SOURCE,MERGE,TREE)
        config=ci_vm.cloud_config('sdk','YWJjZGVmZ2hpamtsbW5vcA==',source_disk=disk)
        env=next(item for item in config['write_files'] if item['path']=='/etc/sandproof-ci.env')
        self.assertIn('CARGO_BUILD_JOBS=4\nRUST_TEST_THREADS=2\n',env['content'])
        wrapper=next(item for item in config['write_files'] if item['path']==cache.GUEST_BIN+'/git')
        compile(wrapper['content'],'<CI git wrapper>','exec')
        self.assertEqual(wrapper['owner'],'root:root')
        self.assertIn('ro,nosuid,nodev,noexec',str(config['runcmd']))
        original=ci_vm.qemu_command(Path('/private/run'),'sdk')
        with_disk=ci_vm.qemu_command(Path('/private/run'),'sdk',disk)
        self.assertEqual(with_disk[with_disk.index('-m')+1],'6144')
        self.assertEqual(with_disk[with_disk.index('-smp')+1],'8')
        index=with_disk.index('file=/private/source.iso,if=virtio,format=raw,readonly=on')
        self.assertEqual(with_disk[:index-1]+with_disk[index+1:],original)
