"""Controlled checks for expiry observations, not an alternative lifecycle model."""
import copy
import json
from pathlib import Path
import sys
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock,patch

SYSTEM=Path(__file__).resolve().parents[2]/'tests/system'
sys.path.insert(0,str(SYSTEM))
import lifecycle_unload_expiry as expiry


class ExpiryObservationTests(unittest.TestCase):
    def fixture(self):
        key=dict(deployment='deployment',owner='1',window=1,sequence=3)
        resume=dict(session='session',key=key,accepted_version=3)
        lifetime=dict(started_at_ms=1000,end_at_ms=76000,max_end_at_ms=601000,startup_deadline_ms=301000)
        binding=dict(session='session',node={'node':'original'},protocol_version=2,
                     key=key,expected={'state':'version','version':2})
        prior=dict(binding=binding,accepted_version=3,center={'center':'original'},
                   retention={'slot':1},operation={'kind':'resume'},acceptance_time={'kind':'sampled','at_ms':2000})
        terminal=copy.deepcopy(prior);terminal.update(accepted_version=4,operation={'kind':'destroy'},acceptance_time={'kind':'unsampled'})
        terminal['binding'].update(key=dict(key,window=2,sequence=7),expected={'state':'version','version':3})
        row=dict(create=dict(binding={'session':'session'},timeout_ms=75000),
                 operations=[dict(operation={'kind':'create'}),dict(operation={'kind':'unload'}),prior,terminal],
                 terminating=True,released=False,last_running=dict(lifetime,observed_at_ms=2000))
        return {'sandboxes':[row]},resume,lifetime

    def test_only_observed_internal_terminal_key_is_returned(self):
        audit,resume,lifetime=self.fixture()
        accepted=expiry.expiry_acceptance(audit,'session',resume,lifetime,76000)
        self.assertEqual(accepted,dict(session='session',key=dict(resume['key'],window=2,sequence=7),accepted_version=4))
        self.assertEqual(resume['key']['window'],1)
        self.assertEqual(audit['sandboxes'][0]['operations'][-1]['acceptance_time'],{'kind':'unsampled'})

    def test_clock_or_pending_history_cannot_be_promoted_to_expiry(self):
        audit,resume,lifetime=self.fixture()
        with self.assertRaises(ValueError):expiry.expiry_acceptance(audit,'session',resume,lifetime,75999)
        row=audit['sandboxes'][0];row['operations'].pop();row['terminating']=False
        self.assertIsNone(expiry.expiry_acceptance(audit,'session',resume,lifetime,76000))

    def test_foreign_or_public_destroy_changed_lifetime_and_early_release_fail(self):
        original,resume,lifetime=self.fixture()
        mutations=[
            lambda r:r['operations'][-1]['binding']['key'].update(window=1),
            lambda r:r['operations'][-1]['binding']['key'].update(owner='2'),
            lambda r:r['operations'][-1]['binding']['expected'].update(version=2),
            lambda r:r['operations'][-1].update(accepted_version=5),
            lambda r:r['last_running'].update(end_at_ms=76001),
            lambda r:r.update(released=True),
            lambda r:r['operations'][-1].update(acceptance_time={'kind':'sampled','at_ms':76000}),
        ]
        for mutate in mutations:
            audit=copy.deepcopy(original);mutate(audit['sandboxes'][0])
            with self.subTest(mutate=mutate),self.assertRaises(ValueError):
                expiry.expiry_acceptance(audit,'session',resume,lifetime,76000)

    def test_explicit_destroy_is_rejected_before_any_background_observation(self):
        runtime=Mock();client=SimpleNamespace(destroy_request={'window':1})
        with self.assertRaises(ValueError):
            expiry.observe_center_expiry(runtime,client,{}, {},time.monotonic()+1,host_delivered=True)
        self.assertEqual(runtime.mock_calls,[])

    def test_wait_for_real_time_uses_original_deadline(self):
        with patch.object(expiry,'now_ms',side_effect=[75900,76000]),patch.object(expiry.time,'sleep') as sleep:
            expiry.wait_clock({'end_at_ms':76000},time.monotonic()+1)
        sleep.assert_called_once_with(.1)
        with patch.object(expiry,'now_ms',return_value=76000),self.assertRaises(ValueError):
            expiry.require_before_expiry({'end_at_ms':76000})

    def test_cleaned_wait_does_not_send_or_time_a_destroy(self):
        client=Mock();client.owner_key='key';client.path.return_value='/session';client.current_acceptance={'original':'expiry'}
        client.call.side_effect=[(200,json.dumps({'phase':'terminating'}).encode()),(200,json.dumps({'phase':'cleaned'}).encode())]
        with patch.object(expiry.time,'sleep'):
            self.assertEqual(expiry.wait_expired_cleaned(client,time.monotonic()+1),{'phase':'cleaned'})
        self.assertTrue(all(c.args[0]=='GET' for c in client.call.call_args_list))
        client.destroy.assert_not_called();client.operation_sent.assert_not_called()

    def test_expired_wait_rejects_a_revived_running_state(self):
        client=Mock();client.owner_key='key';client.path.return_value='/session'
        client.call.return_value=(200,b'{"phase":"running"}')
        with self.assertRaises(ValueError):expiry.wait_expired_cleaned(client,time.monotonic()+1)


