#!/usr/bin/env python3
"""Keep each Chromium tree tied to its launching driver's lifetime (Linux).

The original tests kill ChromeDriver before calling quit. This launcher keeps
the browser running while that driver lives and reaps its own descendants after
driver death. It never changes browser flags, page content or test assertions.
"""
import ctypes
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import time

LIBC = ctypes.CDLL(None, use_errno=True)


def pidfd_open(pid):
    # Python standalone/Conda builds can omit os.pidfd_open despite a capable
    # running kernel/libc. The fixed guest distributions expose the libc API.
    if hasattr(LIBC, 'pidfd_open'):
        fd = LIBC.pidfd_open(ctypes.c_int(pid), ctypes.c_uint(0))
    elif os.uname().machine in ('x86_64', 'aarch64'):
        fd = LIBC.syscall(ctypes.c_long(434), ctypes.c_int(pid), ctypes.c_uint(0))
    else:
        raise RuntimeError('pidfd API unavailable on this platform')
    if fd < 0:
        raise OSError(ctypes.get_errno(), 'pidfd_open failed')
    return fd


def pidfd_signal(fd, sig):
    if hasattr(LIBC, 'pidfd_send_signal'):
        result = LIBC.pidfd_send_signal(ctypes.c_int(fd), ctypes.c_int(sig),
                                       ctypes.c_void_p(), ctypes.c_uint(0))
    elif os.uname().machine in ('x86_64', 'aarch64'):
        result = LIBC.syscall(ctypes.c_long(424), ctypes.c_int(fd), ctypes.c_int(sig),
                              ctypes.c_void_p(), ctypes.c_uint(0))
    else:
        raise RuntimeError('pidfd API unavailable on this platform')
    if result < 0:
        raise OSError(ctypes.get_errno(), 'pidfd_send_signal failed')


def children():
    path = Path('/proc/self/task') / str(os.getpid()) / 'children'
    return [int(pid) for pid in path.read_text().split()]


def signal_children(sig):
    for pid in children():
        try:
            fd = pidfd_open(pid)
        except ProcessLookupError:
            continue
        try:
            # Confirm this still belongs to us after opening the stable handle.
            stat = Path('/proc', str(pid), 'stat').read_text().rsplit(')', 1)[1].split()
            if int(stat[1]) == os.getpid():
                pidfd_signal(fd, sig)
        except (FileNotFoundError, ProcessLookupError):
            pass
        finally:
            os.close(fd)


def reap():
    while True:
        try:
            pid, _ = os.waitpid(-1, os.WNOHANG)
        except ChildProcessError:
            return
        if pid == 0:
            return


def supervise(command, owner, log=None):
    if LIBC.prctl(36, 1, 0, 0, 0) != 0:  # PR_SET_CHILD_SUBREAPER
        raise OSError(ctypes.get_errno(), 'cannot supervise browser descendants')
    owner_fd = pidfd_open(owner)
    poll = select.poll()
    poll.register(owner_fd, select.POLLIN)
    child = None
    reason = 'browser_exit'
    try:
        if os.getppid() != owner or poll.poll(0):
            return 1
        child = subprocess.Popen(command, start_new_session=True)
        while child.poll() is None:
            if poll.poll(100):
                reason = 'driver_exit'
                break
        return_code = child.returncode if child.returncode is not None else 0
    finally:
        if child is not None:
            if child.poll() is None:
                try:
                    os.killpg(child.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            try:
                child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=2)
            # Crashpad can detach from the browser's session. As a subreaper we
            # own those orphans; signal only confirmed direct children by pidfd.
            deadline = time.monotonic() + 3
            while children() and time.monotonic() < deadline:
                signal_children(signal.SIGKILL)
                reap()
                time.sleep(0.02)
            reap()
            remaining = children()
            if log:
                record = {'driver_pid': owner, 'browser_pid': child.pid,
                          'reason': reason, 'remaining_owned_children': remaining}
                with open(log, 'a') as stream:
                    stream.write(json.dumps(record) + '\n')
            if remaining:
                raise RuntimeError('browser descendants did not exit')
        os.close(owner_fd)
    return return_code


if __name__ == '__main__':
    command = ['/usr/bin/chromium', *sys.argv[1:]]
    if '--version' in sys.argv or '--help' in sys.argv:
        os.execv(command[0], command)
    sys.exit(supervise(command, os.getppid(), '/logs/verifier/browser-lifecycle.jsonl'))
