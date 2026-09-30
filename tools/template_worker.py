#!/usr/bin/env python3
"""Guest-only, one-shot BuildKit/umoci/ext4 candidate worker (#123).

Run ONLY in a new, bounded, disposable build VM. The external supervisor owns
the VM identity, deadline and cleanup. A candidate is NOT a published template.
Never run this with tenant input on a host; no Docker socket is used.
"""

import argparse
import json
import os
from pathlib import Path, PurePosixPath
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import threading
import time

if __package__:
    from .template_inputs import InvalidTemplate, bounded_json, extract_archive, extract_context, inspect_oci, sha256, startup_config
else:
    from template_inputs import InvalidTemplate, bounded_json, extract_archive, extract_context, inspect_oci, sha256, startup_config


MIB = 1024 * 1024
PLATFORM = "/opt/sandproof"
BOOT = """#!/opt/sandproof/busybox sh
set -eu
B=/opt/sandproof/busybox
$B mount -t proc proc /proc
$B mount -o remount,rw /
$B mount -t sysfs sysfs /sys
if ! $B grep -q ' /dev devtmpfs ' /proc/mounts; then $B mount -t devtmpfs devtmpfs /dev; fi
$B mkdir -p /dev/pts /run /tmp
$B mount -t devpts devpts /dev/pts
$B mount -t tmpfs -o mode=755 tmpfs /run
$B chmod 1777 /tmp
$B ln -sf /proc/self/fd /dev/fd
$B ln -sf /proc/self/fd/0 /dev/stdin
$B ln -sf /proc/self/fd/1 /dev/stdout
$B ln -sf /proc/self/fd/2 /dev/stderr
$B ip addr add 172.16.0.2/30 dev eth0
$B ip link set eth0 up
$B ip link set lo up
$B ip route add 169.254.169.254/32 dev eth0
$B ip route add default via 172.16.0.1 dev eth0
$B touch /run/sandproof-boot-ready
"""
INITTAB = """::sysinit:/opt/sandproof/boot
::respawn:/opt/sandproof/agent
::ctrlaltdel:/opt/sandproof/busybox reboot
::shutdown:/opt/sandproof/busybox umount -a -r
"""
AGENT = """#!/opt/sandproof/busybox sh
test -f /run/sandproof-boot-ready || exit 1
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
exec /opt/sandproof/envd -verbose -no-cgroups
"""


class LogBudget:
    def __init__(self, remaining):
        self.remaining = remaining
        self.lock = threading.Lock()

    def take(self, size):
        with self.lock:
            accepted = min(size, self.remaining)
            self.remaining -= accepted
            return accepted


class Commands:
    def __init__(self, deadline, log, max_log_bytes, *, budget=None):
        self.deadline, self.log = deadline, log
        self.budget = budget or LogBudget(max_log_bytes)

    def run(self, args, *, daemon=False):
        if time.monotonic() >= self.deadline:
            raise TimeoutError("build deadline")
        child = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                 stderr=subprocess.STDOUT, start_new_session=True,
                                 env={"PATH": "/usr/local/bin:/usr/sbin:/usr/bin:/bin", "LC_ALL": "C",
                                      "HOME": "/var/empty", "DOCKER_CONFIG": "/var/empty"})
        if daemon:
            return child
        try:
            self.drain(child)
            if child.wait() != 0:
                if time.monotonic() >= self.deadline:
                    raise TimeoutError("build deadline")
                raise RuntimeError("tool failed: " + Path(args[0]).name)
        finally:
            terminate(child)
            child.stdout.close()

    def drain(self, child):
        with selectors.DefaultSelector() as ready:
            ready.register(child.stdout, selectors.EVENT_READ)
            while ready.get_map():
                if time.monotonic() >= self.deadline:
                    raise TimeoutError("build deadline")
                for key, _ in ready.select(timeout=min(0.2, max(0, self.deadline - time.monotonic()))):
                    data = os.read(key.fd, 65536)
                    if not data:
                        ready.unregister(key.fileobj)
                    else:
                        accepted = self.budget.take(len(data))
                        self.log.write(data[:accepted])
                        self.log.flush()
                        if accepted < len(data):
                            raise InvalidTemplate("build log limit")


