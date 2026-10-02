"""Run only the two existing C02 probes on an approved isolated Linux/KVM host.

No host execution is authorized by invoking this CLI. Supply an independently
approved exact commit, server digest, and existing host.example.json profile.
Every run retains a private directory; --export-run reads a safe summary only
and never replays an interrupted profile or attempts host-direct cleanup.
"""

import argparse
from contextlib import ExitStack
import fcntl
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools import check_kvm_host as host
from tools.system_acceptance import (
    Failure, PHASES, PROBES, digest, json_file, new_report, read_report,
    record_failure, require, safe_export, valid_sandbox_id, validate_probe, write_once,
)
from server_process_crash import (
    free_loopback_port, process_start_ticks, start_server, stop_server, wait_ready,
)

# Existing system supervisors import this public helper.
sha256_file = digest

MAX_VM_ID = 1000
EXPECTED_SANDBOXES = 4
CHECKOUT = Path(__file__).resolve().parents[2]
SOURCE_FILES = {
    "runner": "tests/system/sdk_c02_acceptance.py",
    "report_helper": "tools/system_acceptance.py",
    "preflight_helper": "tools/check_kvm_host.py",
    "server_helper": "tests/system/server_process_crash.py",
    "sdk_server_smoke.py": "tests/system/sdk_server_smoke.py",
    "sdk_data_contract.py": "tests/system/sdk_data_contract.py",
    "sdk_data_samples.py": "tests/compat/sdk_data_samples.py",
}


def processes_in_uid_range(first, last):
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            status = (entry / "status").read_text()
        except (FileNotFoundError, ProcessLookupError):
            continue
        for line in status.splitlines():
            if line.startswith("Uid:"):
                if any(first <= int(uid) <= last for uid in line.split()[1:]):
                    found.append(int(entry.name))
                break
        else:
            raise Failure("resource_unknown")
    return sorted(found)


def durable_rows(database, namespace):
    """Only the private persistent DB supplies sandbox IDs, never logs or VM IDs."""
    require(database.is_file() and not database.is_symlink(), "database_unknown")
    try:
        with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=2) as connection:
            rows = connection.execute(
                "SELECT sandbox_id, owner_id, state_blob FROM sandboxes ORDER BY sandbox_id"
            ).fetchall()
        observed = {}
        for ident, owner, raw in rows:
            require(valid_sandbox_id(ident, namespace) and owner == "tenant-1", "identity_unknown")
            value = json.loads(raw, object_pairs_hook=host.unique_keys)
            require(isinstance(value, dict) and type(value.get("owner")) is int
                    and value["owner"] == 1 and value.get("phase") in
                    ("Starting", "Running", "Pausing", "Paused", "Revoking", "Destroyed")
                    and type(value.get("terminal")) is bool, "database_unknown")
            for field in ("current_vm", "net_slot"):
                number = value[field]
                require(number is None or (type(number) is int and 0 <= number < 2**64), "database_unknown")
            observed[ident] = value
        require(len(observed) <= EXPECTED_SANDBOXES, "identity_unknown")
        return observed
    except (sqlite3.Error, ValueError, TypeError, KeyError) as error:
        raise Failure("database_unknown") from error


def destroyed(rows):
    return all(value["phase"] == "Destroyed" and value["terminal"]
               and value["current_vm"] is None and value["net_slot"] is None
               for value in rows.values())


def wait_for_destroyed(database, namespace, identities, *, timeout=30):
    deadline = time.monotonic() + timeout
    while True:
        rows = durable_rows(database, namespace)
        require(set(rows) == set(identities), "identity_unknown")
        if destroyed(rows):
            return rows
        if time.monotonic() >= deadline:
            raise Failure("terminal_state_invalid")
        time.sleep(0.2)


def entries(path):
    try:
        return list(path.iterdir())
    except FileNotFoundError:
        return []


