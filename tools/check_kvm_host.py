#!/usr/bin/env python3
"""Read-only prerequisites for the isolated Linux/KVM test host (Python 3.11+).

Print a JSON report; never start a VM, configure networking, or clean resources.
Exit 0 only when every prerequisite passes, 1 for an incomplete/failed check,
and 2 for invalid configuration. Passing does not constitute system acceptance.
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ("firecracker", "jailer", "kernel", "rootfs")
CONFIG_KEYS = {
    "firecracker_version", "artifacts", "chroot_base",
    "min_free_disk_bytes", "min_available_memory_bytes",
}


class CheckFailed(Exception):
    def __init__(self, message, **evidence):
        super().__init__(message)
        self.evidence = evidence


def require_keys(value, keys, label):
    if not isinstance(value, dict) or set(value) != set(keys):
        raise ValueError(f"{label} must contain exactly: {', '.join(sorted(keys))}")


def absolute_path(value):
    if (not isinstance(value, str) or "\x00" in value
            or not Path(value).is_absolute() or ".." in Path(value).parts):
        raise ValueError("paths must be absolute and contain no NUL or '..' component")
    return Path(value)


def unique_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def load_config(path):
    raw = path.read_bytes()
    config = json.loads(raw, object_pairs_hook=unique_keys)
    require_keys(config, CONFIG_KEYS, "configuration")
    version = config["firecracker_version"]
    if not isinstance(version, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ValueError("firecracker_version must be an exact release such as 1.16.1")
    require_keys(config["artifacts"], ARTIFACTS, "artifacts")
    for name in ARTIFACTS:
        artifact = config["artifacts"][name]
        require_keys(artifact, {"path", "sha256"}, f"artifacts.{name}")
        absolute_path(artifact["path"])
        digest = artifact["sha256"]
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise ValueError(f"artifacts.{name}.sha256 must be 64 lowercase hex characters")
    absolute_path(config["chroot_base"])
    for key in ("min_free_disk_bytes", "min_available_memory_bytes"):
        if type(config[key]) is not int or config[key] <= 0:
            raise ValueError(f"{key} must be a positive integer")
    return config, hashlib.sha256(raw).hexdigest()


def trusted_path(path):
    """Match the runtime's root-owned, non-writable, non-symlink path policy."""
    path = absolute_path(str(path))
    current = Path(path.anchor)
    for part in path.parts[1:]:
        current /= part
        metadata = current.lstat()
        if (stat.S_ISLNK(metadata.st_mode) or metadata.st_uid != 0
                or metadata.st_mode & 0o022):
            raise CheckFailed("path component is symlinked, non-root-owned or writable",
                              component=str(current))
    return path


def command(argv):
    # No shell and no credential/environment dump in reports. All executed host
    # artifacts must first pass ownership, mode and expected-hash checks.
    return subprocess.run(argv, capture_output=True, text=True, check=True,
                          timeout=10).stdout.strip()


def source_revision():
    # Operators run a reviewed checkout through sudo. Permit only this checkout
    # for these read-only Git calls; do not change global safe.directory rules.
    prefix = ["git", "--no-optional-locks", "-c", f"safe.directory={ROOT}", "-C", str(ROOT)]
    revision = command([*prefix, "rev-parse", "HEAD"])
    dirty = bool(command([*prefix, "status", "--porcelain=v1", "--untracked-files=all"]))
    if dirty:
        raise CheckFailed("checkout has uncommitted changes; commit before collecting evidence",
                          revision=revision, dirty=True)
    return {"revision": revision, "dirty": False}


def linux_host():
    observed = {"system": platform.system(), "architecture": platform.machine(),
                "kernel_release": platform.release()}
    if observed["system"] != "Linux" or observed["architecture"] != "x86_64":
        raise CheckFailed("the current guest baseline requires Linux x86_64", **observed)
    return observed


def root_identity():
    euid = os.geteuid() if hasattr(os, "geteuid") else None
    if euid != 0:
        raise CheckFailed("run the preflight as the same root identity used by jailer", euid=euid)
    return {"euid": euid}


def device(path, *, kvm=False):
    if platform.system() != "Linux":
        raise CheckFailed("Linux device probe is unavailable on this platform")
    if not stat.S_ISCHR(path.stat().st_mode):
        raise CheckFailed("expected a character device", path=str(path))
    # KVM_GET_API_VERSION is read-only; deliberately do not call KVM_CREATE_VM
    # or TUNSETIFF. Opening /dev/net/tun without ioctl creates no interface.
    fd = os.open(path, os.O_RDWR | os.O_CLOEXEC)
    try:
        observed = {"path": str(path), "read_write_open": True}
        if kvm:
            import fcntl
            version = fcntl.ioctl(fd, 0xAE00, 0)
            observed["api_version"] = version
            if version != 12:
                raise CheckFailed("unsupported KVM API version", **observed)
        return observed
    finally:
        os.close(fd)


