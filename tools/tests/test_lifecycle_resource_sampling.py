"""Real evidence files, controlled providers and a separate Linux procfs check."""
import copy
import hashlib
import json
import os
from pathlib import Path
from contextlib import ExitStack
import sys
import tempfile
import threading
import time
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'tests/system'))
import lifecycle_resource_sampling as sampling
import lifecycle_unload_scenario as scene


class SamplerFixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.volume = self.root/'volume'; self.volume.mkdir()
        self.clock = 100
        self.group = dict(path='original-private-path',identity=dict(device=1,inode=2),limits={'original':1},
            cpu_stat={'usage_usec':100},memory_current=4096,memory_peak=8192)
        self.roles = {name:dict(binary='/fixed/'+name,config='/private/'+name,uid=i+1000,gid=i+1000)
            for i,name in enumerate(('center','redis','host-0'))}
        self.children = {name:Mock(pid=11+i) for i,name in enumerate(self.roles)}
        for child in self.children.values():child.poll.return_value=None
        self.runtime = NS(root=self.root,quota={'mount':str(self.volume)},runs={},
            services=NS(manifest={'candidate':'a'*40},processes=NS(bindings=self.roles,children=self.children,record='original-record'),
                native_pools=[{'namespace':'original'}]))
        self.start_patch(patch.object(sampling,'boot_id',return_value='original-boot'))
        self.start_patch(patch.object(sampling,'private_path',side_effect=lambda p,**_:Path(p)))
        self.group_reader = self.start_patch(patch.object(sampling.service_cgroup,'observe',
            side_effect=lambda *_:copy.deepcopy(self.group)))
        self.facts = self.start_patch(patch.object(sampling,'process_facts',side_effect=self.process_facts))
        self.reader = self.start_patch(patch.object(sampling,'process_sample',side_effect=self.metrics))
        self.alive = True
        self.start_patch(patch.object(sampling,'original_process',side_effect=lambda _:self.alive))
        self.live = self.start_patch(patch.object(sampling,'live_identity',
            return_value=dict(pid=30,start_time_ticks=90,disk=[1,2,3])))
        self.start_patch(patch.object(sampling,'used_volume_bytes',return_value=1000))
        self.probe = sampling.LifecycleResources(self.runtime,phase=lambda:'original-phase',
            interval_ms=500,deadline=time.monotonic()+30)
        self.addCleanup(self.close_probe)

    def start_patch(self, item):
        self.addCleanup(item.stop)
        return item.start()

    def close_probe(self):
        if not self.probe.closed:self.probe.close()

    def process_facts(self, child,*_,uid,gid,**__):
        return dict(pid=child.pid,start_time_ticks=child.pid*3,uid=uid,gid=gid,
            executable={'device':1,'inode':child.pid})

    def metrics(self, identity):
        return dict(cpu_ticks=self.clock+identity['pid'],rss_bytes=self.clock*4096,
            kernel_process_peak_rss_bytes=1000*4096)

    def add_vm(self):
        self.runtime.runs['session'] = dict(source=dict(pid=30,start_time_ticks=90,
            boot_id='original-boot'),context=dict(position=0,binding=dict(owner=1,sandbox=1,generation=1,vm=1)))


