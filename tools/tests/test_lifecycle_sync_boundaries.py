"""Original capture consumer with controlled providers; no product VM or deployment."""
import copy
from contextlib import ExitStack
import os
from pathlib import Path
import sys
import tempfile
import time
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'tests/system'))
import lifecycle_unload_capture_faults as capture
import lifecycle_unload_runtime as runtime
import lifecycle_unload_scenario as scene
from tools.tests import test_lifecycle_unload_runtime as original_fixture


class SyncProtocolTests(unittest.TestCase):
    def question(self,stage):
        return dict(schema_version=3,run_identity='a'*64,action={},worker={},source={},
                    full_work_binding=[1]*32,capture_key='d'*64,phase=capture.SYNC_PHASES[stage])

    def test_real_arm_uses_original_host_variable_allowlist_and_selected_schema(self):
        import _center_host_services as services
        for stage in capture.SYNC_PHASES:
            with tempfile.TemporaryDirectory() as directory:
                root=Path(directory);processes=NS(stop=Mock())
                def start(name,*,env):
                    self.assertEqual(name,'host-0')
                    self.assertEqual(services.fault_environment(env),
                        {'SANDPROOF_TEST_CAPTURE_SYNC_GATE':str(root/'capture-sync-gate')})
                    return NS(pid=101)
                processes.start=Mock(side_effect=start)
                runner=NS(lease=NS(identity='a'*64),quota={'deployment_control':str(root)},
                    services=NS(native_pools=[dict(namespace='0123456789abcdef')],
                                processes=processes,wait_ready=Mock()))
                prior=dict(session='original',context={'binding':{'generation':2}},
                           local={'sandbox':4},source={})
                p=capture.CaptureHandoffProbe(runner,NS(session='original'),prior,stage,root/'capture-sync-gate')
                p._configuration=Mock();p._save_file=Mock()
                try:
                    with patch.object(capture,'open_original_candidate_pidfd',return_value=None):
                        result=p.arm(time.monotonic()+5,1000)
                    self.assertTrue(result['armed']);processes.start.assert_called_once()
                    import json
                    config=json.loads((root/'capture-sync-gate/config.json').read_bytes())
                    self.assertEqual(config['schema_version'],3)
                    self.assertEqual(config['sync_phase'],'before-sync' if stage=='sync-before' else 'after-sync')
                    processes.stop.assert_called_once_with('host-0')
                finally:p.close()

    def test_each_sync_question_rejects_other_phase_measurement_extra_and_invalid_key(self):
        for stage in capture.SYNC_PHASES:
            question=self.question(stage)
            self.assertIs(capture.validate_sync_question(question,'a'*64,stage),question)
            for key,value in [('schema_version',2),('schema_version',True),('phase','measuring-cloned-artifact'),
                              ('phase',capture.SYNC_PHASES['sync-after' if stage=='sync-before' else 'sync-before']),
                              ('run_identity','b'*64),('capture_key','../other'),('capture_key','D'*64)]:
                bad=dict(question);bad[key]=value
                with self.subTest(stage=stage,key=key),self.assertRaises(ValueError):
                    capture.validate_sync_question(bad,'a'*64,stage)
            for bad in (dict(question,measured_bytes=0),dict(question,total_bytes=0),dict(question,published=True)):
                with self.assertRaises(ValueError):capture.validate_sync_question(bad,'a'*64,stage)
            with self.assertRaises(ValueError):capture.validate_measurement_question(question,'a'*64)

    def probe(self,stage):
        accepted,source,record,question=original_fixture.ContinuedExitConsumerTests().fixture()
        question.update(self.question(stage));question.update(action=record['capture']['action'],
            worker=record['capture']['worker'],source=record['work']['requirement']['source'])
        record['capture']['end']['observation'].update(outcome={'kind':'returned','http_status':500},
            full_work_binding=question['full_work_binding'])
        p=capture.CaptureHandoffProbe(NS(),NS(session='original'),dict(session='original',source=source),stage)
        p.action=question['action'];p.accepted=accepted;p.question=question;p.question_digest='c'*64
        p.config=dict(schema_version=3,run_identity='a'*64);p.deadline=time.monotonic()+5
        p.fd=123;p.worker_pidfd=456;p._gate_unchanged=Mock()
        p._read=Mock(return_value=(None,[record],[]))
        p._save_file=Mock(side_effect=lambda name: (dict(schema_version=3,run_identity='a'*64,
            observation_sha256='c'*64,full_work_binding=question['full_work_binding'],command='fail'),None))
        return p,record

    def test_finish_requires_actual_returned_end_worker_exit_and_exact_consumption(self):
        for stage in capture.SYNC_PHASES:
            for fault in ('none','no-request','success','changed-worker','adopted','consumption'):
                p,record=self.probe(stage)
                if fault=='no-request':record['capture']['end']['observation']['outcome']={'kind':'no_request'}
                if fault=='success':record['capture']['end']['observation']['outcome']['kind']='captured'
                if fault=='changed-worker':record['capture']['end']['observation']['worker']={}
                if fault=='adopted':record['capture']['completed']=[1]*32
                if fault=='consumption':p._save_file.return_value=({},None);p._save_file.side_effect=None
                with patch.object(capture,'gate_publish') as publish,patch.object(capture,'pidfd_exited',return_value=True), \
                        patch.object(capture,'original_process',return_value=False):
                    if fault=='none':
                        result=p.finish(p.deadline)
                        self.assertTrue(result['actual_worker_end']);self.assertFalse(result['capacity_released'])
                        self.assertTrue(p.recovered)
                    else:
                        with self.assertRaises(ValueError):p.finish(p.deadline)
                        self.assertFalse(p.recovered)
                    self.assertEqual(publish.call_args.args[2]['schema_version'],3)

    def test_live_worker_or_missing_end_keeps_failure_and_does_not_infer_completion(self):
        for fault in ('live-worker','missing-end'):
            p,record=self.probe('sync-before')
            if fault=='missing-end':record['capture']['end']=None
            with patch.object(capture,'gate_publish'),patch.object(capture,'pidfd_exited',return_value=False), \
                    patch.object(capture,'time',NS(monotonic=time.monotonic,sleep=Mock(side_effect=TimeoutError('bounded wait')))):
                with self.assertRaises(TimeoutError):p.finish(p.deadline)
            self.assertFalse(p.recovered)

    def test_sync_material_observes_original_unsealed_memory_metadata_and_refuses_publication(self):
        for stage,mode in [('sync-before',0o600),('sync-after',0o400)]:
            for fault in ('none','mode','sealed','completed','manifest','oversize','symlink'):
                with tempfile.TemporaryDirectory() as directory:
                    root=Path(directory);store=root/'s';store.mkdir(mode=0o700)
                    work=store/('staging-'+'d'*64);work.mkdir(mode=0o700)
                    memory=work/'memory.bin';memory.write_bytes(b'original');memory.chmod(mode)
                    if fault=='mode':memory.chmod(0o644)
                    if fault=='sealed':(store/('capture-'+'d'*64)).mkdir()
                    if fault=='completed':(store/('completed-'+'d'*64)).write_bytes(b'completion')
                    if fault=='manifest':(work/'manifest.json').write_bytes(b'{}')
                    if fault=='symlink':memory.unlink();memory.symlink_to(root/'elsewhere')
                    p,_=self.probe(stage);p.runtime=NS(captures=[dict(layout_parent=str(root))])
                    if fault=='none':
                        result=p._sync_material(100)
                        self.assertFalse(result['published']);self.assertFalse(result['syscall_failure_claimed'])
                        self.assertEqual(result['memory_mode'],oct(mode));self.assertEqual(result['memory_bytes'],8)
                    else:
                        with self.assertRaises((ValueError,OSError)):p._sync_material(1 if fault=='oversize' else 100)

    def test_same_inode_mode_change_during_metadata_read_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);store=root/'s';store.mkdir(mode=0o700)
            work=store/('staging-'+'d'*64);work.mkdir(mode=0o700)
            memory=work/'memory.bin';memory.write_bytes(b'original');memory.chmod(0o600)
            p,_=self.probe('sync-before');p.runtime=NS(captures=[dict(layout_parent=str(root))])
            original_fstat=os.fstat
            def changed(fd):
                result=original_fstat(fd)
                memory.chmod(0o400)  # same original inode; never a path-only replacement
                return result
            with patch.object(capture.os,'fstat',side_effect=changed) as reading:
                with self.assertRaises(ValueError):p._sync_material(100)
                reading.assert_called_once()
            self.assertEqual(memory.stat().st_mode & 0o777,0o400)

    def test_raw_gate_archive_is_exact_private_no_clobber_and_rejects_changed_bytes(self):
        import hashlib
        for fault in ('none','changed'):
            with tempfile.TemporaryDirectory() as directory:
                root=Path(directory);gate=root/'gate';gate.mkdir(mode=0o700)
                p,_=self.probe('sync-before');p.runtime=NS(root=root)
                p.fd=os.open(gate,os.O_RDONLY|os.O_DIRECTORY)
                try:
                    p.files={}
                    for name in ('config.json','observation.json','ready.json','command.json','consumed.json'):
                        path=gate/name;path.write_bytes(b'{"original":true}');path.chmod(0o600)
                        m=path.stat();p.files[name]=(hashlib.sha256(path.read_bytes()).hexdigest(),
                            (m.st_dev,m.st_ino,m.st_size,m.st_mtime_ns,m.st_ctime_ns,m.st_mode,m.st_uid,m.st_nlink))
                    if fault=='changed':(gate/'observation.json').write_bytes(b'{"changed":true}')
                    if fault=='none':
                        result=p.archive_sync_gate();self.assertEqual(len(result['files']),5)
                        for item in result['files']:
                            exported=root/result['directory']/item['name']
                            self.assertEqual(exported.read_bytes(),(gate/item['name']).read_bytes())
                            self.assertEqual(exported.stat().st_mode & 0o777,0o600)
                        with self.assertRaises(FileExistsError):p.archive_sync_gate()
                    else:
                        with self.assertRaises(ValueError):p.archive_sync_gate()
                        self.assertTrue((gate/'observation.json').exists())
                finally:os.close(p.fd)

    def test_continued_observation_preserves_charge_and_rejects_source_stop(self):
        for fault in ('none','adopted','missing-end','charge','stop'):
            p,record=self.probe('sync-after');p.recovered=True;p.sync_reserved_bytes=100;p.sync_material={}
            if fault=='adopted':record['capture']['completed']=[1]*32
            if fault=='missing-end':record['capture']['end']=None
            actions=[] if fault!='stop' else [(dict(command={'logical':{'command':{'StopVm':{}}}}), 'dispatched',None)]
            p._read.return_value=(None,[record],actions)
            with patch.object(capture,'material_source_charge',return_value=99 if fault=='charge' else 100):
                if fault=='none':self.assertFalse(p.assert_sync_continued(p.deadline)['capacity_released'])
                else:
                    with self.assertRaises(ValueError):p.assert_sync_continued(p.deadline)


