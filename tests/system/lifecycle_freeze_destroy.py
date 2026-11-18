"""One E8 Freeze-success/pre-receipt -> public Destroy consumer.

Reuses the original Freeze exit86, pre-Stop gate and Cleanup exit86. Holds the
checked original pidfd only to order terminal delivery before re-observation;
never writes a journal, activates a VM, manufactures a receipt or retries a
business operation. Read-only VM state samples are not execution authority.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import copy
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import socket
import signal
import stat
import struct
import time

from _internal_fault_relay import _fields, _u64, accepted_request
from lifecycle_unload_capture_faults import CaptureHandoffProbe, _AwaitTerminal
from lifecycle_unload_runtime import material_source_charge, original_json, original_process, pidfd_exited, remaining
from tools.check_kvm_host import unique_keys
from tools.system_acceptance import private_path
from tools.template_build_guest import equal


def paused_source(runtime, prior, deadline, *, expected_state='Paused'):
    """One GET on the actual original VM's checked Unix stream; no VM mutation.

    SO_PEERCRED belongs to this connection. A pathname, old Frozen result or
    process state T alone cannot replace the current Firecracker observation.
    """
    if expected_state not in ('Paused','Running'):
        raise ValueError('one exact current physical VMM state required')
    native=runtime.services.native_pools[0];source=prior['source']
    resource='vm-'+native['namespace']+'-'+str(prior['context']['binding']['vm'])
    if source['resource_id']!=resource or not original_process(source):
        raise ValueError('original source process required before pause query')
    claim=Path(native['chroot_base'])/'.claims'/resource/'record.json'
    if not equal(original_json(claim,16384),source):raise ValueError('original process claim changed')
    root=Path(native['chroot_base'])/Path(native['firecracker']).name/resource/'root'
    if root.resolve(strict=True)!=root:raise ValueError('canonical original jail required')
    fd=os.open(root,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
    try:
        held=os.fstat(fd);before=os.stat('api.socket',dir_fd=fd,follow_symlinks=False)
        uid=native['bounded_host']['uid_base']+prior['context']['position']
        if not stat.S_ISSOCK(before.st_mode) or before.st_uid!=uid:
            raise ValueError('original VMM API socket required')
        with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as peer:
            peer.settimeout(min(2,remaining(deadline)))
            peer.connect('/proc/self/fd/'+str(fd)+'/api.socket')
            actual=struct.unpack('3i',peer.getsockopt(socket.SOL_SOCKET,socket.SO_PEERCRED,12))
            if actual!=(source['pid'],uid,uid):raise ValueError('pause query peer differs from original VMM')
            peer.sendall(b'GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n')
            response=http.client.HTTPResponse(peer)
            try:
                response.begin();lengths=response.headers.get_all('Content-Length',[])
                if (response.status!=200 or response.headers.get_all('Transfer-Encoding',[])
                        or len(response.headers)>32 or len(lengths)!=1 or not lengths[0].isdecimal()
                        or not 0<int(lengths[0])<=8192):
                    raise ValueError('bounded explicit successful original pause reply required')
                data=response.read(8193)
                if len(data)!=int(lengths[0]):raise ValueError('complete original pause reply required')
                value=json.loads(data,object_pairs_hook=unique_keys)
            finally:response.close()
        _fields(value,'app_name id state vmm_version')
        after=os.stat('api.socket',dir_fd=fd,follow_symlinks=False);current=root.lstat()
        if (value!=dict(app_name='Firecracker',id=resource,state=expected_state,vmm_version='1.16.1')
                or (before.st_dev,before.st_ino)!=(after.st_dev,after.st_ino)
                or (held.st_dev,held.st_ino)!=(current.st_dev,current.st_ino)
                or not equal(original_json(claim,16384),source) or not original_process(source)):
            raise ValueError('original paused VMM changed during observation')
        remaining(deadline)
        return dict(source=copy.deepcopy(source),state=expected_state,peer_pid=actual[0],uid=uid,
                    socket_identity=[after.st_dev,after.st_ino],read_only=True,execution_authority=False)
    finally:os.close(fd)


def original_backing(runtime, acceptance, rows):
    """The original complete Resume binding and its positive material-use duty."""
    a=acceptance
    matches=[r for r in rows if r['restore'] is not None and r['restore']['material_use'] is True
        and equal(dict(session=r['restore']['intent']['request']['binding']['session'],
                       key=r['restore']['intent']['request']['binding']['key'],
                       accepted_version=r['restore']['intent']['accepted_version']),a)]
    if len(matches)!=1 or material_source_charge(matches[0])<=0:
        raise ValueError('one original restored backing pin and positive charge required')
    capture=matches[0]['capture'];intent=matches[0]['restore']['intent']
    accepted_request(intent,runtime.services.host_configs[0]['inbox']['node'])
    if (intent['request']['operation']['kind']!='resume'
            or not equal(capture['completed'],intent['request']['operation']['manifest_digest'])):
        raise ValueError('original pin and complete Resume source differ')
    return matches[0]


def stop_is_registered(conn, stop, capacity):
    """Bind the action's canonical SQL identifier to its full logical command."""
    logical = stop['identity']['logical_action']
    match = re.fullmatch(r'effect-([1-9][0-9]{0,18})', logical) if type(logical) is str else None
    if match is None or int(match[1]) >= 2**63:
        raise ValueError('original canonical SQL logical action identifier required')
    row = conn.execute(
        'SELECT CAST(logical_json AS BLOB) FROM controller_effect_keys '
        'WHERE id=? AND length(CAST(logical_json AS BLOB)) BETWEEN 1 AND ?',
        (int(match[1]), capacity['max_sql_value_bytes']),
    ).fetchone()
    if row is None:
        raise ValueError('bounded original logical effect row required')
    return equal(json.loads(row[0], object_pairs_hook=unique_keys), stop['command']['logical'])