def assert_no_exact_resources(root, namespace, first_uid, firecracker_name, *, expected=None):
    """Observe only; never stop a host PID or delete a claim/resource directly."""
    require(re.fullmatch(r"[0-9a-f]{16}", namespace) is not None, "identity_unknown")
    prefix = f"vm-{namespace}-"
    claims = entries(root / "jails/.claims")
    require(all(re.fullmatch(re.escape(prefix) + r"(0|[1-9][0-9]*)", p.name)
                and not p.is_symlink() and p.is_dir() for p in claims), "resource_unknown")
    if expected is not None:
        require(len(claims) == expected, "resource_unknown")
    require(len(claims) <= EXPECTED_SANDBOXES, "resource_unknown")
    network_inodes = set()
    for claim in claims:
        try:
            record = json_file(claim / "record.json")
            require(isinstance(record, dict) and set(record) == {"resource_id", "pid", "start_time_ticks"}
                    and record["resource_id"] == claim.name
                    and type(record["pid"]) is int and 0 < record["pid"] < 2**32
                    and type(record["start_time_ticks"]) is int and 0 < record["start_time_ticks"] < 2**64,
                    "resource_unknown")
            require(process_start_ticks(record["pid"]) != record["start_time_ticks"], "resource_residual")
            if (claim / "network.json").exists():
                network = json_file(claim / "network.json")
                require(isinstance(network, dict) and set(network) == {"resource_id", "namespace_inode"}
                        and network["resource_id"] == claim.name and type(network["namespace_inode"]) is int
                        and network["namespace_inode"] > 0, "resource_unknown")
                network_inodes.add(f"net:[{network['namespace_inode']}]")
        except (OSError, ValueError, TypeError) as error:
            raise Failure("resource_unknown") from error
    for parent in (root / "jails" / firecracker_name, Path("/run/netns")):
        require(not any(p.name.startswith(prefix) for p in entries(parent)), "resource_residual")
    # os.walk's default swallows permission errors. An incomplete audit fails closed.
    def unavailable(error):
        raise Failure("resource_unknown") from error
    require(Path("/sys/fs/cgroup").is_dir(), "resource_unknown")
    for _directory, dirs, files in os.walk("/sys/fs/cgroup", onerror=unavailable):
        require(not any(name.startswith(prefix) for name in dirs + files), "resource_residual")
    require(not processes_in_uid_range(first_uid, first_uid + MAX_VM_ID), "resource_residual")
    if network_inodes:
        for process in Path("/proc").iterdir():
            if process.name.isdecimal():
                try:
                    require(os.readlink(process / "ns/net") not in network_inodes, "resource_residual")
                except (FileNotFoundError, ProcessLookupError):
                    pass
    return len(claims)


def probe_environment(owner_key, foreign_key):
    # No inherited proxies, PYTHONOPTIMIZE, PYTHONPATH, user startup hooks or secrets.
    return {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8",
            "SANDPROOF_TEST_API_KEY": owner_key, "SANDPROOF_TEST_FOREIGN_API_KEY": foreign_key}


def run_probe(root, name, base_url, environment, namespace, *, timeout=240):
    require(name in PROBES, "invalid_configuration")
    process = None
    primary = None
    secondary = []
    with (root / f"{name}.stdout").open("xb") as out, (root / f"{name}.stderr").open("xb") as err:
        try:
            process = subprocess.Popen(
                [sys.executable, "-E", "-s", str(CHECKOUT / "tests/system" / name),
                 "--api-url", base_url, "--sandbox-url", base_url],
                stdin=subprocess.DEVNULL, stdout=out, stderr=err, env=environment,
                start_new_session=True,
            )
            try:
                code = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired as error:
                raise Failure("timeout") from error
            require(code == 0, "probe_failed")
        except BaseException as error:
            primary = error
        finally:
            # The caller cannot discover/clean DB identities until this child is reaped.
            try:
                stop_server(process)
            except BaseException as error:
                secondary.append(error)
            for output in (out, err):
                try:
                    output.flush()
                    os.fsync(output.fileno())
                except BaseException as error:
                    secondary.append(error)
        if primary is not None or secondary:
            failure = primary if primary is not None else secondary.pop(0)
            failure.secondary = secondary
            failure.child_reaped = process is None or process.poll() is not None
            raise failure
    try:
        value = json_file(root / f"{name}.stdout")
        return validate_probe(name, value, namespace)
    except (OSError, ValueError, TypeError, KeyError) as error:
        raise Failure("probe_output_invalid") from error


