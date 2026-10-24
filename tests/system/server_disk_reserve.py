"""Real dual-service VM admission/cleanup on one private bounded filesystem.

Uses the pinned SDK and a reviewed root-only service/configuration. By default
this tests admission reserve; --exhaustion selects real block/inode ENOSPC.
Failed physical cleanup
keeps the mount namespace alive for exact recovery; do not kill its supervisor
until the reported resources have been inspected and reclaimed.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import copy
from contextlib import closing
import http.client
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import sqlite3
import subprocess
import sys
import traceback
import time

from e2b import Sandbox
from e2b.exceptions import RateLimitException

from sdk_server_smoke import options
from server_process_crash import (
    free_loopback_port, process_start_ticks,
)
from _storage_probe_utils import command, digest, git, trusted
from _complete_runtime import complete_runtime
from _runtime_services import RuntimeServices, add_arguments


MIB = 1024**2
SCRATCH_BYTES = 64 * 1024 * MIB
SCRATCH_INODES = 65536
RESERVE = 256 * MIB


def free_bytes(path):
    status = os.statvfs(path)
    return status.f_bavail * status.f_frsize


def database_count(path):
    # The connection's own context manager commits/rolls back, but does not
    # close the handle; an open DB/WAL descriptor prevents the final unmount.
    with closing(sqlite3.connect(f"file:{path}?mode=ro", uri=True)) as connection:
        return connection.execute("SELECT count(*) FROM sandboxes").fetchone()[0]


class Resources:
    def __init__(self, config):
        self.config = config
        self.claims = Path(config["chroot_base"]) / ".claims"
        self.samples = {}

    def sample(self):
        parent = Path(self.config["host_resources"]["cgroup_parent"])
        expected = {
            "cpu.max": "100000 100000", "cpu.max.burst": "0",
            "memory.max": "805306368", "memory.swap.max": "0", "memory.oom.group": "1",
        }
        for claim in self.claims.iterdir():
            record = json.loads((claim / "record.json").read_text())
            if process_start_ticks(record["pid"]) != record["start_time_ticks"]:
                continue
            group = parent / claim.name
            inode = group.stat().st_ino
            assert inode == json.loads((claim / "cgroup.json").read_text())["inode"]
            membership = Path(f"/proc/{record['pid']}/cgroup").read_text().strip()
            assert membership == "0::/" + str(group.relative_to("/sys/fs/cgroup"))
            actual = {name: (group / name).read_text().strip() for name in expected}
            assert actual == expected
            jail = (Path(self.config["chroot_base"]) /
                    Path(self.config["firecracker"]).name / claim.name / "root")
            with socket.socket(socket.AF_UNIX) as connection:
                connection.settimeout(5)
                connection.connect(str(jail / "api.socket"))
                connection.sendall(b"GET /machine-config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                with http.client.HTTPResponse(connection) as response:
                    response.begin()
                    assert response.status == 200
                    machine = json.load(response)
            assert machine["vcpu_count"] == self.config["vcpu_count"] == 1
            assert machine["mem_size_mib"] == self.config["memory_mib"] == 512
            disk_bytes = (jail / "rootfs.ext4").stat().st_size
            assert disk_bytes == Path(self.config["rootfs"]).stat().st_size == 1024 * MIB
            self.samples[claim.name] = dict(
                pid=record["pid"], ticks=record["start_time_ticks"], cgroup_inode=inode,
                vcpu_count=machine["vcpu_count"], memory_mib=machine["mem_size_mib"],
                private_disk_bytes=disk_bytes,
                cgroup=str(group), namespace_inode=json.loads(
                    (claim / "network.json").read_text())["namespace_inode"], limits=actual,
            )

    def audit(self, ident=None):
        targets = self.samples if ident is None else {ident: self.samples[ident]}
        for name, record in targets.items():
            assert process_start_ticks(record["pid"]) != record["ticks"]
            assert not Path(record["cgroup"]).exists()
            assert not Path("/run/netns", name).exists()
            assert not (Path(self.config["chroot_base"]) /
                        Path(self.config["firecracker"]).name / name).exists()
        inodes = {record["namespace_inode"] for record in targets.values()}
        for proc in Path("/proc").iterdir():
            if proc.name.isdigit():
                try:
                    assert (proc / "ns/net").stat().st_ino not in inodes
                except FileNotFoundError:
                    pass
        if ident is None:
            assert {claim.name for claim in self.claims.iterdir()} == set(self.samples)


def healthy(sandbox, marker, payload):
    info = sandbox.get_info()
    assert info.state == "running" and info.cpu_count == 1 and info.memory_mb == 512
    assert sandbox.commands.run("printf disk-reserve-healthy", timeout=15).stdout == "disk-reserve-healthy"
    sandbox.files.write(marker, payload)
    assert sandbox.files.read(marker, format="bytes") == payload


def prepare_services(args, mounted, template):
    """One fresh approved native pool, not a legacy config upgrader.

    All center, Redis, cache and local allocations share this private filesystem.
    The caller's exact bind-mounted 0711 staging parent lets distinct service
    accounts traverse their OWN 0700 role roots, not the private evidence root.
    """
    from server_templates import descriptor
    config = dict(template)
    for name in ("db", "state", "p"):
        (mounted / name).mkdir(mode=0o700)
    key = secrets.token_hex(32)
    port = free_loopback_port()
    config.update(
        namespace=secrets.token_hex(8), database=str(mounted / "db/controller.db"),
        node_state_dir=str(mounted / "state"), chroot_base=str(mounted / "p/v"),
        max_active_global=4, max_active_per_tenant=4,
        max_disk_bytes_global=8192 * MIB, max_disk_bytes_per_tenant=4096 * MIB,
        cleanup_reserve_bytes=RESERVE, public_ipv4_enabled=False,
        max_timeout_secs=600,max_lifetime_secs=600,startup_timeout_secs=300,
        expiry_scan_seconds=1,
    )
    bound=descriptor('storage-runtime',Path(config['kernel']),Path(config['rootfs']),1,512)
    catalog=dict(templates=[bound],aliases=dict(base=bound['template_id']),compute_limits=dict(
        max_vcpus_global=4,max_vcpus_per_tenant=4,
        max_memory_bytes_global=4*768*MIB,max_memory_bytes_per_tenant=4*768*MIB))
    config['templates']=catalog
    config=complete_runtime(config,slots=4,inbox_sessions=4+config["max_active_global"])
    registry=dict(root=str(mounted/'unused-registry'),max_templates_global=1,max_templates_per_tenant=1,
        max_artifact_bytes_global=2*1024*MIB,max_artifact_bytes_per_tenant=2*1024*MIB,
        max_template_disk_bytes=1024*MIB,validation_timeout_secs=120)
    services=RuntimeServices.prepare(args,mounted,[config],[catalog],key,f'127.0.0.1:{port}',
        args.service_cgroup_record,job_count=4,registry_config=registry)
    return services,services.native_pools[0],key,services.sdk_url


def wait_disk_veto(services):
    """Observe the authenticated original host's physical veto, not a sleep.

    This preserves the pre-admission 429 scenario. A stale positive observation
    followed by an accepted-but-unanswered request is a DIFFERENT unknown case,
    never evidence of rejection without side effects.
    """
    deadline=time.monotonic()+30
    while time.monotonic()<deadline:
        audit=services.planned_audit('center')
        nodes=[node for node in audit['nodes']
               if node['identity']==services.host_configs[0]['inbox']['node']]
        assert len(nodes)==1
        if nodes[0]['free']['disk_bytes']==0:
            return audit
        time.sleep(.1)
    raise TimeoutError('original physical veto not observed; no creation replay')


def exercise(args, mounted, template):
    services,config,key,url=prepare_services(args,mounted,template)
    resources = Resources(config)
    sdk = options(key, url, url)
    live = []
    filler = mounted / "own-filler"
    completed = False
    try:
        services.start()
        services.wait_ready()
        assert Sandbox.list(**sdk).next_items() == []
        for _ in range(2):
            live.append(Sandbox.create(template="base", timeout=300,
                                       allow_internet_access=False, **sdk))
            resources.sample()
        victim, control = live
        marker = f"/var/tmp/reserve-{secrets.token_hex(8)}"
        payload = b"independent guest remains writable\n"
        for sandbox in live:
            healthy(sandbox, marker, payload)
        initial = free_bytes(mounted)
        disk_bytes = Path(config["rootfs"]).stat().st_size
        assert initial > disk_bytes + RESERVE, "a third disk must fit before pressure"
        before_count = database_count(config["database"])
        before_claims = {path.name for path in resources.claims.iterdir()}
        assert before_count == len(before_claims) == 2
        allocated = initial - 384 * MIB
        assert 0 < allocated < SCRATCH_BYTES
        with filler.open("xb") as stream:
            os.posix_fallocate(stream.fileno(), 0, allocated)
            os.fsync(stream.fileno())
        pressured = free_bytes(mounted)
        assert RESERVE <= pressured < disk_bytes + RESERVE
        original_global=wait_disk_veto(services)
        try:
            Sandbox.create(template="base", timeout=300, allow_internet_access=False, **sdk)
        except RateLimitException as error:
            assert error.status_code == 429
        else:
            raise AssertionError("low physical space admitted a new sandbox")
        assert database_count(config["database"]) == before_count
        assert {path.name for path in resources.claims.iterdir()} == before_claims
        after_global=services.planned_audit('center')
        assert after_global['sandbox_highwater']==original_global['sandbox_highwater']
        assert after_global['operation_highwater']==original_global['operation_highwater']
        for sandbox in live:
            healthy(sandbox, marker, payload)
        # Keep the filler in place through physical destruction.
        assert filler.stat().st_size == allocated
        _kill_confirmed = (Sandbox.kill(victim.sandbox_id, **sdk))
        assert _kill_confirmed
        live.remove(victim)
        resources.sample()
        stopped = [name for name, sample in resources.samples.items()
                   if process_start_ticks(sample["pid"]) != sample["ticks"]]
        assert len(stopped) == 1
        resources.audit(stopped[0])
        healthy(control, marker, payload)
        after_kill = free_bytes(mounted)
        filler.unlink()
        recovered = free_bytes(mounted)
        assert recovered > after_kill and recovered >= disk_bytes + RESERVE
        services.wait_ready()
        fresh = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **sdk)
        live.append(fresh)
        resources.sample()
        assert not fresh.files.exists(marker), "fresh VM inherited an earlier private disk"
        healthy(fresh, marker, payload)
        for sandbox in list(live):
            _kill_confirmed = (Sandbox.kill(sandbox.sandbox_id, **sdk))
            assert _kill_confirmed
            live.remove(sandbox)
        assert Sandbox.list(**sdk).next_items() == []
        resources.audit()
        assert len(resources.samples) == 3
        services.processes.stop_all()
        services.cleanup_staging()
        completed = True
        return dict(
            available_before=initial, allocated_filler_bytes=allocated,
            available_under_pressure=pressured, available_after_kill=after_kill,
            available_after_filler_release=recovered, reserve_bytes=RESERVE,
            rejected_http_status=429, rejection_created_no_admission=True,
            killed_while_filler_present=True, fresh_vm_clean=True,
            audited_before_service_exit=True, resources=resources.samples,
        )
    finally:
        if not completed:
            # Restore only this run's own allocation before attempting cleanup.
            filler.unlink(missing_ok=True)
            for sandbox in list(live):
                try:
                    Sandbox.kill(sandbox.sandbox_id, **sdk)
                except Exception:
                    pass  # Physical audit below remains the acceptance gate.
            resources.audit()
        services.processes.stop_all()


def child(args):
    assert os.stat("/proc/self/ns/mnt").st_ino != args.parent_mount_inode
    assert not git("status", "--porcelain")
    revision = git("rev-parse", "HEAD")
    trusted(args.root.parent, private=True)
    assert free_bytes(args.root.parent) > SCRATCH_BYTES + 1024 * MIB
    args.root.mkdir(mode=0o700)
    mounted = args.root / "m"
    mounted.mkdir(mode=0o700)
    image = args.root / "scratch.ext4"
    with image.open("xb") as stream:
        stream.truncate(SCRATCH_BYTES)
    template=dict(jailer=str(args.jailer),firecracker=str(args.firecracker),
        kernel=str(args.kernel),rootfs=str(args.rootfs),uid_base=args.uid_base,
        vcpu_count=1,memory_mib=512,file_policy_digest=1,
        host_resources=dict(cgroup_parent=str(args.cgroup_parent),memory_max_bytes=768*MIB))
    assert Path(template["rootfs"]).stat().st_size == 1024 * MIB
    assert template["vcpu_count"] == 1 and template["memory_mib"] == 512
    assert template["host_resources"]["memory_max_bytes"] == 805306368
    base_hash = digest(Path(template["rootfs"]))
    loop = Path(command("losetup", "--find", "--show", "--nooverlap", str(image)))
    device = loop.stat().st_rdev
    backing = Path(f"/sys/dev/block/{os.major(device)}:{os.minor(device)}/loop/backing_file")
    assert Path(backing.read_text().strip()).samefile(image)
    mounted_ok = False
    try:
        inode_options = ["-N", str(SCRATCH_INODES)]
        command("mkfs.ext4", "-q", "-F", "-m", "0", *inode_options, str(loop))
        # Jailer creates its private KVM/TUN device nodes and executable inside
        # this filesystem; nodev/noexec would invalidate the runtime fixture.
        command("mount", "-t", "ext4", "-o", "nosuid", str(loop), str(mounted))
        mounted_ok = True
        mounted.chmod(0o700)
        assert mounted.stat().st_dev == device != args.root.stat().st_dev
    except BaseException:
        # No server has started yet, so setup failure can release its own mount.
        if mounted_ok:
            command("umount", str(mounted))
        assert Path(backing.read_text().strip()).samefile(image)
        command("losetup", "--detach", str(loop))
        raise
    staging=args.service_staging_parent / ('storage-'+secrets.token_hex(16))
    staging.mkdir(mode=0o711)
    staging.chmod(0o711)
    inner_staging=mounted/'services'
    inner_staging.mkdir(mode=0o711)
    inner_staging.chmod(0o711)
    stage_identity=(staging.stat().st_dev,staging.stat().st_ino)
    try:
        command('mount','--bind',str(inner_staging),str(staging))
        assert staging.stat().st_dev==mounted.stat().st_dev
        fixture_args=copy.copy(args)
        fixture_args.service_staging_parent=staging
        if args.exhaustion:
            from host_enospc import exercise_enospc
            report = exercise_enospc(fixture_args, mounted, template)
        else:
            report = exercise(fixture_args, mounted, template)
        assert digest(Path(template["rootfs"])) == base_hash
        assert git("rev-parse", "HEAD") == revision and not git("status", "--porcelain")
        (args.root / "observations.json").write_text(json.dumps(report, indent=2))
        assert not any(staging.iterdir()), 'exact service staging cleanup incomplete'
        command('umount',str(staging))
        assert (staging.stat().st_dev,staging.stat().st_ino)==stage_identity
        staging.rmdir()
        command("umount", str(mounted))
        assert mounted.stat().st_dev == args.root.stat().st_dev
        assert Path(backing.read_text().strip()).samefile(image)
        command("losetup", "--detach", str(loop))
        assert not backing.exists()
        # Successful evidence is small; the run's exact detached backing image
        # is not a reusable template and must not retain pressure-test blocks.
        image.unlink()
    except BaseException:
        (args.root / "failure.log").write_text(traceback.format_exc())
        print(json.dumps(dict(failed=True, recovery_supervisor_pid=os.getpid(),
                              root=str(args.root), mount_retained=True)), flush=True)
        # Do not let namespace teardown hide leaked resources on a failed audit.
        while True:
            signal.pause()
    report.update(
        revision=revision, script_sha256=digest(Path(__file__)), kernel=os.uname().release,
        center_sha256=digest(args.center),host_sha256=digest(args.host),base_sha256=base_hash,
        scratch_bytes=SCRATCH_BYTES, unmounted=True, loop_detached=True,
        scratch_image_removed=True,
        scope=("selected host ENOSPC and recovery; not final budgets or protection from arbitrary host writes"
               if args.exhaustion else
               "selected physical admission reserve and cleanup; not ENOSPC or final budgets"),
    )
    if args.exhaustion:
        report["enospc_helper_sha256"] = digest(Path(__file__).with_name("host_enospc.py"))
    (args.root / "result.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('jailer','firecracker','kernel','rootfs','cgroup-parent'):
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--uid-base',type=int,required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--exhaustion", choices=("blocks", "inodes"))
    parser.add_argument("--parent-mount-inode", type=int, help=argparse.SUPPRESS)
    add_arguments(parser)
    args = parser.parse_args()
    assert os.geteuid() == 0 and sys.platform == "linux"
    assert importlib.metadata.version("e2b") == "2.51.0"
    os.umask(0o077)
    for name in ('center','host','jailer','firecracker','kernel','rootfs','candidate_manifest'):
        source=getattr(args,name).resolve(strict=True)
        trusted(source,private=True)
        setattr(args,name,source)
    parent=args.service_staging_parent.resolve(strict=True)
    trusted(parent)
    assert parent.stat().st_uid==0 and parent.stat().st_mode&0o7777==0o711
    args.service_staging_parent=parent
    assert args.root.is_absolute() and not args.root.exists()
    if args.parent_mount_inode is None:
        inode = os.stat("/proc/self/ns/mnt").st_ino
        subprocess.run([
            "unshare", "--mount", "--propagation", "private", sys.executable,
            str(Path(__file__).resolve()), *sys.argv[1:], "--parent-mount-inode", str(inode),
        ], check=True)
    else:
        child(args)


if __name__ == "__main__":
    main()
