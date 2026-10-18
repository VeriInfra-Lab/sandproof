"""Deterministic probe regressions; none of these tests run a VM or signal a PID."""
import json
import importlib.util
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'system'))
import sdk_guest_faults as probe

SPEC = importlib.util.spec_from_file_location('envd_acceptance_harness',
    Path(__file__).resolve().parents[1]/'system/envd_acceptance.py')
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class AcceptanceHarnessTests(unittest.TestCase):
    """The real harness imports the fixed SDK; keep these in its locked lane."""
    def test_failed_probe_observes_original_center_once_before_returning_original_error(self):
        for observation_error in (None, TimeoutError('unavailable')):
            with self.subTest(error=observation_error), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                services = Mock(sdk_url='http://127.0.0.1')
                services.processes.center_readiness.return_value = {'nodes': [], 'catalogs': {}}
                services.processes.center_readiness.side_effect = observation_error
                deployment = SimpleNamespace(services=services, key='private')
                pins = dict(release='release', manifest_sha256='manifest', configuration='config',
                            configuration_sha256='config-hash')
                original = RuntimeError('original probe failed')
                with patch.object(runner, 'load_profile', return_value=(pins, {})), \
                        patch.object(runner.time, 'monotonic', side_effect=[10, 908, 909]), \
                        patch.object(runner, 'Commands') as commands, \
                        patch.object(runner.service_cgroup, 'observe', return_value={'memory_peak': 7}) as observe, \
                        patch.object(runner, 'no_resources') as cleanup, \
                        self.assertRaises(RuntimeError) as caught:
                    commands.return_value.run.side_effect = original
                    runner.run_probe(deployment, {'candidate_manifest': str(root/'manifest')},
                                     root, 'file-permissions', 'rust')
                self.assertIs(caught.exception, original)
                services.processes.center_readiness.assert_called_once_with(timeout=2)
                observe.assert_called_once_with(root/'service-cgroup.json')
                commands.return_value.run.assert_called_once()
                cleanup.assert_not_called()
                self.assertEqual([item[0] for item in services.mock_calls],
                                 ['wait_ready', 'processes.center_readiness'])
                diagnostic = json.loads((root/'file-permissions-failure.json').read_text())
                self.assertEqual(diagnostic['probe_failure'], 'RuntimeError')
                self.assertEqual(diagnostic['cgroup'], {'memory_peak': 7})
                self.assertLessEqual(diagnostic['started_unix_ms'], diagnostic['finished_unix_ms'])
                self.assertEqual(diagnostic['elapsed_seconds'], 1)
                if observation_error is None:
                    self.assertEqual(diagnostic['center'], {'nodes': [], 'catalogs': {}})
                else:
                    self.assertNotIn('center', diagnostic)
                    self.assertEqual(diagnostic['center_failure'], 'TimeoutError')

    def test_failure_observation_respects_original_deadline_and_keeps_write_error(self):
        services = Mock()
        primary, write = RuntimeError('probe failed'), OSError('evidence unavailable')
        with patch.object(runner.time, 'monotonic', side_effect=[901, 901]), \
                patch.object(runner.service_cgroup, 'observe', side_effect=ValueError('identity changed')), \
                patch.object(runner, 'write_once', side_effect=write) as save, \
                self.assertRaises(BaseExceptionGroup) as caught:
            runner.observe_probe_failure(SimpleNamespace(services=services), Path('/unused'),
                                         'file-permissions', 900, primary)
        services.processes.center_readiness.assert_not_called()
        self.assertEqual(caught.exception.exceptions, (primary, write))
        diagnostic = save.call_args.args[1]
        self.assertNotIn('center', diagnostic)
        self.assertEqual(diagnostic['center_failure'], 'probe_deadline_elapsed')
        self.assertEqual(diagnostic['cgroup_failure'], 'ValueError')

    def test_final_storage_failure_preserves_completed_cases_and_original_cleanup_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            from tools.service_storage_observation import ObservationUnknown
            error = ObservationUnknown('original inventory unavailable')
            services = Mock()
            services.storage_diagnostics.return_value = {'failure':{'kind':'inventory_rejected'}}
            deployment = SimpleNamespace(services=services)
            report = dict(status='pass', checks=[{'name':'faults','returncode':0}],
                          resource_cleanup_confirmed=False, staging_cleanup_confirmed=False)
            def reclaimed(_deployment, value, **_kwargs): value['resource_cleanup_confirmed'] = True
            with patch.object(runner, 'reclaim_resources', side_effect=reclaimed), \
                    patch.object(runner, 'stop_services', side_effect=error), \
                    self.assertRaises(BaseExceptionGroup) as caught:
                runner.finish_worker(deployment, report, root, services_started=True, primary=None)
            self.assertEqual(caught.exception.exceptions, (error,))
            saved = json.loads((root/'report.json').read_text())
            self.assertEqual(saved['status'], 'failed')
            self.assertEqual(saved['checks'], [{'name':'faults','returncode':0}])
            self.assertTrue(saved['resource_cleanup_confirmed'])
            self.assertFalse(saved['staging_cleanup_confirmed'])
            self.assertEqual(saved['cleanup_failures'], [{'phase':'services','error_type':'ObservationUnknown'}])
            self.assertEqual(saved['storage_diagnostics'], services.storage_diagnostics.return_value)

    def test_primary_cleanup_and_report_write_failures_remain_distinct_without_retries(self):
        primary, resource, service, write = (ValueError('primary'), OSError('resource'),
                                           RuntimeError('service'), OSError('write'))
        deployment = SimpleNamespace(services=Mock())
        deployment.services.storage_diagnostics.return_value = {'failure':None}
        report = dict(status='failed')
        with patch.object(runner, 'reclaim_resources', side_effect=resource) as reclaim, \
                patch.object(runner, 'stop_services', side_effect=service) as stop, \
                patch.object(runner, '_save', side_effect=write) as save, \
                self.assertRaises(BaseExceptionGroup) as caught:
            runner.finish_worker(deployment, report, Path('/unused'), services_started=True, primary=primary)
        self.assertEqual(caught.exception.exceptions, (primary, resource, service, write))
        reclaim.assert_called_once()
        stop.assert_called_once()
        save.assert_called_once()

    def test_restart_preparation_uses_original_probe_deadline_and_protected_handoff(self):
        import _guest_restart_fixture as fixture
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            deployment = SimpleNamespace(services=Mock(sdk_url='http://127.0.0.1'), key='private')
            pins = dict(release='release', manifest_sha256='manifest', configuration='config',
                        configuration_sha256='config-hash')
            with patch.object(runner, 'load_profile', return_value=(pins, {'guest':'identity'})), \
                    patch.object(fixture, 'prepare', return_value=root/'handoff') as prepare, \
                    patch.object(runner.time, 'monotonic', side_effect=[10, 50]), \
                    patch.object(runner, 'Commands') as commands, \
                    patch.object(runner, 'no_resources') as cleaned:
                result = runner.run_probe(deployment, {'candidate_manifest':str(root/'manifest')},
                                          root, 'faults', 'rust')
            prepare.assert_called_once_with(deployment, root, 'rust', {'guest':'identity'})
            self.assertEqual(commands.call_args.args[0], 910)
            self.assertEqual(result['seconds'], 40)
            self.assertEqual(commands.return_value.run.call_args.args[0][-2:],
                             ['--restart-fixture', str(root/'handoff')])
            cleaned.assert_called_once_with(deployment)

    def test_failed_worker_persists_real_cleanup_but_still_raises(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve(); root.chmod(0o700)
            config=dict(candidate='a'*40,run_id='b'*64)
            guest={'source_commit':'a'*40}
            report=dict(**config,guest=guest,status='failed',checks=[],faults=[],
                resource_cleanup_confirmed=True,staging_cleanup_confirmed=True,
                service_cleanup_confirmed=False)
            path=root/'report.json'; path.write_text(json.dumps(report)); path.chmod(0o600)
            with patch.object(runner.service_cgroup,'remove',return_value={'removed':True}) as remove:
                with self.assertRaisesRegex(RuntimeError,'does not make it pass'):
                    runner.finish_run(config,guest,root,root/'service-cgroup.json',1)
            remove.assert_called_once_with(root/'service-cgroup.json')
            actual=json.loads(path.read_text())
            self.assertIs(actual['service_cleanup_confirmed'],True)
            self.assertEqual(actual['status'],'failed')
            self.assertEqual(actual['checks'],[])
            self.assertIs(json.loads((root/'service-cleanup.json').read_text())['removed'],True)

    def test_unknown_vm_or_role_cleanup_never_removes_the_cgroup(self):
        for field in ('candidate','resource_cleanup_confirmed','staging_cleanup_confirmed'):
            with tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary).resolve(); root.chmod(0o700)
                config=dict(candidate='a'*40,run_id='b'*64); guest={'source_commit':'a'*40}
                report=dict(**config,guest=guest,status='failed',resource_cleanup_confirmed=True,
                    staging_cleanup_confirmed=True,service_cleanup_confirmed=False)
                report[field]='wrong' if field=='candidate' else False
                path=root/'report.json'; path.write_text(json.dumps(report)); path.chmod(0o600)
                with patch.object(runner.service_cgroup,'remove') as remove:
                    with self.subTest(field=field),self.assertRaises(ValueError):
                        runner.finish_run(config,guest,root,root/'service-cgroup.json',1)
                remove.assert_not_called()
                self.assertFalse((root/'service-cleanup.json').exists())

    def test_partial_initialization_does_not_reinitialize_or_wait_for_an_absent_center(self):
        deployment=Mock()
        report=dict(resource_cleanup_confirmed=False)
        runner.reclaim_resources(deployment,report,services_started=False)
        self.assertEqual(deployment.mock_calls,[])
        self.assertIs(report['resource_cleanup_confirmed'],False)

    def test_service_staging_is_reclaimed_only_after_real_vm_cleanup_and_exact_stop(self):
        for cleaned in (False, True):
            services = Mock()
            services.storage_peaks.return_value = {'measured': True}
            report = dict(resource_cleanup_confirmed=cleaned, staging_cleanup_confirmed=False)
            runner.stop_services(SimpleNamespace(services=services), report)
            self.assertEqual(report['staging_cleanup_confirmed'], cleaned)
            self.assertEqual(report['storage_peaks'], {'measured': True})
            self.assertEqual([x[0] for x in services.method_calls],
                             ['stop_all', 'storage_peaks'] + (['cleanup_staging'] if cleaned else []))
        services.cleanup_staging.side_effect = ValueError('identity changed')
        report = dict(resource_cleanup_confirmed=True, staging_cleanup_confirmed=False)
        with self.assertRaises(ValueError):
            runner.stop_services(SimpleNamespace(services=services), report)
        self.assertIs(report['staging_cleanup_confirmed'], False)

    def test_registry_preflight_receives_rust_profile_and_required_registry_tool(self):
        from tools import envd_acceptance as envd
        from tools.template_fetch import REGCTL_SHA256

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            fields = ('center', 'host', 'candidate_manifest', 'redis_material', 'tls_material',
                      'service_staging_parent', 'kernel', 'firecracker', 'jailer', 'tools',
                      'cgroup_parent', 'build_cgroup_parent', 'window_helper')
            config = {name: str(root / name) for name in fields}
            config.update(center_uid=1001, center_gid=1001, redis_uid=1002, redis_gid=1002,
                          uid_base=10000, rust_envd_acceptance=dict(go_rootfs='go', builder_rootfs='builder',
                          policy_template='policy', tls_material='stage-tls',registry_limits={'approved': 1},
                          build_limits={'approved': 2}, data_transfer={'approved': 3}))
            policy = {'public': {'bounded_builds': {'helper': {}}}}
            with patch.object(runner, 'json_file', return_value={}), \
                    patch.object(runner, 'instantiate_policy', return_value=policy), \
                    patch.object(runner, 'digest', return_value='a'*64), \
                    patch.object(runner, 'write_once'), \
                    patch.object(runner, 'free_loopback_port', side_effect=[1, 2, 3]), \
                    patch.object(runner, 'prepare_registry_runtime', return_value=SimpleNamespace()) as prepare:
                runner.prepare_deployment(config, root, root/'record')
            kwargs = prepare.call_args.kwargs
            self.assertEqual(kwargs['rust_guest_profile'], envd.prepared_paths(config)['profile'])
            self.assertNotIn('envd', kwargs['tool_hashes'])
            self.assertEqual(kwargs['tool_hashes']['regctl'], REGCTL_SHA256)
            self.assertEqual(kwargs['data_transfer'], {'approved': 3})
            self.assertEqual(kwargs['activation_ceiling_bytes'], 200*1024**3)
            self.assertEqual(prepare.call_args.args[0].tls_material,Path('stage-tls'))