def preflight(args, report):
    require(__debug__, "optimized_python")
    require(re.fullmatch(r"[0-9a-f]{40}", args.expected_commit or "") is not None
            and re.fullmatch(r"[0-9a-f]{64}", args.server_sha256 or "") is not None,
            "invalid_configuration")
    require(sys.platform == "linux" and os.geteuid() == 0, "preflight_failed")
    require(1000 <= args.uid_base and args.uid_base + MAX_VM_ID < 2**32, "invalid_configuration")
    require(importlib.metadata.version("e2b") == "2.51.0", "sdk_mismatch")
    source = host.source_revision()
    require(source["revision"] == args.expected_commit and not source["dirty"], "source_mismatch")
    report["commit"] = source["revision"]
    config, config_digest = host.load_config(host.trusted_path(args.host_config))
    report["hashes"]["host_config"] = config_digest
    # Match retained old flags to the one existing approved host profile.
    for name in host.ARTIFACTS:
        path = host.trusted_path(getattr(args, name))
        require(path == Path(config["artifacts"][name]["path"]), "invalid_configuration")
        observed = digest(path)
        require(observed == config["artifacts"][name]["sha256"], "hash_mismatch")
        report["hashes"][name] = observed
    server = host.trusted_path(args.server)
    require(os.access(server, os.X_OK) and digest(server) == args.server_sha256, "hash_mismatch")
    report["hashes"]["server"] = args.server_sha256
    report["hashes"]["python"] = digest(Path(sys.executable).resolve(strict=True))
    for name, relative in SOURCE_FILES.items():
        path = CHECKOUT / relative
        require(not path.is_symlink(), "source_mismatch")
        report["hashes"][name] = digest(path)
        # Also catch assume-unchanged index entries; bind probe bytes to HEAD.
        committed = subprocess.check_output(
            ["git", "--no-optional-locks", "-c", f"safe.directory={CHECKOUT}",
             "-C", str(CHECKOUT), "show", f"{args.expected_commit}:{relative}"], timeout=10)
        require(hashlib.sha256(committed).hexdigest() == report["hashes"][name], "source_mismatch")
    # Only after hashes/clean commit are checked may approved host binaries run
    # their --version probes. This is the existing read-only preflight, not KVM acceptance.
    result = host.inspect_host(config, config_digest)
    report["private_preflight"] = result
    report["preflight"] = {item["name"]: item["status"] for item in result["checks"]}
    require(result["prerequisites_passed"] and
            all(value == "pass" for value in report["preflight"].values()), "preflight_failed")
    require(not processes_in_uid_range(args.uid_base, args.uid_base + MAX_VM_ID), "preflight_failed")
    require(shutil.disk_usage(args.scratch_parent).free >= args.rootfs.stat().st_size * 3 + 256 * 1024 * 1024,
            "preflight_failed")


def prepare(args, root, namespace, owner_key, foreign_key, port, report):
    firecracker = args.firecracker.name
    longest_socket = root / "jails" / firecracker / f"vm-{namespace}-{MAX_VM_ID}" / "root/api.socket"
    require(len(os.fsencode(longest_socket)) < 108, "invalid_configuration")
    for name in ("db", "state", "jails"):
        (root / name).mkdir(mode=0o700)
    report["private_staged"] = {}
    report["staged_copies"] = "retained"
    for name, source, destination in (("server", args.server, root / "sandproof-server"),
                                     ("kernel", args.kernel, root / "vmlinux"),
                                     ("rootfs", args.rootfs, root / "rootfs.ext4")):
        report["staged_files"][name] = "unknown"
        shutil.copyfile(source, destination)
        destination.chmod(0o700 if name == "server" else 0o644)
        require(digest(destination) == report["hashes"][name], "hash_mismatch")
        metadata = destination.lstat()
        report["private_staged"][name] = {"device": metadata.st_dev, "inode": metadata.st_ino,
                                          "bytes": metadata.st_size}
        report["staged_files"][name] = "retained"

    disk_bytes = args.rootfs.stat().st_size
    config = {
        "namespace": namespace, "database": str(root / "db/controller.db"),
        "node_state_dir": str(root / "state"), "jailer": str(args.jailer),
        "firecracker": str(args.firecracker), "chroot_base": str(root / "jails"),
        "kernel": str(root / "vmlinux"), "rootfs": str(root / "rootfs.ext4"),
        "uid_base": args.uid_base, "max_vm_id": MAX_VM_ID, "vcpu_count": 1,
        "memory_mib": 512, "file_policy_digest": 1, "bind": f"127.0.0.1:{port}",
        "domain": "example.invalid",
        "credentials": [{"tenant": 1, "secret": owner_key}, {"tenant": 2, "secret": foreign_key}],
        "max_active_global": 2, "max_active_per_tenant": 2,
        "max_disk_bytes_global": disk_bytes * 6, "max_disk_bytes_per_tenant": disk_bytes * 4,
        "cleanup_reserve_bytes": disk_bytes, "max_timeout_secs": 600,
        "max_lifetime_secs": 600, "default_timeout_secs": 300, "expiry_scan_seconds": 1,
    }
    write_once(root / "config.json", config)
    report["hashes"]["service_config"] = digest(root / "config.json")


