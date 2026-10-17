"""Deterministic probe regressions; none of these tests run a VM or signal a PID."""
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'system'))
import sdk_guest_faults as probe


class FaultProbeTests(unittest.TestCase):
    def test_unknown_start_posts_once_and_does_not_consume_a_pid_or_replay(self):
        client=Mock()
        response=Mock(status_code=200)
        client.stream.return_value.__enter__=Mock(return_value=response)
        client.stream.return_value.__exit__=Mock(return_value=False)
        vm=Mock(_envd_access_token='test-only')
        vm.files.exists.return_value=True
        vm.files.read.return_value='once'
        vm.commands.run.return_value=SimpleNamespace(stdout='healthy')
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


if __name__=='__main__':
    unittest.main()
