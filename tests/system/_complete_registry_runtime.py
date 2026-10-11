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
import os
from pathlib import Path
import secrets
import shutil
import sqlite3
import sys
import tempfile
import time
import urllib.parse

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from tools.bounded_acceptance import (
    acceptance_pool_disk_budget, bounded_acceptance_layout, provision_acceptance_coordination,
    require_bounded_host_tools, validate_acceptance_socket_paths,
)
from tools.template_build_config import acceptance_build_policy, apply_acceptance_build_policy
from tools.template_build_service import Backend, configuration
from tools.template_stage import file_hash

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
    urls: list
    keys: list
    binary: Path
    build_config: Path

    def open_builder(self, *, timeout=600, internet=False):
        """Persist the original private identity before its only VM POST."""
        from e2b import Sandbox
        from e2b.connection_config import ConnectionConfig
        from packaging.version import Version
        backend = Backend(configuration(self.build_config))
        opened = backend.request("builder", "POST", "/sandproof/internal-build-windows", {})
        window = opened["requestWindow"]
        identity = secrets.token_hex(16)
        intent = {"id": identity, "requestWindow": window, "sandboxID": None, "settled": False}
        _save(self.root / ("builder-intent-" + identity + ".json"), intent, fresh=True)
        vm = backend.request("builder", "POST", "/sandproof/internal-build-vms/" + identity,
            {"timeout": timeout, "allowInternetAccess": internet, "requestWindow": window}, timeout=timeout)
        session = vm.get("session")
        if session is None:
            raise RuntimeError("original private builder session is unavailable")
        intent["sandboxID"] = session["sandboxID"]
        _save(self.root / ("builder-intent-" + identity + ".json"), intent)
        settings, url = self.settings[1], self.urls[1]
        connection = ConnectionConfig(api_url=url, sandbox_url=url, domain=settings["domain"],
            api_key=self.keys[1], retries=0, request_timeout=30,
            extra_sandbox_headers={"X-Access-Token": session["envdAccessToken"]})
        sandbox = Sandbox(sandbox_id=session["sandboxID"], sandbox_domain=settings["domain"],
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
                + urllib.parse.quote(window, safe="") + "/close", {})
            assert fenced == {"requestWindow": window, "closed": True}
            query = "/sandproof/internal-build-vms/" + identity + "?" + urllib.parse.urlencode({"requestWindow": window})
            from tools.template_build_store import BuildError
            try:
                vm = backend.request("builder", "GET", query)
            except BuildError as error:
                if error.code != 404:
                    raise
                vm = None  # Only the committed fence permits this absence conclusion.
            if vm is not None and not vm["cleanupComplete"]:
                backend.request("builder", "DELETE", "/sandboxes/" + vm["sandboxID"])
                deadline = time.monotonic() + 60
                while not vm["cleanupComplete"]:
                    assert time.monotonic() < deadline, "original builder cleanup remains unknown"
                    time.sleep(0.1)
                    vm = backend.request("builder", "GET", query)
            settled = {"requestWindow": window, "sandboxID": vm["sandboxID"] if vm else None}
            intent["builder_settled"] = settled
            _save(path, intent)
        acknowledged = backend.request("builder", "POST", "/sandproof/internal-build-vms/" + identity + "/settle", settled)
        assert acknowledged == {"requestWindow": window, "settled": True}
        intent["settled"] = True
        _save(path, intent)

    def close_builders(self):
        for path in sorted(self.root.glob("builder-intent-*.json")):
            self.close_builder(json.loads(path.read_text())["id"])


def prepare_registry_runtime(args, *, tool_hashes, registry_limits, build_limits,
                             runtime_shape=(1, 256), builder_shape=(2, 2048), ports):
    """Create fresh DTOs only from an explicit complete operator policy."""
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
    binary = root / "server"
    shutil.copyfile(args.server, binary)
    binary.chmod(0o700)
    for name in ("images", "inputs", "builds", "registry", "registry/inbox", "registry/artifacts"):
        (root / name).mkdir(mode=0o700)
    urls = [f"http://127.0.0.1:{port}" for port in ports]
    keys = [secrets.token_hex(32) for _ in range(3)]
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
        config = {"namespace": namespaces[index], "database": str(pool / "db/controller.db"),
            "node_state_dir": str(pool / "state"), "jailer": str(args.jailer), "firecracker": str(firecracker),
            "chroot_base": str(pool / "jails"), "kernel": str(args.kernel), "rootfs": str(rootfs),
            "uid_base": args.uid_base + index * 100, "vcpu_count": cpu, "memory_mib": memory,
            "host_resources": {"cgroup_parent": str(args.build_cgroup_parent if index else args.cgroup_parent),
                               "memory_max_bytes": (memory + 256) * MIB},
            "file_policy_digest": 1, "bind": urls[index].removeprefix("http://"), "domain": "example.invalid",
            "credentials": [{"tenant": 1, "secret": keys[index]}], "max_active_global": 2,
            "max_active_per_tenant": 2, "max_timeout_secs": 1300, "startup_timeout_secs": 40,
            "expiry_scan_seconds": 2, "public_ipv4_enabled": bool(index),
            **acceptance_pool_disk_budget(policy, "builder" if index else "runtime", rootfs.stat().st_size),
            "templates": {"templates": [base], "aliases": {"base": base["template_id"]},
                "compute_limits": {"max_vcpus_global": 4, "max_vcpus_per_tenant": 4,
                    "max_memory_bytes_global": 4608 * MIB, "max_memory_bytes_per_tenant": 4608 * MIB}},
            "data_transfer": {"file_upload_bytes": 512 * MIB, "file_download_bytes": 2048 * MIB,
                "command_input_bytes": MIB, "rpc_request_bytes": MIB,
                "rpc_response_bytes": MIB, "max_transfers": 8, "max_transfers_per_vm": 4, "control_reserve": 1,
                "request_idle_seconds": 60, "file_idle_seconds": 60, "write_idle_seconds": 60}}
        if index:
            config["internal_build_vms"] = True
        else:
            config["credentials"].append({"tenant": 2, "secret": keys[2]})
            config["template_registry"] = {"root": str(root / "registry"), **copy.deepcopy(registry_limits)}
        settings.append(config)
    build = {"root": str(root / "builds"), "bind": urls[2].removeprefix("http://"),
        "runtime_config": str(root / "0/config.json"), "builder_config": str(root / "1/config.json"),
        "tools": {name: {"path": str((args.tools / name).resolve(strict=True)), "sha256": digest}
                  for name, digest in tool_hashes.items()},
        "dns_servers": ["1.1.1.1", "8.8.8.8"], "limits": copy.deepcopy(build_limits)}
    apply_acceptance_build_policy(settings[0], settings[1], build, policy)
    for index, config in enumerate(settings):
        _save(root / str(index) / "config.json", config, fresh=True)
    build_config = root / "build.json"
    _save(build_config, build, fresh=True)
    configuration(build_config, require_physical_lease=False)
    return RegistryDeployment(root, namespaces, settings, urls, keys, binary, build_config)


def add_registry_arguments(parser):
    for name in ("bounded-policy", "run-root", "build-cgroup-parent"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("namespace", "build-namespace"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--builder-rootfs", type=Path, help="fixed builder base; defaults to --rootfs")
