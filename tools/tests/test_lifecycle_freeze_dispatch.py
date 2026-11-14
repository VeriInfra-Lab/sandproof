"""Same Unload/Destroy contract at pre-Pause dispatch; no product VM."""
import copy
from contextlib import ExitStack
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import Mock,patch

sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'tests/system'))
import lifecycle_freeze_dispatch as dispatch
import lifecycle_freeze_destroy as frozen
import lifecycle_unload_capture_faults as capture
from tools.tests import test_lifecycle_freeze_destroy as legacy


def fixture(root):
    original=legacy.fixture(root)
    p=dispatch.FreezeDispatchDestroyProbe.__new__(dispatch.FreezeDispatchDestroyProbe)
    p.__dict__.update(original.probe.__dict__)
    p.host.poll.return_value=87
    original.probe=p
    return original


class FreezeDispatchTests(unittest.TestCase):
    def test_only_original_before_call_selector_and_exit87_are_selected(self):
        with tempfile.TemporaryDirectory() as directory:
            p=fixture(Path(directory)).probe
            self.assertEqual(p._fault_environment(),{'SANDPROOF_TEST_EXIT_BEFORE_HOST_ACTION':'freeze_vm'})
            from _center_host_services import fault_environment
            self.assertEqual(fault_environment(p._fault_environment()),p._fault_environment())
            self.assertEqual(p._cut_exit_code(),87)
            self.assertEqual(p._cut_stage_label(),'freeze-before-effect')
            self.assertFalse(p._paused_cut())
            self.assertEqual(frozen.FreezeDestroyProbe._cut_exit_code(p),86)
            self.assertEqual(frozen.FreezeDestroyProbe._fault_environment(p),
                {'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION':'freeze_vm'})

    def test_same_full_contract_checks_precede_live_source_observation_and_no_signal_is_sent(self):
        for fault in ('none','exit86','receipt','source','resource','pin','material'):
            with tempfile.TemporaryDirectory() as directory,ExitStack() as stack,self.subTest(fault=fault):
                f=fixture(Path(directory));p=f.probe;p.accepted=None;p.host.wait.return_value=86 if fault=='exit86' else 87
                if fault=='receipt':f.snapshot['original_actions'][0]=(f.freeze,'receipted',{})
                if fault=='source':f.rows[1]['work']['requirement']['source']['pid']+=1
                if fault=='resource':f.freeze['identity']['resource']['id']='vm-other'
                if fault=='pin':f.rows[0]['restore']['material_use']=False
                if fault=='material':f.runner._materials.return_value={'changed':'bytes'}
                p._read=Mock(return_value=(p.unload_entry,f.rows,f.snapshot['original_actions']))
                p._source_state=Mock(return_value='S')
                observed=stack.enter_context(patch.object(dispatch,'paused_source',return_value={'state':'Running'}))
                send=stack.enter_context(patch.object(frozen.signal,'pidfd_send_signal'))
                stack.enter_context(patch.object(frozen,'pidfd_exited',return_value=False))
                stack.enter_context(patch.object(capture,'pidfd_exited',return_value=False))
                stack.enter_context(patch.object(capture,'original_process',return_value=True))
                if fault=='none':
                    result=p.after_acceptance(f.accepted(f.unload),p.deadline)
                    self.assertEqual(result['stage'],'freeze-before-effect')
                    self.assertFalse(result['current_pause_observed']);self.assertFalse(result['original_pidfd_stopped'])
                    self.assertNotIn('original_frozen_unload',result)
                    self.assertEqual(result['original_dispatched_unload']['source_observation']['state'],'Running')
                    self.assertFalse(p.pause_requested);self.assertFalse(p.candidate_paused)
                    self.assertEqual(p.action,f.freeze)
                    self.assertTrue(all(call.kwargs=={'expected_state':'Running'} for call in observed.call_args_list))
                else:
                    with self.assertRaises(ValueError):p.after_acceptance(f.accepted(f.unload),p.deadline)
                    observed.assert_not_called()
                send.assert_not_called()

    def test_stopped_or_unavailable_source_is_not_relabelled_as_running_or_no_request(self):
        for state in ('S','T','t'):
            with tempfile.TemporaryDirectory() as directory:
                p=fixture(Path(directory)).probe;p._source_state=Mock(return_value=state)
                with patch.object(dispatch,'paused_source',return_value={'state':'Running'}) as read, \
                        patch.object(frozen.signal,'pidfd_send_signal') as signal:
                    if state=='S':
                        p._hold_source(p.deadline);self.assertEqual(p._release_source_hold(p.deadline),{'state':'Running'})
                        self.assertFalse(p.pause_requested);self.assertFalse(p.continue_sent)
                    else:
                        with self.assertRaises(ValueError):p._hold_source(p.deadline)
                        read.assert_not_called()
                    signal.assert_not_called()
        with tempfile.TemporaryDirectory() as directory:
            p=fixture(Path(directory)).probe;p._source_state=Mock(return_value='S')
            with patch.object(dispatch,'paused_source',side_effect=TimeoutError('unknown original VMM')):
                with self.assertRaises(TimeoutError):p._hold_source(p.deadline)

    def test_old_freeze_stays_unknown_and_a_new_frozen_result_cannot_be_hidden(self):
        with tempfile.TemporaryDirectory() as directory:
            f=fixture(Path(directory));p=f.probe
            self.assertEqual(p._inventory(f.snapshot),f.snapshot['original_actions']+f.snapshot['terminal_actions'])
            row=f.snapshot['original_actions'][0]
            result=f.receipt(f.freeze,'Frozen',dict(pid=123,start_ticks=456,boot_id='boot'))
            f.snapshot['original_actions'][0]=(row[0],'receipted',result)
            with self.assertRaises(ValueError):p._inventory(f.snapshot)

    def test_terminal_cleanup_requires_original_stop_kernel_exit_and_no_observer_signals(self):
        for fault in ('none','observer-stop','observer-continue','no-exit','still-live','wrong-stop'):
            with tempfile.TemporaryDirectory() as directory,self.subTest(fault=fault):
                f=fixture(Path(directory));p=f.probe;p.recovered=p.stop_released=True
                p.before_stop=copy.deepcopy(f.snapshot);snapshot=copy.deepcopy(f.cleanup)
                p.terminal_host=Mock();p.terminal_host.wait.return_value=86
                p._read_terminal=Mock(return_value=snapshot)
                p.runtime.services.processes.start=Mock();p.runtime.services.wait_ready=Mock()
                if fault=='observer-stop':p.pause_requested=True
                if fault=='observer-continue':p.continue_sent=True
                if fault=='wrong-stop':snapshot['terminal_actions'][1][2]['outcome']['details']['result']['Stopped']['pid']+=1
                with patch.object(frozen,'pidfd_exited',return_value=fault!='no-exit'), \
                        patch.object(frozen,'original_process',return_value=fault=='still-live'):
                    if fault=='none':self.assertTrue(p.assert_terminal_before_gc(p.deadline)['original_source_exited'])
                    else:
                        with self.assertRaises(ValueError):p.assert_terminal_before_gc(p.deadline)
                        p.runtime.services.processes.start.assert_not_called()


