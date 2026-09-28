"""Real ENOSPC scenarios driven by server_disk_reserve.py's private filesystem.

Only the run's own bounded fillers are released. Creation is never replayed
after an unknown reply; an admitted inode-failed create recovers its original
durable identity. The optional old-503 mode is bug reproduction, not acceptance.
"""

from contextlib import closing
import errno
import json
import os
from pathlib import Path
import secrets
import sqlite3
import time
import urllib.request

from e2b import Sandbox
from e2b.exceptions import (
    RateLimitException, SandboxException, SandboxNotFoundException, ServiceBusyException,
)

from sdk_server_smoke import options
from server_disk_reserve import MIB, RESERVE, SCRATCH_BYTES, Resources, free_bytes, healthy
from server_process_crash import (
    free_loopback_port, process_start_ticks, start_server, stop_server, wait_ready,
)


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
            assert 0 < before.f_favail <= before.f_files <= 2048
            self.path.mkdir(mode=0o700)
            self.created = True
            for index in range(2048):
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


def inode_case(args, config, mounted, resources, sdk, live, filler, marker, payload):
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
    assert len(new_ids) == 1 and len(after) == len(before) + 1
    ident = new_ids.pop()
    assert after[ident]["phase"] == "Starting"
    assert {p.name for p in resources.claims.iterdir()} == claims
    # The unknown create already consumes admission; it has not booted a VM.
    for sandbox in live:
        healthy(sandbox, marker, payload)
    filler.release()
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
    assert set(rows(config)) == set(after), "recovery admitted a second create"
    assert not recovered.files.exists(marker)
    healthy(recovered, marker, payload)
    return dict(scenario="inodes", fill=filled, create_http_status=status,
                expected_create_http_status=args.expected_inode_status,
                durable_create_before_error=True, no_vm_before_inode_release=True,
                original_create_recovered_without_replay=True,
                old_busy_bug_reproduced=status == 503)


def block_case(config, mounted, resources, sdk, live, filler, marker, payload, vm_ids):
    before = rows(config)
    claims = {p.name for p in resources.claims.iterdir()}
    filled = filler.fill("blocks")
    assert free_bytes(mounted) < Path(config["rootfs"]).stat().st_size + RESERVE
    try:
        Sandbox.create(template="base", timeout=300, allow_internet_access=False, **sdk)
    except RateLimitException as error:
        assert error.status_code == 429
    else:
        raise AssertionError("full block filesystem admitted a create")
    assert set(rows(config)) == set(before)
    assert {p.name for p in resources.claims.iterdir()} == claims
    victim, control = live
    # Check guest data and liveness before requesting a known identity's kill.
    for sandbox in live:
        healthy(sandbox, marker, payload)
    try:
        assert Sandbox.kill(victim.sandbox_id, **sdk)
        kill_status = 204
        resources.audit(vm_ids[victim.sandbox_id])
    except (SandboxException, ServiceBusyException) as error:
        kill_status = error.status_code
        assert kill_status in (500, 503), "unrelated kill error"
        if kill_status == 503:
            # A genuine refused call must not already have revoked/stopped it.
            assert rows(config)[victim.sandbox_id]["phase"] == "Running"
            sample = resources.samples[vm_ids[victim.sandbox_id]]
            assert process_start_ticks(sample["pid"]) == sample["ticks"]
    filler.release()
    # This is explicit cleanup of a known ID, never replay of Create/Start.
    Sandbox.kill(victim.sandbox_id, **sdk)
    resources.audit(vm_ids[victim.sandbox_id])
    live.remove(victim)
    healthy(control, marker, payload)
    fresh = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **sdk)
    live.append(fresh)
    resources.sample()
    assert not fresh.files.exists(marker)
    healthy(fresh, marker, payload)
    return dict(scenario="blocks", fill=filled, create_http_status=429,
                no_new_admission=True, full_disk_kill_http_status=kill_status,
                explicit_cleanup_after_release=True, fresh_vm_clean=True)


def exercise_enospc(args, mounted, template):
    config = dict(template)
    for name in ("db", "state", "jails"):
        (mounted / name).mkdir(mode=0o700)
    key = secrets.token_hex(32)
    port = free_loopback_port()
    config.update(
        namespace=secrets.token_hex(8), database=str(mounted / "db/controller.db"),
        node_state_dir=str(mounted / "state"), chroot_base=str(mounted / "jails"),
        bind=f"127.0.0.1:{port}", credentials=[dict(tenant=1, secret=key)],
        max_active_global=4, max_active_per_tenant=4,
        max_disk_bytes_global=8192 * MIB, max_disk_bytes_per_tenant=4096 * MIB,
        cleanup_reserve_bytes=RESERVE, public_ipv4_enabled=False,
    )
    config_path = args.root / "config.json"
    config_path.write_text(json.dumps(config))
    config_path.chmod(0o600)
    resources = Resources(config)
    url = f"http://127.0.0.1:{port}"
    sdk = options(key, url, url)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    process = start_server(args.server, config_path, args.root / "server.log", None)
    live, vm_ids = [], {}
    filler = Filler(mounted)
    completed = False
    try:
        wait_ready(opener, url, key, process)
        assert Sandbox.list(**sdk).next_items() == []
        for _ in range(2):
            previous = set(resources.samples)
            sandbox = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **sdk)
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
            report = inode_case(args, config, mounted, resources, sdk, live, filler, marker, payload)
        else:
            report = block_case(config, mounted, resources, sdk, live, filler, marker, payload, vm_ids)
        for sandbox in list(live):
            assert Sandbox.kill(sandbox.sandbox_id, **sdk)
            live.remove(sandbox)
        assert Sandbox.list(**sdk).next_items() == []
        resources.audit()
        assert len(resources.samples) == 3
        (args.root / "enospc-observations.json").write_text(json.dumps(report, indent=2))
        if args.exhaustion == "inodes":
            assert report["create_http_status"] == args.expected_inode_status
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
        stop_server(process)
