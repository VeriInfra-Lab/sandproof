"""Pure mandatory-DK launcher/identity checks; never a DK or KVM pass."""
import argparse
import copy
from contextlib import contextmanager, ExitStack
import json
from pathlib import Path
import sys
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'tests/system'))
import center_host_faults as runner
import _center_fault_harness as harness


class CenterHostFaultsTests(unittest.TestCase):
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

    def test_only_original_finite_service_budget_is_forwarded_not_a_second_capacity_policy(self):
        self.assertEqual(self.validate(),dict(cpu_quota_us=100000,memory_bytes=2*1024**3))
        for name,value in (('rounds',True),('rounds',5),('rounds',7),('rounds',17),('wait_seconds',1),
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
        self.assertTrue({'rounds','stale_probe','stale_probe_sha256','cgroup_parent',
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
        execution=dict(run_id=args.run_identity,candidate_manifest_sha256='a'*64)
        report=dict(services=dict(candidate='b'*40),execution_manifest=execution,service_processes={},
            dk=dict(schema_version=1,candidate='b'*40,execution_manifest_sha256='a'*64,
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
            ('cancelled',KeyboardInterrupt(),KeyboardInterrupt),
            ('deadline',[subprocess.TimeoutExpired(['owned-worker'],7200),0],subprocess.TimeoutExpired)):
            args=self.bootstrap_args(name)
            with self.subTest(name=name),self.bootstrap_dependencies(args,outcome) as (process,removed,_), \
                    self.assertRaises(error):
                runner.bootstrap(args,[],dict(cpu_quota_us=100000,memory_bytes=2*1024**3))
            removed.assert_not_called()
            self.assertTrue(args.service_cgroup_record.exists())
            self.assertFalse((args.run_root/'service-cleanup.json').exists())
            if name=='deadline':
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
        removed.assert_called_once_with(args.service_cgroup_record)
        self.assertEqual(json.loads((args.run_root/'service-cleanup.json').read_text()),dict(removed=True))
        args=self.bootstrap_args('live-member')
        with self.bootstrap_dependencies(args,0,cleanup_busy=True) as (_,removed,_),self.assertRaises(ValueError):
            runner.bootstrap(args,[],dict(cpu_quota_us=100000,memory_bytes=2*1024**3))
        removed.assert_called_once_with(args.service_cgroup_record)
        self.assertFalse((args.run_root/'service-cleanup.json').exists())

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
