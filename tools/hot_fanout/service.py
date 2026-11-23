"""Bounded local transport for the privileged fan-out guest adapter."""

import os
from pathlib import Path
import socket
import re
import stat
import struct
import time

from .control import GuestFanout, private_directory
from .linux import MAX_MESSAGE, _decode, _encode, _packet, _receive

CONTROL_PORT = 770


def _dispatch(owner, message):
    if type(message) is not dict:
        raise ValueError("invalid fan-out operation")
    operation = message.get("operation")
    if operation == "start" and message.keys() == {"operation", "plan"}:
        return owner.start(message["plan"])
    if operation == "observe" and message.keys() == {"operation", "batch_id"}:
        return owner.observe(message["batch_id"])
    if operation == "release" and message.keys() in (
        {"operation", "batch_id"}, {"operation", "batch_id", "branch_id"},
    ):
        return owner.release(message["batch_id"], message.get("branch_id"))
    raise ValueError("invalid fan-out operation")


def _reply(owner, connection, receive, send):
    connection.settimeout(1)
    try:
        try:
            message = receive(connection)
            if message is None:
                return
            result = _dispatch(owner, message)
        except (ValueError, RecursionError):
            result = {"status": "rejected"}
        send(connection, result)
    except OSError:
        # Disconnect, timeout, or a filesystem error is an inconclusive
        # exchange. The caller can observe the original batch identity.
        return


def _read_exact(connection, size, deadline):
    result = bytearray()
    while len(result) < size:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("fan-out control read expired")
        connection.settimeout(remaining)
        chunk = connection.recv(size - len(result))
        if not chunk:
            raise OSError("fan-out control exchange ended")
        result.extend(chunk)
    return result


def _read_frame(connection, deadline=None):
    if deadline is None:
        deadline = time.monotonic() + connection.gettimeout()
    size = int.from_bytes(_read_exact(connection, 4, deadline), "big")
    if not 1 <= size <= MAX_MESSAGE:
        raise ValueError("invalid fan-out stream frame length")
    return _decode(_read_exact(connection, size, deadline))


def _write_frame(connection, value):
    payload = _encode(value)
    connection.sendall(len(payload).to_bytes(4, "big") + payload)


def serve_vsock(owner):
    """Serve host-originated requests only on Firecracker's VM-private channel."""
    with socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM) as listener:
        listener.bind((socket.VMADDR_CID_ANY, CONTROL_PORT))
        listener.listen(16)
        while True:
            connection, peer = listener.accept()
            with connection:
                if peer[0] == socket.VMADDR_CID_HOST:
                    _reply(owner, connection, _read_frame, _write_frame)


def _firecracker_exchange(connection, request, deadline):
    connection.sendall(f"CONNECT {CONTROL_PORT}\n".encode("ascii"))
    line = bytearray()
    while len(line) < 16 and not line.endswith(b"\n"):
        line.extend(_read_exact(connection, 1, deadline))
    if not re.fullmatch(rb"OK [1-9][0-9]{0,9}\n", line):
        raise OSError("Firecracker fan-out control connection refused")
    connection.settimeout(max(0.001, deadline - time.monotonic()))
    _write_frame(connection, request)
    response = _read_frame(connection, deadline)
    if type(response) is not dict:
        raise OSError("fan-out operation result unavailable")
    return response


def call_firecracker_vsock(uds, request, timeout):
    """The caller owns and verifies this exact runtime's protected UDS path."""
    if timeout <= 0:
        raise ValueError("positive control timeout required")
    deadline = time.monotonic() + timeout
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(timeout)
        connection.connect(os.fspath(uds))
        return _firecracker_exchange(connection, request, deadline)


def serve(owner, endpoint):
    """Serve one request at a time after claiming the incarnation's journal."""
    endpoint = Path(endpoint)
    private_directory(endpoint.parent)
    try:
        meta = endpoint.lstat()
    except FileNotFoundError:
        pass
    else:
        if not stat.S_ISSOCK(meta.st_mode) or meta.st_uid != os.geteuid():
            raise ValueError("fan-out control endpoint occupied")
        endpoint.unlink()
    with socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET) as listener:
        listener.bind(str(endpoint))
        endpoint.chmod(0o600)
        listener.listen(16)
        try:
            while True:
                connection, _ = listener.accept()
                with connection:
                    peer = connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
                    _, uid, _ = struct.unpack("iII", peer)
                    if uid == os.geteuid():
                        _reply(owner, connection, _receive, _packet)
        finally:
            endpoint.unlink(missing_ok=True)


def serve_from_checkpoint(backend, *, state, checkpoint_id, endpoint=None,
                          max_branches, max_batches, ports, workload=None):
    """Use at the trusted workload's single-threaded preparation boundary."""
    with GuestFanout(state, checkpoint_id, backend, max_branches=max_branches,
                     max_batches=max_batches, ports=ports) as owner:
        backend.attach_prepared_donor(workload, manager_fds=(owner._lock,))
        try:
            if endpoint is None:
                serve_vsock(owner)
            else:
                serve(owner, endpoint)
        finally:
            backend.close()


def call(endpoint, request, timeout):
    with socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET) as connection:
        connection.settimeout(timeout)
        connection.connect(os.fspath(endpoint))
        _packet(connection, request)
        response = _receive(connection)
        if type(response) is not dict:
            raise OSError("fan-out operation result unknown")
        return response
