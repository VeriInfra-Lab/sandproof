"""Small report boundary for the fixed C02 runner; no host actions or replay.

Private intent is durable before execution. Export constructs a new allowlist,
never copies arbitrary dictionaries, paths, exceptions, logs or credentials.
"""

import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile

from tools.check_kvm_host import unique_keys

STAGED_FILES = ("server", "kernel", "rootfs")
PROBES = ("sdk_server_smoke.py", "sdk_data_contract.py")
PHASES = ("preflight", "prepare", "server_start", *PROBES, "terminal_state",
          "resource_audit", "source_unchanged", "cleanup", "server_stop", "artifact_cleanup", "report")
STATUSES = {"not_run", "pass", "fail", "unknown", "interrupted", "timeout"}
CODES = {"invalid_configuration", "preflight_failed", "source_mismatch", "hash_mismatch",
         "sdk_mismatch", "optimized_python", "probe_failed", "probe_output_invalid",
         "timeout", "interrupted", "io_error", "database_unknown", "identity_unknown",
         "terminal_state_invalid", "resource_unknown", "resource_residual", "server_exited",
         "cleanup_failed", "report_write_failed", "unexpected_error", "unresolved_intent",
         "export_write_failed"}
HASH_NAMES = ("host_config", "service_config", "server", "firecracker", "jailer", "kernel",
              "rootfs", "python", "runner", "assertions_guard", "report_helper", "preflight_helper", "server_helper",
              "sdk_server_smoke.py", "sdk_data_contract.py", "sdk_data_samples.py")
PREFLIGHT_NAMES = ("source", "host", "identity", "kvm", "tun", "cgroup_v2", "host_tools",
                   "disk", "memory", "firecracker", "firecracker_version", "jailer",
                   "jailer_version", "kernel", "rootfs")


class Failure(Exception):
    def __init__(self, code):
        if code not in CODES:
            raise ValueError("unknown failure code")
        self.code = code
        super().__init__(code)


def require(condition, code):
    if not condition:
        raise Failure(code)


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def json_file(path, limit=1024 * 1024):
    # Refuse symlinks and oversized/nonregular evidence; never follow a planted
    # report link while exporting. All run directories are private throughout.
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as source:
        info = os.fstat(source.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_size <= limit, "io_error")
        return json.loads(source.read(limit + 1), object_pairs_hook=unique_keys)


