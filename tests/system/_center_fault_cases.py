"""Mandatory DK01–DK05 assertions on one original, actually running cohort."""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

from concurrent.futures import ThreadPoolExecutor
import http.client
import json
import os
import secrets
import signal

from _center_fault_harness import center_record, physical_record, same_running_process, vm_gone
from server_unready_abort import wait_for
from tools.runtime_create_recovery import find_runtime_create


def sdk_from_detail(harness, sandbox_id):
    """The pinned SDK's original no-create constructor; does not renew lifetime."""
    from e2b import Sandbox
    status, raw = harness.fetch('GET','/sandboxes/'+sandbox_id)
    assert status==200
    value=json.loads(raw)
    return Sandbox(sandbox_id=sandbox_id,envd_version=value['envdVersion'],
        envd_access_token=value['envdAccessToken'],**harness.sdk_options())


def dk01(h):
    from e2b import Sandbox
    sandbox=h.create()
    ident=sandbox.sandbox_id
    assert sandbox.get_info().state=='running'
    assert [row.sandbox_id for row in Sandbox.list(**h.sdk_options()).next_items()]==[ident]
    status,body=h.fetch('GET','/sandboxes/'+ident)
    details=json.loads(body)
    assert status==200 and details['cpuCount']==1 and details['memoryMB']==256
    assert details['diskSizeMB']==h.args.rootfs.stat().st_size//1024**2
    assert h.fetch('GET','/sandboxes/'+ident,foreign=True)[0]==404
    assert h.fetch('DELETE','/sandboxes/'+ident,foreign=True)[0]==404
    assert sandbox.commands.run('printf dk-unified-entry').stdout=='dk-unified-entry'
    sandbox.files.write('/tmp/sandproof-dk-file','original-file')
    assert sandbox.files.read('/tmp/sandproof-dk-file')=='original-file'
    original=center_record(h.center(),h.known[ident]['create'])
    Sandbox.set_timeout(ident,240,**h.sdk_options())
    renewed=center_record(h.center(),h.known[ident]['create'])
    assert renewed['started_at_ms']==original['started_at_ms']
    assert renewed['max_end_at_ms']==original['max_end_at_ms']
    assert renewed['end_at_ms']>original['end_at_ms']
    assert same_running_process(h.known[ident]['physical'])
    terminal=h.kill(ident)
    assert h.fetch('GET','/sandboxes/'+ident)[0]==404
    h.complete_case('DK01',dict(create=h.known[ident]['create'],renewed_end_at_ms=renewed['end_at_ms'],
        original_started_at_ms=original['started_at_ms'],terminal=terminal,
        sdk='2.51.0',management='create/get/list/renew/kill',data='command/file'))


def dk02(h):
    barrier=h.relay.barrier
    barrier.arm()
    metadata=dict(dk_ready_loss=secrets.token_hex(16))
    with ThreadPoolExecutor(max_workers=1) as worker:
        response=worker.submit(h.fetch,'POST','/v2/sandboxes',dict(templateID='base',timeout=180,
            allow_internet_access=False,metadata=metadata))
        try:
            assert barrier.ready.wait(timeout=h.args.wait_seconds), 'no real durable Ready reached test barrier'
            captured=barrier.captured.copy()
            assert barrier.forwarded==0 and not barrier.release.is_set()
            host_before_audit=h.services.processes.observe()['host-0']
            # The Ready barrier must not let the ordinary readiness helper
            # restart the center before the explicit receipt-loss fault.
            host=h.host(resume_ready=False)
            current=[row for row in host['operations'] if row['current'] and row['identity']==captured['identity']]
            assert len(current)==1 and current[0]['response']['fact']==captured['fact']
            before=physical_record(h.root,host,captured['identity'])
            processes=h.services.processes
            old_center=processes.observe()['center']
            old_host=processes.observe()['host-0']
            assert not same_running_process(host_before_audit)
            processes.stop('center',force=True)
            # The barrier was reached BEFORE a single centerward response byte;
            # center's original Redis record still cannot contain this Ready.
            audit=processes.audit('center')
            identity=find_runtime_create(audit,metadata,deployment=h.services.center_config['deployment_id'])
            assert identity==captured['identity']
            record=center_record(audit,identity)
            assert record['started_at_ms'] is None
            assert record['operations'][-1]['response'] is None or record['operations'][-1]['response']['fact']['kind']=='recorded'
            assert same_running_process(before) and same_running_process(old_host)
            barrier.unblock(discard=True)
            try:
                result=response.result(timeout=45)
                assert result[0]>=500
            except (OSError,http.client.HTTPException):
                pass  # the exact killed public center may close its connection
            processes.start('center')
            h.services.wait_ready(for_admission=False)
            def recovered():
                value=center_record(h.center(),identity)
                return value if value['started_at_ms'] is not None else None
            record=wait_for(recovered,h.args.wait_seconds,'original Ready replay into the center')
            assert (record['started_at_ms'],record['end_at_ms'],record['max_end_at_ms'])==(
                captured['fact']['started_at_ms'],captured['fact']['end_at_ms'],captured['fact']['max_end_at_ms'])
            assert same_running_process(old_host), 'healthy host restarted during center receipt recovery'
            after=physical_record(h.root,h.host(),identity)
            assert before==after
            h.known[identity['sandbox_id']]=dict(create=identity,physical=after)
            sandbox=sdk_from_detail(h,identity['sandbox_id'])
            assert sandbox.commands.run('printf after-ready-loss').stdout=='after-ready-loss'
            terminal=h.kill(sandbox.sandbox_id)
            h.complete_case('DK02',dict(original_ready=captured,original_vm=before,
                old_center=old_center,host_independent=old_host,centerward_ready_bytes=0,
                planned_offline_ready_audit=dict(before=host_before_audit,after=old_host),
                terminal=terminal))
        finally:
            barrier.unblock(discard=True)


