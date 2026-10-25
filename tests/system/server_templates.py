"""Trusted multi-template SDK/KVM acceptance, without a builder or paid model.

Use an isolated host, two approved prebuilt images, an empty delegated cgroup
parent and an exclusive UID range. Failure retains exact private artifacts.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import urllib.request

from e2b import Sandbox, SandboxQuery

from sdk_server_smoke import options
from server_process_crash import (
    assert_idle_reopen, complete_runtime, fetch, free_loopback_port, idle_snapshot, process_start_ticks,
    original_create_identity, registered_cache_template, terminal_cleanup_confirmed,
)
from _runtime_services import RuntimeServices, add_arguments
from _center_host_services import local_binding
from tools.capture_acceptance import original_chroot


MIB = 1024 * 1024


def assert_refusal_unchanged(before, after):
    assert_idle_reopen(before, after)
    assert after['states'] == before['states'] and after['actions'] == before['actions'], (
        'pre-admission refusal changed durable sandbox state or work')


def descriptor(ident, kernel, rootfs, cpu, memory):
    from sdk_c02_acceptance import sha256_file
    return {
        "template_id": ident, "revision": "r1", "owner": 1,
        "kernel": {"path": str(kernel), "sha256": sha256_file(kernel), "size_bytes": kernel.stat().st_size},
        "rootfs": {"path": str(rootfs), "sha256": sha256_file(rootfs), "size_bytes": rootfs.stat().st_size},
        "resources": {"cpu_count": cpu, "memory_mib": memory,
                      "disk_bytes": rootfs.stat().st_size, "host_memory_bytes": (memory + 256) * MIB},
        "envd_version": "0.9.0",
    }


def record_for(root, *, native_id):
    """Read one native instance selected by the original host inbox audit.

    A global SDK id is never a native id and must not be parsed as one.
    Callers supply the exact mapping after stopping that original host.
    """
    import re
    matched = re.fullmatch(r"sb-([0-9a-f]{16})-([1-9][0-9]*)", native_id)
    if matched is None or int(matched[2]) >= 2**64:
        raise ValueError("exact native identity from host audit required")
    secret = json.loads((root / "state/secrets" / native_id).read_text())
    slots = [json.loads(path.read_text()) for path in (root / "state/slots").glob("*.json")]
    matches = [value for value in slots if value["sandbox"] == int(matched[2])]
    if len(matches) != 1:
        raise ValueError("original native physical binding is missing or ambiguous")
    slot = matches[0]
    name = f"vm-{matched[1]}-{slot['vm']}"
    claim = json.loads((original_chroot(root) / '.claims' / name / "record.json").read_text())
    return secret, name, claim


def assert_shape(root, firecracker, cgroup_parent, sandbox, expected, opener, url, key, *, native_id,
                 native_template):
    info = sandbox.get_info()
    shape = expected["resources"]
    assert (info.template_id, info.cpu_count, info.memory_mb) == (
        expected["template_id"], shape["cpu_count"], shape["memory_mib"],
    )
    status, body = fetch(opener, url, "GET", f"/sandboxes/{sandbox.sandbox_id}", key)
    assert status == 200
    # The pinned SDK SandboxInfo intentionally omits diskSizeMB; check the
    # actual management response instead of inventing a Python SDK attribute.
    assert json.loads(body)["diskSizeMB"] == shape["disk_bytes"] // MIB
    secret, name, claim = record_for(root, native_id=native_id)
    assert secret["template"] == native_template
    assert process_start_ticks(claim["pid"]) == claim["start_time_ticks"]
    assert (original_chroot(root) / firecracker.name / name / "root/rootfs.ext4").stat().st_size == shape["disk_bytes"]
    cgroup = cgroup_parent / name
    assert int((cgroup / "memory.max").read_text()) == shape["host_memory_bytes"]
    assert (cgroup / "cpu.max").read_text().strip() == f"{shape['cpu_count'] * 100000} 100000"
    assert str(claim["pid"]) in (cgroup / "cgroup.procs").read_text().split()
    assert int(sandbox.commands.run("getconf _NPROCESSORS_ONLN").stdout.strip()) == shape["cpu_count"]
    return claim


def mapped_template(services, global_id, template_id):
    """Offline original mapping plus one exact registered cache observation."""
    process = services.processes.children.get('host-0')
    if process is not None and process.poll() is None:
        raise RuntimeError('stop original host before mapping its local immutable descriptor')
    local = local_binding(services.processes.audit('host-0'),global_id,
                          expected_node=services.host_configs[0]['inbox']['node'])
    descriptor = json.loads(json.dumps(registered_cache_template(services,template_id)))
    if descriptor['owner'] is not None:
        descriptor['owner'] = local['owner']
    native_id = 'sb-'+services.native_pools[0]['namespace']+'-'+str(local['sandbox'])
    return native_id,descriptor


def exercise(args):
    from sdk_c02_acceptance import processes_in_uid_range, sha256_file
    from server_vm_id_capacity import check_no_resources
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("requires root on an isolated Linux/KVM host")
    assert importlib.metadata.version("e2b") == "2.51.0"
    slots = 5  # Keep the original active quota; compute admission limits concurrency to two.
    if processes_in_uid_range(args.uid_base, args.uid_base + slots - 1):
        raise RuntimeError("exclusive test UID range occupied")
    if args.uid_base < 1000 or args.uid_base + slots - 1 >= 2**32 - 1:
        raise RuntimeError("invalid exclusive test UID range")
    parent = args.scratch_parent.resolve(strict=True)
    if parent.stat().st_uid != 0 or parent.stat().st_mode & 0o077:
        raise RuntimeError("scratch parent must be private/root-owned")
    if (args.cgroup_parent / "cgroup.procs").read_text().strip() or any(path.is_dir() for path in args.cgroup_parent.iterdir()):
        raise RuntimeError("cgroup parent must be empty")
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"]).strip():
        raise RuntimeError("test checkout must be clean")
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    original_digests = [sha256_file(path) for path in (args.kernel, args.rootfs, args.second_rootfs)]
    disk = args.rootfs.stat().st_size + args.second_rootfs.stat().st_size
    if shutil.disk_usage(parent).free < disk * 3 + 256 * MIB:
        raise RuntimeError("insufficient disk including cleanup reserve")
    root = Path(tempfile.mkdtemp(prefix="tm-", dir=parent))
    namespace = secrets.token_hex(8)
    firecracker = args.firecracker.resolve(strict=True)
    if len(os.fsencode(root / "p/v" / firecracker.name / f"vm-{namespace}-2/root/api.socket")) >= 108:
        root.rmdir()
        raise RuntimeError("Firecracker socket path too long")
    for name in ("db", "state", "p", "images"):
        (root / name).mkdir(mode=0o700)
    kernel, first_image, second_image = (root / "images" / name for name in ("vmlinux", "small.ext4", "large.ext4"))
    for source, destination in ((args.kernel, kernel), (args.rootfs, first_image), (args.second_rootfs, second_image)):
        subprocess.run(["/usr/bin/cp", "--reflink=auto", "--sparse=always", str(source), str(destination)], check=True)
        destination.chmod(0o444)
    templates = [descriptor("small-r1", kernel, first_image, 1, 256), descriptor("large-r1", kernel, second_image, 2, 512)]
    key, foreign_key = secrets.token_hex(32), secrets.token_hex(32)
    url = f"http://127.0.0.1:{free_loopback_port()}"
    database = root / "db/controller.db"
    settings = {
        "namespace": namespace, "database": str(database), "node_state_dir": str(root / "state"),
        "jailer": str(args.jailer.resolve(strict=True)), "firecracker": str(firecracker),
        "chroot_base": str(root / "p/v"), "kernel": str(args.kernel.resolve(strict=True)),
        "rootfs": str(args.rootfs.resolve(strict=True)), "uid_base": args.uid_base,
        "vcpu_count": 1, "memory_mib": 256,
        "host_resources": {"cgroup_parent": str(args.cgroup_parent), "memory_max_bytes": 512 * MIB},
        "file_policy_digest": 1,
        "max_active_global": 5, "max_active_per_tenant": 5,
        "max_disk_bytes_global": disk * 4, "max_disk_bytes_per_tenant": disk * 4,
        "cleanup_reserve_bytes": 256 * MIB, "max_timeout_secs": 600, "expiry_scan_seconds": 3600,
        "templates": {"templates": templates, "aliases": {"chosen": "small-r1"},
                      "compute_limits": {"max_vcpus_global": 3, "max_vcpus_per_tenant": 3,
                                         "max_memory_bytes_global": 1280 * MIB, "max_memory_bytes_per_tenant": 1280 * MIB}},
    }
    settings = complete_runtime(settings, slots=slots, inbox_sessions=5+settings["max_active_global"])
    artifact_bytes = sum(binding[kind]['size_bytes'] for binding in templates for kind in ('kernel','rootfs'))
    registry = dict(root=str(root/'registry'),max_templates_global=2,max_templates_per_tenant=2,
        max_artifact_bytes_global=artifact_bytes,max_artifact_bytes_per_tenant=artifact_bytes,validation_timeout_secs=60)
    services = RuntimeServices.prepare(args,root=root,native_pools=[settings],registered_catalogs=[settings['templates']],
        key=key,public_address=url.removeprefix('http://'),record=args.service_cgroup_record,
        retained_sessions=5 + settings['max_active_global'],registry_config=registry,extra_credentials=[{'tenant':2,'secret':foreign_key}])
    settings = services.native_pools[0]
    config = services.processes.bindings['center']['config']
    original_config = config.read_bytes()
    process = None

    def reject_changed_catalog(change):
        # Static deployment registration is immutable. Legitimate alias updates
        # use the original public publication path tested separately.
        services.processes.stop('center')
        changed = json.loads(original_config)
        change(changed['templates'])
        try:
            config.write_text(json.dumps(changed))
            rejected = services.processes.start('center')
            assert rejected.wait(timeout=30)!=0, 'changed static catalog silently replaced authority'
        finally:
            config.write_bytes(original_config)
            services.processes.start('center')
            services.wait_ready(for_admission=False)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    owner = options(key, url, url)
    process = None
    completed = False
    try:
        services.start()
        services.wait_ready()
        process = services.processes.children['host-0']
        for token, name, expected in ((foreign_key, "chosen", 404), (key, "missing", 404)):
            before = idle_snapshot(database)
            assert fetch(opener, url, "POST", "/v2/sandboxes", token, {"templateID": name, "allow_internet_access": False})[0] == expected
            assert_refusal_unchanged(before, idle_snapshot(database))
        small = Sandbox.create(template="chosen", timeout=300, allow_internet_access=False, **owner)
        large = Sandbox.create(template="large-r1", timeout=300, allow_internet_access=False, **owner)
        services.processes.stop('host-0')
        mappings = [mapped_template(services,sandbox.sandbox_id,template['template_id'])
                    for sandbox,template in zip((small,large),templates,strict=True)]
        process = services.processes.start('host-0')
        services.wait_ready(for_admission=False)
        original_claims = [assert_shape(root,firecracker,args.cgroup_parent,sandbox,template,opener,url,key,
            native_id=mapping[0],native_template=mapping[1])
            for sandbox,template,mapping in zip((small,large),templates,mappings,strict=True)]
        for sandbox, marker in ((small, "small-private"), (large, "large-private")):
            sandbox.files.write("/tmp/template-marker", marker)
        assert small.files.read("/tmp/template-marker") == "small-private"
        assert large.files.read("/tmp/template-marker") == "large-private"
        listing = Sandbox.list(query=SandboxQuery(template="large-r1"), **owner).next_items()
        assert [item.sandbox_id for item in listing] == [large.sandbox_id]
        before = idle_snapshot(database)
        assert fetch(opener, url, "POST", "/v2/sandboxes", key, {"templateID": "chosen", "allow_internet_access": False})[0] == 429
        assert_refusal_unchanged(before, idle_snapshot(database))
        reject_changed_catalog(lambda catalog:catalog['aliases'].update(chosen='large-r1'))
        services.processes.stop('host-0')
        process = services.processes.start('host-0')
        services.wait_ready(for_admission=False)
        assert [assert_shape(root,firecracker,args.cgroup_parent,sandbox,template,opener,url,key,
            native_id=mapping[0],native_template=mapping[1])
            for sandbox,template,mapping in zip((small,large),templates,mappings,strict=True)] == original_claims
        assert small.files.read("/tmp/template-marker") == "small-private"
        assert Sandbox.connect(small.sandbox_id, **owner).get_info().template_id == "small-r1"
        # Validate the original artifact in this new service process, then
        # refuse on its persisted compute charge. This also warms the cache
        # whose invalidation is checked by the following mutation.
        before = idle_snapshot(database)
        assert fetch(opener, url, "POST", "/v2/sandboxes", key, {"templateID": "small-r1", "allow_internet_access": False})[0] == 429
        assert_refusal_unchanged(before, idle_snapshot(database))
        _kill_confirmed = (Sandbox.kill(large.sandbox_id, **owner))
        assert _kill_confirmed
        # Changing an artifact after its first cached validation must invalidate
        # that cache and fail before admission, without affecting the live VM.
        cached_rootfs = Path(mappings[0][1]['rootfs']['path'])
        with cached_rootfs.open("r+b") as image:
            original_byte = image.read(1)
            image.seek(0)
            image.write(bytes([original_byte[0] ^ 1]))
            image.flush()
            os.fsync(image.fileno())
        before = idle_snapshot(database)
        marker = {'damaged_cache_intent':secrets.token_hex(16)}
        assert fetch(opener,url,'POST','/v2/sandboxes',key,
            {'templateID':'small-r1','allow_internet_access':False,'metadata':marker})[0]>=500
        assert_refusal_unchanged(before, idle_snapshot(database))
        original = original_create_identity(services,marker)
        assert fetch(opener,url,'DELETE','/sandboxes/'+original['binding']['session'],key)[0]==204
        with cached_rootfs.open("r+b") as image:
            image.write(original_byte)
            image.flush()
            os.fsync(image.fileno())
        # The original small image is no longer available. It must not prevent
        # restart/observation/stop of its already-running private VM.
        registered = next(binding for binding in services.center_config['templates']['templates']
                          if binding['template_id']=='small-r1')
        origin = Path(registered['rootfs']['path'])
        original_inode = (origin.stat().st_dev,origin.stat().st_ino)
        held = origin.with_name(origin.name+'-held')
        origin.rename(held)
        try:
            services.processes.stop('host-0')
            process = services.processes.start('host-0')
            # A confirmed renewal goes through the original host. Do not invoke
            # wait_ready's planned center restart while its source is absent.
            resumed = Sandbox.connect(small.sandbox_id,**owner)
            assert resumed.commands.run("printf old-binding-ok").stdout == "old-binding-ok"
            _kill_confirmed = (Sandbox.kill(small.sandbox_id, **owner))
            assert _kill_confirmed
        finally:
            held.rename(origin)
            assert (origin.stat().st_dev,origin.stat().st_ino)==original_inode
        # Same immutable ID with a different shape must fail without admitting
        # a third VM. Alias retargeting to a new ID is allowed.
        def altered_shape(catalog):
            next(binding for binding in catalog['templates'] if binding['template_id']=='large-r1')['resources']['cpu_count']=1
        before = idle_snapshot(database)
        reject_changed_catalog(altered_shape)
        assert_refusal_unchanged(before,idle_snapshot(database))
        replacement = Sandbox.create(template="large-r1", timeout=300, allow_internet_access=False, **owner)
        services.processes.stop('host-0')
        replacement_mapping = mapped_template(services,replacement.sandbox_id,'large-r1')
        process = services.processes.start('host-0')
        services.wait_ready(for_admission=False)
        original_claims.append(assert_shape(root, firecracker, args.cgroup_parent,
            replacement,templates[1],opener,url,key,native_id=replacement_mapping[0],native_template=replacement_mapping[1]))
        assert not replacement.files.exists("/tmp/template-marker")
        _kill_confirmed = (Sandbox.kill(replacement.sandbox_id, **owner))
        assert _kill_confirmed
        check_no_resources(root,namespace,firecracker,args.uid_base,original_claims,native=settings)
        assert all(terminal_cleanup_confirmed(database,mapping[0]) for mapping in mappings+[replacement_mapping])
        # Complete maintenance may retire SQL/options rows. Permanent identity
        # frontiers, not legacy schema versions or tombstone counts, show that
        # the three original creates allocated no replacement identities.
        assert idle_snapshot(database)["frontiers"][("sandbox", "")] == 3
        assert original_digests == [sha256_file(path) for path in (args.kernel, args.rootfs, args.second_rootfs)]
        assert sha256_file(first_image) == templates[0]["rootfs"]["sha256"]
        assert sha256_file(second_image) == templates[1]["rootfs"]["sha256"]
        completed = True
        print(json.dumps({"source_commit": commit, "host_sha256": sha256_file(args.host),
                          "center_sha256":sha256_file(args.center),
                          "probe_sha256": sha256_file(Path(__file__)), "host_kernel": os.uname().release,
                          "sdk": "2.51.0", "templates": templates, "vms_destroyed": 3,
                          "result": "shapes/private files/owner/quota/immutable static attach refusals/source loss/precise cleanup passed",
                          "scope": "trusted prebuilt inputs only; no Dockerfile/OCI builder, self-service registry, public networking or proofs"}))
    finally:
        services.stop_all()
        if completed:
            services.cleanup_staging()
            shutil.rmtree(root)
        else:
            print(f"Exact failed-run artifacts retained: {root}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    for argument in ("jailer", "firecracker", "kernel", "rootfs", "second-rootfs", "scratch-parent", "cgroup-parent"):
        parser.add_argument(f"--{argument}", type=Path, required=True)
    parser.add_argument("--uid-base", type=int, required=True)
    exercise(parser.parse_args())