def terminate(child):
    if child is None:
        return
    if child.poll() is None:
        try:
            os.killpg(child.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait(timeout=3)


def guest_path(root, name):
    """Resolve image symlinks in guest coordinates, never host coordinates."""
    todo = list(PurePosixPath(name).parts)
    current, count = [], 0
    while todo:
        part = todo.pop(0)
        if part in ("/", "."):
            continue
        if part == "..":
            if not current:
                raise InvalidTemplate("guest path escapes root")
            current.pop()
            continue
        path = root.joinpath(*current, part)
        if path.is_symlink():
            count += 1
            if count > 40:
                raise InvalidTemplate("guest symlink loop")
            target = os.readlink(path)
            if target.startswith("/"):
                current = []
            todo = list(PurePosixPath(target).parts) + todo
        else:
            current.append(part)
    return root.joinpath(*current)


def install_platform(root, envd, busybox):
    release = guest_path(root, "/etc/os-release")
    if release.stat().st_size > 8192:
        raise InvalidTemplate("os-release limit")
    os_id = next((line[3:].strip('"') for line in release.read_text().splitlines() if line.startswith("ID=")), "")
    if os_id not in ("debian", "ubuntu"):
        raise InvalidTemplate("only Debian/Ubuntu compatible payloads supported")
    passwd = guest_path(root, "/etc/passwd")
    if passwd.stat().st_size > MIB or not any(line.startswith("root:x:0:0:") for line in passwd.read_text().splitlines()):
        raise InvalidTemplate("image needs a normal root account")
    platform = guest_path(root, PLATFORM)
    if platform != root / "opt/sandproof" or platform.exists():
        raise InvalidTemplate("reserved platform path occupied or redirected")
    platform.mkdir(parents=True, mode=0o755)
    for source, name in ((envd, "envd"), (busybox, "busybox")):
        shutil.copyfile(source, platform / name)
        (platform / name).chmod(0o755)
    for name, content in (("boot", BOOT), ("agent", AGENT)):
        (platform / name).write_text(content)
        (platform / name).chmod(0o755)
    # Intentionally replace the container's init selection, not arbitrary
    # resolved link targets. User CMD is recorded separately, never PID 1.
    sbin = guest_path(root, "/sbin")
    sbin.mkdir(parents=True, exist_ok=True)
    init = sbin / "init"
    if init.exists() and init.is_dir():
        raise InvalidTemplate("init is a directory")
    init.unlink(missing_ok=True)
    init.symlink_to(PLATFORM + "/busybox")
    etc = guest_path(root, "/etc")
    inittab = etc / "inittab"
    if inittab.is_symlink() or (inittab.exists() and not inittab.is_file()):
        raise InvalidTemplate("unsafe inittab target")
    inittab.write_text(INITTAB)
    inittab.chmod(0o644)
    for name in ("/proc", "/sys", "/dev", "/run", "/tmp"):
        path = guest_path(root, name)
        if path != root / name.lstrip("/"):
            raise InvalidTemplate("platform mount path redirected")
        path.mkdir(exist_ok=True)


def validate_request(request):
    expected = {"kind", "disk_mib", "min_free_mib", "max_input_bytes", "max_expanded_bytes", "max_entries",
                "timeout_seconds", "max_log_bytes"}
    if set(request) != expected or request["kind"] not in ("dockerfile", "oci"):
        raise InvalidTemplate("invalid worker request")
    bounds = {"disk_mib": (512, 32768), "min_free_mib": (16, 32768),
              "max_input_bytes": (1, 8 * 1024 * MIB), "max_expanded_bytes": (10240, 32 * 1024 * MIB),
              "max_entries": (1, 200000), "timeout_seconds": (1, 3600), "max_log_bytes": (1, 16 * MIB)}
    for field, (lower, upper) in bounds.items():
        if type(request[field]) is not int or not lower <= request[field] <= upper:
            raise InvalidTemplate("invalid budget: " + field)
    if request["min_free_mib"] >= request["disk_mib"]:
        raise InvalidTemplate("minimum free disk must be smaller than total")


def check_tools(profile):
    required = {"buildctl", "buildkitd", "buildkit-runc", "umoci", "envd", "busybox"}
    if set(profile) != required:
        raise InvalidTemplate("worker tool profile incomplete")
    tools = {}
    for name, artifact in profile.items():
        if set(artifact) != {"path", "sha256"}:
            raise InvalidTemplate("invalid tool descriptor")
        path = Path(artifact["path"])
        if (not path.is_absolute() or not stat.S_ISREG(path.lstat().st_mode)
                or sha256(path) != artifact["sha256"]):
            raise InvalidTemplate("tool fingerprint mismatch: " + name)
        tools[name] = path
    # BusyBox builds differ in enabled applets. Do not merely assume an applet
    # exists because the binary/version matched the operator's descriptor.
    applets = subprocess.check_output([str(tools["busybox"]), "--list"], timeout=5).decode().splitlines()
    if not {"sh", "init", "mount", "grep", "mkdir", "chmod", "ln", "ip", "touch", "reboot", "umount"}.issubset(applets):
        raise InvalidTemplate("BusyBox missing required platform applets")
    return tools


def build(work, source, request, profile, *, guest_boot_id):
    validate_request(request)
    # Accidental-host guard only, NOT remote attestation. The supervisor must
    # create a new VM, read that VM's boot_id and execute this worker there.
    if (sys.platform != "linux" or os.geteuid() != 0
            or Path("/proc/sys/kernel/random/boot_id").read_text().strip() != guest_boot_id
            or not Path("/dev/vda").is_block_device()):
        raise InvalidTemplate("worker requires its assigned disposable build VM")
    tools = check_tools(profile)
    if not stat.S_ISREG(source.lstat().st_mode) or source.stat().st_size > request["max_input_bytes"]:
        raise InvalidTemplate("input archive limit or type")
    if shutil.disk_usage(work.parent).free < request["disk_mib"] * MIB + request["max_expanded_bytes"] + 64 * MIB:
        raise InvalidTemplate("build disk budget unavailable")
    work.mkdir(mode=0o700)  # exact identity is new; never reuse another job/cache
    daemon = None
    state = {"state": "building", "ready": False}
    input_digest = sha256(source)
    filesystem_tools = {name: sha256(Path(name)) for name in ("/usr/sbin/mke2fs", "/usr/sbin/e2fsck")}
    (work / "status.json").write_text(json.dumps(state))
    try:
        with (work / "build.log").open("xb") as log:
            commands = Commands(time.monotonic() + request["timeout_seconds"], log, request["max_log_bytes"])
            limits = {"max_bytes": request["max_expanded_bytes"], "max_entries": request["max_entries"]}
            layout = work / "oci"
            if request["kind"] == "dockerfile":
                extract_context(source, work / "context", **limits)
                address = "unix://" + str(work / "buildkit.sock")
                daemon = commands.run([str(tools["buildkitd"]), "--root", str(work / "cache"), "--addr", address,
                                       "--oci-worker=true",
                                       "--oci-worker-binary", str(tools["buildkit-runc"]), "--oci-worker-snapshotter", "native",
                                       "--oci-worker-net", "host", "--containerd-worker=false"], daemon=True)
                # Daemon logging is intentionally a separate bounded file,
                # drained while the build client runs (see daemon reader below).
                daemon_error = []
                def read_daemon():
                    try:
                        with (work / "daemon.log").open("xb") as daemon_log:
                            Commands(commands.deadline, daemon_log, request["max_log_bytes"], budget=commands.budget).drain(daemon)
                    except Exception as error:
                        daemon_error.append(error)
                        terminate(daemon)
                    finally:
                        daemon.stdout.close()
                reader = threading.Thread(target=read_daemon, daemon=True)
                reader.start()
                try:
                    commands.run([str(tools["buildctl"]), "--addr", address, "--wait", "build", "--progress", "plain",
                                  "--frontend", "dockerfile.v0", "--local", "context=" + str(work / "context"),
                                  "--local", "dockerfile=" + str(work / "context"), "--opt", "platform=linux/amd64",
                                  "--output", "type=oci,tar=false,compression=gzip,dest=" + str(layout)])
                finally:
                    terminate(daemon)
                    reader.join(timeout=4)
                if reader.is_alive() or daemon_error:
                    raise RuntimeError("builder daemon did not finish cleanly")
            else:
                extract_archive(source, layout, **limits)
            image = inspect_oci(layout, **limits)
            startup = startup_config(image["container_config"])
            index = bounded_json(layout / "index.json")
            index["manifests"][0].setdefault("annotations", {})["org.opencontainers.image.ref.name"] = "candidate"
            (layout / "index.json").write_text(json.dumps(index))
            commands.run([str(tools["umoci"]), "unpack", "--image", str(layout) + ":candidate", str(work / "bundle")])
            root = work / "bundle/rootfs"
            install_platform(root, tools["envd"], tools["busybox"])
            disk = work / "candidate.ext4"
            with disk.open("xb") as output:
                output.truncate(request["disk_mib"] * MIB)
            commands.run(["/usr/sbin/mke2fs", "-t", "ext4", "-F", "-b", "4096", "-m", "0",
                          "-E", "lazy_itable_init=0,lazy_journal_init=0", "-d", str(root), str(disk)])
            commands.run(["/usr/sbin/e2fsck", "-fn", str(disk)])
            # Read structural accounting from the ext4 superblock. Do not use
            # builder filesystem free space as the candidate's free capacity.
            import struct
            with disk.open("rb") as output:
                output.seek(1024)
                superblock = output.read(1024)
            blocks, free, shift = (struct.unpack_from("<I", superblock, offset)[0] for offset in (4, 12, 24))
            if superblock[56:58] != b"\x53\xef" or shift != 2 or blocks * 4096 != request["disk_mib"] * MIB:
                raise InvalidTemplate("unexpected ext4 accounting")
            if free * 4096 < request["min_free_mib"] * MIB:
                raise InvalidTemplate("candidate minimum free disk not met")
            state = {"state": "candidate", "ready": False, **image, "startup": startup,
                     "startup_applied": False, "rootfs_sha256": sha256(disk), "disk_bytes": disk.stat().st_size,
                     "free_bytes": free * 4096, "min_free_bytes": request["min_free_mib"] * MIB,
                     "input_sha256": input_digest, "tools": profile, "filesystem_tools": filesystem_tools,
                     "requires": ["external VM cleanup", "startup defaults integration", "launch/envd/owner publish gate"]}
            disk.chmod(0o444)
    except BaseException as error:
        state = {"state": "failed", "ready": False, "error_type": type(error).__name__}
        raise
    finally:
        terminate(daemon)
        (work / "status.json").write_text(json.dumps(state, indent=2))
    return state


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("work", "source", "request", "profile"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--guest-boot-id", required=True)
    args = parser.parse_args()
    print(json.dumps(build(args.work, args.source, bounded_json(args.request), bounded_json(args.profile),
                           guest_boot_id=args.guest_boot_id)))