def dk03(h):
    processes=h.services.processes
    processes.stop('host-0')
    processes.start('host-0',env={'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION':'activate_vm'})
    h.services.wait_ready()
    starting=processes.children['host-0']
    metadata=dict(dk_host_start=secrets.token_hex(16))
    status,_=h.fetch('POST','/v2/sandboxes',dict(templateID='base',timeout=180,
        allow_internet_access=False,metadata=metadata))
    assert status>=500, 'uncommitted actual host action was reported successful'
    assert starting.wait(timeout=30)==86, 'approved material did not execute the original deterministic host fault'
    center=h.center()
    identity=find_runtime_create(center,metadata,deployment=h.services.center_config['deployment_id'])
    before=physical_record(h.root,h.host(),identity)
    assert center_record(center,identity)['started_at_ms'] is None
    processes.start('host-0')
    h.services.wait_ready(for_admission=False)
    wait_for(lambda:h.fetch('GET','/sandboxes/'+identity['sandbox_id'])[0]==200,
             h.args.wait_seconds,'same accepted startup reconciliation')
    after=physical_record(h.root,h.host(),identity)
    assert before==after
    h.known[identity['sandbox_id']]=dict(create=identity,physical=after)
    sandbox=sdk_from_detail(h,identity['sandbox_id'])
    assert sandbox.commands.run('printf host-recovered').stdout=='host-recovered'
    processes.stop('host-0')
    processes.start('host-0',env={'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION':'stop_vm'})
    h.services.wait_ready(for_admission=False)
    stopping=processes.children['host-0']
    status,_=h.fetch('DELETE','/sandboxes/'+sandbox.sandbox_id)
    assert status>=500
    assert stopping.wait(timeout=30)==86
    assert not same_running_process(before), 'exact physical stop did not precede the injected exit'
    pending=center_record(h.center(),identity)
    assert pending['terminating'] and not pending['released']
    processes.start('host-0')
    h.services.wait_ready(for_admission=False)
    terminal=h.terminal(sandbox.sandbox_id)
    h.complete_case('DK03',dict(original_create=identity,original_vm=before,
        startup_exit=starting.returncode,stop_exit=stopping.returncode,terminal=terminal))


