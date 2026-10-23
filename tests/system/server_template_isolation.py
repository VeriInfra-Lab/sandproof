"""Two templates, two concurrent private package databases per template.

Run on an isolated Linux/KVM host with approved Debian/Ubuntu images. Package
fixtures are built inside guests without downloads. Guest initializer failures
and invalid-kernel startup failures are distinct cases; neither is a controller
crash test. Failed runs retain their exact private directory for recovery.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import asyncio
import http.client
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request

import httpx
from e2b import AsyncSandbox, Sandbox

from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from sdk_private_root import (
    CONFIG, EXECUTABLE, FAILED_INIT, PACKAGE, clean, failed_initializer,
    install, installed, run,
)
from sdk_server_smoke import options
from sdk_template_defaults import (
    LITERAL_ARG, START_CONTEXT, START_COUNT, START_PID, defaults, expected_context,
    exercise as check_defaults,
)
from server_process_crash import (
    complete_runtime, free_loopback_port, idle_snapshot, process_start_ticks,
    original_create_identity, terminal_cleanup_confirmed,
)
from server_templates import MIB, assert_shape, descriptor, mapped_template, record_for
from _runtime_services import RuntimeServices, add_arguments


VM_COUNT = 8  # Four initial VMs, two failed boots, two clean replacements.
PHYSICAL_SLOTS = 4


def shared_network():
    def stable(value):
        if isinstance(value, dict):
            return {key: stable(item) for key, item in value.items()
                    if key not in ("metainfo", "packets", "bytes")}
        if isinstance(value, list):
            return [stable(item) for item in value
                    if not isinstance(item, dict) or "metainfo" not in item]
        return value
    rules = json.loads(subprocess.check_output(["/usr/sbin/nft", "-j", "list", "ruleset"]))
    return {"forward": Path("/proc/sys/net/ipv4/ip_forward").read_text(),
            "interfaces": sorted(path.name for path in Path("/sys/class/net").iterdir()),
            "rules": stable(rules)}


def phases(database):
    with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
        return {ident: json.loads(blob)["phase"] for ident, blob in
                connection.execute("SELECT sandbox_id, state_blob FROM sandboxes")}


def audit_gone(root, firecracker, cgroup_parent, name, identity=None):
    claim = root / "jails/.claims" / name
    # Capture while live, before complete maintenance can retire claim files.
    # The fallback is for callers that deliberately observe an unretired claim.
    record = identity if identity is not None else json.loads((claim / "record.json").read_text())
    assert record["resource_id"] == name
    assert process_start_ticks(record["pid"]) != record["start_time_ticks"]
    assert not (root / "jails" / firecracker.name / name).exists()
    assert not Path("/run/netns", name).exists()
    assert not (cgroup_parent / name).exists()
    inode = (record["network_inode"] if identity is not None else
             json.loads((claim / "network.json").read_text())["namespace_inode"])
    for proc in Path("/proc").iterdir():
        if proc.name.isdecimal():
            try:
                assert (proc / "ns/net").stat().st_ino != inode
            except (FileNotFoundError, ProcessLookupError):
                pass
    return {"resource_id": name, "pid": record["pid"],
            "start_time_ticks": record["start_time_ticks"], "network_inode": inode,
            "exact_resources_absent": True}


def live_shape(root, args, sandbox, expected, opener, url, key, *, native_id, native_template):
    record = assert_shape(root, args.firecracker, args.cgroup_parent,
                          sandbox,expected,opener,url,key,native_id=native_id,native_template=native_template)
    _, name, _ = record_for(root,native_id=native_id)
    private = root / "jails" / args.firecracker.name / name / "root/rootfs.ext4"
    source = Path(expected["rootfs"]["path"]).stat()
    disk = private.stat()
    assert (source.st_dev, source.st_ino) != (disk.st_dev, disk.st_ino)
    group = args.cgroup_parent / name
    bound = json.loads((root / "jails/.claims" / name / "cgroup.json").read_text())
    assert group.stat().st_ino == bound["inode"]
    assert Path("/proc", str(record["pid"]), "cgroup").read_text().strip() == (
        "0::/" + str(group.relative_to("/sys/fs/cgroup")))
    limits = {key: (group / key).read_text().strip() for key in
              ("cpu.max", "cpu.max.burst", "memory.max", "memory.swap.max", "memory.oom.group")}
    shape = expected["resources"]
    assert limits == {"cpu.max": f"{shape['cpu_count'] * 100000} 100000", "cpu.max.burst": "0",
                      "memory.max": str(shape["host_memory_bytes"]), "memory.swap.max": "0",
                      "memory.oom.group": "1"}
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(5)
        connection.connect(str(private.parent / "api.socket"))
        connection.sendall(b"GET /machine-config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        with http.client.HTTPResponse(connection) as response:
            response.begin()
            assert response.status == 200
            machine = json.load(response)
    assert (machine["vcpu_count"], machine["mem_size_mib"]) == (
        shape["cpu_count"], shape["memory_mib"])
    return {"native_id":native_id,"resource_id": name, "pid": record["pid"], "start_time_ticks": record["start_time_ticks"],
            "network_inode": json.loads((root / "jails/.claims" / name / "network.json").read_text())["namespace_inode"],
            "disk": {"device": disk.st_dev, "inode": disk.st_ino, "bytes": disk.st_size},
            "cgroup_inode": bound["inode"], "limits": limits,
            "machine": {"cpus": machine["vcpu_count"], "memory_mib": machine["mem_size_mib"]}}


async def exercise(args):
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an isolated Linux/KVM host")
    assert importlib.metadata.version("e2b") == "2.51.0"
    assert args.uid_base >= 1000 and args.uid_base + PHYSICAL_SLOTS < 2**32 - 1
    assert not processes_in_uid_range(args.uid_base, args.uid_base + PHYSICAL_SLOTS - 1)
    parent = args.scratch_parent.resolve(strict=True)
    assert parent.stat().st_uid == 0 and not parent.stat().st_mode & 0o077
    args.cgroup_parent = args.cgroup_parent.resolve(strict=True)
    assert not (args.cgroup_parent / "cgroup.procs").read_text().strip()
    assert not any(path.is_dir() for path in args.cgroup_parent.iterdir())
    assert {"cpu", "memory"} <= set((args.cgroup_parent / "cgroup.subtree_control").read_text().split())
    checkout = Path(__file__).resolve().parents[2]
    assert not subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"])
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    for name in ("center","host", "jailer", "firecracker", "kernel", "rootfs", "second_rootfs"):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    inputs = {name: {"sha256": sha256_file(getattr(args, name)), "bytes": getattr(args, name).stat().st_size}
              for name in ("center","host", "jailer", "firecracker", "kernel", "rootfs", "second_rootfs")}
    assert inputs["rootfs"] != inputs["second_rootfs"], "use distinct approved template images"
    assert inputs["rootfs"]["bytes"] != inputs["second_rootfs"]["bytes"], "exercise two disk sizes"
    disk_sum = inputs["rootfs"]["bytes"] + inputs["second_rootfs"]["bytes"]
    assert shutil.disk_usage(parent).free >= 3 * disk_sum + 1024 * MIB
    available = int(next(line.split()[1] for line in Path("/proc/meminfo").read_text().splitlines()
                         if line.startswith("MemAvailable:"))) * 1024
    assert available >= (4096 + 4096) * MIB and len(os.sched_getaffinity(0)) >= 6
    before_network = shared_network()
    root = Path(tempfile.mkdtemp(prefix="ti-", dir=parent))
    namespace = secrets.token_hex(8)
    assert len(os.fsencode(root / "jails" / args.firecracker.name / f"vm-{namespace}-7/root/api.socket")) < 108
    print(json.dumps({"event": "run_created", "root": str(root), "namespace": namespace,
                      "source_commit": commit}), flush=True)
    for name in ("db", "state", "jails", "images"):
        (root / name).mkdir(mode=0o700)
    kernel = root / "images/kernel"
    bad_kernel = root / "images/unbootable-kernel"
    images = [root / "images/first.ext4", root / "images/second.ext4"]
    for source, target in ((args.kernel, kernel), (args.rootfs, images[0]), (args.second_rootfs, images[1])):
        subprocess.run(["/usr/bin/cp", "--reflink=auto", "--sparse=always", str(source), str(target)], check=True)
        target.chmod(0o444)
        assert sha256_file(target) == sha256_file(source)
    shutil.copyfile(kernel, bad_kernel)
    with bad_kernel.open("r+b") as stream:
        stream.write(bytes(4096))  # Corrupt only this private negative fixture.
        stream.flush()
        os.fsync(stream.fileno())
    bad_kernel.chmod(0o444)
    templates = [descriptor("first-r1", kernel, images[0], 1, 512),
                 descriptor("second-r1", kernel, images[1], 2, 1024)]
    for index, template in enumerate(templates):
        template["startup"] = defaults(index)
    bad_templates = [descriptor(f"unbootable-{index}-r1", bad_kernel, images[index],
                                value["resources"]["cpu_count"], value["resources"]["memory_mib"])
                     for index, value in enumerate(templates)]
    key = secrets.token_hex(32)
    url = f"http://127.0.0.1:{free_loopback_port()}"
    database = root / "db/controller.db"
    settings = {
        "namespace": namespace, "database": str(database), "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer), "firecracker": str(args.firecracker), "chroot_base": str(root / "jails"),
        "kernel": str(kernel), "rootfs": str(images[0]), "uid_base": args.uid_base,
        "vcpu_count": 1, "memory_mib": 512,
        "host_resources": {"cgroup_parent": str(args.cgroup_parent), "memory_max_bytes": 768 * MIB},
        "file_policy_digest": 1, "max_active_global": 4, "max_active_per_tenant": 4,
        "max_disk_bytes_global": 2 * disk_sum + 1024 * MIB, "max_disk_bytes_per_tenant": 2 * disk_sum,
        "cleanup_reserve_bytes": 1024 * MIB, "max_timeout_secs": 1200, "startup_timeout_secs": 120,
        # This probe owns explicit boot/initializer failures, not expiry races.
        "expiry_scan_seconds": 3600, "templates": {"templates": templates + bad_templates,
        "aliases": {"first-choice": "first-r1", "second-choice": "second-r1"},
        "compute_limits": {"max_vcpus_global": 6, "max_vcpus_per_tenant": 6,
                           "max_memory_bytes_global": 4096 * MIB, "max_memory_bytes_per_tenant": 4096 * MIB}},
    }
    settings = complete_runtime(settings, slots=PHYSICAL_SLOTS)
    artifact_bytes=sum(binding[kind]['size_bytes'] for binding in templates+bad_templates for kind in ('kernel','rootfs'))
    registry=dict(root=str(root/'registry'),max_templates_global=4,max_templates_per_tenant=4,
        max_artifact_bytes_global=artifact_bytes,max_artifact_bytes_per_tenant=artifact_bytes,validation_timeout_secs=60)
    services=RuntimeServices.prepare(args,root=root,native_pools=[settings],registered_catalogs=[settings['templates']],
        key=key,public_address=url.removeprefix('http://'),record=args.service_cgroup_record,
        job_count=VM_COUNT,registry_config=registry)
    settings=services.native_pools[0]
    config=services.processes.bindings['center']['config']
    original_config=config.read_bytes()
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    owner = options(key, url, url)
    # Large private disks need time for validation/copy before create returns.
    # This explicit probe request budget does not change production defaults.
    owner["request_timeout"] = 180
    live, evidence, cases = {}, [], []
    process = None
    completed = False

    async def check_startup(sandbox, row):
        deadline = time.monotonic() + 10
        while not sandbox.files.exists(START_PID):
            assert time.monotonic() < deadline, "template command did not publish its marker"
            await asyncio.sleep(0.05)
        assert sandbox.files.read(START_COUNT) == "once\n"
        context = bytes(sandbox.files.read(START_CONTEXT, format="bytes")).decode().split("\0")
        assert context == expected_context(row["template"]["startup"], row["instance"]) + [
            "literal space", LITERAL_ARG, ""]
        assert not sandbox.files.exists("/tmp/sandproof-template-injected")
        pid = int(sandbox.files.read(START_PID))
        stat = await run(sandbox, f"cat /proc/{pid}/stat")
        fields = stat.rsplit(")", 1)[1].split()
        assert fields[0] != "Z"
        identity = {"pid": pid, "start_ticks": int(fields[19])}
        if "startup_process" in row:
            assert row["startup_process"] == identity, "template command was replaced"
        row["startup_process"] = identity
        next(item for item in evidence if item["sandbox_id"] == sandbox.sandbox_id)["startup_process"] = identity
        record = json.loads((root / "state/startup" / (row["host"]["resource_id"] + ".json")).read_text())
        assert record["guest_pid"] == pid
        for file in row.get("default_files", []):
            assert sandbox.files.read(file["path"], user="root") == file["content"]
        return identity

    async def check_live(exclude=None):
        disks = set()
        for sandbox, row in live.items():
            if sandbox is exclude:
                continue
            current = live_shape(root,args,sandbox,row['template'],opener,url,key,
                native_id=row['mapping'][0],native_template=row['mapping'][1])
            assert current == row["host"], "survivor identity or resources changed"
            identity = (current["disk"]["device"], current["disk"]["inode"])
            assert identity not in disks, "live instances share a writable disk"
            disks.add(identity)
            if row["version"] is None:
                await clean(sandbox)
            else:
                shape = row["template"]["resources"]
                await installed(sandbox, row["version"], sandbox.sandbox_id, config=row.get("config"),
                                resources=(shape["cpu_count"], shape["memory_mib"]))
            assert not sandbox.files.exists(FAILED_INIT)
            await check_startup(sandbox, row)

    def create(template, *, alias=None):
        nonlocal process
        instance = secrets.token_hex(8)
        sandbox = Sandbox.create(template=alias or template["template_id"], timeout=900,
                                 envs={"INSTANCE_MARKER": instance, "LAYERED": "create-" + instance},
                                 allow_internet_access=False, **owner)
        # Record identity before further assertions so finally can stop this VM.
        live[sandbox] = {"template": template, "version": None, "instance": instance}
        services.processes.stop('host-0')
        mapping=mapped_template(services,sandbox.sandbox_id,template['template_id'])
        live[sandbox]['mapping']=mapping
        process=services.processes.start('host-0')
        services.wait_ready(for_admission=False)
        live[sandbox]["host"] = live_shape(root,args,sandbox,template,opener,url,key,
            native_id=mapping[0],native_template=mapping[1])
        evidence.append({"sandbox_id": sandbox.sandbox_id, "template_id": template["template_id"],
                         "host": live[sandbox]["host"]})
        print(json.dumps({"event": "created", "sandbox_id": sandbox.sandbox_id,
                          "template_id": template["template_id"], "live": len(live)}), flush=True)
        return sandbox

    try:
        services.start()
        services.wait_ready()
        process=services.processes.children['host-0']
        original = [create(template, alias=("first-choice", "second-choice")[index])
                    for index, template in enumerate(templates) for _ in range(2)]
        await check_live()
        for stage in ("before-restart", "after-immutable-alias-refusal-and-restart"):
            if stage != "before-restart":
                services.processes.stop('center')
                changed=json.loads(original_config)
                changed['templates']['aliases']={'first-choice':'second-r1','second-choice':'first-r1'}
                try:
                    config.write_text(json.dumps(changed))
                    rejected=services.processes.start('center')
                    assert rejected.wait(timeout=30)!=0,'changed static alias replaced the original authority'
                finally:
                    config.write_bytes(original_config)
                    services.processes.start('center')
                services.processes.stop('host-0')
                process=services.processes.start('host-0')
                services.wait_ready(for_admission=False)
                await check_live()
            for sandbox in original:
                row = live[sandbox]
                clients = [sandbox, await AsyncSandbox.connect(sandbox.sandbox_id, timeout=900, **owner)]
                for client in clients:
                    result = await check_defaults(client, row["template"]["startup"], row["instance"])
                    row.setdefault("default_files", []).extend(result["files"])
                    cases.append({"startup_defaults": {"stage": stage, "sandbox_id": sandbox.sandbox_id,
                                                       **result}})
                await check_live()
            print(json.dumps({"event": "startup_defaults_verified", "stage": stage}), flush=True)
        cases.append("four original VMM and startup PID/start identities survive immutable alias attach refusal and both-role restart without replay")
        for index, sandbox in enumerate(original):
            version = f"{index + 1}.0"
            await install(sandbox, version, sandbox.sandbox_id)
            live[sandbox]["version"] = version
            await check_live()
        cases.append("four concurrent VMs have independent files and dpkg versions at the same paths")
        print(json.dumps({"event": "four_package_databases_verified"}), flush=True)
        for index in (0, 2):
            victim = original[index]
            victim.files.write(CONFIG, victim.sandbox_id + "-changed\n", user="root")
            live[victim]["config"] = victim.sandbox_id + "-changed"
            await check_live()
            await run(victim, f"dpkg --purge {PACKAGE}")
            assert not victim.files.exists(CONFIG) and not victim.files.exists(EXECUTABLE)
            assert await run(victim, f"if dpkg-query -W {PACKAGE} >/dev/null 2>&1; then printf present; fi") == ""
            await check_live(exclude=victim)
            version = f"{index + 5}.0"
            await install(victim, version, victim.sandbox_id)
            live[victim].update(version=version, config=None)
            await check_live()
        cases.append("system edits and package purge/reinstall preserve both templates' other instances")
        for index in (0, 2):
            victim = original[index]
            template = live[victim]["template"]
            default_paths = {item["path"] for item in live[victim].get("default_files", [])}
            failure = await failed_initializer(victim)
            await check_live(exclude=victim)
            name = live[victim]["host"]["resource_id"]
            host_identity = live[victim]["host"]
            _kill_confirmed = (victim.kill())
            assert _kill_confirmed, "failed initializer VM destruction unconfirmed"
            del live[victim]
            failure["cleanup"] = audit_gone(root, args.firecracker, args.cgroup_parent, name, host_identity)
            failure["template_id"] = template["template_id"]
            cases.append({"guest_initializer_exit_42": failure})
            print(json.dumps({"event": "guest_failure_reclaimed", "template_id": template["template_id"]}), flush=True)
            await check_live()
            # The valid manifest describes an intentionally invalid private
            # kernel. This reaches physical preparation, unlike a hash refusal.
            before_id = idle_snapshot(database)["frontiers"][("sandbox", "")]
            marker={'invalid_kernel_intent':secrets.token_hex(16)}
            response = httpx.post(url + "/v2/sandboxes", headers={"X-API-Key": key},
                                  json={"templateID": bad_templates[index // 2]["template_id"],
                                        "timeout": 300, "allow_internet_access": False,"metadata":marker},
                                  timeout=180, trust_env=False)
            status = response.status_code
            assert status >= 500, f"invalid kernel unexpectedly accepted: {status}"
            assert idle_snapshot(database)["frontiers"][("sandbox", "")] == before_id + 1
            original_create=original_create_identity(services,marker)
            services.processes.stop('host-0')
            ident=services.native_id_for(original_create['binding']['session'])
            local=services.local_for(original_create['binding']['session'])
            assert local==before_id+1,'original accepted boot request allocated another native identity'
            process=services.processes.start('host-0')
            services.wait_ready(for_admission=False)
            slots = [json.loads(path.read_text()) for path in (root / "state/slots").glob("*.json")]
            slot = next(value for value in slots if value["sandbox"] == local)
            name = f"vm-{namespace}-{slot['vm']}"
            assert (root / "jails/.claims" / name / "failed-prepare.json").is_file()
            failed_identity = json.loads((root / "jails/.claims" / name / "record.json").read_text())
            failed_identity["network_inode"] = json.loads((root / "jails/.claims" / name / "network.json").read_text())["namespace_inode"]
            evidence.append({"sandbox_id":original_create['binding']['session'],"native_id":ident,
                             "template_id": bad_templates[index // 2]["template_id"],
                             "host": failed_identity})
            # An HTTP failure is not cleanup. Destroy this exact original global
            # request through the public authenticated path, then inspect native.
            assert httpx.delete(url+'/sandboxes/'+original_create['binding']['session'],headers={'X-API-Key':key},
                                timeout=180,trust_env=False).status_code==204
            deadline = time.monotonic() + 150
            while not terminal_cleanup_confirmed(database, ident):
                assert process.poll() is None and time.monotonic() < deadline, "failed boot cleanup unknown"
                time.sleep(0.1)
            assert idle_snapshot(database)["frontiers"][("sandbox", "")] == before_id + 1, "failed create allocated another identity"
            cases.append({"invalid_kernel_startup": {"template_id": bad_templates[index // 2]["template_id"],
                          "http_status": status, "terminal": "Destroyed",
                          "cleanup": audit_gone(root, args.firecracker, args.cgroup_parent, name, failed_identity)}})
            print(json.dumps({"event": "boot_failure_reclaimed", "template_id": template["template_id"]}), flush=True)
            await check_live()
            replacement = create(template)
            await clean(replacement)
            assert all(not replacement.files.exists(path, user="root") for path in default_paths)
            await check_live()
        cases.append("replacements inherit no package records, system edits or failed initialization state")
        for sandbox in list(live):
            _kill_confirmed = (sandbox.kill())
            assert _kill_confirmed
            del live[sandbox]
        assert len(evidence) == VM_COUNT
        assert all(terminal_cleanup_confirmed(database,item.get('native_id',item['host'].get('native_id')))
                   for item in evidence)
        with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
            assert connection.execute("SELECT count(*) FROM outbox WHERE dispatch_state='dispatched'").fetchone()[0] == 0
        assert idle_snapshot(database)["frontiers"][("sandbox", "")] == VM_COUNT
        cleanup = [audit_gone(root, args.firecracker, args.cgroup_parent,
                              item["host"]["resource_id"], item["host"]) for item in evidence]
        assert not processes_in_uid_range(args.uid_base, args.uid_base + PHYSICAL_SLOTS - 1)
        services.stop_all()
        assert process.poll() is not None
        assert not any(path.is_dir() for path in args.cgroup_parent.iterdir())
        assert shared_network() == before_network
        for name, fingerprint in inputs.items():
            assert sha256_file(getattr(args, name)) == fingerprint["sha256"]
        for template in templates:
            assert sha256_file(Path(template["rootfs"]["path"])) == template["rootfs"]["sha256"]
        assert sha256_file(kernel) == inputs["kernel"]["sha256"]
        assert subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip() == commit
        assert not subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"])
        report = {"source_commit": commit, "probe_sha256": sha256_file(Path(__file__)),
                  "defaults_probe_sha256": sha256_file(Path(__file__).with_name("sdk_template_defaults.py")),
                  "private_root_probe_sha256": sha256_file(Path(__file__).with_name("sdk_private_root.py")),
                  "host_kernel": os.uname().release, "sdk": "2.51.0", "namespace": namespace,
                  "inputs": inputs, "templates": templates, "observations": evidence, "cases": cases,
                  "vms_destroyed": VM_COUNT, "cleanup": cleanup, "shared_network_unchanged": True,
                  "probe_timeouts_seconds": {"request": 180, "startup": 120, "sandbox": 900},
                  "service_exited": True, "result": "passed",
                  "scope": "trusted prebuilt templates; four concurrent VMs, sync/async startup defaults and file paths, "
                           "immutable static-alias refusal plus both-role restart without command replay, local packages, guest-init and boot failures; "
                           "no self-service build, arbitrary crash, cloud/model, full V1 or proof claim"}
        (root / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report), flush=True)
        completed = True
    finally:
        # Only stop handles returned by this run. Unknown create outcomes keep
        # their original private journal for recovery; never guess a new VM.
        for sandbox in list(live):
            try:
                if sandbox.kill():
                    del live[sandbox]
            except Exception as error:
                print(f"cleanup unconfirmed for {sandbox.sandbox_id}: {type(error).__name__}", file=sys.stderr)
        services.stop_all()
        if completed and not args.keep_artifacts:
            services.cleanup_staging()
            shutil.rmtree(root)
        else:
            print(f"Exact {'completed' if completed else 'failed'} run retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    for argument in ("jailer", "firecracker", "kernel", "rootfs", "second-rootfs", "scratch-parent", "cgroup-parent"):
        parser.add_argument(f"--{argument}", type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    parser.add_argument("--keep-artifacts", action="store_true")
    asyncio.run(exercise(parser.parse_args()))
