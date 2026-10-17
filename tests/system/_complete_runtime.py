"""Explicit complete deployment data for fresh, private KVM test pools.

These values are test inputs, not product defaults or a configuration upgrader.
Call once before the first server startup; preserve the resulting configuration
and coordination identity for every restart. Missing space is an error, never
a skipped or successful KVM result.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

from contextlib import closing
import copy
import json
import os
from pathlib import Path
import sqlite3
import stat

MIB = 1024 * 1024
ONLINE_BYTES = 1042 * MIB
_envelope = _runpy.run_path(
    str(Path(__file__).resolve().parents[2] / "tools/template_physical_storage.py")
)["envelope"]


def complete_runtime(settings, *, slots, file_size=None, checkpoint_tail_inputs=1000):
    """Construct a single-runtime test DTO from explicit artifact/path inputs.

    The caller already owns the fresh private roots. No files are changed here;
    the separate provision step requires empty roots and a new coordination file.
    Existing complete settings must not be overwritten or relocated by this helper.
    """
    if type(slots) is not int or not 1 <= slots <= 64:
        raise ValueError("explicit finite test slot count required")
    if type(checkpoint_tail_inputs) is not int or not 16 <= checkpoint_tail_inputs <= 1000:
        raise ValueError("explicit finite test checkpoint tail budget required")
    if any(key in settings for key in ("max_vm_id", "bounded_host", "maintenance")):
        raise ValueError("fresh test configuration required, not legacy or existing deployment")
    if (settings.get("internal_build_vms") or settings.get("template_registry")
            or settings.get("build_retry_windows")):
        raise ValueError("this fixture owns only a runtime pool, not a build service")
    if not 0 < settings["max_active_per_tenant"] <= settings["max_active_global"] <= slots:
        raise ValueError("test admission must fit the declared physical slots")
    if not 1000 <= settings["uid_base"] < 2**32 - slots:
        raise ValueError("exclusive non-reserved test UID range required")
    result = copy.deepcopy(settings)
    database = Path(result["database"])
    node = Path(result["node_state_dir"])
    jail = Path(result["chroot_base"])
    root = database.parent.parent
    if (database != root / "db/controller.db" or node != root / "state"
            or jail != root / "jails"):
        raise ValueError("fresh private test db/state/jails layout required")
    size = file_size or (lambda path: Path(path).stat().st_size)
    kernel_bytes = size(result["kernel"])
    runtime_bytes = size(result["firecracker"])
    disk_bytes = size(result["rootfs"])
    catalog = result.get("templates", {}).get("templates", [])
    if catalog:
        kernel_bytes = max([kernel_bytes] + [size(item["kernel"]["path"]) for item in catalog])
    if min(kernel_bytes, runtime_bytes, disk_bytes) <= 0:
        raise ValueError("nonempty approved artifacts required")
    # Conservative bound for <=64 positions, <=128 route cells/position and
    # the node's enforced 16 MiB staged payload / 1 MiB runtime-record caps.
    # Rust independently derives the exact envelope and rejects undersizing.
    metadata_bytes = 384 * MIB * slots
    metadata_inodes = 1024 * slots
    cleanup = ONLINE_BYTES + metadata_bytes
    rootfs_budget = result["max_disk_bytes_global"] - result["cleanup_reserve_bytes"]
    if rootfs_budget < disk_bytes or result["max_disk_bytes_per_tenant"] <= 0:
        raise ValueError("explicit usable test disk quota must fit one base VM")
    result.update(max_disk_bytes_global=rootfs_budget + cleanup,
                  cleanup_reserve_bytes=cleanup)
    if "data_transfer" not in result:
        result["data_transfer"] = {
            "file_upload_bytes": 64 * MIB, "file_download_bytes": 96 * MIB,
            "command_input_bytes": 8 * MIB,
            "rpc_request_bytes": 64 * 1024, "rpc_response_bytes": MIB,
            "max_transfers": 8, "max_transfers_per_vm": 4, "control_reserve": 1,
            "request_idle_seconds": 30, "file_idle_seconds": 30, "write_idle_seconds": 30,
        }
    transfers = result["data_transfer"]
    if (transfers["max_transfers_per_vm"] > 128 or transfers["max_transfers"] > 8192
            or not 0 < transfers["control_reserve"] < transfers["max_transfers_per_vm"]):
        raise ValueError("test metadata envelope supports at most 128 routes per position")
    result["maintenance"] = {
        "capacity": {
            "max_live_sandboxes": 64, "max_sql_rows": 4096, "max_sql_bytes": 16 * MIB,
            # The retained inbox is ONE original SQL row. Let it use the
            # existing 16 MiB aggregate ceiling; Rust derives/reserves its
            # exact future row size and still refuses insufficient TOTAL SQL.
            # Neither aggregate SQL nor physical online storage is increased.
            "max_sql_value_bytes": 16 * MIB, "max_receipt_bytes": 16 * 1024,
            "max_transaction_inputs": 4, "max_transaction_bytes": 32 * 1024,
            "max_database_bytes": 512 * MIB, "max_wal_bytes": 520 * MIB,
            "max_shm_bytes": 2 * MIB, "checkpoint_scratch_bytes": 8 * MIB,
            "max_online_storage_bytes": ONLINE_BYTES,
        },
        "retention_ms": 60_000, "max_checkpoint_bytes": 4 * MIB,
        "max_records": 10_000, "max_references": 1_000_000,
        "max_tail_inputs": checkpoint_tail_inputs, "max_tail_bytes": 4 * MIB,
        "max_input_bytes": 8192, "prune_batch_rows": 7,
    }
    result["bounded_host"] = {
        "console_policy": "discard", "uid_base": result["uid_base"], "uplink_base": 0,
        "maintenance": {"max_host_slots": slots, "max_retained_results": 128,
                        "windows": {"max_open_windows": 2, "max_duration_ms": 1000}},
        "physical_storage": {
            "version": 1, "coordination_file": str(root / "physical-profile"),
            "pools": [{
                "role": "runtime", "namespace": result["namespace"], "database": str(database),
                "node_state_dir": str(node), "chroot_base": str(jail),
                "rootfs_budget_bytes": rootfs_budget, "max_host_slots": slots,
                "kernel_copy_bytes": kernel_bytes, "runtime_copy_bytes": runtime_bytes,
                "cleanup_reserve_bytes": cleanup, "host_metadata_bytes": metadata_bytes,
                "host_metadata_inodes": metadata_inodes, "jail_inodes_per_slot": 12,
            }],
            "public_build": None,
            "host_template_caches": [],
            "service_data": [],
            "filesystem_overhead_bytes": 16 * MIB * slots,
            "filesystem_overhead_inodes": 2048 * slots,
        },
    }
    _envelope(result["bounded_host"]["physical_storage"])
    longest = jail / Path(result["firecracker"]).name / f"vm-{result['namespace']}-{2**64 - 1}" / "root/api.socket"
    if len(os.fsencode(longest)) >= 108:
        raise ValueError("private test path exceeds Firecracker maximum identity socket length")
    return result


def provision_runtime(settings):
    """Create exactly one fresh empty coordination identity after free-space checks."""
    profile = settings["bounded_host"]["physical_storage"]
    peak_bytes, peak_inodes = _envelope(profile)
    if len(profile["pools"]) != 1 or profile["public_build"] is not None:
        raise ValueError("single runtime fixture required")
    pool = profile["pools"][0]
    roots = [Path(pool["database"]).parent, Path(pool["node_state_dir"]), Path(pool["chroot_base"])]
    devices = set()
    for path in roots:
        metadata = path.lstat()
        if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_mode & 0o077
                or metadata.st_uid != os.geteuid() or any(path.iterdir())):
            raise ValueError("fresh private empty test allocation roots required")
        devices.add(metadata.st_dev)
    coordination = Path(profile["coordination_file"])
    parent_metadata = coordination.parent.lstat()
    if (not stat.S_ISDIR(parent_metadata.st_mode) or parent_metadata.st_mode & 0o077
            or parent_metadata.st_uid != os.geteuid()):
        raise ValueError("private owned test coordination parent required")
    devices.add(parent_metadata.st_dev)
    if len(devices) != 1:
        raise ValueError("test physical profile requires one filesystem")
    free = os.statvfs(coordination.parent)
    if free.f_bavail * free.f_frsize < 2 * peak_bytes or free.f_favail < 2 * peak_inodes:
        raise RuntimeError("insufficient bytes/inodes for complete KVM profile; test not executed")
    descriptor = os.open(coordination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.flush()
        os.fsync(output.fileno())
    directory = os.open(coordination.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def terminal_cleanup_confirmed(database, sandbox):
    """Observe exact Destroyed or its durable, quiescent retirement, not mere absence."""
    number = int(sandbox.rsplit("-", 1)[1])
    with closing(sqlite3.connect(f"file:{database}?mode=ro", uri=True)) as connection:
        connection.execute("BEGIN")
        state = connection.execute("SELECT state_blob FROM sandboxes WHERE sandbox_id=?", (sandbox,)).fetchone()
        unresolved = connection.execute(
            "SELECT action_json FROM outbox WHERE sandbox_id=? AND dispatch_state='dispatched'", (sandbox,)
        ).fetchall()
        if connection.execute(
            "SELECT count(*) FROM outbox WHERE sandbox_id=? AND dispatch_state NOT IN ('receipted','dispatched')",
            (sandbox,),
        ).fetchone()[0]:
            # Destroyed publication is not evidence that queued CleanupVm /
            # CleanupSlot work has executed. RetiredByStop only covers an
            # already-dispatched original activation, never pending cleanup.
            return False
        if state is not None:
            if json.loads(state[0])["phase"] != "Destroyed":
                return False
            if not unresolved:
                return True
            cut, payload = connection.execute("SELECT input_cut,payload FROM controller_checkpoint WHERE id=1").fetchone()
            core = json.loads(payload)["core"]
            retired_activations = [effect for effect, status in core.get("effects", [])
                                   if isinstance(status, dict) and "RetiredByStop" in status]
            for (encoded,) in connection.execute("SELECT input_json FROM controller_inputs WHERE sequence>?", (cut,)):
                retirement = json.loads(encoded)["input"].get("Effect", {}).get("RetireActivation")
                if retirement:
                    retired_activations.append(retirement["activation"])
            # An unknown ActivateVm covered by the exact committed stop is
            # deliberately not fabricated as a successful worker receipt.
            return all(encoded is not None and json.loads(encoded)["command"] in retired_activations
                       for (encoded,) in unresolved)
        if unresolved or connection.execute("SELECT count(*) FROM outbox WHERE sandbox_id=?", (sandbox,)).fetchone()[0]:
            return False
        cut, payload = connection.execute("SELECT input_cut,payload FROM controller_checkpoint WHERE id=1").fetchone()
        retained = json.loads(payload)["core"]["maintenance"]["retained"]
        if any(key["sandbox"] == number and
               (value["ledger"]["result"] == "Destroyed" or
                isinstance(value["ledger"]["result"], dict) and "Failed" in value["ledger"]["result"])
               for key, value in retained):
            return True
        for (encoded,) in connection.execute("SELECT input_json FROM controller_inputs WHERE sequence>?", (cut,)):
            retirement = json.loads(encoded)["input"].get("Maintenance", {}).get("Retire")
            if retirement and retirement["sandbox"] == number:
                certificate = retirement["quiescence"]
                return (certificate["sandbox"] == number and all(certificate[key] == "Confirmed" for key in (
                    "worker_drained", "data_connections_drained", "host_resources_reclaimed", "external_pins_released")))
        return False


def idle_snapshot(database):
    """Read current projections/identity frontiers; maintenance inputs may advance on reopen."""
    with closing(sqlite3.connect(f"file:{database}?mode=ro", uri=True)) as connection:
        connection.execute("BEGIN")
        return {
            "frontiers": {(kind, scope): int(value) for kind, scope, value in connection.execute(
                "SELECT kind,scope,high_water FROM controller_id_highwaters")},
            "states": connection.execute("SELECT sandbox_id,state_blob FROM sandboxes ORDER BY sandbox_id").fetchall(),
            "actions": connection.execute("SELECT id,dispatch_state,action_json,receipt_json FROM outbox ORDER BY id").fetchall(),
            "contexts": connection.execute("SELECT position,context_json FROM controller_host_slots ORDER BY position").fetchall(),
        }


def retired_sandboxes(connection, namespace, *, owner=1):
    """Read exact retained results and committed retirement tail in one DB snapshot."""
    if type(owner) is not int or not 0 < owner < 2**64:
        raise ValueError("exact native owner required")
    cut, payload = connection.execute("SELECT input_cut,payload FROM controller_checkpoint WHERE id=1").fetchone()
    envelope = json.loads(payload)
    if envelope["namespace"] != namespace:
        raise ValueError("checkpoint namespace differs from the private test pool")
    retired = set()
    for key, value in envelope["core"]["maintenance"]["retained"]:
        ledger = value["ledger"]
        result = ledger["result"]
        if (type(value["owner"]) is int and value["owner"] == owner and key["sandbox"] == ledger["sandbox"]
                and (result == "Destroyed" or isinstance(result, dict) and "Failed" in result)):
            retired.add(f"sb-{namespace}-{key['sandbox']}")
    for (encoded,) in connection.execute("SELECT input_json FROM controller_inputs WHERE sequence>?", (cut,)):
        request = json.loads(encoded)["input"].get("Maintenance", {}).get("Retire")
        if request is not None:
            certificate = request["quiescence"]
            if (type(request["owner"]) is int and type(certificate["owner"]) is int
                    and request["owner"] == certificate["owner"] == owner
                    and request["sandbox"] == certificate["sandbox"] and all(certificate[key] == "Confirmed" for key in (
                        "worker_drained", "data_connections_drained", "host_resources_reclaimed", "external_pins_released"))):
                retired.add(f"sb-{namespace}-{request['sandbox']}")
    return retired


def assert_idle_reopen(before, after):
    removed = set(before["states"]) - set(after["states"])
    assert all(json.loads(blob)["phase"] == "Destroyed" for _, blob in removed), (
        "idle reopen discarded nonterminal state")
    retired_scopes = {ident.rsplit("-", 1)[1] for ident, _ in removed}
    missing = set(before["frontiers"]) - set(after["frontiers"])
    assert not set(after["frontiers"]) - set(before["frontiers"]), "idle reopen added an allocator domain"
    assert all(kind == "operation" and scope in retired_scopes for kind, scope in missing), (
        "idle reopen lost a permanent or nonterminal allocator domain")
    for key, value in before["frontiers"].items():
        if key in missing:
            continue  # Scoped operation cursors retire with their exact terminal SQL row.
        if key == ("input", ""):
            assert after["frontiers"][key] >= value, "maintenance input frontier reset"
        else:
            assert after["frontiers"][key] == value, "idle reopen allocated a new identity"
    assert after["contexts"] == before["contexts"], "idle reopen replaced a physical binding"
    # Automatic complete-mode retirement may remove terminal rows and their
    # outbox, but cannot add new ones, change receipts, or resurrect a sandbox.
    assert set(after["states"]) <= set(before["states"]), "idle reopen changed/resurrected a sandbox"
    assert set(after["actions"]) <= set(before["actions"]), "idle reopen dispatched or rewrote work"
