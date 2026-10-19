"""Pure mandatory-DK launcher/identity checks; never a DK or KVM pass."""
import argparse
import copy
from contextlib import contextmanager, ExitStack
import json
import http.server
import threading
from pathlib import Path
import sys
import subprocess
import tempfile
from types import ModuleType, SimpleNamespace
import unittest
from unittest.mock import Mock, patch
from tools.tests.storage_observation_fixture import declaration, observation

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'tests/system'))
import center_host_faults as runner
import _center_fault_harness as harness
import _runtime_services as services
import _center_fault_cases as cases


class CenterHostFaultsTests(unittest.TestCase):
    def center_listener_fixture(self):
        value=harness.Harness.__new__(harness.Harness)
        value.args=SimpleNamespace(wait_seconds=5)
        value.next_center_audit=0
        value.services=Mock()
        value.services.center_config=dict(expiry_scan_ms=1000,poll_interval_ms=100)
        value.services.processes.children={'center':Mock()}
        value.services.processes.children['center'].poll.return_value=None
        value.services.wait_ready.side_effect=AssertionError('partitioned host must not need fresh health')
        value.key='synthetic-listener-key'
        value.opener=harness.urllib.request.build_opener(harness.urllib.request.ProxyHandler({}))
        snapshot={'original-offline-snapshot':True}
        value.services.planned_audit.return_value=snapshot
        return value,snapshot

    def test_planned_center_audit_waits_for_real_HTTP_but_keeps_offline_facts_and_partition(self):
        value,snapshot=self.center_listener_fixture()
        requests=[]
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                requests.append((self.path,self.headers.get('X-API-Key')))
                self.send_response(200)
                self.send_header('Content-Length','2')
                self.end_headers()
                self.wfile.write(b'[]')
            def log_message(self,*args):
                pass
        server=http.server.HTTPServer(('127.0.0.1',0),Handler,bind_and_activate=False)
        server.server_bind()  # Real bound port, deliberately not listening yet.
        value.services.sdk_url='http://127.0.0.1:'+str(server.server_address[1])
        thread=None
        entered=threading.Event()
        def start(_seconds):
            nonlocal thread
            self.assertIsNone(thread,'original listener still unavailable after controlled startup')
            server.server_activate()
            def serve():
                entered.set()
                server.serve_forever()
            thread=threading.Thread(target=serve,daemon=True)
            thread.start()
            self.assertTrue(entered.wait(5))
        try:
            with patch.object(harness.time,'sleep',side_effect=start):
                self.assertIs(value.center(),snapshot)
            self.assertEqual(requests,[('/v2/sandboxes','synthetic-listener-key')])
            value.services.planned_audit.assert_called_once_with('center')
            value.services.wait_ready.assert_not_called()
        finally:
            if thread is not None:
                server.shutdown()
                thread.join(timeout=5)
                self.assertFalse(thread.is_alive())
            server.server_close()

    def test_center_listener_timeout_never_returns_an_audit_as_online_readiness(self):
        value,_=self.center_listener_fixture()
        value.services.sdk_url='http://127.0.0.1:1'
        now=[100.0]
        def advance(_seconds):
            now[0]+=1
        with patch.object(harness.time,'monotonic',side_effect=lambda:now[0]), \
             patch.object(harness.time,'sleep',side_effect=advance), \
             patch.object(harness,'fetch',return_value=(503,b'')) as fetch:
            with self.assertRaisesRegex(RuntimeError,'HTTP listener remained unavailable'):
                value.center()
        self.assertEqual(fetch.call_count,5)
        self.assertTrue(all(call.args[2:4]==('GET','/v2/sandboxes') for call in fetch.call_args_list))
        value.services.wait_ready.assert_not_called()

    def test_center_exit_after_audit_never_probes_an_unowned_listener(self):
        value,_=self.center_listener_fixture()
        value.services.processes.children['center'].poll.return_value=1
        with patch.object(harness,'fetch') as fetch:
            with self.assertRaisesRegex(RuntimeError,'original center exited'):
                value.center()
        fetch.assert_not_called()
        value.services.wait_ready.assert_not_called()

    def redis_fault_fixture(self):
        identity=dict(sandbox_id='original-sandbox',node=dict(node_id='original-node'))
        value=Mock()
        value.create.return_value=SimpleNamespace(sandbox_id=identity['sandbox_id'])
        value.known={identity['sandbox_id']:dict(create=identity,physical=dict(pid=123))}
        value.center.return_value=dict(sandboxes=[dict(create=dict(identity=identity))])
        value.services.processes.children={'redis':SimpleNamespace(pid=456)}
        module=ModuleType('e2b');module.Sandbox=Mock()
        return value,module

    def test_redis_fault_refuses_to_signal_if_restarted_center_is_not_ready(self):
        value,module=self.redis_fault_fixture()
        value.services.wait_ready.side_effect=RuntimeError('original readiness unobserved')
        with patch.dict(sys.modules,{'e2b':module}),patch.object(cases.os,'kill') as kill:
            with self.assertRaisesRegex(RuntimeError,'original readiness unobserved'):
                cases.dk05(value)
        value.services.wait_ready.assert_called_once_with(for_admission=False)
        value.services.processes.observe.assert_not_called()
        kill.assert_not_called()

    def test_redis_fault_begins_only_after_original_online_readiness(self):
        value,module=self.redis_fault_fixture()
        events=[]
        value.services.wait_ready.side_effect=lambda **kwargs:events.append('ready')
        def observe():
            events.append('observe')
            return {'host-0':dict(pid=123)}
        value.services.processes.observe.side_effect=observe
        class BarrierReached(Exception):
            pass
        def stop(pid,number):
            self.assertEqual((pid,number),(456,cases.signal.SIGSTOP))
            self.assertEqual(events,['ready','observe'])
            raise BarrierReached()
        with patch.dict(sys.modules,{'e2b':module}),patch.object(cases.os,'kill',side_effect=stop):
            with self.assertRaises(BarrierReached):
                cases.dk05(value)
        value.services.wait_ready.assert_called_once_with(for_admission=False)
        value.fetch.assert_not_called()

    def test_recovery_constructor_uses_exact_required_sdk_fields_without_create_or_renew(self):
        # No **kwargs on these fixtures: the previous incomplete constructor
        # must fail just as it did in the real pinned SandboxBase signature.
        class StrictVersion:
            def __init__(self, value):
                self.value=value
        class StrictConnection:
            def __init__(self, *, api_url, sandbox_url, api_key, domain, retries,
                         request_timeout, extra_sandbox_headers):
                self.values=dict(api_url=api_url,sandbox_url=sandbox_url,api_key=api_key,
                    domain=domain,retries=retries,request_timeout=request_timeout,
                    extra_sandbox_headers=extra_sandbox_headers)
        class StrictSandbox:
            def __init__(self, sandbox_id, envd_version, envd_access_token,
                         sandbox_domain, connection_config, traffic_access_token=None):
                self.sandbox_id=sandbox_id
                self.version=envd_version
                self.token=envd_access_token
                self.domain=sandbox_domain
                self.connection=connection_config
            create=Mock(side_effect=AssertionError('unexpected create'))
            connect=Mock(side_effect=AssertionError('unexpected renew'))
            set_timeout=Mock(side_effect=AssertionError('unexpected renew'))
        modules={name:ModuleType(name) for name in (
            'e2b','e2b.connection_config','packaging','packaging.version')}
        modules['e2b'].Sandbox=StrictSandbox
        modules['e2b.connection_config'].ConnectionConfig=StrictConnection
        modules['packaging.version'].Version=StrictVersion
        value=dict(domain='actual-detail.invalid',envdVersion='0.5.14',
                   envdAccessToken='synthetic-guest-token')
        options=dict(api_url='http://127.0.0.1:12345',sandbox_url='http://127.0.0.1:12345',
                     api_key='synthetic-owner-key',domain='fallback.invalid',retries=0,request_timeout=30)
        probe=SimpleNamespace(fetch=Mock(return_value=(200,json.dumps(value).encode())),
                              sdk_options=Mock(return_value=options))
        with patch.dict(sys.modules,modules):
            sandbox=cases.sdk_from_detail(probe,'exact-original-sandbox')
        probe.fetch.assert_called_once_with('GET','/sandboxes/exact-original-sandbox')
        self.assertEqual(sandbox.sandbox_id,'exact-original-sandbox')
        self.assertEqual(sandbox.domain,value['domain'])
        self.assertEqual(sandbox.version.value,value['envdVersion'])
        self.assertEqual(sandbox.token,value['envdAccessToken'])
        self.assertEqual(sandbox.connection.values,dict(options,
            extra_sandbox_headers={'X-Access-Token':value['envdAccessToken']}))
        for method in (StrictSandbox.create,StrictSandbox.connect,StrictSandbox.set_timeout):
            method.assert_not_called()

    def test_finish_requires_original_storage_evidence_and_stop_updates_the_same_report_after_exit(self):
        value=harness.Harness.__new__(harness.Harness)
        value.manifest=dict(services=dict(candidate='a'*40),service_data=declaration())
        value.execution_sha='b'*64
        value.last_processes={}
        value.cases=[dict(id='DK0'+str(index),status='pass',exact_cleanup='passed',
            execution_manifest_sha256='b'*64,evidence_sha256='c'*64) for index in range(1,7)]
        value.services=Mock()
        value.relay=Mock()
        value.materials=Mock()
        with patch.object(harness.ci_e2e_artifacts,'public_identity',return_value={'candidate':'a'*40}), \
                patch.object(harness.ci_e2e_artifacts,'require_service_boundary'), \
                patch.object(harness.ci_e2e_artifacts,'require_process_evidence'):
            value.services.storage_peaks.side_effect=ValueError('original budget exceeded')
            with self.assertRaises(ValueError):
                value.finish()
            self.assertFalse(hasattr(value,'finished_report'))
            value.services.storage_peaks.side_effect=None
            value.services.storage_peaks.return_value=dict(observation(),sampler_exited=False)
            report=value.finish()
        self.assertIs(report['dk']['service_storage']['sampler_exited'],False)
        self.assertEqual(report['dk']['service_data'],value.manifest['service_data'])
        self.assertIsNot(report['dk']['service_data'],value.manifest['service_data'])
        value.services.storage_peaks.return_value=observation()
        value.stop(successful=True)
        self.assertEqual(report['dk']['service_storage'],observation())
        storage_calls=[index for index,call in enumerate(value.services.mock_calls)
                       if call==unittest.mock.call.storage_peaks()]
        self.assertLess(value.services.mock_calls.index(unittest.mock.call.stop_all()),storage_calls[-1])
        value.services.cleanup_staging.assert_called_once()

    def test_finish_stop_cannot_accept_an_observation_from_another_original_cohort(self):
        value=harness.Harness.__new__(harness.Harness)
        value.manifest=dict(services=dict(candidate='a'*40),service_data=declaration())
        value.execution_sha='b'*64
        value.last_processes={}
        value.cases=[dict(id='DK0'+str(index),status='pass',exact_cleanup='passed',
            execution_manifest_sha256='b'*64,evidence_sha256='c'*64) for index in range(1,7)]
        value.services=Mock()
        value.relay=Mock()
        value.materials=Mock()
        with patch.object(harness.ci_e2e_artifacts,'public_identity',return_value={'candidate':'a'*40}), \
                patch.object(harness.ci_e2e_artifacts,'require_service_boundary'), \
                patch.object(harness.ci_e2e_artifacts,'require_process_evidence'):
            value.services.storage_peaks.return_value=dict(observation(),sampler_exited=False)
            value.finish()
        foreign=observation()
        foreign['roots'][0]['identity'][1] += 1
        value.services.storage_peaks.return_value=foreign
        with self.assertRaisesRegex(ValueError,'original execution declaration'):
            value.stop(successful=True)
        value.services.cleanup_staging.assert_not_called()
        value.materials.remove.assert_not_called()

    def test_unknown_final_storage_never_removes_original_redis_or_materials(self):
        value=harness.Harness.__new__(harness.Harness)
        value.services=Mock()
        value.relay=Mock()
        value.materials=Mock()
        value.finished_report={'dk':{'candidate':'a'*40}}
        value.services.storage_peaks.side_effect=ValueError('final observation unknown')
        with self.assertRaises(ValueError):
            value.stop(successful=True)
        value.services.stop_all.assert_called_once()
        value.services.cleanup_staging.assert_not_called()
        value.materials.remove.assert_not_called()

    def test_failure_diagnostics_include_bounded_code_locations_not_credentials_or_paths(self):
        secret='credential-never-print-this'
        try:
            runner.failure_summary(17)  # Exercise a frame in approved source.
        except AttributeError as error:
            error.args=(secret,)
            value=runner.failure_summary(error)
        else:
            self.fail('expected diagnostic error')
        self.assertEqual(value['failure_type'],'AttributeError')
        self.assertTrue(any(frame['file']=='center_host_faults.py' for frame in value['frames']))
        self.assertLessEqual(len(value['frames']),12)
        raw=json.dumps(value)
        self.assertNotIn(secret,raw)
        self.assertNotIn(str(ROOT),raw)
        self.assertLess(len(raw),4096)

    def test_fault_harness_imports_the_original_service_activation_ceiling(self):
        self.assertEqual(harness.MAX_ACTIVATION_BYTES, services.MAX_ACTIVATION_BYTES)
        self.assertEqual(harness.MAX_ACTIVATION_BYTES, 200 * 1024**3)

    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name).resolve()
        self.root.chmod(0o700)
        profile=self.root/'profile.json'
        profile.write_text(json.dumps(dict(schema_version=1,runtime={},builder={},host=dict(
            control_cpu_allowance=1,cpu_headroom=1,
            control_memory_allowance_bytes=2*1024**3,memory_headroom_bytes=2*1024**3))))
        profile.chmod(0o600)
        self.args=argparse.Namespace(run_identity='exact-test-01234567',rounds=6,wait_seconds=90,
            checkpoint_tail_inputs=128,
            stale_probe_sha256='a'*64,uid_base=2000,run_root=self.root/'run',
            scratch_parent=self.root,service_cgroup_record=self.root/'run/service-cgroup.json',
            cgroup_parent=Path('/fixed/vms'),service_cgroup_parent=Path('/fixed/services'),
            service_profile=profile,worker=False,
            **{key:self.root/key for key in ('center','host','kernel','rootfs','jailer','firecracker',
                'stale_probe','candidate_manifest')})

    def validate(self):
        with patch.object(runner.sys,'platform','linux'),patch.object(runner.os,'geteuid',return_value=0), \
                patch.object(runner.importlib.metadata,'version',return_value='2.51.0'), \
                patch.object(runner,'trusted_path',side_effect=lambda path:path), \
                patch.object(runner,'private_path',side_effect=lambda path,**_:Path(path)), \
                patch.object(runner,'digest',return_value='a'*64):
            return runner.validate(self.args)

    def operator_barrier(self, action='host-failure'):
        value=harness.Harness.__new__(harness.Harness)
        value.root=self.root
        value.args=self.args
        value.manifest=dict(services=dict(candidate='b'*40))
        value.execution_sha='c'*64
        value.services=Mock()
        report=dict(mappings=[{'original':1},{'original':2}],identities=[{'pid':11},{'pid':12}])
        control=dict(schema_version=1,candidate='b'*40,run_id=self.args.run_identity,
                     execution_manifest_sha256='c'*64,action=action)
        return value,report,control

    def write_operator_control(self, control):
        path=self.root/'operator-dk06-control.json'
        path.write_text(json.dumps(control))
        path.chmod(0o600)
        return path

    def test_normal_fault_group_does_not_wait_or_create_an_operator_stage(self):
        value,report,_=self.operator_barrier()
        with patch.object(harness.time,'sleep') as sleep,patch.object(harness,'write_once') as write:
            value.operator_checkpoint(report)
        sleep.assert_not_called()
        write.assert_not_called()
        value.services.processes.observe.assert_not_called()

    def test_fault_barrier_captures_both_original_identities_before_observing_host_exit(self):
        value,report,control=self.operator_barrier()
        self.write_operator_control(control)
        value.services.processes.children={'host-0':Mock()}
        value.services.processes.children['host-0'].poll.side_effect=[None,93]
        with patch.object(harness,'private_path',side_effect=lambda path:Path(path)), \
                patch.object(harness,'digest',return_value='d'*64), \
                patch.object(harness.time,'monotonic',side_effect=[0,1]), \
                patch.object(harness.time,'sleep') as sleep, \
                self.assertRaisesRegex(RuntimeError,'original host exited'):
            value.operator_checkpoint(report)
        saved=json.loads((self.root/'operator-dk06-stage.json').read_text())
        self.assertEqual(saved,dict(control,stage='original_pair_ready',
            mappings=report['mappings'],identities=report['identities'],producer_sha256='d'*64))
        value.services.processes.observe.assert_called_once()
        value.services.processes.start.assert_not_called()
        value.services.processes.stop.assert_not_called()
        sleep.assert_called_once_with(.1)

    def test_barrier_deadline_is_the_original_case_deadline_and_cannot_be_resumed_as_pass(self):
        value,report,control=self.operator_barrier()
        self.write_operator_control(control)
        value.services.processes.children={'host-0':Mock(poll=Mock(return_value=None))}
        with patch.object(harness,'private_path',side_effect=lambda path:Path(path)), \
                patch.object(harness,'digest',return_value='d'*64), \
                patch.object(harness.time,'monotonic',side_effect=[0,self.args.wait_seconds]), \
                patch.object(harness.time,'sleep') as sleep, \
                self.assertRaises(TimeoutError):
            value.operator_checkpoint(report)
        sleep.assert_not_called()
        self.assertTrue((self.root/'operator-dk06-stage.json').exists())

    def test_cancel_at_barrier_propagates_without_starting_or_stopping_a_vm(self):
        value,report,control=self.operator_barrier('cancel')
        self.write_operator_control(control)
        value.services.processes.children={'host-0':Mock(poll=Mock(return_value=None))}
        with patch.object(harness,'private_path',side_effect=lambda path:Path(path)), \
                patch.object(harness,'digest',return_value='d'*64), \
                patch.object(harness.time,'sleep',side_effect=KeyboardInterrupt()), \
                self.assertRaises(KeyboardInterrupt):
            value.operator_checkpoint(report)
        value.services.processes.start.assert_not_called()
        value.services.processes.stop.assert_not_called()

    def test_wrong_operator_cohort_action_or_incomplete_pair_cannot_publish_stage(self):
        value,report,control=self.operator_barrier()
        variants=[dict(control,**{field:'wrong'}) for field in
                  ('candidate','run_id','execution_manifest_sha256','action')]
        variants += [dict(control,extra=True),dict(control,schema_version=True),control]
        for number,changed in enumerate(variants):
            with self.subTest(number=number):
                path=self.write_operator_control(changed)
                current=report if changed is not control else dict(report,identities=report['identities'][:1])
                with patch.object(harness,'private_path',side_effect=lambda path:Path(path)), \
                        patch.object(harness,'write_once') as write,self.assertRaises(ValueError):
                    value.operator_checkpoint(current)
                write.assert_not_called()
                path.unlink()

    def test_only_original_finite_service_budget_is_forwarded_not_a_second_capacity_policy(self):
        self.assertEqual(self.validate(),dict(cpu_quota_us=100000,memory_bytes=2*1024**3))
        for name,value in (('rounds',True),('rounds',5),('rounds',7),('rounds',17),('wait_seconds',1),
                           *[('checkpoint_tail_inputs',item) for item in (None,True,15,129,'128',128.0)],
                           ('wait_seconds',121),('run_identity','../unknown'),('stale_probe_sha256','bad')):
            original=getattr(self.args,name)
            setattr(self.args,name,value)
            with self.subTest(name=name,value=value),self.assertRaises(ValueError):
                self.validate()
            setattr(self.args,name,original)

    def test_native_and_service_parents_and_new_exact_record_are_not_interchangeable(self):
        self.args.cgroup_parent=self.args.service_cgroup_parent
        with self.assertRaises(ValueError):
            self.validate()
        self.args.cgroup_parent=Path('/fixed/vms')
        self.args.service_cgroup_record=self.root/'old-record.json'
        with self.assertRaises(ValueError):
            self.validate()
        self.args.service_cgroup_record=self.args.run_root/'service-cgroup.json'
        self.args.run_root=self.root/'other'/'run'
        with self.assertRaises(ValueError):
            self.validate()

    def test_parser_requires_iterations_and_approved_late_action_helper_no_silent_skip(self):
        parser=runner.parser()
        required={action.dest for action in parser._actions if action.required}
        self.assertTrue({'rounds','checkpoint_tail_inputs','stale_probe','stale_probe_sha256','cgroup_parent',
            'service_cgroup_parent','service_profile','candidate_manifest'}<=required)
        self.assertNotIn('server',required)

    def test_existing_unknown_root_is_never_adopted_or_cleaned(self):
        self.args.run_root.mkdir(mode=0o700)
        marker=self.args.run_root/'unrelated'
        marker.write_text('retain')
        with patch.object(runner.service_cgroup,'prepare') as prepare, \
                self.assertRaises(ValueError):
            runner.bootstrap(self.args,[],dict(cpu_quota_us=100000,memory_bytes=2*1024**3))
        prepare.assert_not_called()
        self.assertEqual(marker.read_text(),'retain')

    @contextmanager
    def bootstrap_dependencies(self,args,outcome,*,proof_bad=False,cleanup_busy=False):
        """Synthetic process/receipt fixtures; no host or cleanup proof is made."""
        process=Mock()
        if isinstance(outcome,BaseException):
            process.wait.side_effect=outcome
        elif isinstance(outcome,list):
            process.wait.side_effect=outcome
        else:
            process.wait.return_value=outcome
        execution=dict(run_id=args.run_identity,candidate_manifest_sha256='a'*64,service_data=declaration())
        report=dict(services=dict(candidate='b'*40),execution_manifest=execution,service_processes={},
            dk=dict(schema_version=1,candidate='b'*40,execution_manifest_sha256='a'*64,
                service_storage=observation(),service_data=declaration(),
                cases=[dict(id='DK0'+str(index),status='pass',exact_cleanup='passed',
                    execution_manifest_sha256='a'*64,evidence_sha256='c'*64) for index in range(1,7)]))
        if proof_bad:
            report['dk']['cases'].pop()
        def prepare(_parent,_name,record,**_budget):
            runner.write_once(record,dict(exact='original-receipt'))
        def read(path,**_limits):
            return report if Path(path).name=='report.json' else execution
        with ExitStack() as stack:
            stack.enter_context(patch.object(runner,'private_path',side_effect=lambda path,**_:Path(path)))
            stack.enter_context(patch.object(runner,'digest',return_value='a'*64))
            stack.enter_context(patch.object(runner,'json_file',side_effect=read))
            stack.enter_context(patch.object(runner.service_cgroup,'prepare',side_effect=prepare))
            stack.enter_context(patch.object(runner.service_cgroup,'prefix',return_value=[]))
            removed=stack.enter_context(patch.object(runner.service_cgroup,'remove',return_value=dict(removed=True)))
            if cleanup_busy:
                removed.side_effect=ValueError('original live member')
            stack.enter_context(patch.object(runner.subprocess,'Popen',return_value=process))
            stack.enter_context(patch.object(runner.ci_e2e_artifacts,'require_service_boundary'))
            stack.enter_context(patch.object(runner.ci_e2e_artifacts,'require_execution_manifest',return_value=execution))
            stack.enter_context(patch.object(runner.ci_e2e_artifacts,'require_process_evidence'))
            yield process,removed,report

    def bootstrap_args(self,name):
        args=copy.copy(self.args)
        args.run_root=self.root/name
        args.service_cgroup_record=args.run_root/'service-cgroup.json'
        return args

    def test_failed_or_unknown_worker_exit_preserves_original_leaf_receipt(self):
        for index,outcome in enumerate((1,None,True)):
            args=self.bootstrap_args('failed-'+str(index))
            with self.subTest(outcome=outcome),self.bootstrap_dependencies(args,outcome) as (_,removed,_), \
                    self.assertRaises(RuntimeError):
                runner.bootstrap(args,[],dict(cpu_quota_us=100000,memory_bytes=2*1024**3))
            removed.assert_not_called()
            self.assertEqual(json.loads(args.service_cgroup_record.read_text()),dict(exact='original-receipt'))
            self.assertFalse((args.run_root/'service-cleanup.json').exists())

    def test_cancel_or_deadline_never_removes_unknown_original_recovery_identity(self):
        for name,outcome,error in (
            ('cancelled',[KeyboardInterrupt(),0],KeyboardInterrupt),
            ('deadline',[subprocess.TimeoutExpired(['owned-worker'],7200),0],subprocess.TimeoutExpired)):
            args=self.bootstrap_args(name)
            with self.subTest(name=name),self.bootstrap_dependencies(args,outcome) as (process,removed,_), \
                    self.assertRaises(error):
                runner.bootstrap(args,[],dict(cpu_quota_us=100000,memory_bytes=2*1024**3))
            removed.assert_not_called()
            self.assertTrue(args.service_cgroup_record.exists())
            self.assertFalse((args.run_root/'service-cleanup.json').exists())
            process.terminate.assert_called_once()

    def test_missing_mandatory_success_proof_cannot_remove_an_empty_service_leaf(self):
        args=self.bootstrap_args('bad-proof')
        with self.bootstrap_dependencies(args,0,proof_bad=True) as (_,removed,_),self.assertRaises(ValueError):
            runner.bootstrap(args,[],dict(cpu_quota_us=100000,memory_bytes=2*1024**3))
        removed.assert_not_called()
        self.assertTrue(args.service_cgroup_record.exists())
        self.assertFalse((args.run_root/'service-cleanup.json').exists())

    def test_complete_success_alone_uses_original_exact_remove_and_publishes_receipt(self):
        args=self.bootstrap_args('success')
        with self.bootstrap_dependencies(args,0) as (_,removed,report):
            result=runner.bootstrap(args,[],dict(cpu_quota_us=100000,memory_bytes=2*1024**3))
        self.assertEqual(result,report)
        self.assertEqual(json.loads((args.run_root/'dk-launch.json').read_text())['checkpoint_tail_inputs'],128)
        removed.assert_called_once_with(args.service_cgroup_record)
        self.assertEqual(json.loads((args.run_root/'service-cleanup.json').read_text()),dict(removed=True))
        args=self.bootstrap_args('live-member')
        with self.bootstrap_dependencies(args,0,cleanup_busy=True) as (_,removed,_),self.assertRaises(ValueError):
            runner.bootstrap(args,[],dict(cpu_quota_us=100000,memory_bytes=2*1024**3))
        removed.assert_called_once_with(args.service_cgroup_record)
        self.assertFalse((args.run_root/'service-cleanup.json').exists())

    def test_checkpoint_trigger_drift_rejects_the_worker_before_any_original_role_is_started(self):
        args=self.bootstrap_args('trigger-drift')
        with self.bootstrap_dependencies(args,1),self.assertRaises(RuntimeError):
            runner.bootstrap(args,[],dict(cpu_quota_us=100000,memory_bytes=2*1024**3))
        args.checkpoint_tail_inputs=64
        with patch.object(runner,'digest',return_value='a'*64), \
                patch.object(runner.service_cgroup,'observe') as observe, \
                patch.object(harness,'Harness') as constructor, \
                self.assertRaisesRegex(ValueError,'test-launch intent mismatch'):
            runner.worker(args)
        observe.assert_not_called()
        constructor.assert_not_called()
        self.assertEqual(json.loads((args.run_root/'dk-launch.json').read_text())['checkpoint_tail_inputs'],128)

    def test_harness_passes_the_explicit_trigger_to_the_original_renderer_before_initialization(self):
        self.args.run_root.mkdir(mode=0o700)
        class PreparedBoundary(Exception):
            pass
        with patch.object(harness,'prepared_native',return_value={'fresh':'native'}), \
                patch.object(harness,'free_loopback_port',return_value=12345), \
                patch.object(harness,'prepare_fault_services',side_effect=PreparedBoundary) as prepared, \
                self.assertRaises(PreparedBoundary):
            harness.Harness(self.args)
        self.assertEqual(prepared.call_args.kwargs['checkpoint_tail_inputs'],128)
        self.assertEqual(prepared.call_args.kwargs['slots'],2)
        self.assertEqual(prepared.call_args.kwargs['job_count'],2)

    def test_fixed_relay_destinations_cannot_be_urls_ipv6_or_another_host(self):
        self.assertEqual(harness.tuple_address('127.0.0.1:1234'),('127.0.0.1',1234))
        for value in ('other.test:1234','https://other.test:443','[::1]:1234',
                      '127.0.0.1:0','127.0.0.1:65536','127.0.0.1:hello'):
            with self.subTest(value=value),self.assertRaises(ValueError):
                harness.tuple_address(value)

    def test_physical_claim_comes_from_checked_local_vm_mapping_not_global_suffix(self):
        node=dict(deployment_id='test',node_id='host-runtime',node_generation=1)
        identity=dict(protocol_version=2,node=node,tenant_id='1',sandbox_id='sandbox-999',
            operation_id='operation-999',intent_version=1,retention_slot=1,request_window=1)
        local=dict(sandbox=7,owner=4,operation=9)
        host=dict(node=node,operations=[dict(identity=identity,current=True,local=local,
            response=dict(identity=identity))],native=[dict(sandbox=7,current_vm=42)])
        control=self.root/'control'
        control.mkdir(mode=0o700)
        path=control/'host.json'
        path.write_text(json.dumps(dict(native=dict(namespace='a'*16))))
        path.chmod(0o600)
        claim=self.root/'jails/.claims'/('vm-'+'a'*16+'-42')
        claim.mkdir(parents=True,mode=0o700)
        record=claim/'record.json'
        record.write_text(json.dumps(dict(resource_id=claim.name,pid=123,start_time_ticks=456)))
        record.chmod(0o600)
        with patch.object(harness,'process_start_ticks',return_value=456):
            result=harness.physical_record(self.root,host,identity)
        self.assertEqual(result['resource_id'],claim.name)
        self.assertEqual(result['local'],local)
        self.assertNotIn('999',result['resource_id'])
        wrong=copy.deepcopy(host)
        wrong['native'][0]['sandbox']=999
        with self.assertRaises(ValueError):
            harness.physical_record(self.root,wrong,identity)

    def test_a_pid_with_a_changed_clock_is_not_the_original_living_vm(self):
        with patch.object(harness,'process_start_ticks',return_value=456):
            self.assertTrue(harness.same_running_process(dict(pid=123,start_time_ticks=456)))
            self.assertFalse(harness.same_running_process(dict(pid=123,start_time_ticks=457)))
        with patch.object(harness,'process_start_ticks',return_value=None):
            self.assertFalse(harness.same_running_process(dict(pid=123,start_time_ticks=456)))

    def test_original_host_audit_is_offline_and_never_bypasses_its_live_writer_lease(self):
        value=harness.Harness.__new__(harness.Harness)
        value.services=Mock()
        child=Mock()
        child.poll.return_value=None
        value.services.processes.children={'host-0':child}
        value.services.planned_audit.return_value={'original':'durable'}
        self.assertEqual(value.host(),{'original':'durable'})
        value.services.planned_audit.assert_called_once_with('host-0')
        value.services.processes.audit.assert_not_called()
        value.services.wait_ready.assert_called_once_with(for_admission=False)
        value.services.reset_mock()
        self.assertEqual(value.host(resume_ready=False),{'original':'durable'})
        value.services.planned_audit.assert_called_once_with('host-0')
        value.services.wait_ready.assert_not_called()
        child.poll.return_value=86
        value.services.processes.audit.return_value={'original':'exited'}
        self.assertEqual(value.host(),{'original':'exited'})
        value.services.processes.audit.assert_called_once_with('host-0')

    def test_missing_or_duplicate_original_center_record_never_becomes_absence_success(self):
        identity=dict(sandbox_id='sandbox-1')
        row=dict(create=dict(identity=identity))
        self.assertEqual(harness.center_record(dict(sandboxes=[row]),identity),row)
        for records in ([],[row,row]):
            with self.assertRaises(ValueError):
                harness.center_record(dict(sandboxes=records),identity)

    def test_finite_private_evidence_is_sealed_only_after_original_cleanup(self):
        value=harness.Harness.__new__(harness.Harness)
        value.args=self.args
        value.root=self.root
        value.native=dict(namespace='a'*16,database='fixed',uid_base=2000)
        value.known={}
        value.center=Mock(return_value=dict(sandboxes=[]))
        value.host=Mock(return_value=dict(operations=[],native=[]))
        value.relay=Mock()
        value.relay.failed.is_set.return_value=False
        value.relay.facts.return_value=dict(kind='test_only_in_process_mtls_relay')
        value.services=Mock()
        value.manifest=dict(services=dict(candidate='a'*40))
        value.services.execution.return_value=(value.manifest,{})
        value.execution_sha='b'*64
        value.history=[]
        value.cases=[]
        value.evidence_root=self.root
        with patch.object(harness,'database_snapshot',return_value=dict(counts=dict(sandboxes=0))), \
                patch.object(harness,'assert_budget'), \
                patch.object(harness,'check_no_resources',side_effect=AssertionError('known residual')), \
                self.assertRaises(AssertionError):
            value.complete_case('DK01',{})
        self.assertEqual(value.cases,[])
        self.assertFalse((self.root/'DK01.json').exists())

    def test_private_fault_evidence_never_increases_the_original_service_storage_budget(self):
        value=harness.Harness.__new__(harness.Harness)
        record=dict(root=str(self.root),owner_uid=0,peak_bytes=100,inode_budget=3)
        value.evidence_budget=record
        records={'root':(1,1,harness.stat.S_IFDIR,4096,None),
                 'file':(1,2,harness.stat.S_IFREG,80,None)}
        with patch.object(harness,'tree_records',return_value=records):
            self.assertEqual(value.check_evidence_budget()['logical_file_bytes'],80)
            with self.assertRaises(ValueError):
                value.check_evidence_budget(additional_bytes=21)
            with self.assertRaises(ValueError):
                value.check_evidence_budget(additional_entries=2)
        self.assertEqual(record,dict(root=str(self.root),owner_uid=0,peak_bytes=100,inode_budget=3))


if __name__=='__main__':
    unittest.main()
