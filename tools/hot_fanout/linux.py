"""Linux resources for cooperative, same-trust-domain hot fan-out.

The batched single-threaded donor and per-child OverlayFS root follow DeltaBox
at 1054de552758229f34daebd34f696da0d56e7919 (MIT, see DELTABOX-LICENSE).
A trusted template supplies the prepared process and frozen root layers.
Sandproof places each child in its reserved cgroup in the fork syscall.
"""

import ctypes
import fcntl
import hashlib
import http.client
import json
import logging
import os
from pathlib import Path
import re
import select
import shutil
import signal
import sys
import socket
import stat
import time

from .control import identifier, private_directory

ENVD_SHA256 = "cef25869662d730fcfe0df28dc1a43d1cf019abd9285d1aa33505940b6adc749"
MAX_MESSAGE = 16384
CLONE_NEWNS = 0x00020000
CLONE_NEWPID = 0x20000000
CLONE_INTO_CGROUP = 0x200000000
SYS_CLONE3 = 435  # Linux x86_64 and aarch64.
MS_NOSUID = 2
MS_NODEV = 4
MS_NOEXEC = 8
MS_REC = 16384
MS_PRIVATE = 1 << 18
PR_SET_NO_NEW_PRIVS = 38
PR_CAPBSET_DROP = 24
MNT_DETACH = 2
DEVICE_CAPS = {0, 1, 3, 4, 5, 6, 7}
OVERLAY_PATH = re.compile(r"/[A-Za-z0-9_./-]+")
LOGGER = logging.getLogger(__name__)


def _checked(result, operation):
    if result == -1:
        raise OSError(ctypes.get_errno(), operation)


def _mount(libc, source, target, kind=None, flags=0, options=None):
    def encode(value):
        return None if value is None else os.fsencode(value)

    _checked(libc.mount(encode(source), encode(target), encode(kind),
                        ctypes.c_ulong(flags), encode(options)), "mount fan-out filesystem")


def _pivot(libc):
    # pivot_root(".", ".") stacks the old root above the overlay mount.
    # Detaching it before workload execution removes the guest control root.
    numbers = {"x86_64": 155, "aarch64": 41}
    syscall = numbers.get(os.uname().machine)
    if syscall is None:
        raise RuntimeError("unsupported pivot_root architecture")
    _checked(libc.syscall(ctypes.c_long(syscall), b".", b"."), "pivot_root fan-out")
    _checked(libc.umount2(b".", MNT_DETACH), "detach fan-out old root")
    os.chdir("/")


def _restrict(libc, last_cap):
    for cap in range(last_cap + 1):
        if cap not in DEVICE_CAPS:
            _checked(libc.prctl(PR_CAPBSET_DROP, cap, 0, 0, 0), "drop fan-out capability")
    _checked(libc.prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), "set fan-out no_new_privs")

    class CapHeader(ctypes.Structure):
        _fields_ = [("version", ctypes.c_uint32), ("pid", ctypes.c_int)]

    class CapData(ctypes.Structure):
        _fields_ = [("effective", ctypes.c_uint32), ("permitted", ctypes.c_uint32),
                    ("inheritable", ctypes.c_uint32)]

    header = CapHeader(0x20080522, 0)
    data = (CapData * 2)()
    _checked(libc.capget(ctypes.byref(header), data), "read fan-out capabilities")
    mask = sum(1 << cap for cap in DEVICE_CAPS)
    data[0] = CapData(data[0].effective & mask, data[0].permitted & mask, 0)
    data[1] = CapData(0, 0, 0)
    _checked(libc.capset(ctypes.byref(header), data), "set fan-out capabilities")


