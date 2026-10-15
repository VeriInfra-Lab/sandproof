#!/usr/bin/env python3
"""Guest-only, one-shot BuildKit/umoci/ext4 candidate worker (#123).

Run ONLY in a new, bounded, disposable build VM. The external supervisor owns
the VM identity, deadline and cleanup. A candidate is NOT a published template.
Never run this with tenant input on a host; no Docker socket is used.
"""

import argparse
from contextlib import contextmanager
import ipaddress
import json
import os
import re
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
    from .template_inputs import MAX_INPUT_BYTES, InvalidTemplate, blob, bounded_json, extract_archive, extract_context, inspect_oci, sha256, startup_config, platform_dns_servers, validate_tool_names
else:
    from template_inputs import MAX_INPUT_BYTES, InvalidTemplate, blob, bounded_json, extract_archive, extract_context, inspect_oci, sha256, startup_config, platform_dns_servers, validate_tool_names


MIB = 1024 * 1024
PLATFORM = "/opt/sandproof"
BOOT = """#!/opt/sandproof/busybox sh
set -eu
B=/opt/sandproof/busybox
$B mount -t proc proc /proc
$B mount -o remount,rw /
# The root can initially be read-only. Invalidate before the remaining setup;
# AGENT also rejects on-disk markers if either preceding mount fails.
$B rm -f /run/sandproof-boot-ready
$B mount -t sysfs sysfs /sys
if ! $B grep -q ' /dev devtmpfs ' /proc/mounts; then $B mount -t devtmpfs devtmpfs /dev; fi
$B mkdir -p /dev/pts /dev/shm /run /tmp
$B mount -t devpts devpts /dev/pts
$B mount -t tmpfs -o nosuid,nodev,noexec,mode=1777,size=64m tmpfs /dev/shm
$B mount -t tmpfs -o mode=755 tmpfs /run
$B mkdir -p /run/lock
$B chmod 1777 /run/lock
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
B=/opt/sandproof/busybox
# Candidates cold-boot from ext4: imported /run contents are not readiness.
# Check the actual filesystem, not an image-supplied /proc/mounts file.
test ! -L /run || exit 1
run_type=$($B stat -f -c %t /run) || exit 1
test "$run_type" = 1021994 || exit 1
test ! -L /run/sandproof-boot-ready || exit 1
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


COMMAND_ENV = {"PATH": "/usr/local/bin:/usr/sbin:/usr/bin:/bin", "LC_ALL": "C",
               "HOME": "/var/empty", "DOCKER_CONFIG": "/var/empty"}


class Commands:
    def __init__(self, deadline, log, max_log_bytes, *, budget=None):
        self.deadline, self.log = deadline, log
        self.budget = budget or LogBudget(max_log_bytes)
        self.started = time.monotonic()

    @contextmanager
    def phase(self, name):
        """Flush bounded diagnostics before work; preserve its original failure."""
        started = time.monotonic()

        def event(state):
            data = ("\n[sandproof-phase] " + json.dumps({
                "phase": name, "state": state,
                "started_seconds": round(started - self.started, 6),
                "elapsed_seconds": round(time.monotonic() - started, 6),
            }, separators=(",", ":")) + "\n").encode()
            accepted = self.budget.take(len(data))
            self.log.write(data[:accepted])
            self.log.flush()
            if accepted < len(data):
                raise InvalidTemplate("build log limit")

        event("started")
        try:
            yield
        except BaseException:
            try:
                event("failed")
            except Exception:
                pass  # Diagnostic I/O or exhausted budget must not hide the cause.
            raise
        else:
            event("completed")

    def run(self, args, *, daemon=False, env=None, cwd=None):
        if time.monotonic() >= self.deadline:
            raise TimeoutError("build deadline")
        child = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                 stderr=subprocess.STDOUT, start_new_session=True,
                                 env=COMMAND_ENV if env is None else env, cwd=cwd)
        if daemon:
            return child
        try:
            self.drain(child)
            # EOF only closes the log stream; the tool can still be running.
            # Waiting for it must use the same deadline, including a late exit.
            remaining = self.deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("build deadline")
            try:
                result = child.wait(timeout=remaining)
            except subprocess.TimeoutExpired as error:
                raise TimeoutError("build deadline") from error
            if time.monotonic() >= self.deadline:
                raise TimeoutError("build deadline")
            if result != 0:
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


def install_platform(root, envd, busybox, dns_servers=None):
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
    install_loopback_hosts(root)
    if dns_servers:
        resolver = etc / "resolv.conf"
        if resolver.exists() and resolver.is_dir():
            raise InvalidTemplate("resolv.conf is a directory")
        resolver.unlink(missing_ok=True)  # replace container link, not its target
        resolver.write_text("".join("nameserver " + v + "\n" for v in dns_servers))
        resolver.chmod(0o644)
    for name in ("/proc", "/sys", "/dev", "/run", "/tmp"):
        path = guest_path(root, name)
        if path != root / name.lstrip("/"):
            raise InvalidTemplate("platform mount path redirected")
        path.mkdir(exist_ok=True)
    # Remove only this reserved directory entry, never its symlink/hardlink
    # target. A directory or failed unlink rejects the candidate. The checked
    # /run parent cannot redirect this write out of the candidate tree.
    (root / "run/sandproof-boot-ready").unlink(missing_ok=True)


def install_loopback_hosts(root):
    """Supply the container runtime's localhost entry in the candidate VM."""
    hosts = guest_path(root, "/etc") / "hosts"
    content = b""
    if not hosts.is_symlink() and hosts.exists():
        if not hosts.is_file() or hosts.stat().st_size > 65536:
            raise InvalidTemplate("hosts file type or size limit")
        content = hosts.read_bytes()
    ipv4_localhost = False
    for line in content.splitlines():
        fields = line.split(b"#", 1)[0].split()
        if len(fields) < 2 or b"localhost" not in [word.lower() for word in fields[1:]]:
            continue
        try:
            address = ipaddress.ip_address(fields[0].decode("ascii"))
        except (ValueError, UnicodeDecodeError) as error:
            raise InvalidTemplate("invalid localhost address") from error
        if not address.is_loopback:
            raise InvalidTemplate("localhost must resolve to loopback")
        ipv4_localhost |= address.version == 4
    if not ipv4_localhost:
        content += (b"\n" if content and not content.endswith(b"\n") else b"") + b"127.0.0.1 localhost\n"
    if len(content) > 65536:
        raise InvalidTemplate("hosts file size limit")
    # Replace the directory entry, never follow a container symlink or alter
    # another image file that was hard-linked to hosts. The worker is guest-only.
    hosts.unlink(missing_ok=True)
    with hosts.open("xb") as output:
        output.write(content)
    hosts.chmod(0o644)


