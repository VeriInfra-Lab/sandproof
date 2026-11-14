"""Exact private obstacle ownership with controlled original capture providers."""
import copy
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import time
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'tests/system'))
import lifecycle_capture_publish_conflict as publish
import lifecycle_unload_capture_faults as capture


class EmptyTargetTests(unittest.TestCase):
    def test_exclusive_empty_target_and_unchanged_sibling_are_retired_only_explicitly(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);p=publish.EmptyCaptureTarget(root,'a'*64)
            try:
                made=p.create();self.assertTrue(made['created']);self.assertFalse(made['removed'])
                (root/'other-original-bundle').mkdir(mode=0o700)
                (root/'other-original-bundle'/'original').write_bytes(b'untouched')
                p.unchanged();result=p.remove();self.assertTrue(result['removed'])
                self.assertEqual((root/'other-original-bundle'/'original').read_bytes(),b'untouched')
                with self.assertRaises(ValueError):p.remove()
            finally:p.close()

    def test_existing_file_directory_or_symlink_is_never_adopted_or_deleted(self):
        for kind in ('directory','file','symlink'):
            with tempfile.TemporaryDirectory() as temporary,self.subTest(kind=kind):
                root=Path(temporary);target=root/('capture-'+'a'*64)
                if kind=='directory':target.mkdir(mode=0o700)
                elif kind=='file':target.write_bytes(b'original')
                else:target.symlink_to(root/'absent')
                original=target.lstat();p=publish.EmptyCaptureTarget(root,'a'*64)
                try:
                    with self.assertRaises(FileExistsError):p.create()
                    self.assertFalse(p.created)
                finally:p.close()
                self.assertEqual((target.lstat().st_dev,target.lstat().st_ino),(original.st_dev,original.st_ino))

    def test_unknown_or_changed_empty_target_is_preserved_on_close_and_remove_refuses(self):
        for fault in ('none','nonempty','mode','inode','symlink','parent-mode'):
            with tempfile.TemporaryDirectory() as temporary,self.subTest(fault=fault):
                root=Path(temporary);p=publish.EmptyCaptureTarget(root,'a'*64);p.create();target=root/p.name
                if fault=='nonempty':(target/'unknown').write_bytes(b'preserve')
                elif fault=='mode':target.chmod(0o500)
                elif fault in ('inode','symlink'):
                    target.rename(root/'held-original')
                    if fault=='inode':target.mkdir(mode=0o700)
                    else:target.symlink_to(root/'held-original')
                elif fault=='parent-mode':root.chmod(0o750)
                try:
                    if fault!='none':
                        with self.assertRaises(ValueError):p.remove()
                finally:p.close();root.chmod(0o700)
                self.assertTrue(target.exists());self.assertFalse(p.removed)

    def test_invalid_selector_has_no_filesystem_effect(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for key in ('../other','A'*64,'a'*63,True):
                with self.subTest(key=key),self.assertRaises(ValueError):publish.EmptyCaptureTarget(root,key)
            self.assertEqual(list(root.iterdir()),[])


def fixture(root):
    full=dict(pause=dict(binding={'original':'accepted capture'}),compatibility={'original':'profile'},
              state_bytes=100,memory_bytes=100,disk_bytes=100)
    store=root/'s';store.mkdir(mode=0o700)
    obstacle=publish.EmptyCaptureTarget(store,'a'*64);obstacle.create()
    stage=store/('staging-'+'a'*64);stage.mkdir(mode=0o700)
    entries=[]
    for name in ('state.bin','memory.bin','disk-0.img'):
        raw=('original '+name).encode();path=stage/name;path.write_bytes(raw);path.chmod(0o400)
        entries.append(dict(relative_path=name,length=len(raw),digest=list(hashlib.sha256(raw).digest())))
    manifest=dict(binding=full['pause']['binding'],compatibility=full['compatibility'],files=entries)
    (stage/'manifest.json').write_text(json.dumps(manifest));(stage/'manifest.json').chmod(0o400);stage.chmod(0o500)
    p=publish.PublishConflictProbe.__new__(publish.PublishConflictProbe)
    p.obstacle=obstacle;p.conflict=None;p.record={'capture':{'full':full}}
    p.stage='sync-after';p.recovered=True;p.question={'capture_key':'a'*64};p.worker_pidfd=999999
    p.stage_identity=stage.stat().st_dev,stage.stat().st_ino
    memory=(stage/'memory.bin').stat()
    p.sync_material=dict(memory_identity=[memory.st_dev,memory.st_ino],memory_bytes=memory.st_size)
    p.prior={'source':{'original':'VMM'}};p.assert_sync_continued=Mock(return_value={'capacity_released':False})
    return NS(probe=p,obstacle=obstacle,stage=stage,manifest=manifest,store=store)


class PublishConflictTests(unittest.TestCase):
    def test_original_finish_consumes_release_but_still_requires_its_full_returned_end(self):
        from tools.tests import test_lifecycle_sync_boundaries as sync
        for fault in ('none','no-request','consumption'):
            with tempfile.TemporaryDirectory() as temporary,self.subTest(fault=fault):
                original,record=sync.SyncProtocolTests().probe('sync-after')
                p=publish.PublishConflictProbe.__new__(publish.PublishConflictProbe)
                p.__dict__.update(original.__dict__)
                p.obstacle=publish.EmptyCaptureTarget(Path(temporary),p.question['capture_key']);p.obstacle.create();p.conflict=None
                p._save_file.side_effect=None
                p._save_file.return_value=(dict(schema_version=3,run_identity='a'*64,observation_sha256='c'*64,
                    full_work_binding=p.question['full_work_binding'],command='fail' if fault=='consumption' else 'release'),None)
                if fault=='no-request':record['capture']['end']['observation']['outcome']={'kind':'no_request'}
                try:
                    with patch.object(capture,'gate_publish') as sent,patch.object(capture,'pidfd_exited',return_value=True), \
                            patch.object(capture,'original_process',return_value=False):
                        if fault=='none':self.assertTrue(capture.CaptureHandoffProbe.finish(p,p.deadline)['actual_worker_end'])
                        else:
                            with self.assertRaises(ValueError):capture.CaptureHandoffProbe.finish(p,p.deadline)
                        self.assertEqual(sent.call_args.args[2]['command'],'release')
                finally:p.obstacle.close()

    def test_release_requires_the_same_live_obstacle_and_original_parent_failure_stays_fail(self):
        with tempfile.TemporaryDirectory() as temporary:
            f=fixture(Path(temporary))
            try:
                self.assertEqual(f.probe._capture_gate_command(),'release')
                self.assertEqual(capture.CaptureHandoffProbe._capture_gate_command(f.probe),'fail')
                f.probe.question['capture_key']='b'*64
                with self.assertRaises(ValueError):f.probe._capture_gate_command()
                f.probe.question['capture_key']='a'*64
                (f.store/f.obstacle.name/'unknown').write_bytes(b'preserve')
                with self.assertRaises(ValueError):f.probe._capture_gate_command()
            finally:f.obstacle.close();f.stage.chmod(0o700)

    def test_complete_original_stage_is_checked_after_worker_end_without_claiming_errno_trace(self):
        for fault in ('none','binding','digest','mode','memory-inode','stage-inode','extra','completion','obstacle'):
            with tempfile.TemporaryDirectory() as temporary,ExitStack() as stack,self.subTest(fault=fault):
                f=fixture(Path(temporary));p=f.probe;f.stage.chmod(0o700)
                if fault=='binding':f.manifest['binding']={}
                if fault=='digest':f.manifest['files'][0]['digest']=[0]*32
                if fault in ('binding','digest'):
                    (f.stage/'manifest.json').chmod(0o600);(f.stage/'manifest.json').write_text(json.dumps(f.manifest));(f.stage/'manifest.json').chmod(0o400)
                if fault=='mode':(f.stage/'memory.bin').chmod(0o600)
                if fault=='memory-inode':
                    memory=f.stage/'memory.bin';raw=memory.read_bytes();memory.rename(f.stage/'old-memory')
                    memory.write_bytes(raw);memory.chmod(0o400);(f.stage/'old-memory').unlink()
                if fault=='stage-inode':p.stage_identity=(0,0)
                if fault=='extra':(f.stage/'extra').write_bytes(b'not allowed')
                if fault=='completion':(f.store/('completed-'+'a'*64)).write_bytes(b'unknown completion')
                if fault=='obstacle':(f.store/f.obstacle.name/'unknown').write_bytes(b'preserve')
                f.stage.chmod(0o500)
                # Actual files/digests, but root ownership is a controlled
                # provider here. These are not protected product materials.
                original_lstat=Path.lstat
                def lstat(path):
                    value=original_lstat(path)
                    if path==f.stage:return NS(st_mode=value.st_mode,st_uid=0,st_dev=value.st_dev,st_ino=value.st_ino)
                    return value
                def measured(path,limit,*,mode):
                    metadata=path.stat();raw=path.read_bytes()
                    if metadata.st_mode & 0o777!=mode or not 0<len(raw)<=limit:raise ValueError('original file shape')
                    return dict(bytes=len(raw),sha256=list(hashlib.sha256(raw).digest()),device=metadata.st_dev,inode=metadata.st_ino)
                ended=stack.enter_context(patch.object(capture.CaptureHandoffProbe,'finish',return_value={'actual_worker_end':True}))
                stack.enter_context(patch.object(Path,'lstat',lstat))
                stack.enter_context(patch.object(publish,'original_json',side_effect=lambda path,_:json.loads(path.read_bytes())))
                stack.enter_context(patch.object(publish,'checked_file',side_effect=measured))
                try:
                    if fault=='none':
                        result=p.finish(time.monotonic()+5)
                        self.assertFalse(result['full_adopted']);self.assertFalse(result['capacity_released'])
                        self.assertEqual(len(result['complete_stage_files']),3)
                        self.assertTrue(result['callback_released_normally']);self.assertIn('not separately traced',result['syscall_errno'])
                    else:
                        with self.assertRaises(ValueError):p.finish(time.monotonic()+5)
                        self.assertIsNone(p.conflict)
                    ended.assert_called_once();self.assertTrue((f.store/f.obstacle.name).is_dir())
                finally:f.obstacle.close();f.stage.chmod(0o700)

    def test_only_confirmed_worker_exit_live_original_vm_and_retained_charge_allow_removal(self):
        for fault in ('none','end','worker','vm','charge'):
            with tempfile.TemporaryDirectory() as temporary,self.subTest(fault=fault):
                f=fixture(Path(temporary));p=f.probe;p.conflict=None if fault=='end' else {'original':'refusal'}
                if fault=='charge':p.assert_sync_continued.side_effect=ValueError('changed positive charge')
                with patch.object(publish,'pidfd_exited',return_value=fault!='worker'), \
                        patch.object(publish,'original_process',return_value=fault!='vm'):
                    try:
                        if fault=='none':
                            result=p.remove_after_continued(time.monotonic()+5)
                            self.assertTrue(result['obstacle']['removed']);self.assertFalse(result['retained_responsibility']['capacity_released'])
                        else:
                            with self.assertRaises(ValueError):p.remove_after_continued(time.monotonic()+5)
                            self.assertTrue((f.store/f.obstacle.name).is_dir())
                    finally:f.obstacle.close();f.stage.chmod(0o700)


class PublicationSceneTests(unittest.TestCase):
    def test_original_two_cycles_continue_then_remove_only_own_obstacle_before_new_unload(self):
        from tools.tests import test_lifecycle_sync_boundaries as sync
        sync.SyncSceneTests().run_scene('sync-after',publication=True)

    def test_unknown_original_end_sends_no_new_unload_destroy_or_obstacle_removal(self):
        from tools.tests import test_lifecycle_sync_boundaries as sync
        sync.SyncSceneTests().run_scene('sync-after',fail=True,publication=True)


if __name__=='__main__':unittest.main()
