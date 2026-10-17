"""Fresh complete runtime/registry and private-builder deployments for KVM probes.

The operator supplies the unchanged two-pool physical policy. These probes may
drive the trusted guest worker directly, but its VM still uses the production
private request-window, fence and settlement protocol. No public create is used
as a substitute for a private builder, and no profile is activated by Python.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

from dataclasses import dataclass
import copy
import json
import http.client
import os
from pathlib import Path
import secrets
import shutil
import sqlite3
import sys
import tempfile
import time
import urllib.parse
import urllib.request
import urllib.error

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from tools.bounded_acceptance import (
    acceptance_pool_disk_budget, bounded_acceptance_layout, provision_acceptance_coordination,
    require_bounded_host_tools, validate_acceptance_socket_paths,
)
from tools.template_build_config import acceptance_build_policy, apply_acceptance_build_policy
from tools.template_build_service import Backend, configuration
from tools.template_stage import file_hash
from _runtime_services import RuntimeServices, add_arguments as add_service_arguments
from server_process_crash import fetch

MIB = 1024 * 1024


def sandbox_highwater(database):
    """Permanent allocation evidence survives bounded retirement of SQL rows."""
    from contextlib import closing
    with closing(sqlite3.connect(Path(database).as_uri() + "?mode=ro", uri=True)) as connection:
        row = connection.execute("SELECT high_water FROM controller_id_highwaters WHERE kind='sandbox' AND scope='' ").fetchone()
        if row is None:
            raise ValueError("permanent sandbox allocation frontier is missing")
        return int(row[0])


def assert_terminal_rows(database):
    """Require every still-retained sandbox projection to be Destroyed."""
    from contextlib import closing
    with closing(sqlite3.connect(Path(database).as_uri() + "?mode=ro", uri=True)) as connection:
        phases = [json.loads(row[0])["phase"] for row in connection.execute("SELECT state_blob FROM sandboxes")]
    assert all(phase == "Destroyed" for phase in phases), phases


def _save(path, value, *, fresh=False):
    path = Path(path)
    descriptor, temporary = tempfile.mkstemp(prefix=path.name + ".", dir=path.parent)
    with os.fdopen(descriptor, "w") as output:
        json.dump(value, output)
        output.flush()
        os.fsync(output.fileno())
    if fresh:
        os.link(temporary, path)  # Atomic no-overwrite publication of this intent.
        os.unlink(temporary)
    else:
        os.replace(temporary, path)
    directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def _descriptor(name, kernel, rootfs, cpu, memory):
    return {"template_id": name, "revision": "r1", "owner": 1,
        "kernel": {"path": str(kernel), "sha256": file_hash(kernel), "size_bytes": kernel.stat().st_size},
        "rootfs": {"path": str(rootfs), "sha256": file_hash(rootfs), "size_bytes": rootfs.stat().st_size},
        "resources": {"cpu_count": cpu, "memory_mib": memory,
            "disk_bytes": rootfs.stat().st_size, "host_memory_bytes": (memory + 256) * MIB},
        "envd_version": "0.9.0"}


@dataclass
class RegistryDeployment:
    root: Path
    namespaces: list
    settings: list
    key: str
    foreign_key: str
    services: RuntimeServices
    build_config: Path

    def wait_center_listener(self):
        """Observe read-only readiness, without requiring a faulted host alive."""
        child = self.services.processes.children['center']
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if child.poll() is not None:
                raise RuntimeError('original center exited after audit restart')
            try:
                if fetch(opener, self.services.sdk_url, 'GET', '/v2/sandboxes', self.key)[0] == 200:
                    return
            except (OSError, http.client.HTTPException):
                pass
            time.sleep(.1)
        raise RuntimeError('original center listener remained unobserved after audit restart')

    def center_snapshot(self):
        """Read the ORIGINAL allowlisted audit under its exact writer lease."""
        processes = self.services.processes
        child = processes.children.get('center')
        running = child is not None and child.poll() is None
        processes.stop('center')
        try:
            return processes.audit('center')
        finally:
            if running:
                processes.start('center')
                self.wait_center_listener()

    def native_id(self, sandbox, *, index=0):
        processes = self.services.processes
        role = 'host-' + str(index)
        child = processes.children.get(role)
        running = child is not None and child.poll() is None
        processes.stop(role)
        try:
            return self.services.native_id_for(sandbox, index=index)
        finally:
            if running:
                processes.start(role)

    def validation_snapshot(self, candidate):
        values = self.center_snapshot()
        matches = [record for record in values['sandboxes'] if record['private'] is True
                   and record['create']['template_revision'] == 'candidate-' + candidate + '-r1']
        if len(matches) != 1:
            raise ValueError('exact original private validation identity is unknown or ambiguous')
        return matches[0]

    def open_builder(self, *, timeout=600, internet=False):
        """Persist the original private identity before its only VM POST."""
        from e2b import Sandbox
        from e2b.connection_config import ConnectionConfig
        from packaging.version import Version
        backend = Backend(configuration(self.build_config))
        opened = backend.request("builder", "POST", "/sandproof/internal-build-windows", {}, owner=1)
        window = opened["requestWindow"]
        identity = secrets.token_hex(16)
        intent = {"id": identity, "requestWindow": window, "sandboxID": None, "settled": False}
        _save(self.root / ("builder-intent-" + identity + ".json"), intent, fresh=True)
        vm = backend.request("builder", "POST", "/sandproof/internal-build-vms/" + identity,
            {"timeout": timeout, "allowInternetAccess": internet, "requestWindow": window}, owner=1, timeout=timeout)
        session = vm.get("session")
        if session is None:
            raise RuntimeError("original private builder session is unavailable")
        intent["sandboxID"] = session["sandboxID"]
        _save(self.root / ("builder-intent-" + identity + ".json"), intent)
        url, center = 'http://' + backend.config['center']['private_address'], backend.config['center']
        connection = ConnectionConfig(api_url=url, sandbox_url=url, domain=center["domain"],
            api_key=backend.backend_key, retries=0, request_timeout=30,
            extra_sandbox_headers={"X-Access-Token": session["envdAccessToken"],
                "x-sandproof-build-key": backend.backend_key, "x-sandproof-build-owner": "1"})
        sandbox = Sandbox(sandbox_id=session["sandboxID"], sandbox_domain=center["domain"],
            envd_version=Version(session["envdVersion"]), envd_access_token=session["envdAccessToken"],
            connection_config=connection)
        return sandbox, identity

    def close_builder(self, identity):
        """Fence, reclaim and settle that same durable private request."""
        path = self.root / ("builder-intent-" + identity + ".json")
        intent = json.loads(path.read_text())
        if intent["id"] != identity:
            raise ValueError("private builder identity mismatch")
        if intent["settled"]:
            return
        backend = Backend(configuration(self.build_config))
        window = intent["requestWindow"]
        settled = intent.get("builder_settled")
        if settled is None:
            fenced = backend.request("builder", "POST", "/sandproof/internal-build-windows/"
                + urllib.parse.quote(window, safe="") + "/close", {}, owner=1)
            assert fenced == {"requestWindow": window, "closed": True}
            query = "/sandproof/internal-build-vms/" + identity + "?" + urllib.parse.urlencode({"requestWindow": window})
            from tools.template_build_store import BuildError
            try:
                vm = backend.request("builder", "GET", query, owner=1)
            except BuildError as error:
                if error.code != 404:
                    raise
                vm = None  # Only the committed fence permits this absence conclusion.
            if vm is not None and not vm["cleanupComplete"]:
                backend.request("builder", "DELETE", "/sandboxes/" + vm["sandboxID"], owner=1)
                deadline = time.monotonic() + 60
                while not vm["cleanupComplete"]:
                    assert time.monotonic() < deadline, "original builder cleanup remains unknown"
                    time.sleep(0.1)
                    vm = backend.request("builder", "GET", query, owner=1)
            settled = {"requestWindow": window, "sandboxID": vm["sandboxID"] if vm else None}
            intent["builder_settled"] = settled
            _save(path, intent)
        acknowledged = backend.request("builder", "POST", "/sandproof/internal-build-vms/" + identity + "/settle", settled, owner=1)
        assert acknowledged == {"requestWindow": window, "settled": True}
        intent["settled"] = True
        _save(path, intent)

    def close_builders(self):
        for path in sorted(self.root.glob("builder-intent-*.json")):
            self.close_builder(json.loads(path.read_text())["id"])


def prepare_registry_runtime(args, *, tool_hashes, registry_limits, build_limits,
                             runtime_shape=(1, 256), builder_shape=(2, 2048), ports,
                             runtime_transfers_per_vm=4, data_transfer=None,
                             activation_ceiling_bytes=None, rust_guest_profile=None):
    """Create fresh DTOs only from an explicit complete operator policy."""
    # Explicit test allocation inside the unchanged eight-transfer host bound.
    # Keep one control slot; never infer extra capacity from a failed request.
    if type(runtime_transfers_per_vm) is not int or not 2 <= runtime_transfers_per_vm <= 8:
        raise ValueError("runtime per-VM transfers must fit the original global bound and control reserve")
    if data_transfer is not None and runtime_transfers_per_vm != 4:
        raise ValueError("choose one explicit transfer allocation, not overlapping profiles")
    policy = acceptance_build_policy(args.bounded_policy)
    require_bounded_host_tools()
    root, namespaces = bounded_acceptance_layout(policy, run_root=args.run_root,
        namespace=args.namespace, build_namespace=args.build_namespace)
    if root.parent != args.scratch_parent.resolve(strict=True):
        raise ValueError("complete registry root must be directly inside approved scratch parent")
    firecracker = args.firecracker.resolve(strict=True)
    validate_acceptance_socket_paths(root, namespaces, firecracker)
    if len(ports) != 3 or len(set(ports)) != 3:
        raise ValueError("three distinct loopback ports required")
    root.mkdir(mode=0o700)
    provision_acceptance_coordination(policy, run_root=root, namespace=namespaces[0], build_namespace=namespaces[1])
    for name in ("images", "inputs", "builds"):
        (root / name).mkdir(mode=0o700)
    key, foreign_key = secrets.token_hex(32), secrets.token_hex(32)
    settings = []
    builder_rootfs = getattr(args, "builder_rootfs", None) or args.rootfs
    for index, shape in enumerate((runtime_shape, builder_shape)):
        pool = root / str(index)
        pool.mkdir(mode=0o700)
        for name in ("db", "state", "jails"):
            (pool / name).mkdir(mode=0o700)
        cpu, memory = shape
        rootfs = builder_rootfs if index else args.rootfs
        base = _descriptor("builder-base" if index else "runtime-base", args.kernel, rootfs, cpu, memory)
        base['owner'] = None
        config = {"namespace": namespaces[index], "database": str(pool / "db/controller.db"),
            "node_state_dir": str(pool / "state"), "jailer": str(args.jailer), "firecracker": str(firecracker),
            "chroot_base": str(pool / "jails"), "kernel": str(args.kernel), "rootfs": str(rootfs),
            "uid_base": args.uid_base + index * 100, "vcpu_count": cpu, "memory_mib": memory,
            "host_resources": {"cgroup_parent": str(args.build_cgroup_parent if index else args.cgroup_parent),
                               "memory_max_bytes": (memory + 256) * MIB},
            "file_policy_digest": 1, "max_active_global": 2,
            "max_active_per_tenant": 2, "max_timeout_secs": 1300, "startup_timeout_secs": 40,
            "max_lifetime_secs": 1300, "expiry_scan_seconds": 2, "public_ipv4_enabled": bool(index),
            **acceptance_pool_disk_budget(policy, "builder" if index else "runtime", rootfs.stat().st_size),
            "templates": {"templates": [base], "aliases": {} if index else {"base": base["template_id"]},
                "compute_limits": {"max_vcpus_global": 4, "max_vcpus_per_tenant": 4,
                    "max_memory_bytes_global": 4608 * MIB, "max_memory_bytes_per_tenant": 4608 * MIB}},
            "data_transfer": {"file_upload_bytes": 512 * MIB, "file_download_bytes": 2048 * MIB,
                "command_input_bytes": MIB, "rpc_request_bytes": MIB,
                "rpc_response_bytes": MIB, "max_transfers": 8,
                "max_transfers_per_vm": 4 if index else runtime_transfers_per_vm, "control_reserve": 1,
                "request_idle_seconds": 60, "file_idle_seconds": 60, "write_idle_seconds": 60}}
        settings.append(config)
        if data_transfer is not None:
            config['data_transfer'] = copy.deepcopy(data_transfer)
    declared = policy['runtime']['bounded_host']['physical_storage']['public_build']
    build = {"root": str(root / "builds"), "registry": {"root": declared['registry_root'], **copy.deepcopy(registry_limits)},
        "registry_owner_uid": declared['registry_owner_uid'],
        "tools": {name: {"path": str((args.tools / name).resolve(strict=True)), "sha256": digest}
                  for name, digest in tool_hashes.items()},
        "dns_servers": ["1.1.1.1", "8.8.8.8"], "limits": copy.deepcopy(build_limits)}
    if rust_guest_profile is not None:
        build['rust_guest_profile'] = str(rust_guest_profile)
    apply_acceptance_build_policy(settings[0], settings[1], build, policy)
    windows = policy['builder']['build_retry_windows']
    services = RuntimeServices.prepare(args, root, settings, [pool['templates'] for pool in settings], key,
        '127.0.0.1:' + str(ports[0]), args.service_cgroup_record, job_count=build_limits['max_jobs'] + 16,
        private_builds=dict(windows=settings[1]['bounded_host']['maintenance']['windows'],
            window_duration_ms=windows['window_duration_ms'], max_anchors=windows['max_anchors'], builder_revision='builder-base-r1',
            policy={name:settings[1][name] for name in ('public_ipv4_enabled','max_timeout_secs',
                                                      'max_lifetime_secs','startup_timeout_secs')}),
        registry_config=build['registry'], extra_credentials=[{'tenant':2,'secret':foreign_key}],
        activation_ceiling_bytes=activation_ceiling_bytes)
    settings = services.native_pools
    build.update(bind=services.layout['helper_backend_address'],
        runtime_config=str(services.processes.bindings['host-0']['config']),
        builder_config=str(services.processes.bindings['host-1']['config']),
        registry=services.center_config['template_registry'],registry_owner_uid=args.center_uid,
        builder_template=next(item for item in services.center_config['templates']['templates']
                              if item['template_id'] == 'builder-base'),
        center=dict(private_address=services.layout['private_address'],domain=services.center_config['domain'],
            credential_file=str(services.layout['build_key_file']),
            private_windows={k:windows[k] for k in ('window_duration_ms','max_anchors')},
            max_timeout_secs=services.center_config['private_builds']['policy']['max_timeout_secs'],
            max_lifetime_secs=services.center_config['private_builds']['policy']['max_lifetime_secs']))
    build_config = root / "build.json"
    _save(build_config, build, fresh=True)
    configuration(build_config, require_physical_lease=False)
    return RegistryDeployment(root, namespaces, settings, key, foreign_key, services, build_config)


def add_registry_arguments(parser):
    add_service_arguments(parser)
    for name in ("bounded-policy", "run-root", "build-cgroup-parent"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("namespace", "build-namespace"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--builder-rootfs", type=Path, help="fixed builder base; defaults to --rootfs")