def kill_one(opener, base_url, owner_key, ident):
    request = urllib.request.Request(base_url + "/sandboxes/" + ident,
                                     headers={"X-API-Key": owner_key}, method="DELETE")
    try:
        with opener.open(request, timeout=5) as response:
            status = response.status
    except urllib.error.HTTPError as error:
        status = error.code
        error.close()
    require(status in (204, 404), "cleanup_failed")


def cleanup(root, namespace, args, opener, base_url, owner_key, process, report):
    """Attempt every observed exact persistent identity even after another fails."""
    report["cleanup"] = "unknown"
    require(report.get("probe_reaped", True), "identity_unknown")
    observed = {ident: None for ident in report.get("identities", [])}
    require(all(valid_sandbox_id(ident, namespace) for ident in observed), "identity_unknown")
    attempted = set()
    def attempt_known(identities):
        for ident in sorted(set(identities) - attempted):
            attempted.add(ident)
            try:
                require(process is not None and process.poll() is None, "server_exited")
                kill_one(opener, base_url, owner_key, ident)
            except Exception as error:
                record_failure(report, "cleanup", error, cleanup=True)
    # Previously validated persistent identities remain usable if the DB later
    # becomes unavailable. Its loss still prevents a cleanup confirmation.
    attempt_known(observed)
    # A late committed create is discovered without replaying the creating probe.
    for _ in range(EXPECTED_SANDBOXES + 1):
        rows = durable_rows(root / "db/controller.db", namespace)
        require(set(observed).issubset(rows), "identity_unknown")
        observed = rows
        report["known_sandboxes"] = len(rows)
        report["identities"] = sorted(rows)
        if not set(rows) - attempted:
            break
        attempt_known(rows)
    wait_for_destroyed(root / "db/controller.db", namespace, observed)
    report["known_claims"] = assert_no_exact_resources(
        root, namespace, args.uid_base, args.firecracker.name)
    require(process is not None and process.poll() is None, "server_exited")
    require(not report["cleanup_errors"], "cleanup_failed")
    if len(observed) == EXPECTED_SANDBOXES:
        require(report["known_claims"] == EXPECTED_SANDBOXES, "resource_unknown")
    # No drain API exists for an in-flight lost create; a partial failed probe
    # cannot prove that every absent profile instance was never dispatched.
    require(not report["failures"] or len(observed) == EXPECTED_SANDBOXES, "identity_unknown")
    report["cleanup"] = "confirmed"
    report["phases"]["cleanup"] = "pass"
    return set(observed)


def unchanged(args, root, report):
    for name in ("server", "kernel", "rootfs", "jailer", "firecracker"):
        require(digest(getattr(args, name)) == report["hashes"][name], "hash_mismatch")
    require(digest(root / "rootfs.ext4") == report["hashes"]["rootfs"], "hash_mismatch")
    for name, path in SOURCE_FILES.items():
        require(digest(CHECKOUT / path) == report["hashes"][name], "hash_mismatch")
    require(host.source_revision()["revision"] == args.expected_commit, "source_mismatch")