def _root(libc, slot, lowers, last_cap):
    _checked(libc.unshare(CLONE_NEWNS), "create fan-out mount namespace")
    _mount(libc, None, "/", flags=MS_PRIVATE | MS_REC)
    options = (f"lowerdir={':'.join(map(str, lowers))},upperdir={slot / 'upper'},"
               f"workdir={slot / 'work'},index=off,metacopy=off,redirect_dir=off")
    if len(os.fsencode(options)) >= os.sysconf("SC_PAGE_SIZE"):
        raise ValueError("fan-out overlay paths exceed kernel option limit")
    merged = slot / "root"
    _mount(libc, "overlay", merged, "overlay", options=options)
    os.chdir(merged)
    _pivot(libc)
    for target in ("/dev", "/proc", "/run", "/tmp"):
        # Reject mountpoint indirection from a prepared root image.
        if not stat.S_ISDIR(os.lstat(target).st_mode):
            raise ValueError("fan-out root mountpoint must be a directory")
    restricted = MS_NODEV | MS_NOSUID | MS_NOEXEC
    # Stock envd raises each command's own oom_score_adj before exec.
    # This procfs belongs to the branch PID namespace and exposes task
    # entries only, so VM-global controls never become writable here.
    _mount(libc, "proc", "/proc", "proc", flags=restricted, options="subset=pid")
    if (not os.path.isfile("/proc/self/oom_score_adj")
            or os.path.lexists("/proc/sys") or os.path.lexists("/proc/sysrq-trigger")):
        raise ValueError("fan-out procfs did not apply its task-only scope")
    _mount(libc, "tmpfs", "/dev", "tmpfs", flags=MS_NOSUID | MS_NOEXEC, options="mode=0755")
    nodes = (("null", 1, 3), ("zero", 1, 5), ("full", 1, 7),
             ("random", 1, 8), ("urandom", 1, 9), ("tty", 5, 0))
    for name, major, minor in nodes:
        path = "/dev/" + name
        os.mknod(path, stat.S_IFCHR | 0o666, os.makedev(major, minor))
        os.chmod(path, 0o666)
    os.makedirs("/dev/pts")
    _mount(libc, "devpts", "/dev/pts", "devpts", flags=MS_NOSUID | MS_NOEXEC,
           options="newinstance,ptmxmode=0666,mode=0620")
    os.symlink("pts/ptmx", "/dev/ptmx")
    os.symlink("/proc/self/fd", "/dev/fd")
    for index, name in enumerate(("stdin", "stdout", "stderr")):
        os.symlink(f"/proc/self/fd/{index}", "/dev/" + name)
    os.mkdir("/dev/shm")
    _mount(libc, "tmpfs", "/dev/shm", "tmpfs", flags=restricted, options="mode=1777")
    _mount(libc, "tmpfs", "/run", "tmpfs", flags=restricted, options="mode=0755")
    os.mkdir("/run/lock", 0o755)
    _mount(libc, "tmpfs", "/tmp", "tmpfs", flags=MS_NODEV | MS_NOSUID, options="mode=1777")
    _restrict(libc, last_cap)
    fd = os.open("/dev/null", os.O_RDWR | os.O_CLOEXEC)
    try:
        for standard in (0, 1, 2):
            os.dup2(fd, standard)
    finally:
        os.close(fd)