def runtime_defaults(root, startup):
    """Normalize to fixed envd's named-user contract, not arbitrary UID:GID.

    Resolution is guest-only, bounded and fail-closed. An explicit alternate
    primary group is unsupported; silently dropping it changes permissions.
    No image environment is ever inherited by trusted PID 1 or envd itself.
    """
    def accounts(filename, count):
        source = guest_path(root, filename)
        if not source.is_file() or source.stat().st_size > MIB:
            raise InvalidTemplate("account database type or size")
        entries = [line.split(":") for line in source.read_text().splitlines() if line and not line.startswith("#")]
        if any(len(row) != count for row in entries):
            raise InvalidTemplate("malformed account database")
        return entries
    users = accounts("/etc/passwd", 7)
    user, colon, group = startup["User"].partition(":")
    matches = [row for row in users if (row[2] == str(int(user)) if user.isascii() and user.isdecimal() else row[0] == user)]
    if len(matches) != 1:
        raise InvalidTemplate("USER must resolve to one real account")
    selected = matches[0]
    if (not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]{0,63}", selected[0])
            or any(not value.isascii() or not value.isdecimal() or int(value) > 2**32-1 for value in selected[2:4])):
        raise InvalidTemplate("unsupported user account")
    if colon:
        groups = accounts("/etc/group", 4)
        matched = [row for row in groups if (row[2] == str(int(group)) if group.isascii() and group.isdecimal() else row[0] == group)]
        if len(matched) != 1 or matched[0][2] != selected[3]:
            raise InvalidTemplate("alternate USER primary group unsupported")
    directory = guest_path(root, startup["WorkingDir"])
    if not directory.is_dir():
        raise InvalidTemplate("WORKDIR does not exist in image")
    env = dict(item.split("=", 1) for item in startup["Env"])
    argv = startup["Entrypoint"] + startup["Cmd"]
    if len(argv) > 256 or (argv and not argv[0]):
        raise InvalidTemplate("invalid startup argv")
    if (any(len(key.encode()) + len(value.encode()) > 8192 for key, value in env.items())
            or sum(len(key.encode()) + len(value.encode()) for key, value in env.items())
            + sum(len(arg.encode()) for arg in argv) > 32 * 1024):
        raise InvalidTemplate("startup defaults byte limit")
    return {"env_vars": env, "user": selected[0], "workdir": startup["WorkingDir"], "argv": argv}


