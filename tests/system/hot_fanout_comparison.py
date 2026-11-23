#!/usr/bin/env python3
"""Compare one production hot fan-out batch with independent production VMs.

Run both --mode values on the same host with the same candidate, receipt, N
and write size. Use a fresh --output directory for each run. The cold clock
starts after service readiness. The prepared clock starts at parent readiness
for hot fan-out and after N independent workloads have been prepared via SDK
commands for the baseline. SDK ready measures the same file and command
validation in both arms.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import fcntl
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import shlex
import struct
import sys
import tempfile
import threading
import time

import hot_fanout_product as product
from _complete_runtime import terminal_cleanup_confirmed
from lifecycle_unload_runtime import original_json, original_process
from lifecycle_unload_scenario import PREFIX, PublicUnloadScenario, failure_details
from sdk_c02_acceptance import processes_in_uid_range
from server_vm_id_capacity import check_no_resources
from tools import service_cgroup
from tools.ci_e2e_artifacts import publish
from tools.ci_e2e_prepare import contract_versions
from tools.single_host_storage import reopen
from tools.system_acceptance import digest, json_file, private_path, write_once

INHERITED = "/run/inherited"
PAYLOAD = "/var/tmp/sp1026-comparison-data"
WORKER_PID = "/run/sp1026-comparison-worker.pid"
# Linux fs.h and fiemap.h UAPI, for the fixture's x86_64/aarch64 XFS host.
FS_IOC_FIEMAP = 0xC020660B
FIEMAP_FLAG_SYNC = 1
FIEMAP_EXTENT_LAST = 1
FIEMAP_EXTENT_UNMAPPED = 2 | 4  # UNKNOWN or DELALLOC
FIEMAP_EXTENT_SHARED = 0x2000
FIEMAP_HEADER = struct.Struct("=QQIIII")
FIEMAP_EXTENT = struct.Struct("=QQQQQIIII")
EXTENT_CAPACITY = 65536
# Same construction as tools/build_hot_fanout_guest.py. Independent VMs
# materialize their own 64 MiB working set once, without a host fan-out channel.
INDEPENDENT_WORKLOAD = """
import hashlib, mmap, os, sys, time
from pathlib import Path
page = os.sysconf("SC_PAGE_SIZE")
heap = mmap.mmap(-1, 64 * 1024**2, flags=mmap.MAP_PRIVATE | mmap.MAP_ANONYMOUS)
expected = hashlib.shake_256(("sandproof-fanout-fixture:" + sys.argv[1]).encode()).digest(len(heap) // page)
heap[::page] = expected
if heap[::page] != expected:
    raise RuntimeError("independent checkpoint workload mismatch")
Path(sys.argv[2]).write_text(str(os.getpid()))
Path("/run/inherited").write_text(expected[:16].hex())
while True:
    time.sleep(60)
"""
WRITE_PAYLOAD = """
import os, sys
value = bytes([int(sys.argv[1]) % 256]) * 1024**2
with open(sys.argv[3], "xb") as output:
    for _ in range(int(sys.argv[2])):
        output.write(value)
    output.flush()
    os.fsync(output.fileno())
with open(sys.argv[3], "rb") as source:
    head = source.read(1)
    source.seek(-1, os.SEEK_END)
    if head != value[:1] or source.read(1) != head:
        raise RuntimeError("comparison payload mismatch")
print(os.stat(sys.argv[3]).st_blocks * 512)
"""


def parallel(count, action):
    gate = threading.Barrier(count, timeout=30)

    def start(index):
        gate.wait()
        return action(index)

    with ThreadPoolExecutor(max_workers=count) as pool:
        tasks = [pool.submit(start, index) for index in range(count)]
        return [task.result(timeout=180) for task in tasks]


def source_claims(native, count):
    claim_dir = Path(native["chroot_base"]) / ".claims"
    claims = [original_json(path, 65536) for path in sorted(claim_dir.glob("*/record.json"))]
    assert len(claims) == count and len({claim["resource_id"] for claim in claims}) == count
    assert all(claim["resource_id"].startswith("vm-" + native["namespace"] + "-")
               and original_process(claim) for claim in claims)
    return claims


def used_bytes(path):
    stat = os.statvfs(path)
    return (stat.f_blocks - stat.f_bfree) * stat.f_frsize


def memory_bytes(path, name):
    return int((path / name).read_text())


def rootfs_extents(fd):
    """Count actual shared and exclusive XFS data allocations after SDK timing."""
    buffer = bytearray(FIEMAP_HEADER.size + EXTENT_CAPACITY * FIEMAP_EXTENT.size)
    FIEMAP_HEADER.pack_into(buffer, 0, 0, (1 << 64) - 1, FIEMAP_FLAG_SYNC, 0, EXTENT_CAPACITY, 0)
    fcntl.ioctl(fd, FS_IOC_FIEMAP, buffer)
    mapped = FIEMAP_HEADER.unpack_from(buffer)[3]
    if not 0 < mapped <= EXTENT_CAPACITY:
        raise ValueError("invalid rootfs extent count")
    shared = exclusive = end = 0
    for index in range(mapped):
        logical, _, length, _, _, flags, *_ = FIEMAP_EXTENT.unpack_from(
            buffer, FIEMAP_HEADER.size + index * FIEMAP_EXTENT.size)
        if length == 0 or logical < end or flags & FIEMAP_EXTENT_UNMAPPED:
            raise ValueError("unresolved rootfs extent allocation")
        if flags & FIEMAP_EXTENT_SHARED:
            shared += length
        else:
            exclusive += length
        end = logical + length
    if not flags & FIEMAP_EXTENT_LAST:
        raise ValueError("rootfs extent inventory exceeds fixture bound")
    return shared, exclusive


@contextmanager
def observe_peak(vm_group, data, report):
    """Bounded low-frequency volume/cgroup measurement without retaining a trace."""
    initial = used_bytes(data)
    result = dict(volume_baseline_bytes=initial, sampled_volume_peak_growth_bytes=0,
                  sampled_vm_cgroup_peak_bytes=0, interval_ms=100)
    report["peak"] = result
    stop = threading.Event()
    errors = []

    def sample():
        while not stop.is_set():
            try:
                result["sampled_volume_peak_growth_bytes"] = max(
                    result["sampled_volume_peak_growth_bytes"], used_bytes(data) - initial)
                result["sampled_vm_cgroup_peak_bytes"] = max(
                    result["sampled_vm_cgroup_peak_bytes"], memory_bytes(vm_group, "memory.current"))
            except Exception as error:
                errors.append(error)
                return
            stop.wait(.1)

    worker = threading.Thread(target=sample, daemon=True)
    worker.start()
    try:
        yield
    finally:
        stop.set()
        worker.join(3)
        assert not worker.is_alive()
        if errors:
            raise errors[0]
    peak = vm_group / "memory.peak"
    result["kernel_vm_cgroup_peak_bytes"] = int(peak.read_text()) if peak.exists() else None


def resource_observation(args, services, claims, vm_group, data, before_volume):
    native = services.native_pools[0]
    rss = pss = allocated = shared = exclusive = 0
    for claim in claims:
        assert original_process(claim)
        rollup = Path(f"/proc/{claim['pid']}/smaps_rollup").read_text()
        labels = {line.split()[0]: int(line.split()[1]) for line in rollup.splitlines()
                  if line.startswith(("Rss:", "Pss:"))}
        rss += labels["Rss:"] * 1024
        pss += labels["Pss:"] * 1024
        path = Path(native["chroot_base"]) / args.firecracker.name / claim["resource_id"] / "root/rootfs.ext4"
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            file_shared, file_exclusive = rootfs_extents(fd)
            allocated += os.fstat(fd).st_blocks * 512
        finally:
            os.close(fd)
        shared += file_shared
        exclusive += file_exclusive
        assert original_process(claim)
    return dict(vms=len(claims), vmm_rss_bytes=rss, vmm_pss_bytes=pss,
                vm_cgroup_bytes=memory_bytes(vm_group, "memory.current"),
                rootfs_st_blocks_bytes_including_shared_extents=allocated,
                rootfs_shared_extent_bytes_summed_per_file=shared,
                rootfs_exclusive_extent_bytes=exclusive,
                observed_data_volume_growth_bytes=used_bytes(data) - before_volume)


class WindowClient(PublicUnloadScenario):
    """Reuse one read-only window while assigning distinct mutation sequences."""

    def reserve_window(self):
        result = self.read_json("GET", PREFIX + "/request-window", key=self.owner_key)
        assert set(result) == {"window"} and type(result["window"]) is int and result["window"] > 0
        return result["window"]

    def create_at(self, parameters, window, sequence):
        assert self.create_request is None and self.session is None
        self.window = window
        self.create_request = dict(window=window, sequence=sequence, parameters=parameters)
        self.step("create-acceptance", 202)
        self.operation_sent("create")
        response = self.read_json("POST", PREFIX + "/create", key=self.owner_key,
                                  body=self.create_request, status=202)
        accepted = self.acceptance(response, sequence=sequence, version=1)
        self.create_acceptance = self.current_acceptance = accepted
        self.session = accepted["session"]
        self.sequence = sequence
        self.operation_accepted()


def prepare_independent(client, owner, checkpoint, parameters, window, index):
    from e2b import Sandbox

    client.create_at(parameters, window, index + 1)
    client.wait_running()
    sandbox = Sandbox.connect(client.session, **owner)
    # Pin the same image bytes. The image unit's boot attempt cannot start
    # a host-controlled donor without the channel, and is stopped before the
    # independent resident workload is started.
    stopped = sandbox.commands.run(
        "systemctl stop sandproof-hot-fanout.service && "
        "test ! -e " + INHERITED + " && test ! -e " + WORKER_PID,
        user="root", timeout=25,
    )
    assert stopped.exit_code == 0
    background = sandbox.commands.run(
        "exec python3 -c " + shlex.quote(INDEPENDENT_WORKLOAD) + " " +
        shlex.quote(checkpoint) + " " + WORKER_PID,
        user="root", background=True, timeout=0)
    background.disconnect()
    ready = sandbox.commands.run(
        "until test -s " + INHERITED + "; do sleep .05; done",
        user="root", timeout=25,
    )
    assert ready.exit_code == 0
    return client.session


def guest_workload_rss(sandbox, hot):
    # Branch PID 1 is the fork of the resident checkpoint process. The
    # independent arm recorded the PID of its own resident process.
    pid = "1" if hot else f"$(cat {WORKER_PID})"
    result = sandbox.commands.run(f"awk '/^VmRSS:/ {{print $2}}' /proc/{pid}/status",
                                  user="root", timeout=20)
    assert result.exit_code == 0
    rss = int(result.stdout.strip()) * 1024
    assert rss >= 64 * product.MIB
    return rss


def measure_workload(args, receipt, services, native, ready, vm_group, data, clients, claims, report):
    from e2b import Sandbox
    from sdk_server_smoke import options as sdk_options

    arm = args.mode
    key = services.key
    owner = sdk_options(key, services.sdk_url, services.sdk_url)
    prepare = product.create_parameters(services, ready, receipt["binding"])
    checkpoint = receipt["native_hot_fanout"]["template"]["checkpoint_id"]
    slots = 1 if arm == "hot" else args.n
    client = clients[0]
    started = time.monotonic()
    before_volume = used_bytes(data)
    with observe_peak(vm_group, data, report):
        if arm == "hot":
            client.create(prepare)
            running = client.wait_running()
            parent = Sandbox.connect(client.session, **owner)
            assert parent.sandbox_id == client.session
            preparation_end = time.monotonic()
            _, _, identities = product.start_batch(client, running, checkpoint, args.n, report)
        else:
            window = client.reserve_window()
            identities = parallel(args.n, lambda index: prepare_independent(
                clients[index], owner, checkpoint, prepare, window, index))
            assert len(set(identities)) == args.n
            preparation_end = time.monotonic()
        report["preparation_seconds"] = preparation_end - started
        sandboxes = product.concurrent_sdk(identities, owner, receipt["expected_inherited_mark"], report)
        sdk_complete = time.monotonic()
        report.update(cold_to_all_sdk_seconds=sdk_complete - started,
                      prepared_to_all_sdk_seconds=sdk_complete - preparation_end)
        claims.extend(source_claims(native, slots))
        report["sdk_ready_resources"] = resource_observation(
            args, services, claims, vm_group, data, before_volume)
        # Per-process RSS counts common forked pages in each branch. Use the
        # host cgroup/PSS measurements above for aggregate physical comparison.
        report["guest_workload_rss_bytes_per_branch"] = parallel(
            args.n, lambda index: guest_workload_rss(sandboxes[index], arm == "hot"))

        def write(index):
            snippet = f"python3 -c {shlex.quote(WRITE_PAYLOAD)} {index} {args.write_mib} {PAYLOAD}"
            result = sandboxes[index].commands.run(snippet, user="root", timeout=45)
            assert result.exit_code == 0
            return int(result.stdout.strip())

        payload_sizes = parallel(args.n, write)
        assert min(payload_sizes) >= args.write_mib * product.MIB
        report["payload_allocated_bytes"] = sum(payload_sizes)
        if arm == "hot":
            # The current private upper and work directories live on guest
            # tmpfs. Their blocks are not host-storage blocks.
            result = parent.commands.run(
                "test \"$(stat -f -c %T /run/sandproof-hot-fanout/branches)\" = tmpfs && "
                "find /run/sandproof-hot-fanout/branches -type d "
                "\\( -name upper -o -name work \\) -prune -exec du -s -B1 {} +",
                user="root", timeout=20,
            )
            assert result.exit_code == 0
            entries = [int(line.split()[0]) for line in result.stdout.splitlines()]
            assert len(entries) == 2 * args.n
            report["guest_overlay_upper_work_allocated_bytes"] = sum(entries)
            result = parent.commands.run(
                "find /sys/fs/cgroup/sandproof-hot-fanout -mindepth 2 -maxdepth 2 "
                "-name memory.current -type f -exec cat {} +",
                user="root", timeout=20,
            )
            assert result.exit_code == 0
            entries = [int(line) for line in result.stdout.splitlines()]
            assert len(entries) in (0, args.n)
            report["guest_branch_cgroup_bytes_after_payload"] = sorted(entries) or None
        report["after_payload_resources"] = resource_observation(
            args, services, claims, vm_group, data, before_volume)


def clean_clients(clients, report, first_destroy_sequence):
    """Destroy only acknowledged sessions. A lost Create reply preserves the whole trial."""
    if not clients:
        return []

    def clean(index):
        client = clients[index]
        if client.session is None:
            return None, client.create_request is not None, None
        try:
            client.deadline = time.monotonic() + 90
            current = client.query()
            if current["phase"] not in ("terminating", "cleaned"):
                client.sequence = max(client.sequence, first_destroy_sequence + index)
                client.destroy(current)
            if current["phase"] != "cleaned":
                client.wait_cleaned()
            return client.session, False, None
        except Exception as failure:
            return None, False, type(failure).__name__

    results = parallel(len(clients), clean)
    report["unresolved_creation"] = any(unresolved for _, unresolved, _ in results)
    report["cleanup_errors"] = [error for _, _, error in results if error is not None]
    return [session for session, _, _ in results if session is not None]


def run(args, receipt, report):
    paths = dict(center=args.center, host=args.host)
    versions = {role: contract_versions(path, product.ENV) for role, path in paths.items()}
    assert versions["center"] == versions["host"]
    publish(args.output / "candidate.json", candidate=args.candidate,
            paths={role: str(path) for role, path in paths.items()}, versions=versions["center"])
    storage = Path(tempfile.mkdtemp(prefix="h.", dir=args.storage_parent))
    storage.chmod(0o711)
    report["storage"] = str(storage)
    write_once(args.output / "location.json", {"storage": str(storage)})
    slots = 1 if args.mode == "hot" else args.n
    with product.phase(report, "provision"):
        services, root, record, groups, installation = product.build_deployment(
            args, receipt, storage, paths, report, slots=slots, hot_fanout=args.mode == "hot")
    native = services.native_pools[0]
    clients = []
    claims = []
    data = storage / "data"
    try:
        with product.phase(report, "service_readiness"):
            services.start()
            ready = services.wait_ready(timeout=180)
        clients = [WindowClient(services.sdk_url, services.key, secrets.token_hex(32),
                                services.center_config["deployment_id"], time.monotonic() + 480)
                   for _ in range(slots)]
        with product.phase(report, "workload"):
            measure_workload(args, receipt, services, native, ready, groups[0],
                             data, clients, claims, report)
    finally:
        with product.phase(report, "cleanup"):
            # Allocate sequences disjoint from every keyed Create. Hot setup
            # also uses sequence 2 for the batch.
            known = clean_clients(clients, report, max(slots, 2))
            services.stop_all()
            check_no_resources(root / "0", native["namespace"], args.firecracker, args.vm_uid,
                               claims, native=native)
            assert not report.get("unresolved_creation") and not report.get("cleanup_errors")
            assert all(terminal_cleanup_confirmed(Path(native["database"]), services.native_id_for(session))
                       for session in known)
            service_cgroup.remove(record)
            services.cleanup_staging()
            for group in groups:
                group.rmdir()
            assert reopen(storage, installation["receipt"]["project"]) == installation["receipt"]
            product.command("/usr/bin/umount", data)
            assert not os.path.ismount(data)
            (storage / "control/volume.xfs").unlink()
            report["physical_test_volume_removed"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("hot", "independent"), required=True)
    parser.add_argument("--write-mib", type=int, choices=(1, 16), default=1)
    args = product.checked_args(parser)
    if args.n > 4:
        parser.error("this first comparison fixture supports two to four branches")
    if args.mode == "independent" and processes_in_uid_range(args.vm_uid, args.vm_uid + args.n - 1):
        parser.error("a reserved independent VM UID already has a process")
    package = importlib.metadata.distribution("e2b")
    provenance = json.loads(package.read_text("direct_url.json"))
    assert package.version == "2.51.0" and provenance["vcs_info"]["commit_id"] == product.SDK_COMMIT
    os.umask(0o077)
    receipt = json_file(private_path(args.receipt), limit=65536)
    assert args.n <= receipt["native_hot_fanout"]["template"]["max_branches"]
    args.output.mkdir(mode=0o700)
    report = dict(mode=args.mode, candidate=args.candidate, requested_branches=args.n,
                  payload_mib_per_branch=args.write_mib, phase_seconds={}, passed=False,
                  fixture_sha256=digest(args.receipt),
                  binaries={role: digest(getattr(args, role))
                            for role in ("center", "host", "firecracker", "jailer")},
                  vm_shape=receipt["binding"]["resources"],
                  sdk_commit=product.SDK_COMMIT)
    try:
        run(args, receipt, report)
        report["passed"] = True
    except Exception as failure:
        report.setdefault("failure", dict(error=type(failure).__name__,
                                          **failure_details(failure, None, "setup")))
    finally:
        write_once(args.output / "result.json", report)
        print(json.dumps(report, sort_keys=True))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
