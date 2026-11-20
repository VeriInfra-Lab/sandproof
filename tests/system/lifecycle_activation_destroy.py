"""One E8 terminal handoff after actual activation, before its SQL receipt.

Consumes the original E7 gate and #793's read-only candidate observation. No
VM command, replacement load, journal write or synthetic host receipt lives
here. The original product owns Stop, Cleanup, material retirement and release.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import copy
import hashlib
import json
import os
from pathlib import Path
import socket
import stat
import struct
import time

from _internal_fault_relay import accepted_request, same_json, _fields, _u64
from lifecycle_resume_receipt_loss import observe_candidate
from lifecycle_unload_capture_faults import CaptureHandoffProbe, _AwaitTerminal
from lifecycle_unload_runtime import material_source_charge, open_original_candidate_pidfd, original_json, original_process, pidfd_exited, remaining
from tools.check_kvm_host import unique_keys
from tools.system_acceptance import private_path


class ActivationDestroyProbe(CaptureHandoffProbe):
    def __init__(self,runtime,client,accepted,receipt,deadline):
        _fields(receipt,'session operation candidate namespace action_sha256 ram_nonce_sha256 activation_nonce_sha256 guest_pid guest_start_time_ticks counter metadata_generation public_authorization_proven')
        baseline=_fields(client.witness,'nonce activation_nonce metadata_generation counter pid start_time_ticks')
        for name in ('guest_pid','guest_start_time_ticks','counter','metadata_generation'):_u64(receipt[name])
        for name in ('action_sha256','ram_nonce_sha256','activation_nonce_sha256'):
            if type(receipt[name]) is not str or len(receipt[name])!=64 or any(c not in '0123456789abcdef' for c in receipt[name]):
                raise ValueError('original bounded gate digest required')
        if (receipt['ram_nonce_sha256']!=hashlib.sha256(baseline['nonce'].encode()).hexdigest()
                or receipt['activation_nonce_sha256']==hashlib.sha256(baseline['activation_nonce'].encode()).hexdigest()
                or not same_json([receipt['guest_pid'],receipt['guest_start_time_ticks'],receipt['counter'],receipt['metadata_generation']],
                    [baseline['pid'],baseline['start_time_ticks'],baseline['counter'],baseline['metadata_generation']+1])):
            raise ValueError('original guest state and a fresh activation RAM witness required')
        gate=runtime.restore_gate
        if (gate is None or gate['command_sent'] or gate['session']!=client.session
                or receipt['session']!=client.session or not same_json(receipt['operation'],accepted['key'])
                or not same_json(accepted,client.current_acceptance)
                or receipt['public_authorization_proven'] is not False):
            raise ValueError('original observed activation gate and current Resume required')
        self.deadline=min(deadline,gate['deadline'])
        # The complete saved intent is selected in the original read-only SQL
        # snapshot below. Only its public key is known to the caller so far.
        prior=copy.deepcopy(runtime.runs[client.session])
        super().__init__(runtime,client,prior,'restore-prepare-destroy')
        self.accepted=copy.deepcopy(accepted);self.receipt=copy.deepcopy(receipt)
        self.host=gate['host'];self.stop_peer=None;self.stop_released=False;self.terminal_cut=None
        self.stop_path=private_path(runtime.quota['deployment_control'],directory=True)/'activation-stop.sock'
        if self.stop_path.exists() or self.stop_path.is_symlink() or len(os.fsencode(self.stop_path))>=108:
            raise ValueError('fresh bounded original terminal Stop socket required')
        self._configuration();runtime._gate_files_unchanged()
        entry,records,actions=self._read(self.deadline)
        accepted_request(entry['intent'],runtime.services.host_configs[0]['inbox']['node'])
        if entry['intent']['request']['operation']['kind']!='resume':raise ValueError('original Resume intent required')
        self.restore_entry=copy.deepcopy(entry)
        candidate=observe_candidate(runtime,entry['intent'],self.deadline)
        self.action=copy.deepcopy(candidate['claim']['activation'])
        self.config=dict(sandbox=self.action['identity']['sandbox'])
        if (set(receipt['candidate'])!={'resource_id','pid','start_time_ticks','boot_id'}
                or not same_json({k:candidate['source'][k] for k in receipt['candidate']},receipt['candidate'])
                or hashlib.sha256(json.dumps(self.action,sort_keys=True,separators=(',',':')).encode()).hexdigest()!=receipt['action_sha256']
                or candidate['context']['position']==prior['context']['position']
                or candidate['context']['binding']['generation']<=prior['context']['binding']['generation']):
            raise ValueError('original observed activated candidate, marker or reused position differs')
        self.prior=dict(session=client.session,local=copy.deepcopy(candidate['claim']['local']),
            context=copy.deepcopy(candidate['context']),source=copy.deepcopy(candidate['source']),
            lifetime=copy.deepcopy(prior['lifetime']))
        self.candidate=copy.deepcopy(candidate);self.initial_record=copy.deepcopy(self._backing(records,initial=True))
        self.material=runtime._materials(self.initial_record['capture']['full'],self.initial_record['capture'])
        self._resume_inventory(actions,initial=True)
        runtime._gate_files_unchanged()
        if self.host.poll() is not None or not original_process(self.prior['source']):
            raise ValueError('original live activated candidate and held manager required')
        self.pidfd=open_original_candidate_pidfd(self.prior['source'])

    def _backing(self,records,*,initial=False):
        target=[r for r in records if r['work']['anchor']['local']['sandbox']==self.prior['local']['sandbox']]
        if len(target)!=1:raise ValueError('one original source, without a new capture, required')
        record=target[0];restore=record['restore'];capture=record['capture']
        if (restore is None or not same_json(restore['intent'],self.restore_entry['intent'])
                or not same_json(restore,self.candidate['claim']) or restore['material_use'] is not True
                or record['retirement'] is not None or record['material_release']['consumer'] is not None
                or capture is None or not same_json(capture['completed'],self.restore_entry['intent']['request']['operation']['manifest_digest'])
                or material_source_charge(record)<=0):
            raise ValueError('same original manifest, activation, running pin and positive charge required')
        if not initial:
            # A historical producer End or producer-release witness may arrive
            # later. Neither changes immutable Full input nor frees its live
            # consumer; preserve raw samples without freezing these histories.
            stable=lambda r:dict(work=r['work'],restore=r['restore'],capture={k:v for k,v in r['capture'].items() if k!='end'})
            if not same_json(stable(record),stable(self.initial_record)):
                raise ValueError('immutable original Full/source/restore work changed')
        return record

    def _resume_inventory(self,rows,*,initial=False):
        prepared=[r for r in rows if 'PrepareVm' in r[0]['command']['logical']['command']]
        activated=[r for r in rows if 'ActivateVm' in r[0]['command']['logical']['command']]
        if len(prepared)!=1 or len(activated)!=1 or not same_json(activated[0][0],self.action):
            raise ValueError('one original Prepare and one original activation marker required')
        if any(set(r[0]['command']['logical']['command']) & {'FreezeVm','SealSnapshot','CommitSnapshot'} for r in rows):
            raise ValueError('terminal activation handoff created another capture')
        process=dict(pid=self.prior['source']['pid'],start_ticks=self.prior['source']['start_time_ticks'],boot_id=self.prior['source']['boot_id'])
        if (not same_json(prepared[0][0]['identity']['resource'],self.action['identity']['resource'])
                or not same_json(self._success_observation(prepared[0],'Prepared'),process)):
            raise ValueError('original prepared process differs from actual activated source')
        if initial and activated[0][1:]!=('dispatched',None):raise ValueError('activation SQL receipt already saved at gate')
        observed=self._success_observation(activated[0],'Activated')
        if observed is not None and not same_json(observed,process):
            raise ValueError('late original activation observation changed process')
        if not initial and not same_json(prepared[0][0],self.prepare_action):
            raise ValueError('original Prepare action changed')
        if initial:self.prepare_action=copy.deepcopy(prepared[0][0])

    def _terminal_entries(self,inbox):
        original,terminal,local=super()._terminal_entries(inbox)
        expected=copy.deepcopy(self.restore_entry['intent'])
        expected['accepted_version']=self.terminal_accepted['accepted_version']
        expected['request']['binding']['key']=copy.deepcopy(self.terminal_accepted['key'])
        expected['request']['binding']['expected']={'state':'version','version':self.accepted['accepted_version']}
        expected['request']['operation']={'kind':'destroy'}
        accepted_request(terminal['intent'],self.runtime.services.host_configs[0]['inbox']['node'])
        if not same_json(terminal['intent'],expected):raise ValueError('complete terminal intent/context/retention changed')
        return original,terminal,local

    def _bind_terminal_acceptance(self,accepted):
        # The candidate was actually activated, never OS-paused by this probe.
        # Reuse the exact public successor contract without the pre-marker
        # probe's physical-Paused precondition.
        if (self.terminal_accepted is not None or self.recovered
                or type(accepted) is not dict or set(accepted)!={'session','key','accepted_version'}
                or not same_json(accepted,self.client.current_acceptance)
                or accepted['session']!=self.accepted['session']
                or _u64(accepted['accepted_version'],positive=True)!=self.accepted['accepted_version']+1
                or same_json(accepted['key'],self.accepted['key'])
                or any(accepted['key'][k]!=self.accepted['key'][k] for k in ('deployment','owner'))
                or not same_json(self.client.destroy_request,dict(window=accepted['key']['window'],
                    sequence=accepted['key']['sequence'],expected_version=self.accepted['accepted_version']))):
            raise ValueError('one exact public Destroy after original activated Resume required')
        self.terminal_accepted=copy.deepcopy(accepted)

    def _terminal_extra(self,conn,local,capacity):
        count,largest=conn.execute("SELECT count(*),max(length(action_json)) FROM outbox WHERE dispatch_state='dispatched'").fetchone()
        if count>capacity['max_sql_rows'] or (largest is not None and largest>capacity['max_sql_value_bytes']):
            raise ValueError('bounded global pending actions required')
        stops=[json.loads(raw,object_pairs_hook=unique_keys) for raw, in conn.execute(
            "SELECT action_json FROM outbox WHERE dispatch_state='dispatched' ORDER BY id")]
        stops=[a for a in stops if 'StopVm' in a['command']['logical']['command']]
        row=conn.execute('SELECT owner_id,started_at_ms,deadline_ms,max_deadline_ms,startup_deadline_ms FROM sandbox_lifetimes WHERE sandbox_id=?',
            (self.config['sandbox'],)).fetchone()
        if row is None or row[0]!='tenant-'+str(local['owner']):raise ValueError('original native lifetime owner required')
        lifetime=dict(zip(('started_at_ms','end_at_ms','max_end_at_ms','startup_deadline_ms'),row[1:],strict=True))
        for value in lifetime.values():_u64(value)
        return dict(global_pending_stops=stops,lifetime=lifetime)

    def _stop_barrier(self,snapshot):
        self._resume_inventory(snapshot['resume_actions']+snapshot['terminal_actions'])
        self._backing(snapshot['sources']);native=snapshot['native'];binding=self.prior['context']['binding']
        operation=native['current_operation'];epoch=native['epoch']
        if (native['terminal'] is not True or native['phase']!='Revoking'
                or native['generation']!=binding['generation'] or native['current_vm']!=binding['vm']
                or native['net_slot']!=binding['network'] or operation is None or operation['kind']!='Kill'
                or operation['id']!=snapshot['terminal']['local_operation'] or operation['epoch']!=epoch
                or not same_json(snapshot['lifetime'],self.prior['lifetime']) or epoch<=self.action['identity']['epoch']):
            raise ValueError('same activated source/lifetime and current terminal Kill required')
        fences=[r for r in snapshot['terminal_actions'] if 'InstallFence' in r[0]['command']['logical']['command']]
        stops=[r for r in snapshot['terminal_actions'] if 'StopVm' in r[0]['command']['logical']['command']]
        if len(fences)!=1 or len(stops)!=1 or stops[0][1:]!=('dispatched',None):raise ValueError('one current fence and pending Stop required')
        stop=stops[0][0];fence=fences[0][0]
        if (not same_json(snapshot['global_pending_stops'],[stop])
                or not same_json(stop['identity']['resource'],self.action['identity']['resource'])
                or stop['identity']['epoch']!=epoch
                or not same_json(stop['command']['logical']['command'],{'StopVm':dict(sandbox=binding['sandbox'],generation=binding['generation'],vm=binding['vm'],epoch=epoch)})
                or fence['identity']['epoch']!=epoch
                or not same_json(fence['identity']['resource'],dict(kind='sandbox',id=self.config['sandbox']))
                or not same_json(fence['command']['logical']['command'],{'InstallFence':dict(sandbox=binding['sandbox'],epoch=epoch,terminal=True)})):
            raise ValueError('generic Stop gate differs from the one original current terminal action')
        observed=self._success_observation(fences[0],'Fence')
        actual=original_json(Path(self.runtime.services.native_pools[0]['node_state_dir'])/'fences'/(self.config['sandbox']+'.json'),16384)
        if not same_json(observed,dict(epoch=epoch,terminal=True)) or not same_json(actual,observed):
            raise ValueError('actual terminal fence and complete original receipt required')
        return actual

    def _live(self):
        if self.pidfd is None or pidfd_exited(self.pidfd) or not original_process(self.prior['source']):
            raise ValueError('exact original activated process is no longer live')

    def recover_after_destroy(self,accepted,deadline,*,observe_peer):
        self._bind_terminal_acceptance(accepted);deadline=min(deadline,self.deadline)
        self._live();self._configuration();self.runtime.finish_resume_gate(self.receipt,deadline)
        current=observe_candidate(self.runtime,self.restore_entry['intent'],deadline)
        if not same_json(current,self.candidate):raise ValueError('original activated candidate changed before terminal recovery')
        # This is an exact supervisor cleanup registration, never public Running.
        self.runtime.runs[self.client.session]=copy.deepcopy(self.prior)
        self.terminal_host=self.runtime.services.processes.start('host-0',env={
            'SANDPROOF_TEST_STOP_GATE':str(self.stop_path),'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION':'cleanup_vm'})
        if self.terminal_host is self.host or self.terminal_host.pid==self.host.pid:raise ValueError('fresh original manager required')
        self.runtime.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)
        while not self.stop_path.exists():
            remaining(deadline)
            if self.terminal_host.poll() is not None:raise ValueError('manager exited before original Stop gate')
            time.sleep(min(.05,remaining(deadline)))
        before=self.stop_path.lstat();role=self.runtime.services.processes.bindings['host-0']
        if not stat.S_ISSOCK(before.st_mode) or before.st_uid!=role['uid']:raise ValueError('original private Stop gate required')
        peer=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);self.stop_peer=peer
        peer.settimeout(min(2,remaining(deadline)));peer.connect(str(self.stop_path))
        after=self.stop_path.lstat()
        if (struct.unpack('3i',peer.getsockopt(socket.SOL_SOCKET,socket.SO_PEERCRED,12))!=(self.terminal_host.pid,role['uid'],role['gid'])
                or (before.st_dev,before.st_ino)!=(after.st_dev,after.st_ino)):
            raise ValueError('Stop gate manager or inode changed')
        self.before_stop=self._read_terminal(deadline);fence=self._stop_barrier(self.before_stop)
        self._live();material=self._backing(self.before_stop['sources'])
        if not same_json(self.runtime._materials(material['capture']['full'],material['capture']),self.material):
            raise ValueError('original backing files changed before terminal Stop')
        progress=observe_peer()
        final=self._read_terminal(deadline);self._stop_barrier(final);self._live();self._configuration()
        if not same_json(self.runtime._materials(material['capture']['full'],material['capture']),self.material):
            raise ValueError('original backing files changed during peer progress')
        self.before_stop=copy.deepcopy(final);peer.settimeout(min(2,remaining(deadline)))
        peer.sendall(b'g');self.stop_released=True;peer.close();self.stop_peer=None;self.recovered=True
        return dict(actual_activation_observed_before_destroy=True,activation_ram_receipt=self.receipt,
            terminal_fence=fence,original_pin_and_positive_charge_retained=True,peer_progress=progress,
            snapshot=self.before_stop,stop_gate_release_sent=True,public_completion=False,capacity_released=False)

    def assert_terminal_before_gc(self,deadline):
        if not self.recovered or not self.stop_released or self.terminal_cut is not None:raise ValueError('one original terminal handoff required')
        deadline=min(deadline,self.deadline)
        if (self.terminal_host is not self.runtime.services.processes.children['host-0']
                or self.terminal_host.wait(timeout=remaining(deadline))!=86):raise ValueError('original Cleanup-success/pre-SQL cut required')
        self._configuration();snapshot=self._read_terminal(deadline)
        self._resume_inventory(snapshot['resume_actions']+snapshot['terminal_actions'])
        stops=[r for r in snapshot['terminal_actions'] if 'StopVm' in r[0]['command']['logical']['command']]
        cleanups=[r for r in snapshot['terminal_actions'] if 'CleanupVm' in r[0]['command']['logical']['command']]
        if len(stops)!=1 or len(cleanups)!=1 or cleanups[0][1:]!=('dispatched',None):raise ValueError('one original Stop and pending Cleanup required')
        epoch=self.before_stop['native']['epoch'];binding=self.prior['context']['binding']
        expected=dict(sandbox=binding['sandbox'],generation=binding['generation'],vm=binding['vm'])
        for row,command in ((stops[0],{'StopVm':dict(expected,epoch=epoch)}),(cleanups[0],{'CleanupVm':expected})):
            if (not same_json(row[0]['command']['logical']['command'],command) or row[0]['identity']['epoch']!=epoch
                    or not same_json(row[0]['identity']['resource'],self.action['identity']['resource'])):
                raise ValueError('terminal Stop/Cleanup changed original candidate or epoch')
        observed=self._success_observation(stops[0],'Stopped')
        if (not same_json(observed,dict(pid=self.prior['source']['pid'],start_ticks=self.prior['source']['start_time_ticks']))
                or not pidfd_exited(self.pidfd) or original_process(self.prior['source']) or self.stop_path.exists()):
            raise ValueError('original Stop and kernel-confirmed candidate exit required')
        self.terminal_cut=copy.deepcopy(snapshot);self._configuration()
        process=self.runtime.services.processes.start('host-0')
        if process is self.terminal_host or process.pid==self.terminal_host.pid:raise ValueError('fresh original cleanup manager required')
        self.runtime.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)
        return dict(original_prepare_and_activation_identity_retained=True,original_stop_observed=True,
            original_candidate_exited=True,cleanup_success_pre_sql_exit86=True,cleanup_sql_ack_pending=True,
            snapshot=self.terminal_cut,public_completion=False,capacity_released=False)

    def terminal_failure_state(self):
        return dict(original_resume=copy.deepcopy(self.accepted),destroy=copy.deepcopy(self.terminal_accepted),
            original_candidate=copy.deepcopy(self.prior['source']),stop_gate_release_sent=self.stop_released,
            public_completion=False,capacity_released=False,action='preserve original work and HostLease; no new business request')

    def cleanup(self):
        if not self.runtime.gc_complete or self.stop_path.exists() or not self.stop_released or self.stop_peer is not None:
            raise ValueError('original product GC and consumed Stop gate required')
        return dict(original_stop_gate_removed=True,observation_handles_only=True)

    def close(self):
        if self.stop_peer is not None:self.stop_peer.close();self.stop_peer=None
        super().close()