class SyncSceneTests(unittest.TestCase):
    def run_scene(self,stage,fail=False):
        events=[];lifetime=dict(started_at_ms=1,end_at_ms=100,max_end_at_ms=200,startup_deadline_ms=20)
        clients={}
        for owner in ('1','2'):
            c=Mock(spec=scene.PublicUnloadScenario);c.session='session-'+owner;c.owner=owner
            c.current_acceptance=dict(key={},accepted_version=1);c.unload_acceptance=dict(key={},accepted_version=2)
            c.concurrent_unload_replies=[c.unload_acceptance]*2;c.resume_request={};c.witness={}
            c.timings=[dict(sent_monotonic=time.monotonic())];c.timing_report.return_value=[]
            c.wait_running.return_value=dict(phase='running');c.wait_unloaded.return_value=dict(phase='unloaded')
            c.wait_phase.return_value=dict(phase='running');c.query.return_value=dict(phase='running')
            c.assert_continued_workload.return_value={};c.last_http_status=None;c.failure_step='controlled'
            for name in ('create','unload','resume','destroy','run_public_workload','assert_resumed_workload','assert_original_replay'):
                getattr(c,name).side_effect=lambda *_,o=owner,n=name,**__:events.append(n+'-'+o) or {}
            clients[owner]=c
        calls=0
        def record(session,*_,**__):
            nonlocal calls
            if session=='session-1':calls+=1
            return dict(session=session,lifetime=lifetime,context=dict(position=0 if session=='session-2' or calls==1 else 1))
        probe=Mock(spec=capture.CaptureHandoffProbe)
        for name in ('arm','after_acceptance','cleanup'):
            getattr(probe,name).return_value={}
        def cut(*_):
            events.append('sync-fail')
            if fail:raise ValueError('unconfirmed original End')
            return dict(actual_worker_end=True)
        probe.finish.side_effect=cut
        probe.assert_sync_continued.return_value=dict(original_material_charge=100)
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            runner=NS(root=Path(directory),owner_key='owner',foreign_key='foreign',quota={'deployment_control':directory},
                services=NS(sdk_url='http://127.0.0.1:1'),measurement_identity=lambda:{},start=lambda _:None,
                create_parameters=lambda **_:{},record_running=Mock(side_effect=record),
                unload_evidence=Mock(return_value=dict(facts=lifetime,original_position=dict(position=0),original_capture_responsibilities=1)),
                restart_managers=Mock(return_value=dict(original_unload_operation={})),confirm_running=Mock(return_value={}),
                destroy_evidence=Mock(return_value={}),settlement_evidence=Mock(return_value={}),finish=Mock(),
                fail_resume_gate=Mock(return_value='preserved'),close_restore_gate=Mock(),close_capture_cancel_gate=Mock())
            stack.enter_context(patch.object(runtime,'prepare',return_value=runner))
            stack.enter_context(patch.object(scene,'PublicUnloadScenario',side_effect=lambda *_,owner='1':clients[owner]))
            stack.enter_context(patch.object(scene,'check_center_configuration',return_value={'deployment':'original'}))
            factory=stack.enter_context(patch.object(capture,'CaptureHandoffProbe',return_value=probe))
            if fail:
                with self.assertRaises(ValueError):scene.exercise(NS(scenario='capture-'+stage,scenario_timeout=60,activation_gate_timeout_ms=60000))
                self.assertFalse(runner.finish.call_args.args[0]['complete'])
                self.assertNotIn('destroy-1',events)
                self.assertEqual(events.count('unload-1'),2)
            else:
                result=scene.exercise(NS(scenario='capture-'+stage,scenario_timeout=60,activation_gate_timeout_ms=60000))
                self.assertTrue(result['complete']);self.assertEqual(len(result['cycles']),2)
                factory.assert_called_once();self.assertEqual(factory.call_args.args[3],stage)
                self.assertEqual(events.count('unload-1'),3)  # original, failed, then new-key recovery
                self.assertLess(events.index('sync-fail'),len(events)-1-events[::-1].index('unload-1'))
                self.assertEqual(events.count('destroy-1'),1);self.assertEqual(events.count('destroy-2'),1)
                self.assertIn('E11',result['acceptance_cases']);probe.assert_sync_continued.assert_called_once()
                probe.cleanup.assert_called_once()
            probe.close.assert_called_once()

    def test_both_sync_windows_continue_original_then_new_unload_resume_and_product_cleanup(self):
        for stage in capture.SYNC_PHASES:self.run_scene(stage)

    def test_unconfirmed_end_preserves_failure_without_new_unload_or_destroy(self):
        for stage in capture.SYNC_PHASES:self.run_scene(stage,fail=True)