class PrepareExpiryBarrierTests(unittest.TestCase):
    def fixture(self):
        probe=expiry.ExpiryPrepareProbe.__new__(expiry.ExpiryPrepareProbe)
        probe.action=dict(identity=dict(epoch=3,generation=2,resource='vm'),
                          command={'logical':{'command':{'PrepareVm':{'vm':2}}}})
        row=(probe.action,'dispatched',None)
        probe.config={'sandbox':'sandbox'}
        probe.runtime=Mock();probe.runtime.services.native_pools=[{'node_state_dir':'/unused'}]
        probe.runtime._materials.return_value={'materials':[1,2,3]}
        probe.restore_record=dict(work='work',capture={'full':'full'},retirement=None,
                                  restore=dict(intent='intent',activation=None,material_use=True))
        probe.restore_entry=probe.restore_record['restore']
        snapshot=dict(original={'local_operation':3},terminal={'local_operation':3},
                      local={'sandbox':1},resume_actions=[row],terminal_actions=[row],
                      sources=[copy.deepcopy(probe.restore_record)],
                      native=dict(epoch=3,generation=2,terminal=True,phase='Revoking',
                                  desired='Closed',effective='Closed',current_vm=None,net_slot=None,
                                  current_operation=dict(id=3,epoch=3,kind='Resume',
                                                         status={'Compensating':'DeadlineExceeded'})))
        return probe,snapshot

    def test_shared_operation_is_counted_once_but_real_duplicates_and_activation_fail(self):
        probe,snapshot=self.fixture()
        self.assertEqual(probe._terminal_inventory(snapshot),snapshot['resume_actions'][0])
        duplicate=copy.deepcopy(snapshot)
        duplicate['resume_actions']*=2;duplicate['terminal_actions']*=2
        with self.assertRaises(ValueError):probe._terminal_inventory(duplicate)
        inconsistent=copy.deepcopy(snapshot);inconsistent['terminal_actions']=[]
        with self.assertRaises(ValueError):probe._terminal_inventory(inconsistent)
        activation=(dict(command={'logical':{'command':{'ActivateVm':{}}}}),'ready',None)
        snapshot['resume_actions'].append(activation);snapshot['terminal_actions'].append(activation)
        with self.assertRaises(ValueError):probe._terminal_inventory(snapshot)

    def test_only_closed_exact_deadline_compensation_allows_fault_withdrawal(self):
        probe,snapshot=self.fixture()
        with patch.object(expiry,'original_json',return_value=dict(epoch=3,terminal=False)), \
                patch.object(expiry,'material_source_charge',return_value=42):
            self.assertEqual(probe._terminal_pause_barrier(snapshot)['kind'],'same-resume-deadline-compensation')
            for changes in ({'effective':'Open'},{'epoch':4},{'terminal':False},
                            {'current_operation':dict(id=3,epoch=3,kind='Resume',status='Running')}):
                bad=copy.deepcopy(snapshot);bad['native'].update(changes)
                with self.subTest(changes=changes),self.assertRaises(ValueError):probe._terminal_pause_barrier(bad)
            bad=copy.deepcopy(snapshot);bad['sources'][0]['restore']['material_use']=False
            with self.assertRaises(ValueError):probe._terminal_pause_barrier(bad)
        with patch.object(expiry,'original_json',return_value=dict(epoch=2,terminal=False)),self.assertRaises(ValueError):
            probe._terminal_pause_barrier(snapshot)

    def test_cleanup_cut_requires_real_matching_terminal_fence_receipt_and_file(self):
        probe,snapshot=self.fixture()
        def receipt(action,kind,value):
            return (action,'receipted',dict(identity=action['identity'],outcome=dict(
                status='observed_success',details=dict(resource=action['identity']['resource'],
                command=action['command']['logical']['command'],result={kind:value}))))
        prepared=receipt(probe.action,'Prepared',dict(pid=1,start_ticks=2,boot_id='boot'))
        fence=dict(identity=dict(epoch=3,resource='fence'),command={'logical':{'command':{
            'InstallFence':dict(sandbox=1,epoch=3,terminal=True)}}})
        expected=dict(epoch=3,terminal=True)
        snapshot['resume_actions']=snapshot['terminal_actions']=[prepared]
        with self.assertRaises(ValueError):probe._terminal_inventory(snapshot)
        snapshot['resume_actions'].append((fence,'dispatched',None))
        with self.assertRaises(ValueError):probe._terminal_inventory(snapshot)
        snapshot['resume_actions'][-1]=receipt(fence,'Fence',expected)
        with patch.object(expiry,'original_json',return_value=expected):
            self.assertEqual(probe._terminal_inventory(snapshot),prepared)
        with patch.object(expiry,'original_json',return_value=dict(epoch=3,terminal=False)),self.assertRaises(ValueError):
            probe._terminal_inventory(snapshot)


if __name__=='__main__':unittest.main()