class LifecycleSamplerTests(SamplerFixture):
    def test_raw_hash_original_epochs_volume_and_cpu_units_are_preserved(self):
        self.add_vm(); self.probe.sample(); self.clock+=7; self.probe.sample()
        result=self.probe.close()
        raw=self.probe.path.read_bytes();rows=[json.loads(line) for line in raw.splitlines()]
        self.assertEqual(len(rows),3)
        self.assertEqual([r['sequence'] for r in rows],[0,1,2])
        self.assertEqual(result['raw']['sha256'],hashlib.sha256(raw).hexdigest())
        self.assertEqual(result['raw']['bytes'],len(raw))
        self.assertEqual(result['coverage'],'sampled_windows')
        self.assertEqual(len(result['epochs']),4)
        self.assertTrue(all(e['observed_cpu_ticks_delta']==7 for e in result['epochs']))
        self.assertEqual(result['volume']['baseline_used_bytes'],1000)
        self.assertEqual(result['volume']['sampled_peak_used_bytes'],1000)
        self.assertNotIn('original-private-path',json.dumps(result))
        self.assertNotIn('/private/',raw.decode())
        self.assertTrue(result['sampler_exited'])
        with self.assertRaises(ValueError):self.probe.close()

    def test_restart_creates_new_epoch_without_carrying_cpu_or_rebinding_old_pid(self):
        self.probe.sample();old=self.children['center'];old.poll.return_value=0
        self.probe.sample()
        replacement=Mock(pid=40);replacement.poll.return_value=None
        self.children['center']=replacement;self.clock=1
        # Old Redis/Host counters may not regress; isolate the new center's
        # counter while preserving their original values.
        self.reader.side_effect=lambda ident:dict(cpu_ticks=1 if ident['pid']==40 else 100+ident['pid'],
            rss_bytes=4096,kernel_process_peak_rss_bytes=8192)
        self.probe.sample();result=self.probe.close()
        centers=[e for e in result['epochs'] if e['role']=='center']
        self.assertEqual([e['identity']['pid'] for e in centers],[11,40])
        self.assertEqual([e['observed_cpu_ticks_delta'] for e in centers],[0,0])
        self.assertEqual(centers[0]['inactive_samples'],1)

    def test_missing_samples_are_null_unknown_not_zero(self):
        self.reader.side_effect=PermissionError('controlled proc reader')
        self.probe.sample();result=self.probe.close()
        self.assertEqual(result['coverage'],'partial')
        self.assertGreater(result['unknown_observations'],0)
        self.assertTrue(all(e['sampled_peak_rss_bytes'] is None and e['observed_cpu_ticks_delta'] is None
            for e in result['epochs']))
        self.assertEqual(json.loads(self.probe.path.read_text().splitlines()[0])['services'][0]['state'],'unknown')

    def test_existing_process_identity_or_cpu_regression_never_becomes_new_epoch(self):
        self.probe.sample();original=self.facts.side_effect
        self.facts.side_effect=lambda *a,**kw:dict(original(*a,**kw),start_time_ticks=999)
        with self.assertRaises(ValueError):self.probe.sample()
        result=self.probe.close()
        self.assertEqual(result['failure'],'ValueError')
        self.assertEqual(len(result['epochs']),3)

    def test_original_cgroup_and_boot_changes_fail_the_sampler(self):
        self.probe.sample();self.group['identity']['inode']=3
        result=self.probe.close();self.assertEqual(result['failure'],'ValueError')

    def test_group_counter_regression_is_not_an_empty_measurement(self):
        self.probe.sample();self.group['cpu_stat']['usage_usec']=99
        result=self.probe.close();self.assertEqual(result['failure'],'ValueError')

    def test_volume_replacement_is_not_adopted(self):
        self.probe.sample();self.volume.rename(self.root/'old-volume');self.volume.mkdir()
        result=self.probe.close();self.assertEqual(result['failure'],'ValueError')

    def test_recorded_vm_exit_is_explicit_and_does_not_sample_a_pid_alias(self):
        self.add_vm();self.probe.sample();self.alive=False
        result=self.probe.close();vm=result['epochs'][-1]
        self.assertEqual(vm['samples'],1);self.assertEqual(vm['inactive_samples'],1)
        row=json.loads(self.probe.path.read_text().splitlines()[-1])
        self.assertEqual(row['vms'],[dict(epoch=vm['index'],state='not_live')])

    def test_vm_still_live_with_replaced_resources_fails_instead_of_refreshing(self):
        self.add_vm();self.probe.sample();self.live.return_value={'pid':31,'start_time_ticks':91}
        result=self.probe.close();self.assertEqual(result['failure'],'ValueError')

    def test_raw_file_replacement_is_rejected_and_descriptors_still_close(self):
        self.probe.sample();self.probe.path.rename(self.root/'original-raw');self.probe.path.write_text('foreign')
        with self.assertRaises(ValueError):self.probe.close()
        self.assertTrue(self.probe.closed)
        self.assertIsNone(self.probe.raw_digest)
        with self.assertRaises(OSError):os.fstat(self.probe.fd)

    def test_row_capacity_keeps_the_original_prefix_and_records_failure(self):
        with patch.object(sampling,'MAX_SAMPLES',1):
            self.probe.sample();result=self.probe.close()
        self.assertEqual(result['failure'],'TimeoutError');self.assertEqual(result['samples'],1)
        self.assertEqual(len(self.probe.path.read_text().splitlines()),1)

    def test_background_thread_has_a_confirmed_exit(self):
        reached=threading.Event();original=self.reader.side_effect
        def read(identity):
            if threading.current_thread() is self.probe.thread:reached.set()
            return original(identity)
        self.reader.side_effect=read
        self.probe.start();self.assertTrue(reached.wait(3))
        result=self.probe.close()
        self.assertFalse(self.probe.thread.is_alive());self.assertTrue(result['sampler_exited'])