def pending_stop_facts(runtime,conn,local,capacity):
    """Read pending Stop, its registration, and lifetime in one caller SQL snapshot."""
    count,largest=conn.execute("SELECT count(*),max(length(action_json)) FROM outbox WHERE dispatch_state='dispatched'").fetchone()
    if count>capacity['max_sql_rows'] or (largest is not None and largest>capacity['max_sql_value_bytes']):
        raise ValueError('bounded global pending actions required')
    stops=[]
    for raw, in conn.execute("SELECT action_json FROM outbox WHERE dispatch_state='dispatched' ORDER BY id"):
        action=json.loads(raw,object_pairs_hook=unique_keys)
        if 'StopVm' in action['command']['logical']['command']:stops.append(action)
    native=runtime.services.native_pools[0]
    name='sb-'+native['namespace']+'-'+str(local['sandbox'])
    row=conn.execute('SELECT owner_id,started_at_ms,deadline_ms,max_deadline_ms,startup_deadline_ms FROM sandbox_lifetimes WHERE sandbox_id=?',(name,)).fetchone()
    if row is None or row[0]!='tenant-'+str(local['owner']):raise ValueError('original lifetime owner required')
    lifetime=dict(zip(('started_at_ms','end_at_ms','max_end_at_ms','startup_deadline_ms'),row[1:],strict=True))
    for value in lifetime.values():_u64(value)
    registered = len(stops) == 1 and stop_is_registered(conn, stops[0], capacity)
    return dict(global_pending_stops=stops,lifetime=lifetime,logical_action_registered=registered)