class FaultProbeTests(unittest.TestCase):
    def test_private_directory_creates_missing_parent_but_never_reuses_a_probe_leaf(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary) / "missing parent 'quoted'"
            directory = parent / "probe 'one'"
            vm = Mock()
            def execute(command, **kwargs):
                self.assertEqual(kwargs, dict(user='root', timeout=15))
                return subprocess.run(['/bin/sh', '-c', command], check=True,
                                      capture_output=True, timeout=15)
            vm.commands.run.side_effect = execute
            probe.prepare_directory(vm, str(directory))
            self.assertEqual(stat.S_IMODE(directory.stat().st_mode), 0o700)
            marker = directory / 'original-effect'
            marker.write_text('once')
            with self.assertRaises(subprocess.CalledProcessError):
                probe.prepare_directory(vm, str(directory))
            self.assertEqual(marker.read_text(), 'once')
            self.assertEqual(vm.commands.run.call_count, 2)
            parent.chmod(0o750)
            probe.prepare_directory(vm, str(parent / 'second'))
            self.assertEqual(stat.S_IMODE(parent.stat().st_mode), 0o750)
            self.assertEqual(stat.S_IMODE((parent / 'second').stat().st_mode), 0o700)
            file_parent = Path(temporary) / 'ordinary-file'
            file_parent.write_text('unchanged')
            with self.assertRaises(subprocess.CalledProcessError):
                probe.prepare_directory(vm, str(file_parent / 'rejected'))
            self.assertEqual(file_parent.read_text(), 'unchanged')
            self.assertEqual(vm.commands.run.call_count, 4)

    def test_unknown_start_posts_once_and_does_not_consume_a_pid_or_replay(self):
        client=Mock()
        response=Mock(status_code=200)
        client.stream.return_value.__enter__=Mock(return_value=response)
        client.stream.return_value.__exit__=Mock(return_value=False)
        vm=Mock(_envd_access_token='test-only')
        vm.files.exists.return_value=True
        vm.files.read.return_value='once'
        vm.commands.run.return_value=SimpleNamespace(stdout='healthy')
        def close_after_effect(*_args):
            vm.files.exists.assert_called_once()
            vm.files.read.assert_called_once()
            return False
        client.stream.return_value.__exit__.side_effect=close_after_effect
        result=probe.lost_start(vm,'/private/test',client)
        self.assertEqual(result,dict(start_posts=1,start_pid_observed=False,effects=1))
        client.stream.assert_called_once()
        response.iter_bytes.assert_not_called()
        response.read.assert_not_called()
        args,kwargs=client.stream.call_args
        self.assertEqual(args,('POST','/process.Process/Start'))
        body=kwargs['content']
        self.assertEqual(body[0],0)
        self.assertEqual(int.from_bytes(body[1:5],'big'),len(body)-5)
        payload=json.loads(body[5:])
        self.assertIn('printf once',payload['process']['args'][1])
        self.assertIn('printf once >> ',payload['process']['args'][1])
        self.assertEqual(kwargs['headers']['Authorization'],'Basic cm9vdDo=')

    def test_unknown_response_or_missing_real_effect_does_not_pass(self):
        client=Mock()
        client.stream.return_value.__enter__=Mock(return_value=Mock(status_code=503))
        client.stream.return_value.__exit__=Mock(return_value=False)
        vm=Mock(_envd_access_token='test-only')
        with self.assertRaises(AssertionError): probe.lost_start(vm,'/private/test',client)
        vm.commands.run.assert_not_called()
        with patch.object(probe.time,'monotonic',side_effect=[0,21]):
            with self.assertRaises(AssertionError): probe.wait_for(lambda:False,bool)

    def test_duplicate_effect_is_not_hidden_by_overwriting_the_marker(self):
        client=Mock()
        client.stream.return_value.__enter__=Mock(return_value=Mock(status_code=200))
        client.stream.return_value.__exit__=Mock(return_value=False)
        vm=Mock(_envd_access_token='test-only')
        vm.files.exists.return_value=True
        vm.files.read.return_value='onceonce'
        with self.assertRaises(AssertionError): probe.lost_start(vm,'/private/test',client)
        client.stream.assert_called_once()
        vm.commands.run.assert_not_called()

    def test_native_restart_is_parseable_and_uses_stable_handles_not_pid_guessing(self):
        compile(probe.RESTART.replace('ROOT',repr('/private/test')),'native-restart','exec')
        self.assertIn('os.pidfd_open',probe.RESTART)
        self.assertIn('signal.pidfd_send_signal',probe.RESTART)
        self.assertIn('old user process unexpectedly disappeared',probe.RESTART)
        self.assertIn("current[52+old_pid//8]",probe.RESTART)
        self.assertNotIn('initialize --config',probe.RESTART)
        self.assertNotIn('os.kill(',probe.RESTART)

    def test_reclaim_refuses_an_unbounded_or_changed_retention_plan_before_any_start(self):
        vm=Mock()
        for capacity,seconds in ((257,30),(True,30),(8,31)):
            settings={'commands':{'owners':{'records':capacity},
                'registry':{'records':capacity,'retention':{'secs':seconds,'nanos':0}}}}
            with self.subTest(capacity=capacity,seconds=seconds),self.assertRaises(AssertionError):
                probe.registry_reclaim(vm,'/private/test',(),settings)
        vm.commands.run.assert_not_called()

    def test_unknown_upload_failure_is_not_a_full_filesystem_success(self):
        vm=Mock()
        vm.files.write.side_effect=OSError('unknown transport')
        with self.assertRaises(OSError): probe.full_filesystem(vm,'/private/test')
        vm.files.read.assert_not_called()

    def test_expiry_or_rate_limit_cannot_substitute_for_an_injected_service_outage(self):
        async def observed(*_args,**_kwargs):
            return {'observation':{'daemon':{'pid':123,'start_ticks':456}}}
        for status in (401,429):
            vm=Mock(_envd_access_token='test-only')
            vm.files.exists.return_value=True
            vm.files.read.return_value='armed'
            vm.get_info.return_value=SimpleNamespace(state='running')
            client=Mock()
            client.get.return_value=SimpleNamespace(status_code=status)
            with patch.object(probe,'observe_candidate',side_effect=observed):
                with self.subTest(status=status),self.assertRaises(AssertionError):
                    probe.unavailable_expiry(vm,{},client,())
            self.assertEqual(vm.files.write.call_count,1)
            vm.commands.run.return_value.disconnect.assert_called_once()


if __name__=='__main__':
    unittest.main()