def validate_request(request):
    expected = {"kind", "disk_mib", "min_free_mib", "max_input_bytes", "max_expanded_bytes", "max_entries",
                "timeout_seconds", "max_log_bytes"}
    if set(request) - {"dns_servers", "docker_hub_mirror"} != expected or request["kind"] not in ("dockerfile", "oci", "oci-reference"):
        raise InvalidTemplate("invalid worker request")
    bounds = {"disk_mib": (512, 32768), "min_free_mib": (16, 32768),
              "max_input_bytes": (1, MAX_INPUT_BYTES), "max_expanded_bytes": (10240, 32 * 1024 * MIB),
              "max_entries": (1, 200000), "timeout_seconds": (1, 3600), "max_log_bytes": (1, 16 * MIB)}
    for field, (lower, upper) in bounds.items():
        if type(request[field]) is not int or not lower <= request[field] <= upper:
            raise InvalidTemplate("invalid budget: " + field)
    if request["min_free_mib"] >= request["disk_mib"]:
        raise InvalidTemplate("minimum free disk must be smaller than total")
    if "dns_servers" in request:
        platform_dns_servers(request["dns_servers"])
    if "docker_hub_mirror" in request:
        if request["kind"] != "oci-reference":
            raise InvalidTemplate("registry mirror only applies to public OCI acquisition")
        if __package__:
            from .template_fetch import mirror_registry
        else:
            from template_fetch import mirror_registry
        mirror_registry(request["docker_hub_mirror"])


def check_tools(profile, *, registry=False):
    validate_tool_names(profile, registry=registry)
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
    if not {"sh", "init", "mount", "grep", "mkdir", "chmod", "ln", "ip", "touch", "reboot", "umount",
            "rm", "stat"}.issubset(applets):
        raise InvalidTemplate("BusyBox missing required platform applets")
    # stat can exist without filesystem/format support in a custom BusyBox.
    # Probe the exact read-only operation used by the cold-boot envd gate.
    filesystem = subprocess.check_output([str(tools["busybox"]), "stat", "-f", "-c", "%t", "/"], timeout=5)
    if not re.fullmatch(rb"[0-9a-fA-F]+\n?", filesystem):
        raise InvalidTemplate("BusyBox filesystem stat format unavailable")
    return tools


def buildkit_environment(tools, work):
    """Select only the operator's checked decoder, in this disposable VM."""
    env = dict(COMMAND_ENV, CONTAINERD_DISABLE_IGZIP="1", CONTAINERD_DISABLE_PIGZ="1")
    if "igzip" in tools:
        directory = work / "gzip-tools"
        directory.mkdir(mode=0o700)
        # Canonical names let containerd and the dynamic loader find the two
        # checked artifacts even when their original filenames include versions.
        for name in ("igzip", "libisal.so.2"):
            os.link(tools[name], directory / name, follow_symlinks=False)
        env.update(PATH=str(directory) + ":" + COMMAND_ENV["PATH"],
                   LD_LIBRARY_PATH=str(directory), CONTAINERD_DISABLE_IGZIP="0")
    return env