def write_once(path, value):
    """Durable no-clobber publication, even if the destination is a symlink."""
    data = (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()
    fd, temporary = tempfile.mkstemp(prefix=".report-", dir=path.parent)
    inode = os.fstat(fd).st_ino
    published = False
    try:
        with os.fdopen(fd, "wb") as out:
            out.write(data)
            out.flush()
            os.fsync(out.fileno())
        os.link(temporary, path, follow_symlinks=False)
        published = True
        # No fallible temporary cleanup is deferred until after commit.
        os.unlink(temporary)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except BaseException as primary:
        secondary = []
        if published:
            try:
                # Only this call's exact newly linked file can be invalidated.
                require(os.lstat(path).st_ino == inode, "identity_unknown")
                os.unlink(path)
            except BaseException as error:
                primary.publication_unknown = True
                secondary.append(error)
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        except BaseException as error:
            secondary.append(error)
        primary.secondary = secondary
        raise


def new_report(run_id):
    return {"schema_version": 1, "profile": "c02", "run_id": run_id,
            "status": "interrupted", "kvm": "not_run", "commit": None, "hashes": {},
            "preflight": {}, "phases": {name: "not_run" for name in PHASES},
            "probes": {}, "failures": [], "cleanup_errors": [], "cleanup": "not_created",
            "known_sandboxes": 0, "known_claims": 0, "staged_copies": "not_created",
            "staged_files": {name: "not_created" for name in STAGED_FILES}}


def record_failure(report, stage, error, *, cleanup=False):
    code = error.code if isinstance(error, Failure) else (
        "interrupted" if isinstance(error, KeyboardInterrupt) else
        "io_error" if isinstance(error, OSError) else "unexpected_error")
    item = {"stage": stage if isinstance(stage, str) and stage in PHASES else "report", "code": code}
    report["cleanup_errors" if cleanup else "failures"].append(item)
    for secondary in getattr(error, "secondary", ()):
        record_failure(report, stage, secondary, cleanup=True)
    report["phases"][item["stage"]] = "interrupted" if code == "interrupted" else (
        "timeout" if code == "timeout" else "fail")


def safe_export(report):
    """Only known scalar fields survive, even for malicious private input."""
    report = report if isinstance(report, dict) else {}
    def mapping(field):
        value = report.get(field)
        return value if isinstance(value, dict) else {}
    def status(value):
        return value if isinstance(value, str) and value in STATUSES else "not_run"
    result = new_report("unknown")
    run_id = report.get("run_id")
    if isinstance(run_id, str) and re.fullmatch(r"sp12-[a-z0-9_]{8,32}", run_id):
        result["run_id"] = run_id
    commit = report.get("commit")
    if isinstance(commit, str) and re.fullmatch(r"[0-9a-f]{40}", commit):
        result["commit"] = commit
    for name in HASH_NAMES:
        value = mapping("hashes").get(name)
        if isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value):
            result["hashes"][name] = value
    for name in PREFLIGHT_NAMES:
        value = mapping("preflight").get(name)
        result["preflight"][name] = status(value)
    for name in PHASES:
        value = mapping("phases").get(name)
        result["phases"][name] = status(value)
    for field in ("failures", "cleanup_errors"):
        items = report.get(field, [])
        for item in items if isinstance(items, list) else [{}]:
            item = item if isinstance(item, dict) else {}
            stage, code = item.get("stage"), item.get("code")
            result[field].append({"stage": stage if isinstance(stage, str) and stage in PHASES else "report",
                                  "code": code if isinstance(code, str) and code in CODES else "unexpected_error"})
    for name in PROBES:
        item = mapping("probes").get(name, {})
        item = item if isinstance(item, dict) else {}
        probe_status = status(item.get("status"))
        result["probes"][name] = {"status": probe_status}
        # Outcome names are static rather than copied from child-controlled JSON.
        if probe_status == "pass":
            result["probes"][name]["outcomes"] = (
                ["management", "data", "authorization", "expiry"] if name == PROBES[0]
                else ["sync_root", "sync_ubuntu", "async_root", "async_ubuntu"])
    for field in ("known_sandboxes", "known_claims"):
        value = report.get(field)
        if type(value) is int and 0 <= value <= 1001:
            result[field] = value
    cleanup = report.get("cleanup")
    if cleanup in ("not_created", "confirmed", "unknown"):
        result["cleanup"] = cleanup
    if report.get("staged_copies") in ("not_created", "retained", "removed", "unknown"):
        result["staged_copies"] = report["staged_copies"]
    for name in STAGED_FILES:
        value = mapping("staged_files").get(name)
        if value in ("not_created", "retained", "removed", "unknown"):
            result["staged_files"][name] = value
    result["kvm"] = report.get("kvm") if report.get("kvm") in ("executed", "attempted") else "not_run"
    complete = (type(report.get("schema_version")) is int and report["schema_version"] == 1
                and report.get("profile") == "c02" and result["run_id"] != "unknown"
                and all(value == "pass" for value in result["phases"].values())
                and all(item["status"] == "pass" for item in result["probes"].values())
                and all(value == "pass" for value in result["preflight"].values())
                and set(result["hashes"]) == set(HASH_NAMES) and result["commit"] is not None
                and result["known_sandboxes"] == 4 and result["known_claims"] == 4
                and result["cleanup"] == "confirmed" and result["kvm"] == "executed"
                and result["staged_copies"] == "removed"
                and all(value == "removed" for value in result["staged_files"].values())
                and not result["failures"] and not result["cleanup_errors"])
    result["status"] = "pass" if report.get("status") == "pass" and complete else (
        "interrupted" if report.get("status") == "interrupted" else "fail")
    result["scope"] = "selected_c02_only"
    return result


