"""The existing gate must not accept a partial, Go-only or wrong-source run."""
import copy
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from tools import envd_acceptance as envd, ci_e2e_supervisor as ci


class ReceiptTests(unittest.TestCase):
    def setUp(self):
        self.guest = dict(source_commit='a'*40, configuration=dict(bytes=3,sha256='b'*64))
        self.report = dict(candidate='a'*40,run_id='c'*64,guest=self.guest,status='pass',
            checks=[dict(name=name,returncode=0,seconds=1) for name in envd.SUITE],
            faults=[dict(name=name,status='pass') for name in envd.FAULTS],
            resource_cleanup_confirmed=True,staging_cleanup_confirmed=True,service_cleanup_confirmed=True,
            immutable_template=dict(bytes=3,sha256='d'*64,unchanged=True),
            environment=dict(installed=True,system_file=True,fresh_control_pristine=True,
                             daemon_unchanged=True,host_system_path_absent=True))

    def reduce(self, report):
        return envd.require_result(report,candidate='a'*40,run_id='c'*64,expected_guest=self.guest)

    def test_complete_same_candidate_original_suite_is_required(self):
        self.assertEqual(self.reduce(self.report),self.report)
        for field,value in (('candidate','d'*40),('run_id','e'*64),('guest',{}),('status','fail'),
                            ('resource_cleanup_confirmed',1),('staging_cleanup_confirmed',False),
                            ('service_cleanup_confirmed',False)):
            changed=copy.deepcopy(self.report); changed[field]=value
            with self.subTest(field=field), self.assertRaises(ValueError): self.reduce(changed)

    def test_missing_duplicate_reordered_or_failed_probe_cannot_pass(self):
        for mutate in (lambda r:r['checks'].pop(),
                       lambda r:r['checks'].reverse(),
                       lambda r:r['checks'].append(r['checks'][0]),
                       lambda r:r['checks'][0].update(returncode=True),
                       lambda r:r['checks'][0].update(returncode=1),
                       lambda r:r.update(checks=[None]),
                       lambda r:r['faults'].pop(),
                       lambda r:r['faults'][0].update(status='unknown'),
                       lambda r:r.update(faults=[None])):
            changed=copy.deepcopy(self.report); mutate(changed)
            with self.assertRaises(ValueError): self.reduce(changed)

    def test_deadline_or_invalid_measurement_is_not_a_success(self):
        for seconds in (True,-1,901,float('nan'),float('inf'),None):
            changed=copy.deepcopy(self.report); changed['checks'][0]['seconds']=seconds
            with self.subTest(seconds=seconds), self.assertRaises(ValueError): self.reduce(changed)

    def test_missing_or_unknown_environment_and_shared_image_evidence_cannot_pass(self):
        for field in ('immutable_template','environment'):
            changed=copy.deepcopy(self.report); changed.pop(field)
            with self.subTest(field=field), self.assertRaises(ValueError): self.reduce(changed)
        for field,key,value in [('immutable_template','unchanged',1),('immutable_template','sha256','bad'),
                                ('environment','installed',False),('environment','fresh_control_pristine',None),
                                ('environment','host_system_path_absent',False)]:
            changed=copy.deepcopy(self.report); changed[field][key]=value
            with self.subTest(field=field,key=key), self.assertRaises(ValueError): self.reduce(changed)

    def test_public_projection_does_not_copy_private_child_fields(self):
        self.report['checks'][0]['secret']='private'
        self.report['faults'][0]['token']='private'
        self.report['private_log']='private'
        self.report['immutable_template']['private_path']='private'
        self.report['environment']['private_payload']='private'
        self.assertNotIn('private',json.dumps(self.reduce(self.report)))

    def test_clean_failure_is_recoverable_but_never_a_success(self):
        self.report.update(status='failed',checks=[],faults=[])
        result=envd.require_cleanup(self.report,candidate='a'*40,run_id='c'*64,expected_guest=self.guest)
        self.assertIs(result['service_cleanup_confirmed'],True)
        with self.assertRaises(ValueError): self.reduce(self.report)