def _write_control(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    try:
        if os.write(fd, value) != len(value):
            raise OSError("short fan-out control write")
    finally:
        os.close(fd)


class CloneArgs(ctypes.Structure):
    """Linux struct clone_args version 2, including the cgroup field."""
    _fields_ = [(name, ctypes.c_uint64) for name in (
        "flags", "pidfd", "child_tid", "parent_tid", "exit_signal", "stack",
        "stack_size", "tls", "set_tid", "set_tid_size", "cgroup",
    )]


def _spawn_member(cgroup, work):
    """Start a new process directly inside one reserved cgroup v2 scope."""
    fd = os.open(cgroup, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW)
    args = CloneArgs(flags=CLONE_INTO_CGROUP, exit_signal=signal.SIGCHLD, cgroup=fd)
    # PyDLL keeps the GIL while crossing the syscall. The donor is verified
    # single-threaded and the CPython fork hooks reset interpreter state.
    api = ctypes.pythonapi
    syscall = ctypes.PyDLL(None, use_errno=True).syscall
    syscall.restype = ctypes.c_long
    api.PyOS_BeforeFork.restype = None
    api.PyOS_AfterFork_Parent.restype = None
    api.PyOS_AfterFork_Child.restype = None
    try:
        api.PyOS_BeforeFork()
        pid = syscall(ctypes.c_long(SYS_CLONE3), ctypes.byref(args),
                      ctypes.c_size_t(ctypes.sizeof(args)))
        error = ctypes.get_errno()
        if pid == 0:
            api.PyOS_AfterFork_Child()
            os.close(fd)
            try:
                work()
            except OSError as error:
                LOGGER.warning("fan-out child setup failed (errno %s)", error.errno)
            except (RuntimeError, ValueError):
                LOGGER.warning("fan-out child setup rejected")
            finally:
                os._exit(1)
        api.PyOS_AfterFork_Parent()
        if pid < 0:
            raise OSError(error, "clone3 into fan-out cgroup")
    finally:
        os.close(fd)


def _wait_exit(pid):
    while True:
        try:
            _, status = os.waitpid(pid, 0)
            return os.waitstatus_to_exitcode(status)
        except InterruptedError:
            continue


def _enter_branch(slot, lowers, envd, last_cap, port, branch_id, workload):
    libc = ctypes.CDLL(None, use_errno=True)
    _checked(libc.unshare(CLONE_NEWPID), "create fan-out PID namespace")
    init = os.fork()
    if init != 0:
        os.close(envd)
        os._exit(max(0, _wait_exit(init)))
    _root(libc, slot, lowers, last_cap)
    environment = {"PATH": "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin", "LANG": "C.UTF-8"}
    envd_pid = os.fork()
    if envd_pid == 0:
        try:
            os.execve(envd, ["envd", "-isnotfc", "-no-cgroups", "-port", str(port)], environment)
        finally:
            os._exit(127)
    os.close(envd)
    os.environ.clear()
    os.environ.update(environment)
    if workload is not None:
        workload(branch_id)
    os._exit(max(0, _wait_exit(envd_pid)))


def _encode(value):
    data = json.dumps(value, separators=(",", ":")).encode()
    if len(data) > MAX_MESSAGE:
        raise ValueError("invalid fan-out message length")
    return data


def _decode(data):
    def unique_keys(items):
        result = dict(items)
        if len(result) != len(items):
            raise ValueError("duplicate fan-out message field")
        return result

    return json.loads(data, object_pairs_hook=unique_keys)


def _packet(channel, value):
    data = _encode(value)
    if channel.send(data) != len(data):
        raise OSError("short fan-out packet write")


def _receive(channel):
    data, _, flags, _ = channel.recvmsg(MAX_MESSAGE)
    if flags & socket.MSG_TRUNC:
        raise ValueError("truncated fan-out packet")
    return _decode(data) if data else None


def _reap_children(signum, frame):
    while True:
        try:
            if os.waitpid(-1, os.WNOHANG)[0] == 0:
                return
        except ChildProcessError:
            return


def _status(port, method, path, timeout, *, token=None, body=None):
    headers = {}
    if token is not None:
        headers["X-Access-Token"] = token
    if body is not None:
        headers["Content-Type"] = "application/json"
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    try:
        connection.request(method, path, body=body, headers=headers)
        return connection.getresponse().status
    finally:
        connection.close()


def _owns_tcp_listener(cgroup, port, *, observe=False):
    # Ports alone are not readiness or ownership evidence. Check that a
    # listening socket is held by a process in the still-reserved ordinal.
    pids = (cgroup / "cgroup.procs").read_text().splitlines()
    if not pids:
        return False
    wanted = f"{port:04X}"
    listening = set()
    for table in ("/proc/net/tcp", "/proc/net/tcp6"):
        try:
            lines = Path(table).read_text().splitlines()[1:]
        except FileNotFoundError:
            if table == "/proc/net/tcp6":
                continue
            raise
        for line in lines:
            fields = line.split()
            if fields[1].endswith(":" + wanted) and fields[3] == "0A":
                listening.add(f"socket:[{fields[9]}]")
    origins = set()
    for pid in pids:
        try:
            for fd in (Path("/proc") / pid / "fd").iterdir():
                try:
                    origin = os.readlink(fd)
                except FileNotFoundError:
                    continue
                if origin in listening:
                    origins.add(origin)
        except FileNotFoundError:
            continue
    if observe and listening and listening != origins:
        raise OSError("original fan-out listener ownership is uncertain")
    return bool(listening) and listening == origins


class LinuxGuest:
    """Bind one pre-prepared live donor and frozen root layers to the guest gate."""

    def __init__(self, data, cgroups, lowers, envd, *, startup_timeout=10, cleanup_timeout=5,
                 default_user="user", default_workdir="/home/user"):
        self.data = Path(data)
        self.cgroups = Path(cgroups)
        self.lowers = tuple(Path(lower) for lower in lowers)
        self.envd = Path(envd)
        self.startup_timeout = startup_timeout
        self.cleanup_timeout = cleanup_timeout
        self.default_user = default_user
        self.default_workdir = default_workdir
        self.channel = None
        self.donor = None
        if sys.platform != "linux" or os.geteuid() != 0 or os.uname().machine not in ("x86_64", "aarch64"):
            raise ValueError("fan-out requires a privileged Linux guest with a supported architecture")
        private_directory(self.data)
        if (not self.cgroups.is_dir() or self.cgroups.resolve() != self.cgroups
                or not (self.cgroups / "cgroup.controllers").is_file()):
            raise ValueError("fan-out needs a dedicated cgroup v2 parent")
        if (not self.lowers or not all(lower.is_dir() and lower.resolve() == lower
                                      and os.statvfs(lower).f_flag & os.ST_RDONLY for lower in self.lowers)
                or not all(OVERLAY_PATH.fullmatch(str(path)) for path in (*self.lowers, self.data))
                or self.data.resolve() != self.data):
            raise ValueError("fan-out needs canonical read-only lower mounts and private work storage")
        self.last_cap = int(Path("/proc/sys/kernel/cap_last_cap").read_text())

    def attach_prepared_donor(self, workload=None, manager_fds=()):
        """Fork only at a cooperative idle point with no live process handles."""
        if self.channel is not None or self.donor is not None:
            raise ValueError("fan-out checkpoint donor already established")
        task = Path("/proc/self/task")
        if len(list(task.iterdir())) != 1 or (task / str(os.getpid()) / "children").read_text().strip():
            raise ValueError("fan-out checkpoint requires one thread and no child processes")
        for fd in Path("/proc/self/fd").iterdir():
            try:
                fcntl.fcntl(int(fd.name), fcntl.F_GETFD)
            except OSError:
                continue
            if int(fd.name) > 2 and int(fd.name) not in manager_fds:
                raise ValueError("fan-out checkpoint requires closed nonstandard descriptors")

        envd = os.memfd_create("sandproof-envd", os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
        try:
            # Execute the checked bytes even if the configured path is later
            # replaced. The donor and its children cannot modify the payload.
            flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK
            pinned = os.open(self.envd, flags)
            with os.fdopen(pinned, "rb") as source, os.fdopen(os.dup(envd), "wb") as target:
                if not stat.S_ISREG(os.fstat(source.fileno()).st_mode):
                    raise ValueError("fan-out envd must be a regular executable image")
                if os.fstat(source.fileno()).st_size > 64 * 1024 * 1024:
                    raise ValueError("fan-out envd executable image too large")
                expected = os.fstat(source.fileno()).st_size
                for _ in range((expected + 1024 * 1024 - 1) // (1024 * 1024)):
                    target.write(source.read(1024 * 1024))
                if source.read(1):
                    raise ValueError("fan-out envd grew while copying the executable")
            os.lseek(envd, 0, os.SEEK_SET)
            with os.fdopen(os.dup(envd), "rb") as payload:
                if hashlib.file_digest(payload, "sha256").hexdigest() != ENVD_SHA256:
                    raise ValueError("fan-out envd does not match the pinned build")
            os.fchmod(envd, 0o500)
            fcntl.fcntl(envd, fcntl.F_ADD_SEALS,
                        fcntl.F_SEAL_SEAL | fcntl.F_SEAL_SHRINK | fcntl.F_SEAL_GROW | fcntl.F_SEAL_WRITE)
            manager, donor = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
            try:
                self.donor = os.fork()
            except OSError:
                manager.close()
                donor.close()
                raise
            if self.donor == 0:
                manager.close()
                for fd in manager_fds:
                    os.close(fd)
                try:
                    self._donate(donor, envd, workload)
                finally:
                    os._exit(0)
            donor.close()
            manager.settimeout(self.startup_timeout)
            self.channel = manager
        finally:
            os.close(envd)

    def checkpoint_available(self):
        if self.channel is None:
            return False
        poller = select.poll()
        failure = select.POLLERR | select.POLLHUP | select.POLLNVAL
        poller.register(self.channel, failure)
        return all(not flags & failure for _, flags in poller.poll(0))

    def _paths(self, batch_id, ordinal):
        name = f"{identifier(batch_id)}.{ordinal}"
        return self.data / name, self.cgroups / name

    def _donate(self, channel, envd, workload):
        signal.signal(signal.SIGCHLD, _reap_children)
        while True:
            request = _receive(channel)
            if request is None:
                return
            batch_id = request["batch_id"]
            success = True
            try:
                for ordinal, branch in enumerate(request["branches"]):
                    slot, cgroup = self._paths(batch_id, ordinal)

                    def start_child():
                        channel.close()
                        signal.signal(signal.SIGCHLD, signal.SIG_DFL)
                        _enter_branch(slot, self.lowers, envd, self.last_cap, branch["port"],
                                      branch["branch_id"], workload)

                    _spawn_member(cgroup, start_child)
            except OSError as error:
                LOGGER.warning("fan-out clone did not complete (errno %s)", error.errno)
                success = False
            _packet(channel, {"batch_id": batch_id, "all_forked": success})

    def _disable_donor(self):
        if self.channel is not None:
            self.channel.close()
            self.channel = None

    def close(self):
        self._disable_donor()
        if self.donor:
            _wait_exit(self.donor)
            self.donor = None

    def start(self, plan):
        if self.channel is None:
            raise RuntimeError("fan-out checkpoint donor unavailable")
        deadline = time.monotonic() + self.startup_timeout
        for ordinal in range(len(plan.branches)):
            slot, cgroup = self._paths(plan.batch_id, ordinal)
            slot.mkdir(mode=0o700)
            (slot / "upper").mkdir(mode=0o755)
            (slot / "work").mkdir(mode=0o700)
            (slot / "root").mkdir(mode=0o700)
            cgroup.mkdir()
            if not (cgroup / "cgroup.kill").is_file():
                raise ValueError("fan-out requires cgroup.kill")
        try:
            _packet(self.channel, {
                "batch_id": plan.batch_id,
                "branches": [{"branch_id": b.branch_id, "port": b.port} for b in plan.branches],
            })
            response = _receive(self.channel)
            if response is None or response["batch_id"] != plan.batch_id:
                raise RuntimeError("fan-out acknowledgement unavailable")
        except (OSError, ValueError, RuntimeError):
            self._disable_donor()
            raise
        if not response["all_forked"]:
            raise RuntimeError("fan-out donor did not fork the full batch")
        self._ready(plan, deadline)

    def _ready(self, plan, deadline):
        remaining = dict(enumerate(plan.branches))
        while remaining and time.monotonic() < deadline:
            for ordinal, branch in list(remaining.items()):
                _, cgroup = self._paths(plan.batch_id, ordinal)
                if not _owns_tcp_listener(cgroup, branch.port):
                    continue
                timeout = min(0.25, max(0.001, deadline - time.monotonic()))
                try:
                    status = _status(branch.port, "GET", "/health", timeout)
                except (OSError, http.client.HTTPException):
                    continue
                if status != 204 or not _owns_tcp_listener(cgroup, branch.port):
                    continue
                body = json.dumps({"accessToken": branch.access_token, "defaultUser": self.default_user,
                                   "defaultWorkdir": self.default_workdir})
                try:
                    ready = (_status(branch.port, "POST", "/init", timeout, body=body) == 204
                             and _status(branch.port, "GET", "/files?path=%2Fetc%2Fpasswd", timeout,
                                         token=branch.access_token) == 200)
                except (OSError, http.client.HTTPException):
                    ready = False
                if not ready:
                    raise RuntimeError("branch envd initialization did not complete")
                del remaining[ordinal]
            if remaining:
                time.sleep(min(0.01, max(0, deadline - time.monotonic())))
        if remaining:
            raise TimeoutError("batch envd readiness deadline exceeded")

    def present(self, batch_id, ordinal, port):
        _, cgroup = self._paths(batch_id, ordinal)
        try:
            return _owns_tcp_listener(cgroup, port, observe=True)
        except FileNotFoundError:
            if cgroup.exists():
                raise
            return False

    def has_batch(self, batch_id):
        prefix = identifier(batch_id) + "."
        return any(entry.name.startswith(prefix)
                   for parent in (self.data, self.cgroups) for entry in parent.iterdir())

    def release(self, batch_id, ordinal):
        slot, cgroup = self._paths(batch_id, ordinal)
        if cgroup.exists():
            _write_control(cgroup / "cgroup.kill", b"1")
            deadline = time.monotonic() + self.cleanup_timeout
            while True:
                events = dict(line.split() for line in (cgroup / "cgroup.events").read_text().splitlines())
                if events.get("populated") == "0":
                    # rmdir fences any later clone3 against this cgroup FD.
                    cgroup.rmdir()
                    break
                if time.monotonic() >= deadline:
                    raise TimeoutError("branch cgroup did not empty")
                time.sleep(0.01)
        if slot.exists():
            shutil.rmtree(slot)