class RunningUnixTests(unittest.TestCase):
    def test_actual_unix_peer_and_one_get_bind_current_running_source(self):
        with legacy.OriginalUnixPauseTests().endpoint(state='Running') as (runner,prior,seen):
            value=frozen.paused_source(runner,prior,time.monotonic()+2,expected_state='Running')
            self.assertEqual(value['state'],'Running');self.assertTrue(value['read_only'])
            self.assertFalse(value['execution_authority']);self.assertEqual(len(seen),1)

    def test_paused_wrong_identity_version_extra_fields_and_framing_fail_same_running_contract(self):
        for fault in ('none','version','resource','extra','length'):
            state='Paused' if fault=='none' else 'Running'
            with self.subTest(fault=fault),legacy.OriginalUnixPauseTests().endpoint(fault,state=state) as (runner,prior,_):
                with self.assertRaises(ValueError):frozen.paused_source(runner,prior,time.monotonic()+2,expected_state='Running')


class OriginalSceneTests(unittest.TestCase):
    def test_real_scene_keeps_original_unload_destroy_contract_and_peer_cleanup_order(self):
        legacy.SceneAndReplayTests().exercise(dispatch=True)

    def test_unknown_terminal_stop_keeps_both_exact_sessions_and_no_replacement_work(self):
        legacy.SceneAndReplayTests().exercise(fault=True,dispatch=True)


if __name__=='__main__':unittest.main()