class SamplingInputTests(unittest.TestCase):
    def test_defaults_and_invalid_options_before_deployment(self):
        self.assertEqual(sampling.options(NS()),(False,500))
        import lifecycle_unload_runtime as runtime
        for extra in (dict(measure_resources=1),dict(resource_sample_interval_ms=True),
                      dict(resource_sample_interval_ms=499),dict(resource_sample_interval_ms=5001)):
            with self.subTest(extra=extra),patch.object(runtime,'prepare') as prepare,self.assertRaises(ValueError):
                scene.exercise(NS(scenario='normal',scenario_timeout=60,**extra))
            prepare.assert_not_called()

    def test_stat_layout_parentheses_and_no_double_count_of_guest_or_child_time(self):
        fields=[b'S']+[b'0']*49
        fields[11],fields[12],fields[13],fields[19],fields[40]=b'21',b'3',b'999',b'17',b'21'
        raw=b'123 (has ) spaces) '+b' '.join(fields)
        self.assertEqual(sampling.stat_fields(raw,123),dict(start_time_ticks=17,cpu_ticks=24))
        with self.assertRaises(ProcessLookupError):sampling.stat_fields(raw,124)


class ResourceReportTests(SamplerFixture):
    def report(self):
        self.add_vm();self.probe.sample();resources=self.probe.close()
        return dict(schema=1,complete=True,scenario='normal',resource_measurements=resources,
            measurement_identity=dict(candidate=dict(candidate='a'*40)),
            operation_timings=[dict(operation='resume',complete=True,send_to_public_completion_seconds=n)
                for n in (3,1,2)]+[dict(operation='destroy',complete=False)])

    def test_report_verifies_original_stream_and_recomputes_percentiles(self):
        from tools.lifecycle_measurement_report import render
        report=self.report();text=render(report,self.probe.path)
        self.assertIn('| resume | 3 | 2.000000 | 3.000000 |',text)
        self.assertIn('未完成操作 1 项',text)
        self.assertIn('不是完整 E16',text)
        self.assertNotIn('original-private-path',text)
        self.assertNotIn('/private/',text)

    def test_changed_summary_identity_volume_missing_counts_or_timings_are_rejected(self):
        from tools.lifecycle_measurement_report import render
        report=self.report()
        for fault in ('cpu','pid','volume','unknown','hash','incomplete','nan'):
            bad=copy.deepcopy(report);resource=bad['resource_measurements']
            if fault=='cpu':resource['epochs'][0]['observed_cpu_ticks_delta']=999
            elif fault=='pid':resource['epochs'][0]['identity']['pid']=999
            elif fault=='volume':resource['volume']['sampled_peak_used_bytes']+=1
            elif fault=='unknown':resource['unknown_observations']+=1
            elif fault=='hash':resource['raw']['sha256']='0'*64
            elif fault=='incomplete':bad['complete']=False
            else:bad['operation_timings'][0]['send_to_public_completion_seconds']=float('nan')
            with self.subTest(fault=fault),self.assertRaises(ValueError):render(bad,self.probe.path)

    def test_modified_or_truncated_raw_stream_cannot_render_a_success(self):
        from tools.lifecycle_measurement_report import render
        report=self.report();raw=self.probe.path.read_bytes()
        for bad in (raw[:-1],raw+b'{}\n',raw.replace(b'"sequence":0',b'"sequence":8')):
            self.probe.path.write_bytes(bad)
            with self.assertRaises((ValueError,KeyError)):render(report,self.probe.path)