def read_report(root):
    """A live writer or a missing lock never exports terminal acceptance."""
    descriptor = None
    available = False
    try:
        try:
            descriptor = os.open(root / "run.lock", os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            require(stat.S_ISREG(os.fstat(descriptor).st_mode), "io_error")
            fcntl.flock(descriptor, fcntl.LOCK_SH | fcntl.LOCK_NB)
            available = True
        except (FileNotFoundError, BlockingIOError):
            pass
        return _read_report(root, available)
    finally:
        if descriptor is not None:
            os.close(descriptor)


def _read_report(root, terminal_available):
    info = root.lstat()
    require(stat.S_ISDIR(info.st_mode) and not info.st_mode & 0o077, "io_error")
    intent = json_file(root / "intent.json")
    require(isinstance(intent, dict), "io_error")
    try:
        ready = json_file(root / "ready.json")
        require(isinstance(ready, dict), "io_error")
        require(ready.get("run_id") == intent.get("run_id"), "identity_unknown")
        intent = ready
    except FileNotFoundError:
        pass
    try:
        if not terminal_available:
            raise FileNotFoundError
        report = json_file(root / "terminal.json")
        require(isinstance(report, dict), "io_error")
        require(report.get("run_id") == intent.get("run_id"), "identity_unknown")
    except FileNotFoundError:
        report = safe_export(intent)
        report["status"] = "interrupted"
        report["cleanup"] = "unknown"
        record_failure(report, "report", Failure("unresolved_intent"))
    report = safe_export(report)
    if (root / "publication-unknown.json").exists():
        record_failure(report, "report", Failure("report_write_failed"))
        report["status"] = "fail"
    if (root / "delivery-failed.json").exists():
        record_failure(report, "report", Failure("export_write_failed"))
        report["status"] = "fail"
    return safe_export(report)


def validate_probe(name, value, namespace):
    """Validate existing structured completions; SDK assertions stay in probes."""
    require(isinstance(value, dict), "probe_output_invalid")
    if name == PROBES[0]:
        expected = {
            "pinned_sdk": "2.51.0",
            "management": "create/connect/get/list/set_timeout/kill",
            "data": "command/timeout/kill/reconnect/large-output/files/directories",
            "authorization": "foreign-tenant/wrong-token/revoked-token/NUL-or-invalid-UTF8-path/duplicate-token/oversized-upload",
            "expiry": "automatic kill observed",
            "scope": "isolated real VM; template API, egress, Harbor and proofs not covered",
        }
        require(value == expected, "probe_output_invalid")
    else:
        require(name == PROBES[1] and set(value) == {"sdk_version", "sync", "async", "scope"}
                and value["sdk_version"] == "2.51.0"
                and value["scope"] == "real controller/gateway/VM data calls; resource residuals and timeout/fault matrices require separate evidence",
                "probe_output_invalid")
        identities = []
        for kind in ("sync", "async"):
            item = value[kind]
            require(isinstance(item, dict) and set(item) == {"sandbox_id", "cases"}, "probe_output_invalid")
            require(valid_sandbox_id(item["sandbox_id"], namespace), "probe_output_invalid")
            identities.append(item["sandbox_id"])
            require(isinstance(item["cases"], list) and len(item["cases"]) == 2, "probe_output_invalid")
            for user, case in zip(("root", "ubuntu"), item["cases"]):
                require(isinstance(case, dict) and set(case) == {"user", "sha256", "reconnected_pid", "killed_pid"}
                        and case["user"] == user, "probe_output_invalid")
                for field in ("reconnected_pid", "killed_pid"):
                    require(type(case[field]) is int and 0 < case[field] < 2**32, "probe_output_invalid")
                hashes = case["sha256"]
                require(isinstance(hashes, dict) and set(hashes) == {"text.txt", "nested/binary.bin", "nested/deep/empty"}
                        and all(isinstance(h, str) and re.fullmatch(r"[0-9a-f]{64}", h) for h in hashes.values()), "probe_output_invalid")
        require(len(set(identities)) == 2, "probe_output_invalid")
    return {"status": "pass"}


def valid_sandbox_id(value, namespace):
    return (isinstance(value, str) and isinstance(namespace, str)
            and re.fullmatch(r"[0-9a-f]{16}", namespace) is not None
            and re.fullmatch(r"sb-" + namespace + r"-(0|[1-9][0-9]*)", value) is not None
            and int(value.rsplit("-", 1)[1]) < 2**64)