class SupervisorStageTests(unittest.TestCase):
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root=Path(self.temporary.name).resolve(); self.root.chmod(0o700)
        self.stage=ci.Supervisor.__new__(ci.Supervisor)
        self.stage.root=self.root
        self.stage.state=dict(candidate='a'*40,id='b'*64,candidate_manifest=str(self.root/'candidate.json'))
        fields=('checkout','center','host','kernel','firecracker','jailer','tools','redis_material','tls_material',
            'service_staging_parent','center_uid','center_gid','redis_uid','redis_gid','uid_base','scratch_parent',
            'cgroup_parent','build_cgroup_parent','service_cgroup_parent','window_helper','budget_profile')
        self.stage.config={name:str(self.root/name) for name in fields}
        self.stage.checks=None
        self.stage.config.update(host_assets={},rust_envd_acceptance={'reviewed':True,'budget_profile':str(self.root/'rust-budget')},
                                 client_id='must-not-enter-stage')
        self.stage.save=Mock()
        self.stage.block=Mock()
        self.stage.verified_candidate=Mock(return_value={})
        self.addCleanup(patch.stopall)
        patch.object(envd,'policy',return_value=self.stage.config['rust_envd_acceptance']).start()
        patch.object(envd,'identity',return_value={'source_commit':'a'*40}).start()

    def test_persists_launching_intent_before_launch_and_excludes_app_configuration(self):
        def launch(group):
            self.assertEqual(group['phase'],'launching')
            self.stage.save.assert_called()
            path=Path(group['command'][-1]); config=json.loads(path.read_text())
            self.assertNotIn('client_id',config)
            self.assertEqual(config['candidate'],'a'*40)
            self.assertEqual(config['run_id'],'b'*64)
            self.assertEqual(config['budget_profile'],str(self.root/'rust-budget'))
            self.assertEqual(self.stage.config['budget_profile'],str(self.root/'budget_profile'))
            self.assertEqual(group['command'][1],str(self.root/'checkout/tests/system/envd_acceptance.py'))
            # Exercise the REAL strict identity parser, not a mock of it. The
            # new stage is an original batch unit, not a second naming scheme.
            response=SimpleNamespace(returncode=0, stdout='LoadState=loaded\nActiveState=active\nExecMainCode=0\n')
            with patch.object(ci.subprocess, 'run', return_value=response):
                self.assertEqual(ci.unit_state(group['unit'])['LoadState'], 'loaded')
        with patch.object(ci,'launch',side_effect=launch) as started:
            self.assertFalse(self.stage.observe_envd())
        started.assert_called_once()
        self.assertEqual(self.stage.state['envd']['phase'],'running')

    def test_lost_launch_reply_does_not_authorize_another_launch(self):
        with patch.object(ci,'launch',side_effect=OSError('unknown launch')):
            with self.assertRaises(OSError): self.stage.observe_envd()
        self.assertEqual(self.stage.state['envd']['phase'],'launching')
        with patch.object(ci,'unit_state',return_value=dict(ActiveState='active',ExecMainCode='0')), \
                patch.object(ci,'launch') as started:
            self.assertFalse(self.stage.observe_envd())
        started.assert_not_called()

    def test_failed_missing_or_unknown_unit_blocks_without_new_work(self):
        for fields in (dict(ActiveState='inactive',ExecMainCode='0'),
                       dict(ActiveState='failed',ExecMainCode='1',ExecMainStatus='1')):
            self.stage.state['envd']=dict(phase='running',unit='owned-envd.service',root=str(self.root/'missing'))
            with patch.object(ci,'unit_state',return_value=fields),patch.object(ci,'launch') as started:
                self.assertFalse(self.stage.observe_envd())
            started.assert_not_called()
            self.stage.block.assert_called_with('rust_envd_failed_or_cleanup_unknown')

    def test_no_config_does_not_claim_rust_acceptance_and_cannot_disable_an_inflight_stage(self):
        with patch.object(envd,'policy',return_value=None):
            self.assertTrue(self.stage.observe_envd())
            self.assertNotIn('envd',self.stage.state)
            self.stage.state['envd']={'phase':'running'}
            self.assertFalse(self.stage.observe_envd())
        self.stage.block.assert_called_with('rust_envd_candidate_identity_unknown')

    def test_harbor_cannot_start_while_rust_stage_is_pending(self):
        self.stage.checks=None
        self.stage.config['ci_state_root']=str(self.root)
        self.stage.state.update(expected=[],all_tasks=[],batches=[])
        self.stage.state.update(phase='running',batches=[{'phase':'new'}])
        with patch.object(self.stage,'observe_dk',return_value=True), \
                patch.object(self.stage,'observe_envd',return_value=False), \
                patch.object(self.stage,'prepare') as prepare,patch.object(ci,'launch') as launch:
            self.stage.tick()
        prepare.assert_not_called()
        launch.assert_not_called()

    def test_success_requires_execution_and_cleanup_before_durable_reclamation(self):
        from tools.system_acceptance import digest
        run=self.root/'owned'; run.mkdir(mode=0o700)
        candidate=Path(self.stage.state['candidate_manifest']); candidate.write_text('{}')
        candidate.chmod(0o600)
        self.stage.state['envd']=dict(phase='running',unit='owned.service',root=str(run))
        report=dict(candidate='a'*40,run_id='b'*64,guest={'source_commit':'a'*40},status='pass',
            checks=[dict(name=name,returncode=0,seconds=1) for name in envd.SUITE],
            faults=[dict(name=name,status='pass') for name in envd.FAULTS],
            resource_cleanup_confirmed=True,staging_cleanup_confirmed=True,service_cleanup_confirmed=True,
            execution_manifest={},service_processes={},
            immutable_template=dict(bytes=3,sha256='d'*64,unchanged=True),
            environment=dict(installed=True,system_file=True,fresh_control_pristine=True,
                             daemon_unchanged=True,host_system_path_absent=True))
        for filename,value in (('report.json',report),('execution-manifest.json',{}),
                               ('service-cleanup.json',{'removed':True})):
            (run/filename).write_text(json.dumps(value)); (run/filename).chmod(0o600)
        execution=dict(run_id='b'*64,candidate_manifest_sha256=digest(candidate))
        with patch.object(ci,'unit_state',return_value=dict(ExecMainCode='1',ExecMainStatus='0')), \
                patch.object(ci.service_artifacts,'require_service_boundary'), \
                patch.object(ci.service_artifacts,'require_execution_manifest',return_value=execution), \
                patch.object(ci.service_artifacts,'require_process_evidence'), \
                patch.object(ci.subprocess,'run') as stop:
            self.assertTrue(self.stage.observe_envd())
        self.assertFalse(run.exists())
        self.assertEqual(self.stage.state['envd']['phase'],'done')
        self.assertEqual(self.stage.state['envd']['result']['guest'],report['guest'])
        stop.assert_called_once()
        self.assertTrue(self.stage.observe_envd())  # Original checked durable result, no rerun.

    def test_failed_unit_releases_only_after_exact_cleanup_and_keeps_failure_evidence(self):
        from tools.system_acceptance import digest
        run=self.root/'failed'; run.mkdir(mode=0o700)
        candidate=Path(self.stage.state['candidate_manifest']); candidate.write_text('{}'); candidate.chmod(0o600)
        self.stage.config['ci_state_root']=str(self.root)
        self.stage.state.update(expected=[],all_tasks=[],batches=[])
        report=dict(candidate='a'*40,run_id='b'*64,guest={'source_commit':'a'*40},status='failed',
            checks=[],faults=[],resource_cleanup_confirmed=True,staging_cleanup_confirmed=True,
            service_cleanup_confirmed=True,execution_manifest={},service_processes={})
        for filename,value in (('report.json',report),('execution-manifest.json',{}),
                               ('service-cleanup.json',{'removed':True})):
            (run/filename).write_text(json.dumps(value)); (run/filename).chmod(0o600)
        execution=dict(run_id='b'*64,candidate_manifest_sha256=digest(candidate))
        for confirmed in (False, True):
            report['resource_cleanup_confirmed']=confirmed
            (run/'report.json').write_text(json.dumps(report))
            self.stage.state['envd']=dict(phase='running',unit='owned.service',root=str(run))
            with patch.object(ci,'unit_state',return_value=dict(ExecMainCode='1',ExecMainStatus='1')), \
                    patch.object(ci.service_artifacts,'require_service_boundary'), \
                    patch.object(ci.service_artifacts,'require_execution_manifest',return_value=execution), \
                    patch.object(ci.service_artifacts,'require_process_evidence'), \
                    patch.object(ci.subprocess,'run') as stop, patch.object(ci,'HostLease') as lease:
                self.assertFalse(self.stage.observe_envd())
                if confirmed:
                    stop.assert_called_once()
                    lease.assert_called_once_with(self.root,'b'*64)
                    lease.return_value.release.assert_called_once()
                    self.assertEqual(self.stage.state['envd']['phase'],'failed')
                    self.assertEqual(self.stage.state['phase'],'done')
                    self.assertIs(self.stage.state['result']['passed'],False)
                    self.assertEqual(self.stage.state['reason'],'rust_envd_checks_failed_cleanup_confirmed')
                else:
                    stop.assert_not_called(); lease.assert_not_called()
                    self.stage.block.assert_called_with('rust_envd_failed_or_cleanup_unknown')
            self.assertTrue((run/'report.json').is_file())
        with patch.object(ci,'launch') as launch,patch.object(ci,'HostLease') as lease:
            self.assertFalse(self.stage.observe_envd())  # Resume durable failed result, never rerun.
        launch.assert_not_called(); lease.return_value.release.assert_called_once()
        with patch.object(self.stage,'observe_dk') as dk,patch.object(ci,'launch') as launch:
            self.stage.tick()  # Terminal failure must never continue into Harbor.
        dk.assert_not_called();launch.assert_not_called()


