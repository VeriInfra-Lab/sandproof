"""Ordinary controlled tests for retained deployment exclusion; no root/VM/CI effects."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock,patch

SYSTEM=Path(__file__).resolve().parents[2]/'tests/system'
sys.path.insert(0,str(SYSTEM))
SPEC=importlib.util.spec_from_file_location('unload_runtime_checks',SYSTEM/'lifecycle_unload_runtime.py')
runtime=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime)


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


if __name__=='__main__':
    unittest.main()