def buildkit_layout(cache, metadata, layout, *, max_bytes):
    """Link one stopped, private BuildKit 0.28 OCI worker's image for validation.

    The image exporter commits the exact manifest/config/layers to its content
    store. Avoid copying those bytes through the OCI exporter back into the same
    VM. This is not a validation cache: inspect_oci must still validate all blob
    digests, diff_ids, tar members and cumulative limits before unpacking.
    """
    def directory(path):
        if not stat.S_ISDIR(path.lstat().st_mode):
            raise InvalidTemplate("BuildKit content directory must not be a link")
    directory(cache)
    stores = []
    for name in ("runc-overlayfs", "runc-fuse-overlayfs", "runc-native"):
        worker = cache / name
        if not worker.exists() and not worker.is_symlink():
            continue
        path = worker
        directory(path)
        for component in ("content", "blobs", "sha256"):
            path /= component
            directory(path)
        stores.append(path)
    if len(stores) != 1:
        raise InvalidTemplate("expected one private BuildKit content store")
    exported = bounded_json(metadata)
    if not isinstance(exported, dict):
        raise InvalidTemplate("invalid BuildKit export metadata")
    descriptor = exported.get("containerimage.descriptor", {})
    media = "application/vnd.oci.image.manifest.v1+json"
    if (not isinstance(descriptor, dict) or descriptor.get("mediaType") != media
            or descriptor.get("digest") != exported.get("containerimage.digest")):
        raise InvalidTemplate("expected one exported OCI manifest")
    layout.mkdir(mode=0o700)
    (layout / "blobs").mkdir(mode=0o700)
    target_blobs = layout / "blobs/sha256"
    target_blobs.mkdir(mode=0o700)
    def link(entry):
        if not isinstance(entry, dict):
            raise InvalidTemplate("invalid BuildKit descriptor")
        digest, size = entry.get("digest", ""), entry.get("size")
        if (not isinstance(digest, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", digest)
                or type(size) is not int or not 0 < size <= max_bytes or entry.get("urls")):
            raise InvalidTemplate("invalid BuildKit descriptor")
        source = stores[0] / digest[7:]
        before = source.lstat()
        if not stat.S_ISREG(before.st_mode) or before.st_size != size:
            raise InvalidTemplate("invalid BuildKit content file")
        target = target_blobs / digest[7:]
        if not target.exists() and not target.is_symlink():
            os.link(source, target, follow_symlinks=False)
        after = target.lstat()
        if (not stat.S_ISREG(after.st_mode) or after.st_size != size
                or (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino)):
            raise InvalidTemplate("BuildKit content identity changed")
        return target
    link(descriptor)
    manifest = bounded_json(blob(layout, descriptor, max_bytes))
    if not isinstance(manifest, dict):
        raise InvalidTemplate("invalid BuildKit image manifest")
    config, layers = manifest.get("config", {}), manifest.get("layers", [])
    if (manifest.get("schemaVersion") != 2 or not isinstance(config, dict)
            or config.get("digest") != exported.get("containerimage.config.digest")
            or config.get("mediaType") != "application/vnd.oci.image.config.v1+json"
            or not isinstance(layers, list) or not 1 <= len(layers) <= 128):
        raise InvalidTemplate("invalid BuildKit image manifest")
    link(config)
    for layer in layers:
        link(layer)
    (layout / "oci-layout").write_text(json.dumps({"imageLayoutVersion": "1.0.0"}))
    (layout / "index.json").write_text(json.dumps({"schemaVersion": 2, "manifests": [descriptor]}))


def unpack_rootfs(commands, umoci, layout, bundle):
    # Both fixed umoci commands use UnpackRootfs with the same ID mappings and
    # whiteout handling. We never repack this bundle, so omit the extra full-tree
    # mtree hashing and unused OCI runtime config. Startup comes from the
    # validated image config; this VM boots the separately installed platform.
    bundle.mkdir(mode=0o700)
    root = bundle / "rootfs"
    commands.run([str(umoci), "raw", "unpack", "--image", str(layout) + ":candidate", str(root)])
    return root


def loop_backing(device):
    path = Path(f"/sys/dev/block/{os.major(device)}:{os.minor(device)}/loop/backing_file")
    try:
        return path.read_text()
    except FileNotFoundError:
        return None


def unmount_ext4(commands, mount):
    device = mount.stat().st_dev
    backing = loop_backing(device)
    commands.run(["/usr/bin/umount", "--detach-loop", str(mount)])
    if os.path.ismount(mount):
        raise RuntimeError("candidate filesystem remains mounted")
    # Linux loop autoclear can finish after umount returns. Observe only this
    # device's original association; never detach a subsequently reused device.
    while backing is not None and loop_backing(device) == backing:
        remaining = commands.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("candidate loop cleanup deadline")
        time.sleep(min(0.01, remaining))


def populate_ext4(commands, root, disk):
    """Populate a new filesystem inside the disposable build VM only."""
    mount = disk.parent / "candidate-mount"
    mount.mkdir(mode=0o700)
    commands.run(["/usr/sbin/mke2fs", "-t", "ext4", "-F", "-b", "4096", "-m", "0",
                  "-E", "lazy_itable_init=0,lazy_journal_init=0", str(disk)])
    try:
        commands.run(["/usr/bin/mount", "-t", "ext4", "-o", "loop,nosuid,nodev,noexec",
                      str(disk), str(mount)])
        if not os.path.ismount(mount):
            raise RuntimeError("candidate filesystem was not mounted")
        # Explicit xattr preservation makes failures fatal (preserve=all can
        # ignore them). Never follow image symlinks or read special-file data.
        commands.run(["/usr/bin/cp", "--recursive", "--no-dereference",
                      "--preserve=mode,ownership,timestamps,links,xattr",
                      "--sparse=always", "--reflink=never", "--", str(root) + "/.", str(mount)])
        # A successful copy can leave dirty filesystem data. Completing its
        # unmount is build work and retains the original remaining deadline.
        # The short grace below is only for reclamation after failure.
        unmount_ext4(commands, mount)
    finally:
        failure = sys.exception()
        try:
            if os.path.ismount(mount):
                # Only reclamation gets a separate bounded grace period; it
                # cannot extend the job deadline or allow a late candidate.
                cleanup = Commands(time.monotonic() + 5, commands.log, 0, budget=commands.budget)
                unmount_ext4(cleanup, mount)
            mount.rmdir()
        except Exception:
            if failure is None:
                raise
            failure.add_note("candidate mount cleanup failed; external VM cleanup required")


def build(work, source, request, profile, *, guest_boot_id):
    validate_request(request)
    # Accidental-host guard only, NOT remote attestation. The supervisor must
    # create a new VM, read that VM's boot_id and execute this worker there.
    if (sys.platform != "linux" or os.geteuid() != 0
            or Path("/proc/sys/kernel/random/boot_id").read_text().strip() != guest_boot_id
            or not Path("/dev/vda").is_block_device()):
        raise InvalidTemplate("worker requires its assigned disposable build VM")
    tools = check_tools(profile, registry=request["kind"] == "oci-reference")
    if not stat.S_ISREG(source.lstat().st_mode) or source.stat().st_size > request["max_input_bytes"]:
        raise InvalidTemplate("input archive limit or type")
    download_space = 2 * request["max_input_bytes"] if request["kind"] == "oci-reference" else 0
    if shutil.disk_usage(work.parent).free < request["disk_mib"] * MIB + request["max_expanded_bytes"] + download_space + 64 * MIB:
        raise InvalidTemplate("build disk budget unavailable")
    work.mkdir(mode=0o700)  # exact identity is new; never reuse another job/cache
    daemon = None
    state = {"state": "building", "ready": False}
    input_digest = sha256(source)
    filesystem_tools = {name: sha256(Path(name)) for name in (
        "/usr/sbin/mke2fs", "/usr/sbin/e2fsck", "/usr/bin/cp", "/usr/bin/mount", "/usr/bin/umount")}
    (work / "status.json").write_text(json.dumps(state))
    try:
        with (work / "build.log").open("xb") as log:
            commands = Commands(time.monotonic() + request["timeout_seconds"], log, request["max_log_bytes"])
            limits = {"max_bytes": request["max_expanded_bytes"], "max_entries": request["max_entries"]}
            layout = work / "oci"
            public_source = None
            with commands.phase("acquire"):
                if request["kind"] == "oci-reference":
                    if __package__:
                        from .template_fetch import fetch
                    else:
                        from template_fetch import fetch
                    public_source, image = fetch(source, work, layout, request, tools["regctl"], commands)
                elif request["kind"] == "dockerfile":
                    extract_context(source, work / "context", **limits)
                    address = "unix://" + str(work / "buildkit.sock")
                    builder_env = buildkit_environment(tools, work)
                    if "igzip" in tools:
                        # Fail before downloading layers if the pinned binary
                        # or its library cannot run in this builder image.
                        commands.run([str(work / "gzip-tools/igzip"), "--version"], env=builder_env)
                    daemon = commands.run([str(tools["buildkitd"]), "--root", str(work / "cache"), "--addr", address,
                                           "--oci-worker=true",
                                           "--oci-worker-binary", str(tools["buildkit-runc"]), "--oci-worker-snapshotter", "auto",
                                           "--oci-worker-net", "host", "--containerd-worker=false"], daemon=True,
                                          env=builder_env)
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
                                      "--metadata-file", str(work / "image-metadata.json"),
                                      "--output", "type=image,oci-mediatypes=true,compression=gzip,push=false,store=false"])
                    finally:
                        terminate(daemon)
                        reader.join(timeout=4)
                    if reader.is_alive() or daemon_error:
                        raise RuntimeError("builder daemon did not finish cleanly")
                    buildkit_layout(work / "cache", work / "image-metadata.json", layout,
                                    max_bytes=request["max_expanded_bytes"])
                else:
                    extract_archive(source, layout, **limits)
            if request["kind"] != "oci-reference":
                with commands.phase("oci_validate"):
                    image = inspect_oci(layout, **limits)
            if request["kind"] == "dockerfile":
                # The daemon has stopped and the selected blobs have their own
                # validated layout links. Drop only this job's unused snapshots
                # before allocating another rootfs and the candidate disk.
                with commands.phase("build_cache_cleanup"):
                    shutil.rmtree(work / "cache")
            startup = startup_config(image["container_config"])
            index = bounded_json(layout / "index.json")
            index["manifests"][0].setdefault("annotations", {})["org.opencontainers.image.ref.name"] = "candidate"
            (layout / "index.json").write_text(json.dumps(index))
            with commands.phase("rootfs_unpack"):
                root = unpack_rootfs(commands, tools["umoci"], layout, work / "bundle")
            with commands.phase("platform_install"):
                install_platform(root, tools["envd"], tools["busybox"], request.get("dns_servers"))
                defaults = runtime_defaults(root, startup)
            disk = work / "candidate.ext4"
            with disk.open("xb") as output:
                output.truncate(request["disk_mib"] * MIB)
            with commands.phase("ext4_create"):
                populate_ext4(commands, root, disk)
            with commands.phase("ext4_check"):
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
            with commands.phase("disk_digest"):
                rootfs_digest = sha256(disk)
            state = {"state": "candidate", "ready": False, **image, "startup": startup, "runtime_defaults": defaults,
                     "startup_applied": False, "rootfs_sha256": rootfs_digest, "disk_bytes": disk.stat().st_size,
                     "free_bytes": free * 4096, "min_free_bytes": request["min_free_mib"] * MIB,
                     "input_sha256": input_digest, "tools": profile, "filesystem_tools": filesystem_tools,
                     "requires": ["external VM cleanup", "launch/envd/owner publish gate"]}
            if public_source is not None:
                if image["image_digest"] != public_source.get("converted_digest", public_source["selected_digest"]):
                    raise InvalidTemplate("public image provenance mismatch")
                state["public_source"] = public_source
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
