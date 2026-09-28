"""Real VM admission/cleanup under low space on a private 4 GiB host filesystem.

Uses the pinned SDK and a reviewed root-only service/configuration. By default
this tests admission reserve; --exhaustion selects real block/inode ENOSPC.
Failed physical cleanup
keeps the mount namespace alive for exact recovery; do not kill its supervisor
until the reported resources have been inspected and reclaimed.
"""

import argparse
from contextlib import closing
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import signal
import sqlite3
import subprocess
import sys
import traceback
import urllib.request

from e2b import Sandbox
from e2b.exceptions import RateLimitException

from sdk_server_smoke import options
from server_process_crash import (
    free_loopback_port, process_start_ticks, start_server, stop_server, wait_ready,
)
from server_storage_filesystems import command, digest, git, trusted


MIB = 1024**2
SCRATCH_BYTES = 4096 * MIB
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
            self.samples[claim.name] = dict(
                pid=record["pid"], ticks=record["start_time_ticks"], cgroup_inode=inode,
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
    assert sandbox.get_info().state == "running"
    assert sandbox.commands.run("printf disk-reserve-healthy", timeout=15).stdout == "disk-reserve-healthy"
    sandbox.files.write(marker, payload)
    assert sandbox.files.read(marker, format="bytes") == payload


def exercise(args, mounted, template):
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
    live = []
    filler = mounted / "own-filler"
    completed = False
    try:
        wait_ready(opener, url, key, process)
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
        try:
            Sandbox.create(template="base", timeout=300, allow_internet_access=False, **sdk)
        except RateLimitException as error:
            assert error.status_code == 429
        else:
            raise AssertionError("low physical space admitted a new sandbox")
        assert database_count(config["database"]) == before_count
        assert {path.name for path in resources.claims.iterdir()} == before_claims
        for sandbox in live:
            healthy(sandbox, marker, payload)
        # Keep the filler in place through physical destruction.
        assert filler.stat().st_size == allocated
        assert Sandbox.kill(victim.sandbox_id, **sdk)
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
        fresh = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **sdk)
        live.append(fresh)
        resources.sample()
        assert not fresh.files.exists(marker), "fresh VM inherited an earlier private disk"
        healthy(fresh, marker, payload)
        for sandbox in list(live):
            assert Sandbox.kill(sandbox.sandbox_id, **sdk)
            live.remove(sandbox)
        assert Sandbox.list(**sdk).next_items() == []
        resources.audit()
        assert len(resources.samples) == 3
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
        stop_server(process)


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
    template = json.loads(args.config.read_text())
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
        inode_options = ["-N", "1024"] if args.exhaustion == "inodes" else []
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
    try:
        if args.exhaustion:
            from host_enospc import exercise_enospc
            report = exercise_enospc(args, mounted, template)
        else:
            report = exercise(args, mounted, template)
        assert digest(Path(template["rootfs"])) == base_hash
        assert git("rev-parse", "HEAD") == revision and not git("status", "--porcelain")
        (args.root / "observations.json").write_text(json.dumps(report, indent=2))
        command("umount", str(mounted))
        assert mounted.stat().st_dev == args.root.stat().st_dev
        assert Path(backing.read_text().strip()).samefile(image)
        command("losetup", "--detach", str(loop))
        assert not backing.exists()
    except BaseException:
        (args.root / "failure.log").write_text(traceback.format_exc())
        print(json.dumps(dict(failed=True, recovery_supervisor_pid=os.getpid(),
                              root=str(args.root), mount_retained=True)), flush=True)
        # Do not let namespace teardown hide leaked resources on a failed audit.
        while True:
            signal.pause()
    report.update(
        revision=revision, script_sha256=digest(Path(__file__)), kernel=os.uname().release,
        server_sha256=args.server_sha256, base_sha256=base_hash,
        scratch_bytes=SCRATCH_BYTES, unmounted=True, loop_detached=True,
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
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--server-sha256", required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--exhaustion", choices=("blocks", "inodes"))
    parser.add_argument("--expected-inode-status", type=int, choices=(500, 503), default=500,
                        help="503 only reproduces the old unsafe busy reply; acceptance requires 500")
    parser.add_argument("--parent-mount-inode", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    assert os.geteuid() == 0 and sys.platform == "linux"
    assert importlib.metadata.version("e2b") == "2.51.0"
    os.umask(0o077)
    trusted(args.server, private=True)
    trusted(args.config, private=True)
    assert digest(args.server) == args.server_sha256
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