def remove_staged_copies(root, report):
    """After successful shutdown/audit, unlink only the three exact staged copies."""
    report["staged_copies"] = "unknown"
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    files = (("server", "sandproof-server"), ("kernel", "vmlinux"), ("rootfs", "rootfs.ext4"))
    try:
        with ExitStack() as opened:
            validated = {}
            # Validate the complete set before removing any file. Retain all
            # available copies when one target has changed or cannot be observed.
            for name, filename in files:
                report["staged_files"][name] = "unknown"
                expected = report["private_staged"][name]
                fd = os.open(filename, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
                source = opened.enter_context(os.fdopen(fd, "rb"))
                info = os.fstat(source.fileno())
                require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1
                        and info.st_uid == os.geteuid()
                        and (info.st_dev, info.st_ino, info.st_size) ==
                        (expected["device"], expected["inode"], expected["bytes"]), "identity_unknown")
                require(hashlib.file_digest(source, "sha256").hexdigest() == report["hashes"][name],
                        "hash_mismatch")
                validated[name] = info
                report["staged_files"][name] = "retained"
            for name, filename in files:
                info = validated[name]
                current = os.stat(filename, dir_fd=directory, follow_symlinks=False)
                require(stat.S_ISREG(current.st_mode) and current.st_nlink == 1
                        and (current.st_dev, current.st_ino, current.st_size) ==
                        (info.st_dev, info.st_ino, info.st_size), "identity_unknown")
                report["staged_files"][name] = "unknown"
                os.unlink(filename, dir_fd=directory)
                report["staged_files"][name] = "removed"
            os.fsync(directory)
    finally:
        os.close(directory)
    report["staged_copies"] = "removed"


def run_profile(args, root, report):
    process = None
    started = False
    identities = None
    namespace = report["private_identity"]["namespace"]
    owner_key, foreign_key = secrets.token_hex(32), secrets.token_hex(32)
    base_url = None
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    stage = "preflight"
    try:
        preflight(args, report)
        report["phases"][stage] = "pass"
        stage = "prepare"
        port = free_loopback_port()
        base_url = f"http://127.0.0.1:{port}"
        prepare(args, root, namespace, owner_key, foreign_key, port, report)
        report["phases"][stage] = "pass"
        # Durable provenance before launching any service that can create VMs.
        write_once(root / "ready.json", report)
        stage = "server_start"
        started = True
        report["cleanup"] = "unknown"
        process = start_server(root / "sandproof-server", root / "config.json", root / "server.log", None)
        report["kvm"] = "attempted"
        wait_ready(opener, base_url, owner_key, process)
        report["phases"][stage] = "pass"
        for name in PROBES:
            stage = name
            report["probes"][name] = run_probe(
                root, name, base_url, probe_environment(owner_key, foreign_key), namespace)
            report["kvm"] = "executed"
            require(process.poll() is None, "server_exited")
            report["phases"][stage] = "pass"
        stage = "terminal_state"
        rows = durable_rows(root / "db/controller.db", namespace)
        report["known_sandboxes"] = len(rows)
        report["identities"] = sorted(rows)
        require(len(rows) == EXPECTED_SANDBOXES, "terminal_state_invalid")
        wait_for_destroyed(root / "db/controller.db", namespace, rows)
        report["phases"][stage] = "pass"
        stage = "resource_audit"
        report["known_claims"] = assert_no_exact_resources(
            root, namespace, args.uid_base, args.firecracker.name, expected=EXPECTED_SANDBOXES)
        report["phases"][stage] = "pass"
        stage = "source_unchanged"
        unchanged(args, root, report)
        report["phases"][stage] = "pass"
    except BaseException as error:
        record_failure(report, stage, error)
        report["probe_reaped"] = getattr(error, "child_reaped", True)
    finally:
        # Repeated SIGINT/TERM cannot abort exact cleanup or replace the first error.
        for sig in (signal.SIGINT, signal.SIGTERM):
            signal.signal(sig, signal.SIG_IGN)
        try:
            if started:
                try:
                    identities = cleanup(root, namespace, args, opener, base_url, owner_key, process, report)
                except BaseException as error:
                    record_failure(report, "cleanup", error, cleanup=True)
                    report["cleanup"] = "unknown"
                    identities = set(report.get("identities", [])) if "identities" in report else None
            else:
                report["cleanup"] = "not_created"
        finally:
            try:
                stop_server(process)
                report["phases"]["server_stop"] = "pass"
            except BaseException as error:
                record_failure(report, "server_stop", error, cleanup=True)
                report["cleanup"] = "unknown"
        # Catch a late accepted request between the cleanup snapshot and shutdown.
        # Never restart/replay automatically when the now-stopped service cannot help.
        if identities is not None:
            try:
                rows = durable_rows(root / "db/controller.db", namespace)
                require(set(rows) == identities and destroyed(rows), "identity_unknown")
                assert_no_exact_resources(root, namespace, args.uid_base, args.firecracker.name)
            except BaseException as error:
                record_failure(report, "cleanup", error, cleanup=True)
                report["cleanup"] = "unknown"
    report["status"] = "interrupted" if any(
        item["code"] == "interrupted" for item in report["failures"]) else "fail"
    if not report["failures"] and not report["cleanup_errors"]:
        try:
            remove_staged_copies(root, report)
            report["phases"]["artifact_cleanup"] = "pass"
            report["status"] = "pass"
        except BaseException as error:
            record_failure(report, "artifact_cleanup", error)
            report["status"] = "fail"