class ResourceSceneTests(unittest.TestCase):
    def run_scene(self,enabled,close_failure=False):
        import lifecycle_unload_runtime as runtime
        import lifecycle_unload_cow as cow
        import lifecycle_unload_retirement as retirement
        events=[];lifetime=dict(started_at_ms=1,end_at_ms=100,max_end_at_ms=200,startup_deadline_ms=20)
        clients={}
        for owner in ('1','2'):
            client=Mock(spec=scene.PublicUnloadScenario)
            client.owner,client.session=owner,'session-'+owner
            client.current_acceptance=dict(key={},accepted_version=1)
            client.unload_acceptance=dict(key={},accepted_version=2)
            client.concurrent_unload_replies=[client.unload_acceptance]*2
            client.resume_request={};client.witness={}
            client.timings=[dict(sent_monotonic=time.monotonic())];client.timing_report.return_value=[]
            client.wait_running.return_value=dict(phase='running');client.wait_unloaded.return_value=dict(phase='unloaded')
            client.query.return_value=dict(phase='running')
            for name in ('create','unload','resume','destroy','run_public_workload','assert_resumed_workload'):
                getattr(client,name).side_effect=lambda *_,o=owner,n=name,**__:events.append(n+'-'+o) or {}
            clients[owner]=client
        calls=0
        def record(session,*_,**__):
            nonlocal calls
            if session=='session-1':calls+=1
            return dict(session=session,lifetime=lifetime,context=dict(position=0 if session=='session-2' or calls==1 else 1))
        probe=Mock();probe.closed=False
        probe.start.side_effect=lambda:events.append('sampler-start')
        def close():
            events.append('sampler-close')
            if close_failure:raise RuntimeError('controlled observer failure')
            probe.closed=True
            return dict(failure=None,sampler_exited=True,coverage='sampled_windows')
        probe.close.side_effect=close
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            runner=NS(root=Path(directory),owner_key='owner',foreign_key='foreign',
                services=NS(sdk_url='http://127.0.0.1:1',storage_diagnostics=Mock(return_value={'original':'storage'})),
                measurement_identity=lambda:{},start=lambda _:events.append('services-ready'),create_parameters=lambda **_:{},
                record_running=Mock(side_effect=record),unload_evidence=Mock(return_value=dict(facts=lifetime,
                    original_position=dict(position=0),original_capture_responsibilities=1)),
                restart_managers=Mock(return_value=dict(original_unload_operation={})),confirm_running=Mock(return_value={}),
                destroy_evidence=Mock(return_value={}),settlement_evidence=Mock(return_value={}),
                finish=Mock(side_effect=lambda report:events.append('finish-'+str(report['complete']))),
                fail_resume_gate=Mock(return_value='preserved'),close_restore_gate=Mock(),close_capture_cancel_gate=Mock())
            factory=stack.enter_context(patch.object(sampling,'LifecycleResources',return_value=probe))
            stack.enter_context(patch.object(runtime,'prepare',return_value=runner))
            stack.enter_context(patch.object(scene,'PublicUnloadScenario',side_effect=lambda *_,owner='1':clients[owner]))
            stack.enter_context(patch.object(scene,'check_center_configuration',return_value={'deployment':'original'}))
            sealed=Mock();sealed.verify_after_guest_write.return_value={}
            stack.enter_context(patch.object(cow,'SealedSourceProbe',return_value=sealed))
            stack.enter_context(patch.object(retirement,'wait_settled',return_value=dict(receipt={},snapshot={})))
            stack.enter_context(patch.object(retirement,'assert_replay_unchanged'))
            result=scene.exercise(NS(scenario='normal',scenario_timeout=60,measure_resources=enabled))
            self.assertEqual(len(result['cycles']),3)
            self.assertEqual(result['timing_scope']['e16_complete'],False)
            if enabled:
                factory.assert_called_once()
                self.assertLess(events.index('services-ready'),events.index('sampler-start'))
                self.assertLess(events.index('sampler-close'),events.index('finish-'+str(not close_failure)))
                self.assertEqual(result['resource_measurements']['service_storage'],{'original':'storage'})
                self.assertEqual(result['complete'],not close_failure)
            else:
                factory.assert_not_called();self.assertNotIn('resource_measurements',result)
                self.assertTrue(result['complete'])
            self.assertEqual(events.count('destroy-1'),1);self.assertEqual(events.count('destroy-2'),1)
            return result

    def test_enabled_and_default_flows_keep_original_three_cycles_and_cleanup_order(self):
        self.run_scene(False);self.run_scene(True)

    def test_observer_close_failure_keeps_failure_and_product_responsibilities(self):
        result=self.run_scene(True,close_failure=True)
        self.assertEqual(result['resource_measurements']['failure'],'RuntimeError')


class LinuxResourceProcessTests(unittest.TestCase):
    def test_real_procfs_rss_cpu_and_wrong_start_refusal(self):
        pid=os.getpid();start=sampling.process_start_ticks(pid)
        observed=sampling.process_sample(dict(pid=pid,start_time_ticks=start))
        self.assertGreater(observed['rss_bytes'],0)
        self.assertGreaterEqual(observed['cpu_ticks'],0)
        self.assertGreater(observed['kernel_process_peak_rss_bytes'],0)
        with self.assertRaises(ValueError):sampling.process_sample(dict(pid=pid,start_time_ticks=start+1))