def artifact_fingerprint(spec, *, executable=False):
    path = trusted_path(Path(spec["path"]))
    if not path.is_file():
        raise CheckFailed("artifact must be a regular file", path=str(path))
    if executable and not os.access(path, os.X_OK):
        raise CheckFailed("artifact is not executable", path=str(path))
    with path.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    evidence = {"path": str(path), "sha256": digest, "expected_sha256": spec["sha256"]}
    if digest != spec["sha256"]:
        raise CheckFailed("artifact fingerprint differs from the approved baseline", **evidence)
    return evidence


def binary_version(path, name, expected):
    output = command([path, "--version"])
    product = "Firecracker" if name == "firecracker" else "Jailer"
    first_line = output.splitlines()[0] if output else ""
    if first_line != f"{product} v{expected}":
        raise CheckFailed("binary version differs from the configured pair",
                          expected=f"{product} v{expected}", observed=first_line[:200])
    return {"version": expected}


def host_tools():
    paths = ["/usr/bin/ip", "/usr/bin/curl"]
    for value in paths:
        path = trusted_path(Path(value))
        if not path.is_file() or not os.access(path, os.X_OK):
            raise CheckFailed("required host tool is not executable", path=value)
    return {"paths": paths}


def cgroup_v2():
    controllers = Path("/sys/fs/cgroup/cgroup.controllers").read_text().split()
    if not {"cpu", "memory"}.issubset(controllers):
        raise CheckFailed("cgroup v2 CPU/memory controllers are unavailable", controllers=controllers)
    return {"controllers": controllers}


def disk_headroom(config):
    path = trusted_path(Path(config["chroot_base"]))
    if not path.is_dir():
        raise CheckFailed("chroot_base must be an existing directory")
    free = shutil.disk_usage(path).free
    minimum = config["min_free_disk_bytes"]
    if free < minimum:
        raise CheckFailed("insufficient free disk space", available_bytes=free, required_bytes=minimum)
    return {"path": str(path), "available_bytes": free, "required_bytes": minimum}


def memory_headroom(config):
    contents = Path("/proc/meminfo").read_text()
    match = re.search(r"^MemAvailable:\s+([0-9]+)\s+kB$", contents, re.MULTILINE)
    if not match:
        raise CheckFailed("MemAvailable could not be read from /proc/meminfo")
    available = int(match[1]) * 1024
    minimum = config["min_available_memory_bytes"]
    if available < minimum:
        raise CheckFailed("insufficient available memory",
                          available_bytes=available, required_bytes=minimum)
    return {"available_bytes": available, "required_bytes": minimum}


def check(name, probe):
    try:
        return {"name": name, "status": "pass", **probe()}
    except CheckFailed as error:
        return {"name": name, "status": "fail", "reason": str(error), **error.evidence}
    except subprocess.TimeoutExpired:
        return {"name": name, "status": "timeout", "reason": "command exceeded 10 seconds"}
    except subprocess.CalledProcessError as error:
        return {"name": name, "status": "fail", "reason": "command returned nonzero",
                "returncode": error.returncode}
    except OSError as error:
        return {"name": name, "status": "unavailable", "reason": str(error)}
    except UnicodeError:
        return {"name": name, "status": "fail", "reason": "probe output is not valid text"}


def inspect_host(config, config_sha256):
    checks = [
        check("source", source_revision),
        check("host", linux_host),
        check("identity", root_identity),
        check("kvm", lambda: device(Path("/dev/kvm"), kvm=True)),
        check("tun", lambda: device(Path("/dev/net/tun"))),
        check("cgroup_v2", cgroup_v2),
        check("host_tools", host_tools),
        check("disk", lambda: disk_headroom(config)),
        check("memory", lambda: memory_headroom(config)),
    ]
    for name in ARTIFACTS:
        spec = config["artifacts"][name]
        executable = name in ("firecracker", "jailer")
        fingerprint = check(name, lambda: artifact_fingerprint(spec, executable=executable))
        checks.append(fingerprint)
        if executable:
            if fingerprint["status"] == "pass":
                checks.append(check(f"{name}_version", lambda: binary_version(
                    spec["path"], name, config["firecracker_version"])))
            else:
                checks.append({"name": f"{name}_version", "status": "not_run",
                               "reason": "binary did not pass trust and fingerprint checks"})
    return {
        "schema_version": 1,
        "scope": "host_prerequisites_only",
        "checked_at": datetime.now(timezone.utc).isoformat(),
        "config_sha256": config_sha256,
        "prerequisites_passed": all(item["status"] == "pass" for item in checks),
        "checks": checks,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, type=Path,
                        help="approved host paths, artifact SHA-256 values and resource minima")
    arguments = parser.parse_args(argv)
    try:
        config, digest = load_config(arguments.config)
    except (OSError, ValueError, UnicodeError) as error:
        print(json.dumps({"schema_version": 1, "scope": "host_prerequisites_only",
                          "prerequisites_passed": False, "configuration_error": str(error)}, indent=2))
        return 2
    report = inspect_host(config, digest)
    print(json.dumps(report, indent=2))
    return 0 if report["prerequisites_passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