def exercise(args):
    root = None
    lock = None
    report = new_report("unknown")
    original_signals = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
    old_umask = os.umask(0o077)
    def interrupted(_signal, _frame):
        raise Failure("interrupted")
    try:
        for sig in original_signals:
            signal.signal(sig, interrupted)
        # Root creation itself is unprivileged filesystem work. Invalid preflight
        # still gets a terminal report whenever a safe private parent is available.
        parent = args.scratch_parent
        require(parent.is_absolute() and not parent.is_symlink(), "invalid_configuration")
        parent = host.trusted_path(parent)
        require(parent.is_dir() and not parent.stat().st_mode & 0o077, "invalid_configuration")
        root = Path(tempfile.mkdtemp(prefix="sp12-", dir=parent))
        lock = os.open(root / "run.lock", os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        report = new_report(root.name)
        report["private_identity"] = {"namespace": secrets.token_hex(8),
                                      "uid_first": args.uid_base, "uid_last": args.uid_base + MAX_VM_ID}
        write_once(root / "intent.json", report)
        run_profile(args, root, report)
    except BaseException as error:
        record_failure(report, "preflight", error)
        report["status"] = "interrupted" if isinstance(error, Failure) and error.code == "interrupted" else "fail"
    finally:
        for sig in original_signals:
            signal.signal(sig, signal.SIG_IGN)
        if root is not None:
            try:
                report["phases"]["report"] = "pass"
                write_once(root / "terminal.json", report)
            except BaseException as error:
                failure = Failure("report_write_failed")
                failure.secondary = getattr(error, "secondary", [])
                record_failure(report, "report", failure)
                report["status"] = "fail"
                if getattr(error, "publication_unknown", False):
                    # Best effort only: a filesystem that cannot record even this
                    # marker requires operator review, never acceptance from files.
                    try:
                        write_once(root / "publication-unknown.json", {"code": "report_write_failed"})
                    except BaseException:
                        pass
    exported = safe_export(report)
    try:
        print(json.dumps(exported, indent=2), flush=True)
    except OSError:
        if root is not None:
            try:
                write_once(root / "delivery-failed.json", {"code": "export_write_failed"})
            except OSError:
                pass
        return 1
    finally:
        if lock is not None:
            os.close(lock)
        for sig, handler in original_signals.items():
            signal.signal(sig, handler)
        os.umask(old_umask)
    return 0 if exported["status"] == "pass" else 1


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--export-run", type=Path, help="read-only safe summary; no resume or cleanup")
    for name in ("server", "jailer", "firecracker", "kernel", "rootfs", "scratch-parent", "host-config"):
        parser.add_argument("--" + name, type=Path)
    parser.add_argument("--uid-base", type=int)
    parser.add_argument("--expected-commit")
    parser.add_argument("--server-sha256")
    args = parser.parse_args(argv)
    if args.export_run is not None:
        try:
            result = read_report(args.export_run)
            print(json.dumps(result, indent=2))
            return 0 if result["status"] == "pass" else 1
        except (OSError, ValueError, TypeError, Failure):
            print(json.dumps({"schema_version": 1, "profile": "c02", "status": "fail",
                              "cleanup": "unknown", "error": "report_unavailable"}))
            return 1
    required = ("server", "jailer", "firecracker", "kernel", "rootfs", "scratch_parent", "host_config",
                "uid_base", "expected_commit", "server_sha256")
    if any(getattr(args, name) is None for name in required):
        parser.error("execution requires existing artifact flags plus --host-config, --expected-commit and --server-sha256")
    return exercise(args)


if __name__ == "__main__":
    sys.exit(main())
