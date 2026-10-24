"""Ordinary controlled tests for retained deployment exclusion; no root/VM/CI effects."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock,Mock,patch

SYSTEM=Path(__file__).resolve().parents[2]/'tests/system'
sys.path.insert(0,str(SYSTEM))
SPEC=importlib.util.spec_from_file_location('unload_runtime_checks',SYSTEM/'lifecycle_unload_runtime.py')
runtime=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime)
SCENE_SPEC=importlib.util.spec_from_file_location('unload_scenario_checks',SYSTEM/'lifecycle_unload_scenario.py')
scene=importlib.util.module_from_spec(SCENE_SPEC)
SCENE_SPEC.loader.exec_module(scene)


class RetainedExclusionTests(unittest.TestCase):
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root=Path(self.temporary.name)
        self.identity='sp638u-'+'1'*16
        self.args=SimpleNamespace(ci_state_root=self.root)
        self.quota=dict(host_lease_identity=self.identity,ci_state_root=str(self.root),deployment_control='/var/lib/sp638u-control')
        for name in ('paused','exclusive-host'):
            path=self.root/name
            path.write_text(json.dumps({'e2e':self.identity}));path.chmod(0o600)

    def test_only_existing_exact_exclusion_can_be_reused(self):
        with patch.object(runtime,'HostLease') as factory:
            self.assertIs(runtime.retained_deployment_lease(self.args,self.quota),factory.return_value)
            factory.assert_called_once_with(self.root,self.identity)
            factory.return_value.pause.assert_not_called()
            factory.return_value.release.assert_not_called()

    def test_foreign_exclusion_does_not_construct_or_replace_a_lease(self):
        (self.root/'exclusive-host').write_text(json.dumps({'e2e':'another-operator'}))
        with patch.object(runtime,'HostLease') as factory,self.assertRaises(ValueError):
            runtime.retained_deployment_lease(self.args,self.quota)
        factory.assert_not_called()
        self.assertEqual(json.loads((self.root/'exclusive-host').read_text()),{'e2e':'another-operator'})

    def test_missing_original_exclusion_is_not_created_from_receipt(self):
        (self.root/'exclusive-host').unlink()
        with patch.object(runtime,'HostLease') as factory,self.assertRaises((ValueError,FileNotFoundError)):
            runtime.retained_deployment_lease(self.args,self.quota)
        factory.assert_not_called()
        self.assertFalse((self.root/'exclusive-host').exists())

    def test_success_stops_managers_but_retains_exclusion_for_disk_teardown(self):
        state=object.__new__(runtime.UnloadRuntime)
        state.services=Mock()
        state.services.processes.audit.return_value={'host_slots':[]}
        state.services.native_pools=[dict(uid_base=3100000,bounded_host=dict(maintenance=dict(max_host_slots=2)))]
        state.running={'source':{'pid':123,'start_time_ticks':1,'boot_id':'test'}}
        state.physical_complete=True
        state.lease=Mock(identity=self.identity)
        state.quota={'deployment_control':str(self.root)}
        report={'complete':True}
        sdk=SimpleNamespace(processes_in_uid_range=lambda _first,_last:[])
        with patch.object(runtime,'original_process',return_value=False),patch.dict(sys.modules,{'sdk_c02_acceptance':sdk}):
            state.finish(report)
        state.services.stop_all.assert_called_once_with()
        state.lease.release.assert_not_called()
        self.assertEqual(report['host_lease'],'retained-until-exact-filesystem-teardown')
        stopped=json.loads((self.root/'scenario-stopped.json').read_text())
        self.assertEqual(stopped['host_lease_identity'],self.identity)
        self.assertEqual(stopped['physical_audit'],'Free')
        self.assertTrue(stopped['managers_exited'])


class ScenarioDiagnosticsTests(unittest.TestCase):
    def client(self):
        import time
        return scene.PublicUnloadScenario('http://127.0.0.1:12345', 'API_SECRET',
            'FOREIGN_SECRET', 'deployment', time.monotonic()+60)

    def assert_safe_stack(self, report):
        self.assertGreater(len(report['failure_stack']), 0)
        self.assertLessEqual(len(report['failure_stack']), 16)
        for frame in report['failure_stack']:
            self.assertEqual(set(frame), {'file', 'function', 'line'})
            self.assertEqual(Path(frame['file']).name, frame['file'])
            self.assertIs(type(frame['line']), int)
            self.assertGreater(frame['line'], 0)

    def test_http_failure_records_status_without_request_response_or_message(self):
        client=self.client()
        client.step('unload-acceptance', 202)
        with patch.object(scene, 'fetch', return_value=(503, b'BODY_SECRET')):
            try:
                client.read_json('GET', '/private?token=URL_SECRET', key='API_SECRET',
                    body={'hidden':'BODY_SECRET'}, status=202)
            except AssertionError as error:
                report=scene.failure_details(error, client, 'public-unload')
            else:
                self.fail('original strict HTTP assertion must reject 503')
        self.assertEqual(report['failure_step'], 'unload-acceptance')
        self.assertEqual(report['last_http_status'], {'expected':202, 'actual':503})
        self.assert_safe_stack(report)
        encoded=json.dumps(report)
        for secret in ('BODY_SECRET', 'API_SECRET', 'FOREIGN_SECRET', 'URL_SECRET',
                '/private', 'returned 503', str(SYSTEM)):
            self.assertNotIn(secret, encoded)

    def test_direct_file_status_is_recorded_before_original_strict_assertion(self):
        client=self.client()
        client.session='sandbox-a'
        client.data_token='TOKEN_SECRET'
        factory=MagicMock()
        factory.return_value.__enter__.return_value.get.return_value.status_code=403
        with patch.dict(sys.modules, {'httpx':SimpleNamespace(Client=factory)}):
            try:
                client.assert_data_closed()
            except AssertionError as error:
                report=scene.failure_details(error, client, 'public-unload')
            else:
                self.fail('original old-token assertion must reject 403')
        self.assertEqual(report['failure_step'], 'old-data-token')
        self.assertEqual(report['last_http_status'], {'expected':401, 'actual':403})
        self.assert_safe_stack(report)
        self.assertNotIn('TOKEN_SECRET', json.dumps(report))

    def test_exercise_keeps_false_report_and_reraises_without_exception_message(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            runner=SimpleNamespace(root=root, services=object(), owner_key='API_SECRET',
                foreign_key='FOREIGN_SECRET', finish=Mock())
            error=AssertionError('BODY_SECRET API_SECRET TOKEN_SECRET')
            args=SimpleNamespace(scenario_timeout=60)
            module=SimpleNamespace(prepare=lambda _args:runner)
            with patch.dict(sys.modules, {'lifecycle_unload_runtime':module}), \
                    patch.object(scene, 'check_center_configuration', side_effect=error), \
                    self.assertRaises(AssertionError) as caught:
                scene.exercise(args)
            self.assertIs(caught.exception, error)
            report=json.loads((root/'lifecycle-unload-report.json').read_text())
            self.assertFalse(report['complete'])
            self.assertEqual(report['phase'], 'configuration')
            self.assertEqual(report['failure_type'], 'AssertionError')
            self.assertEqual(report['failure_step'], 'configuration')
            self.assertIsNone(report['last_http_status'])
            self.assert_safe_stack(report)
            runner.finish.assert_called_once()
            self.assertFalse(runner.finish.call_args.args[0]['complete'])
            for secret in ('BODY_SECRET', 'API_SECRET', 'TOKEN_SECRET', 'FOREIGN_SECRET'):
                self.assertNotIn(secret, json.dumps(report))


if __name__=='__main__':
    unittest.main()
