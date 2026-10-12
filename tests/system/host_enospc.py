"""Real ENOSPC scenarios driven by server_disk_reserve.py's private filesystem.

Only the run's own bounded fillers are released. Creation is never replayed
after an unknown reply; an admitted inode-failed create recovers its original
durable global identity. No legacy public-host or old-error reproduction mode.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


from contextlib import closing
import errno
import json
import os
from pathlib import Path
import secrets
import sqlite3
import time

from e2b import Sandbox
from e2b.exceptions import (
    RateLimitException, SandboxException, SandboxNotFoundException, ServiceBusyException,
)

from sdk_server_smoke import options
from server_disk_reserve import MIB, RESERVE, SCRATCH_BYTES, Resources, free_bytes, healthy
from server_disk_reserve import SCRATCH_INODES,prepare_services,wait_disk_veto
from tools.runtime_create_recovery import recover_original_create,find_runtime_create


def rows(config):
    with closing(sqlite3.connect(f"file:{config['database']}?mode=ro", uri=True)) as db:
        return {ident: json.loads(blob) for ident, blob in
                db.execute("SELECT sandbox_id, state_blob FROM sandboxes")}


class Filler:
    def __init__(self, mounted):
        self.mounted = mounted
        self.path = mounted / "own-enospc-filler"
        self.names = []
        self.created = False

    def release(self):
        if not self.created:
            return
        if self.path.is_dir():
            for name in self.names:
                (self.path / name).unlink()
            self.path.rmdir()
        else:
            self.path.unlink()
        self.created = False

    def fill(self, kind):
        before = os.statvfs(self.mounted)
        assert before.f_blocks * before.f_frsize <= SCRATCH_BYTES
        errors = []
        allocated = 0
        if kind == "blocks":
            with self.path.open("xb") as stream:
                self.created = True
                for chunk in (8 * MIB, before.f_frsize):
                    while allocated + chunk <= SCRATCH_BYTES:
                        try:
                            os.posix_fallocate(stream.fileno(), allocated, chunk)
                        except OSError as error:
                            assert error.errno == errno.ENOSPC
                            errors.append(error.errno)
                            break
                        allocated += chunk
                    else:
                        raise AssertionError("allocation cap reached without ENOSPC")
                os.fsync(stream.fileno())
        else:
            assert 0 < before.f_favail <= before.f_files <= SCRATCH_INODES
            self.path.mkdir(mode=0o700)
            self.created = True
            for index in range(SCRATCH_INODES):
                name = str(index)
                try:
                    with (self.path / name).open("xb"):
                        pass
                except OSError as error:
                    assert error.errno == errno.ENOSPC
                    errors.append(error.errno)
                    break
                self.names.append(name)
            else:
                raise AssertionError("inode allocation cap reached without ENOSPC")
        after = os.statvfs(self.mounted)
        assert errors and errors[-1] == errno.ENOSPC
        if kind == "inodes":
            assert after.f_favail == 0
        return dict(errno=errno.ENOSPC, allocated_bytes=allocated,
                    created_empty_files=len(self.names), remaining_inodes=after.f_favail,
                    remaining_bytes=after.f_bavail * after.f_frsize)


def inode_case(args, services, config, mounted, resources, sdk, live, filler, marker, payload):
    before = rows(config)
    claims = {p.name for p in resources.claims.iterdir()}
    filled = filler.fill("inodes")
    assert free_bytes(mounted) > Path(config["rootfs"]).stat().st_size + RESERVE
    try:
        Sandbox.create(template="base", timeout=300, allow_internet_access=False,
                       metadata={"enospc": "original"}, **sdk)
    except (SandboxException, ServiceBusyException) as error:
        status = error.status_code
        assert status in (500, 503), "unrelated API failure"
    else:
        raise AssertionError("inode exhaustion returned successful create")
    after = rows(config)
    new_ids = set(after) - set(before)
    # The new authoritative admission is the center CAS. Depending on the exact
    # inode failure boundary, the original local admission may not yet exist.
    # Neither absence nor HTTP500 implies the accepted global Create vanished.
    assert len(new_ids) <= 1
    if new_ids:
        assert after[next(iter(new_ids))]['phase']=='Starting'
    global_audit=services.planned_audit('center')
    create=find_runtime_create(global_audit,{'enospc':'original'},
        deployment=services.center_config['deployment_id'])
    ident=create['sandbox_id']
    assert {p.name for p in resources.claims.iterdir()} == claims
    # The unknown create already consumes admission; it has not booted a VM.
    for sandbox in live:
        healthy(sandbox, marker, payload)
    filler.release()
    services.wait_ready(for_admission=False)
    deadline = time.monotonic() + 60
    while True:
        try:
            info = Sandbox.get_info(ident, **sdk)
            break
        except SandboxNotFoundException:
            assert time.monotonic() < deadline, "original create did not recover"
            time.sleep(0.1)  # Polls durable recovery, not a guessed race window.
    assert info.metadata == {"enospc": "original"}
    recovered = Sandbox.connect(ident, **sdk)
    live.append(recovered)
    resources.sample()
    recovered_native=set(rows(config))-set(before)
    assert len(recovered_native)==1, 'original recovery admitted more than one native instance'
    if new_ids:
        assert recovered_native==new_ids, 'original admitted native identity changed'
    center_after=services.planned_audit('center')
    assert find_runtime_create(center_after,{'enospc':'original'},
        deployment=services.center_config['deployment_id'])==create, 'global Create was replaced'
    assert not recovered.files.exists(marker)
    healthy(recovered, marker, payload)
    return dict(scenario="inodes", fill=filled, create_http_status=status,
                durable_create_before_error=True, no_vm_before_inode_release=True,
                original_create_identity=create,native_admitted_before_error=bool(new_ids),
                original_create_recovered_without_replay=True)


def block_case(services, config, mounted, resources, sdk, live, filler, marker, payload, vm_ids):
    before = rows(config)
    claims = {p.name for p in resources.claims.iterdir()}
    filled = filler.fill("blocks")
    assert free_bytes(mounted) < Path(config["rootfs"]).stat().st_size + RESERVE
    global_before=wait_disk_veto(services)
    victim_create=find_runtime_create(global_before,
        dict(enospc_probe=config['namespace'],instance='0'),deployment=services.center_config['deployment_id'])
    try:
        Sandbox.create(template="base", timeout=300, allow_internet_access=False, **sdk)
    except RateLimitException as error:
        assert error.status_code == 429
    else:
        raise AssertionError("full block filesystem admitted a create")
    assert set(rows(config)) == set(before)
    assert {p.name for p in resources.claims.iterdir()} == claims
    global_after=services.planned_audit('center')
    assert global_after['sandbox_highwater']==global_before['sandbox_highwater']
    assert global_after['operation_highwater']==global_before['operation_highwater']
    victim, control = live
    # Check guest data and liveness before requesting a known identity's kill.
    for sandbox in live:
        healthy(sandbox, marker, payload)
    try:
        _kill_confirmed = (Sandbox.kill(victim.sandbox_id, **sdk))
        assert _kill_confirmed
        kill_status = 204
        resources.audit(vm_ids[victim.sandbox_id])
    except (SandboxException, ServiceBusyException) as error:
        kill_status = error.status_code
        assert kill_status in (500, 503), "unrelated kill error"
        # An accepted destroy and a failed reply are not a definite refusal.
        # Never derive native identity or Running from the public ID/503 alone.
    filler.release()
    center_after=services.planned_audit('center')
    original=[record for record in center_after['sandboxes']
              if record['create']['identity']==victim_create]
    already_terminating=(not original or original[0]['terminating'] or original[0]['released'])
    expected=dict(instances=1,cpu=1,memory_bytes=768*MIB,disk_bytes=1024*MIB)
    # If the original center retained Destroy, only observe it. A second DELETE
    # is permitted solely when the stopped original writer's checked authority
    # proves no Destroy was accepted, never on a lost/503 reply by itself.
    recovery=recover_original_create(dict(enospc_probe=config['namespace'],instance='0'),
        expected_resources=expected,read_center=lambda:services.planned_audit('center'),
        read_host=lambda node:services.planned_audit('host-0'),
        deployment=services.center_config['deployment_id'],owner=1,
        create_identity=victim_create,sandbox_id=victim.sandbox_id,
        delete_dispatched=already_terminating,deadline=time.monotonic()+120,
        delete=lambda identity:204 if Sandbox.kill(identity,**sdk) else 404)
    resources.audit(vm_ids[victim.sandbox_id])
    live.remove(victim)
    healthy(control, marker, payload)
    services.wait_ready(for_admission=True)
    fresh = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **sdk)
    live.append(fresh)
    resources.sample()
    assert not fresh.files.exists(marker)
    healthy(fresh, marker, payload)
    return dict(scenario="blocks", fill=filled, create_http_status=429,
                no_new_admission=True, full_disk_kill_http_status=kill_status,
                original_destroy_recovery=recovery,explicit_cleanup_after_release=True,fresh_vm_clean=True)


def exercise_enospc(args, mounted, template):
    services,config,key,url=prepare_services(args,mounted,template)
    resources = Resources(config)
    sdk = options(key, url, url)
    live, vm_ids = [], {}
    filler = Filler(mounted)
    completed = False
    try:
        services.start()
        services.wait_ready()
        assert Sandbox.list(**sdk).next_items() == []
        for _ in range(2):
            previous = set(resources.samples)
            sandbox = Sandbox.create(template="base",timeout=300,allow_internet_access=False,
                metadata=dict(enospc_probe=config['namespace'],instance=str(len(live))),**sdk)
            live.append(sandbox)
            resources.sample()
            added = set(resources.samples) - previous
            assert len(added) == 1
            vm_ids[sandbox.sandbox_id] = added.pop()
        marker = f"/var/tmp/enospc-{secrets.token_hex(8)}"
        payload = b"independent guest survives host pressure\n"
        for sandbox in live:
            healthy(sandbox, marker, payload)
        if args.exhaustion == "inodes":
            report = inode_case(args,services,config,mounted,resources,sdk,live,filler,marker,payload)
        else:
            report = block_case(services,config,mounted,resources,sdk,live,filler,marker,payload,vm_ids)
        for sandbox in list(live):
            _kill_confirmed = (Sandbox.kill(sandbox.sandbox_id, **sdk))
            assert _kill_confirmed
            live.remove(sandbox)
        assert Sandbox.list(**sdk).next_items() == []
        resources.audit()
        assert len(resources.samples) == 3
        (args.root / "enospc-observations.json").write_text(json.dumps(report, indent=2))
        assert report['create_http_status']==(500 if args.exhaustion=='inodes' else 429)
        services.processes.stop_all()
        services.cleanup_staging()
        completed = True
        report.update(resources=resources.samples, audited_before_service_exit=True)
        return report
    finally:
        filler.release()
        if not completed:
            for sandbox in list(live):
                try:
                    Sandbox.kill(sandbox.sandbox_id, **sdk)
                except Exception:
                    pass  # Physical audit, not the API response, gates teardown.
            resources.audit()
        services.processes.stop_all()
