"""Bounded local transport for the privileged fan-out guest adapter."""

import os
from pathlib import Path
import socket
import stat
import struct

from .control import GuestFanout, private_directory
from .linux import _packet, _receive


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


def _reply(owner, connection):
    peer = connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
    _, uid, _ = struct.unpack("iII", peer)
    if uid != os.geteuid():
        return
    connection.settimeout(1)
    try:
        try:
            message = _receive(connection)
            if message is None:
                return
            result = _dispatch(owner, message)
        except (ValueError, RecursionError):
            result = {"status": "rejected"}
        _packet(connection, result)
    except OSError:
        # Disconnect, timeout, or a filesystem error is an inconclusive
        # exchange. The caller can observe the original batch identity.
        return


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
                    _reply(owner, connection)
        finally:
            endpoint.unlink(missing_ok=True)


def serve_from_checkpoint(backend, *, state, endpoint, checkpoint_id,
                          max_branches, max_batches, ports, workload=None):
    """Use at the trusted workload's single-threaded preparation boundary."""
    with GuestFanout(state, checkpoint_id, backend, max_branches=max_branches,
                     max_batches=max_batches, ports=ports) as owner:
        backend.attach_prepared_donor(workload, manager_fds=(owner._lock,))
        try:
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