class PreparationTests(unittest.TestCase):
    def test_stage_tls_is_separate_complete_and_digest_bound(self):
        from tools.ci_e2e_artifacts import tls_material_files
        from tools.system_acceptance import digest
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve(); root.chmod(0o700)
            value=dict(registry_limits={},build_limits={},data_transfer={})
            assets={}
            for name in envd.FILES:
                raw = (json.dumps({'host':{'control_cpu_allowance':4,'control_memory_allowance_bytes':64*1024**2}}).encode()
                       if name=='budget_profile' else b'approved input')
                p=root/name; p.write_bytes(raw); p.chmod(0o600)
                value[name]=str(p); assets[str(p)]=digest(p)
            tls=root/'stage-tls'; tls.mkdir(mode=0o700); value['tls_material']=str(tls)
            for name in tls_material_files(('runtime','builder')):
                p=tls/name; p.write_bytes(name.encode()); p.chmod(0o600);assets[str(p)]=digest(p)
            config=dict(rust_envd_acceptance=value,host_assets=assets,tls_material='unchanged-existing-gate')
            with patch('tools.template_build_config.bounded_data_route_limits'):
                self.assertEqual(envd.policy(config),value)
                builder=tls/'host-builder.der'
                original=assets.pop(str(builder))
                with self.assertRaises(ValueError): envd.policy(config)
                assets[str(builder)]=original
                builder.write_bytes(b'changed')
                with self.assertRaises(ValueError): envd.policy(config)
                builder.write_bytes(builder.name.encode())
                budget=value.pop('budget_profile')
                with self.assertRaises(ValueError): envd.policy(config)
                value['budget_profile']=budget
                path=Path(budget); original_budget=path.read_bytes()
                for cpu,memory in ((True,64*1024**2),(0,64*1024**2),(4,0),(4,3)):
                    path.write_text(json.dumps({'host':{'control_cpu_allowance':cpu,'control_memory_allowance_bytes':memory}}))
                    assets[budget]=digest(path)
                    with self.subTest(cpu=cpu,memory=memory),self.assertRaises(ValueError): envd.policy(config)
                path.write_bytes(original_budget);assets[budget]=digest(path)
                self.assertEqual(envd.policy(config),value)
            self.assertEqual(config['tls_material'],'unchanged-existing-gate')

    def test_missing_profile_does_not_build_or_switch_any_go_material(self):
        self.assertIsNone(envd.prepare({}))
        self.assertIsNone(envd.policy({}))

    def test_only_actual_worker_identity_changes_and_original_budgets_survive(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve(); root.chmod(0o700)
            approved=root/'approved.json'
            before=dict(version=3,worker={'old':'identity'},commands={'owners':{'input_chunk_bytes':4096}},
                        listen='0.0.0.0:49983')
            approved.write_text(json.dumps(before)); approved.chmod(0o600)
            config=dict(candidate='a'*40,candidate_manifest=str(root/'candidate.json'),checkout='checkout',
                        cargo_home='cache',toolchain_bin='compiler')
            actual={'package_version':'0.1.0','artifact':{'sha256':'b'*64,'bytes':17}}
            with patch.object(envd,'policy',return_value={'configuration':str(approved)}), \
                    patch('tools.prepare_guest.prepare',return_value={'manifest_sha256':'c'*64,'worker':actual}) as build, \
                    patch.object(envd.guest_artifacts,'load',return_value={'versions':{'config_version':3}}):
                result=envd.prepare(config)
            after=json.loads(Path(result['configuration']).read_text())
            self.assertEqual(after['worker'],actual)
            after['worker']=before['worker']
            self.assertEqual(after,before)
            self.assertEqual(json.loads(approved.read_text()),before)
            self.assertEqual(build.call_args.args[0]['candidate'],'a'*40)
            self.assertEqual(build.call_args.args[0]['timeout_seconds'],900)