class FreezeDestroyProbe(CaptureHandoffProbe):
    hold_source_process = True
    continue_source_before_stop = True

    def __init__(self,runtime,client,physical_running):
        super().__init__(runtime,client,physical_running,'freeze-receipt')
        self.running_acceptance=copy.deepcopy(client.current_acceptance)
        self.stop_peer=None;self.stop_released=False;self.terminal_cut=None;self.continue_sent=False
        self.stop_path=private_path(runtime.quota['deployment_control'],directory=True)/'freeze-stop.sock'
        if self.stop_path.exists() or self.stop_path.is_symlink() or len(os.fsencode(self.stop_path))>=108:
            raise ValueError('fresh bounded original Stop gate path required')

    def _paused_cut(self):
        return True

    def _observe_cut_source(self,deadline):
        return paused_source(self.runtime,self.prior,deadline)

    def _target_sources(self,rows):
        return [row for row in rows if row['work']['anchor']['local']['sandbox']==self.prior['local']['sandbox']]

    def _responsibilities(self,rows):
        # Source observation samples may refresh; immutable work and all
        # material/pin/retirement duties remain exact. _no_full first checks
        # the one legal new end: the journal's original NeverReleased fact.
        result=copy.deepcopy(self._target_sources(rows))
        for row in result:
            row.pop('observation')
            if equal(row['work']['anchor']['request'],self.unload_entry['intent']) and row['capture'] is not None:
                row['capture']['end']=None
        return result

    def _no_full(self,rows,*,terminal=False):
        matches=[row for row in rows if equal(row['work']['anchor']['request'],self.unload_entry['intent'])]
        if len(matches)!=1 or not equal(matches[0]['work'],self.record['work']):
            raise ValueError('one immutable original frozen Unload work required')
        capture=matches[0]['capture']
        if capture is not None:
            if any(capture[name] is not None for name in ('action','worker','completed')) or capture.get('material_bytes') is not None:
                raise ValueError('pre-Full terminal branch acquired a producer or material result')
            end=capture['end']
            if end is not None:
                digest=list(hashlib.sha256(json.dumps(capture['full'],ensure_ascii=False,separators=(',',':')).encode()).digest())
                if not terminal or not equal(end,dict(state='never_released',full_work_binding=digest,seal_action=None)):
                    raise ValueError('only the original journal NeverReleased end may close this unissued Full')
        return matches[0]

    def _backing(self,rows):
        return original_backing(self.runtime,self.running_acceptance,rows)

    def after_acceptance(self,accepted,deadline):
        result=super().after_acceptance(accepted,deadline)
        entry,rows,_=self._read(min(deadline,self.deadline))
        accepted_request(entry['intent'],self.runtime.services.host_configs[0]['inbox']['node'])
        self.unload_entry=copy.deepcopy(entry);self.initial_sources=copy.deepcopy(self._target_sources(rows))
        resource=dict(kind='vm',id=self.prior['source']['resource_id'])
        if not equal(self.action['identity']['resource'],resource):raise ValueError('original Freeze resource differs')
        self._no_full(rows);backing=self._backing(rows)
        self.backing_material=self.runtime._materials(backing['capture']['full'],backing['capture'])
        if not equal(self.backing_material,self.prior['running_backing']):
            raise ValueError('original backing material changed before frozen Destroy')
        if pidfd_exited(self.pidfd):raise ValueError('original frozen source already exited')
        self.frozen=self._observe_cut_source(min(deadline,self.deadline))
        self._hold_source(min(deadline,self.deadline))
        self.initial=dict(entry=copy.deepcopy(entry),freeze_action=copy.deepcopy(self.action),
            sources=copy.deepcopy(self.initial_sources),paused=copy.deepcopy(self.frozen),
            backing_material=copy.deepcopy(self.backing_material))
        result=dict(result,original_frozen_unload=self.initial,freeze_sql_receipt_pending=True,
                    current_pause_observed=True,original_pidfd_stopped=self.pause_requested,original_backing_pin_and_charge=True)
        if not self._paused_cut():
            self.initial['source_observation']=self.initial.pop('paused')
            result['original_dispatched_unload']=result.pop('original_frozen_unload')
            result['current_pause_observed']=False
        return result

    def observe_destroy_window(self, deadline):
        """Re-read the original cut on either side of the public Destroy."""
        deadline = min(deadline, self.deadline)
        self._configuration()
        entry, rows, actions = self._read(deadline)
        action = self._original_action(actions, entry)
        if not equal(entry, self.unload_entry) or not equal(action, self.action):
            raise ValueError('original Unload or complete Freeze changed at public Destroy')
        self._no_full(rows)
        self._check_source_hold(deadline)
        host = self.runtime.services.processes.children.get('host-0')
        if host is None or host is not self.host or host.poll() != self._cut_exit_code():
            raise ValueError('original Freeze cut Host must remain exited at public Destroy')
        remaining(deadline)
        return dict(host_pid=host.pid, host_exit_code=self._cut_exit_code(),
                    unload_accepted_version=entry['intent']['accepted_version'],
                    freeze_identity=action['identity'], freeze_sql_receipt_pending=True)

    def _source_state(self,deadline):
        remaining(deadline)
        if self.pidfd is None or pidfd_exited(self.pidfd) or not original_process(self.prior['source']):
            raise ValueError('exact original live source and pidfd required')
        fields=(Path('/proc')/str(self.prior['source']['pid'])/'stat').read_text().rsplit(')',1)[1].split()
        if int(fields[19])!=self.prior['source']['start_time_ticks'] or pidfd_exited(self.pidfd):
            raise ValueError('original source changed during state observation')
        return fields[0]

    def _hold_source(self,deadline):
        # Physical FC Paused was independently read above. OS T only prevents
        # old Freeze re-observation winning before Host receives Destroy.
        if self.pause_requested or self.recovered:raise ValueError('one original source hold required')
        if not self.hold_source_process:
            if self._source_state(deadline)=='T':raise ValueError('responsive VMM must not be OS-stopped')
            return
        self._source_state(deadline);self.pause_requested=True
        signal.pidfd_send_signal(self.pidfd,signal.SIGSTOP)
        while self._source_state(deadline)!='T':time.sleep(min(.01,remaining(deadline)))
        self.candidate_paused=True

    def _check_source_hold(self,deadline):
        state=self._source_state(deadline)
        if (state=='T') != self.hold_source_process:
            raise ValueError('original VMM response mode changed')

    def _bind_destroy(self,accepted):
        _fields(accepted,'session key accepted_version')
        if (self.accepted is None or self.terminal_accepted is not None or self.recovered
                or not equal(accepted,self.client.current_acceptance)
                or accepted['session']!=self.accepted['session']
                or _u64(accepted['accepted_version'],positive=True)!=self.accepted['accepted_version']+1
                or equal(accepted['key'],self.accepted['key'])
                or any(accepted['key'][k]!=self.accepted['key'][k] for k in ('deployment','owner'))
                or not equal(self.client.destroy_request,dict(window=accepted['key']['window'],
                    sequence=accepted['key']['sequence'],expected_version=self.accepted['accepted_version']))):
            raise ValueError('exact next public Destroy after original frozen Unload required')
        self.terminal_accepted=copy.deepcopy(accepted)

    def _terminal_entries(self,inbox):
        if self.terminal_accepted is None:raise ValueError('original public Destroy required')
        rows=[r for r in inbox['sessions'] if r['id']==self.accepted['session']]
        if len(rows)!=1:raise ValueError('one original frozen session required')
        row=rows[0];accepted=self.terminal_accepted
        if equal(row['current_version'],self.accepted['accepted_version']) and row['terminal'] is False:
            raise _AwaitTerminal('Destroy has not reached original Host')
        if not equal(row['current_version'],accepted['accepted_version']) or row['terminal'] is not True:
            raise ValueError('another current terminal version')
        old=[e for e in row['entries'] if equal(e['intent'],self.unload_entry['intent'])]
        new=[e for e in row['entries'] if e['intent']['accepted_version']==accepted['accepted_version']
             and equal(e['intent']['request']['binding']['key'],accepted['key'])]
        if (len(old)!=1 or not equal(old[0],self.unload_entry) or len(new)!=1
                or any(not equal(row['local'][k],self.prior['local'][k]) for k in ('sandbox','owner'))):
            raise ValueError('original Unload/history or native binding changed')
        intent=new[0]['intent'];prior=self.unload_entry['intent'];binding=intent['request']['binding']
        accepted_request(intent,self.runtime.services.host_configs[0]['inbox']['node'])
        expected=copy.deepcopy(prior);expected['accepted_version']=accepted['accepted_version']
        expected['request']['binding']['key']=copy.deepcopy(accepted['key'])
        expected['request']['binding']['expected']={'state':'version','version':self.accepted['accepted_version']}
        expected['request']['operation']={'kind':'destroy'}
        if not equal(intent,expected) or binding['session']!=accepted['session']:
            raise ValueError('complete original Destroy intent/context/retention differs')
        if new[0]['local_operation'] is None:raise _AwaitTerminal('native Kill not yet admitted')
        return old[0],new[0],row['local']

    def _terminal_extra(self,conn,local,capacity):
        return pending_stop_facts(self.runtime,conn,local,capacity)

    def _read_terminal(self,deadline):
        snapshot=super()._read_terminal(deadline)
        snapshot['original_actions']=snapshot.pop('resume_actions')
        return snapshot

    def _inventory(self,snapshot):
        originals=snapshot['original_actions'];all_rows=originals+snapshot['terminal_actions']
        freeze=[r for r in originals if 'FreezeVm' in r[0]['command']['logical']['command']]
        if len(freeze)!=1 or not equal(freeze[0][0],self.action):raise ValueError('original Freeze missing or replaced')
        if any(set(r[0]['command']['logical']['command']) & {'SealSnapshot','CommitSnapshot','PrepareVm','ActivateVm'} for r in all_rows):
            raise ValueError('terminal frozen Unload scheduled new capture or execution')
        self._no_full(snapshot['sources'],terminal=True)
        observed=self._success_observation(freeze[0],'Frozen')
        if observed is not None and not equal(observed,dict(pid=self.prior['source']['pid'],
                start_ticks=self.prior['source']['start_time_ticks'],boot_id=self.prior['source']['boot_id'])):
            raise ValueError('late original Freeze observation names another process')
        return all_rows

    def _stop_barrier(self,snapshot,*,held_stop=None):
        self._inventory(snapshot);native=snapshot['native'];operation=native['current_operation']
        binding=self.prior['context']['binding']
        _u64(native['epoch'])
        if (native['terminal'] is not True or native['phase']!='Revoking'
                or not equal(native['generation'],binding['generation']) or not equal(native['current_vm'],binding['vm'])
                or not equal(native['net_slot'],binding['network']) or operation is None or operation['kind']!='Kill'
                or not equal(operation['id'],snapshot['terminal']['local_operation']) or not equal(operation['epoch'],native['epoch'])
                or not equal(snapshot['lifetime'],self.prior['lifetime'])):
            raise ValueError('original current Kill/source/lifetime not retained before Stop')
        if not equal(self._responsibilities(snapshot['sources']),self._responsibilities(self.initial_sources)):
            raise ValueError('original work, backing pin or material responsibility changed before Stop')
        self._backing(snapshot['sources'])
        fences=[r for r in snapshot['terminal_actions'] if 'InstallFence' in r[0]['command']['logical']['command']]
        stops=[r for r in snapshot['terminal_actions'] if 'StopVm' in r[0]['command']['logical']['command']]
        if len(fences)!=1 or len(stops)!=1 or stops[0][1:]!=('dispatched',None):
            raise ValueError('one original terminal fence and unacknowledged Stop required')
        if snapshot['logical_action_registered'] is not True:
            raise ValueError('original terminal Stop lacks its complete logical effect registration')
        fence=fences[0][0];stop=stops[0][0];epoch=native['epoch'];sandbox=snapshot['local']['sandbox']
        expected=dict(sandbox=sandbox,generation=binding['generation'],vm=binding['vm'],epoch=epoch)
        if (not equal(stop['command']['logical']['command'],{'StopVm':expected})
                or not equal(stop['identity']['resource'],self.action['identity']['resource'])
                or not equal(stop['identity']['epoch'],epoch) or not equal(snapshot['global_pending_stops'],[stop])
                or not equal(fence['command']['logical']['command'],{'InstallFence':dict(sandbox=sandbox,epoch=epoch,terminal=True)})
                or not equal(fence['identity']['epoch'],epoch) or epoch<=self.action['identity']['epoch']):
            raise ValueError('generic Stop gate does not name the one exact original terminal Stop')
        if held_stop is not None and not equal(stop,held_stop):
            raise ValueError('complete terminal Stop changed while its gate was held')
        observed=self._success_observation(fences[0],'Fence')
        if observed is None:raise _AwaitTerminal('terminal fence not yet successfully observed')
        path=Path(self.runtime.services.native_pools[0]['node_state_dir'])/'fences'/self.config['sandbox']
        actual=original_json(path.with_suffix('.json'),16384)
        if not equal(observed,dict(epoch=epoch,terminal=True)) or not equal(actual,observed):
            raise ValueError('actual terminal fence differs from exact SQL receipt')
        return actual

    def _terminal_manager_started(self,deadline):
        pass

    def recover_after_destroy(self,accepted,deadline,*,observe_peer):
        self._bind_destroy(accepted);deadline=min(deadline,self.deadline)
        if self.host.poll()!=self._cut_exit_code():raise ValueError('original cut manager required')
        self._check_source_hold(deadline)
        self._configuration()
        self.terminal_host=self.runtime.services.processes.start('host-0',env={
            'SANDPROOF_TEST_STOP_GATE':str(self.stop_path),'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION':'cleanup_vm'})
        if self.terminal_host is self.host or self.terminal_host.pid==self.host.pid:raise ValueError('new original manager required')
        self._terminal_manager_started(deadline)
        self.runtime.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)
        while not self.stop_path.exists():
            remaining(deadline)
            if self.terminal_host.poll() is not None:raise ValueError('Host exited before original Stop gate')
            time.sleep(min(.05,remaining(deadline)))
        before=self.stop_path.lstat();role=self.runtime.services.processes.bindings['host-0']
        if not stat.S_ISSOCK(before.st_mode) or before.st_uid!=role['uid']:raise ValueError('original private Stop socket required')
        peer=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);self.stop_peer=peer
        peer.settimeout(min(2,remaining(deadline)));peer.connect(str(self.stop_path))
        if struct.unpack('3i',peer.getsockopt(socket.SOL_SOCKET,socket.SO_PEERCRED,12))!=(self.terminal_host.pid,role['uid'],role['gid']):
            raise ValueError('Stop gate belongs to another manager')
        after=self.stop_path.lstat()
        if (before.st_dev,before.st_ino)!=(after.st_dev,after.st_ino):raise ValueError('Stop gate replaced during connection')
        snapshot=self._read_terminal(deadline);fence=self._stop_barrier(snapshot)
        held_stop=copy.deepcopy(snapshot['global_pending_stops'][0])
        if pidfd_exited(self.pidfd):raise ValueError('original source exited before Stop release')
        self._check_source_hold(deadline)
        backing=self._backing(snapshot['sources'])
        material=self.runtime._materials(backing['capture']['full'],backing['capture'])
        if not equal(material,self.backing_material):raise ValueError('original backing files changed before Stop')
        if not callable(observe_peer):raise ValueError('original independent peer progress required')
        progress=observe_peer()
        self._configuration();again=self._read_terminal(deadline);self._stop_barrier(again,held_stop=held_stop)
        if not equal(self._responsibilities(again['sources']),self._responsibilities(self.initial_sources)) or pidfd_exited(self.pidfd):
            raise ValueError('original source or responsibilities changed during peer progress')
        self._check_source_hold(deadline)
        if not equal(self.runtime._materials(backing['capture']['full'],backing['capture']),self.backing_material):
            raise ValueError('peer progress changed original frozen backing files')
        paused=self._prepare_source_for_stop(deadline)
        final=self._read_terminal(deadline);self._stop_barrier(final,held_stop=held_stop)
        self.before_stop=copy.deepcopy(final);remaining(deadline)
        peer.settimeout(min(2,remaining(deadline)));peer.sendall(b'g');self.stop_released=True;peer.close();self.stop_peer=None;self.recovered=True
        result=dict(public_destroy_reached_host=True,terminal_fence=fence,paused=paused,
            original_backing_material=material,original_pin_and_positive_charge_retained=True,
            peer_progress=progress,original_pidfd_continued_after_fence=self.continue_sent,source_process_hold=self.hold_source_process,stop_gate_release_sent=True,snapshot=self.before_stop,
            public_completion=False,capacity_released=False)
        if not self._paused_cut():
            result['running_source']=result.pop('paused')
            result['source_observation']=result['running_source']
        return result

    def _prepare_source_for_stop(self,deadline):
        if self.hold_source_process:
            signal.pidfd_send_signal(self.pidfd,signal.SIGCONT);self.continue_sent=True
            while self._source_state(deadline)=='T':time.sleep(min(.01,remaining(deadline)))
        return paused_source(self.runtime,self.prior,deadline)

    def assert_terminal_before_gc(self,deadline):
        if not self.recovered or not self.stop_released or (self.hold_source_process and self.continue_source_before_stop and not self.continue_sent) or self.terminal_cut is not None:
            raise ValueError('one original Stop release required before cleanup cut')
        deadline=min(deadline,self.deadline)
        if self.terminal_host.wait(timeout=remaining(deadline))!=86:raise ValueError('original Cleanup-success/pre-SQL cut required')
        self._configuration();snapshot=self._read_terminal(deadline);self._inventory(snapshot)
        epoch=self.before_stop['native']['epoch'];binding=self.prior['context']['binding']
        expected=dict(sandbox=self.prior['local']['sandbox'],generation=binding['generation'],vm=binding['vm'])
        stops=[r for r in snapshot['terminal_actions'] if 'StopVm' in r[0]['command']['logical']['command']]
        cleanups=[r for r in snapshot['terminal_actions'] if 'CleanupVm' in r[0]['command']['logical']['command']]
        if len(stops)!=1 or len(cleanups)!=1 or cleanups[0][1:]!=('dispatched',None):raise ValueError('original Stop and pending Cleanup receipt required')
        if not equal(stops[0][0],self.before_stop['global_pending_stops'][0]):
            raise ValueError('Cleanup does not retain the complete Stop released at the gate')
        for row,command in ((stops[0],{'StopVm':dict(expected,epoch=epoch)}),(cleanups[0],{'CleanupVm':expected})):
            if (not equal(row[0]['command']['logical']['command'],command) or not equal(row[0]['identity']['epoch'],epoch)
                    or not equal(row[0]['identity']['resource'],self.action['identity']['resource'])):
                raise ValueError('terminal stop/cleanup changed original VM or epoch')
        stopped=self._success_observation(stops[0],'Stopped')
        if (not equal(stopped,dict(pid=self.prior['source']['pid'],start_ticks=self.prior['source']['start_time_ticks']))
                or not pidfd_exited(self.pidfd) or original_process(self.prior['source']) or self.stop_path.exists()):
            raise ValueError('original Stop and kernel-confirmed source exit required')
        self.terminal_cut=copy.deepcopy(snapshot)
        self._configuration();self.runtime.services.processes.start('host-0')
        self.runtime.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)
        return dict(original_freeze_identity_retained=True,original_stop_observed=True,
            original_source_exited=True,cleanup_success_pre_sql_exit86=True,cleanup_sql_ack_pending=True,
            snapshot=self.terminal_cut,public_completion=False,capacity_released=False)

    def terminal_failure_state(self):
        return dict(original_source=copy.deepcopy(self.prior['source']),
            unload=copy.deepcopy(self.accepted),destroy=copy.deepcopy(self.terminal_accepted),
            stop_gate_release_sent=self.stop_released,source_os_stop_requested=self.pause_requested,
            source_os_continue_sent=self.continue_sent,public_completion=False,capacity_released=False,
            action='preserve original responsibility and HostLease; no new business request')

    def cleanup(self):
        if not self.runtime.gc_complete or self.stop_path.exists() or not self.stop_released or self.stop_peer is not None:
            raise ValueError('original product GC and consumed Stop gate required')
        return dict(original_stop_gate_removed=True,observation_handles_only=True)

    def close(self):
        if self.stop_peer is not None:self.stop_peer.close();self.stop_peer=None
        super().close()