def dk04(h):
    from e2b import Sandbox
    destroy=h.create(timeout=120)
    expiring=h.create(timeout=12,foreign=True)
    ids=[destroy.sandbox_id,expiring.sandbox_id]
    old_host=h.services.processes.observe()['host-0']
    h.relay.partition.set()
    try:
        status,_=h.fetch('DELETE','/sandboxes/'+destroy.sandbox_id)
        assert status>=500
        assert h.relay.destroy_seen.wait(timeout=10), 'no exact committed destroy entered the partition'
        pending=h.center()
        rows=[center_record(pending,h.known[ident]['create']) for ident in ids]
        assert all(not row['released'] for row in rows)
        assert rows[0]['terminating']
        assert all(row['create']['identity']['node']==old['create']['node'] for row,old in zip(rows,[h.known[i] for i in ids]))
        # Both quotas remain held while the authenticated host cannot report
        # cleanup, even after its independent local expiry has stopped a VM.
        assert h.fetch('POST','/v2/sandboxes',dict(templateID='base',timeout=60,allow_internet_access=False))[0] in (429,503)
        wait_for(lambda:vm_gone(h.root,h.known[expiring.sandbox_id]['physical'],
            h.args.firecracker,h.args.cgroup_parent),h.args.wait_seconds,'local expiry during center/host partition')
        assert same_running_process(old_host)
        pending=h.center()
        assert all(not center_record(pending,h.known[ident]['create'])['released'] for ident in ids)
        assert h.fetch('GET','/sandboxes/'+expiring.sandbox_id,foreign=True)[0]==404
        partitioned=list(h.relay.partition_identities)
        assert all(value['node']==h.known[destroy.sandbox_id]['create']['node'] for value in partitioned)
    finally:
        h.relay.partition.clear()
    h.services.wait_ready(for_admission=False)
    terminals=[h.terminal(ident) for ident in ids]
    assert Sandbox.list(**h.sdk_options()).next_items()==[]
    assert Sandbox.list(**h.sdk_options(foreign=True)).next_items()==[]
    h.complete_case('DK04',dict(creates=[h.known[i]['create'] for i in ids],
        host_independent=old_host,partition_requests=partitioned,terminals=terminals))


def dk05(h):
    from e2b import Sandbox
    sandbox=h.create(timeout=240)
    identity=h.known[sandbox.sandbox_id]['create']
    original_vm=h.known[sandbox.sandbox_id]['physical']
    original=center_record(h.center(),identity)
    processes=h.services.processes
    actual=processes.observe()
    redis=processes.children['redis']
    os.kill(redis.pid,signal.SIGSTOP)
    try:
        wait_for(lambda:PathState(redis.pid)=='T',10,'exact original Redis SIGSTOP barrier')
        assert same_running_process(actual['host-0']) and same_running_process(original_vm)
        assert h.fetch('POST','/sandboxes/'+sandbox.sandbox_id+'/timeout',dict(timeout=300))[0]>=500
        assert h.fetch('POST','/v2/sandboxes',dict(templateID='base',timeout=60,allow_internet_access=False))[0]>=500
        assert h.fetch('GET','/sandboxes/'+sandbox.sandbox_id,foreign=True)[0] in (404,500,503)
    finally:
        os.kill(redis.pid,signal.SIGCONT)
    # Preserve the same persisted authority, no initialize/fallback on restart.
    processes.stop('center')
    processes.start('center')
    h.services.wait_ready(for_admission=False)
    restored=center_record(h.center(),identity)
    assert restored['create']==original['create'] and restored['started_at_ms']==original['started_at_ms']
    assert restored['end_at_ms']==original['end_at_ms']
    assert len(h.center()['sandboxes'])==1 and same_running_process(original_vm)
    assert sandbox.commands.run('printf redis-resumed').stdout=='redis-resumed'
    before_restart=processes.observe()['redis']
    processes.stop('redis')
    assert same_running_process(actual['host-0']) and same_running_process(original_vm)
    processes.start('redis')
    from _runtime_services import _wait_redis
    _wait_redis(h.services.center_config['redis'],processes.children['redis'])
    processes.stop('center')
    processes.start('center')
    h.services.wait_ready(for_admission=False)
    after_restart=processes.observe()['redis']
    assert after_restart['pid']!=before_restart['pid'] or after_restart['start_time_ticks']!=before_restart['start_time_ticks']
    assert same_running_process(actual['host-0']) and same_running_process(original_vm)
    final=center_record(h.center(),identity)
    assert final['started_at_ms']==original['started_at_ms'] and final['create']==original['create']
    assert final['end_at_ms']==original['end_at_ms']
    sandbox.files.write('/tmp/sandproof-redis-restart','same-vm')
    assert sandbox.files.read('/tmp/sandproof-redis-restart')=='same-vm'
    terminal=h.kill(sandbox.sandbox_id)
    h.complete_case('DK05',dict(original_create=identity,original_vm=original_vm,
        host_independent=actual['host-0'],redis_before=before_restart,redis_after=after_restart,
        terminal=terminal))


def PathState(pid):
    from pathlib import Path
    return Path('/proc/'+str(pid)+'/stat').read_text().rsplit(')',1)[1].split()[0]