class ResponsiveFreezeDestroyProbe(FreezeDestroyProbe):
    """The VM is Paused, but its VMM still answers the original API.

    The original failed SIGSTOP experiment remains separately selectable. This
    consumer never sends SIGSTOP/SIGCONT and retains every Stop/identity/pin,
    peer-progress, late Freeze, kernel exit and Cleanup check of the parent.
    """
    hold_source_process = False


class OrderedFreezeDestroyProbe(FreezeDestroyProbe):
    """Restore VMM response after durable Kill, before waiting for its effects.

    The short test SIGSTOP orders acceptance only. It must not wait on a fence
    or Stop whose scheduling itself needs the original Freeze observation.
    """
    def _terminal_manager_started(self,deadline):
        while True:
            try:
                snapshot=self._read_terminal(deadline)
                break
            except _AwaitTerminal:
                time.sleep(min(.01,remaining(deadline)))
        native=snapshot['native'];binding=self.prior['context']['binding'];operation=native['current_operation']
        _u64(native['epoch'])
        if (native['terminal'] is not True or native['phase']!='Revoking' or operation is None
                or operation['kind']!='Kill' or not equal(operation['id'],snapshot['terminal']['local_operation'])
                or not equal(operation['epoch'],native['epoch']) or not equal(native['generation'],binding['generation'])
                or not equal(native['current_vm'],binding['vm']) or not equal(native['net_slot'],binding['network'])):
            raise ValueError('durable exact original Kill required before restoring VMM response')
        self._no_full(snapshot['sources'],terminal=True)
        backing=self._backing(snapshot['sources'])
        if (not equal(self._responsibilities(snapshot['sources']),self._responsibilities(self.initial_sources))
                or not equal(self.runtime._materials(backing['capture']['full'],backing['capture']),self.backing_material)):
            raise ValueError('original backing responsibility changed before VMM response restored')
        self._check_source_hold(deadline)
        self.response_restore_snapshot=copy.deepcopy(snapshot)
        signal.pidfd_send_signal(self.pidfd,signal.SIGCONT);self.continue_sent=True
        while self._source_state(deadline)=='T':time.sleep(min(.01,remaining(deadline)))
        self.hold_source_process=False

    def recover_after_destroy(self,accepted,deadline,*,observe_peer):
        result=super().recover_after_destroy(accepted,deadline,observe_peer=observe_peer)
        result['original_pidfd_continued_after_fence']=False
        result['source_continued_after_terminal_acceptance']=True
        result['terminal_acceptance_before_response_restored']=self.response_restore_snapshot
        return result


class UnresponsiveFreezeDestroyProbe(FreezeDestroyProbe):
    """Leave the original VMM OS-stopped; only the product's Stop may end it."""
    continue_source_before_stop = False

    def _prepare_source_for_stop(self,deadline):
        self._check_source_hold(deadline)
        if self.continue_sent:raise ValueError('unresponsive case must never send SIGCONT')
        return dict(current_pause_observed=False,source_os_stopped=True,
                    source=copy.deepcopy(self.prior['source']))
