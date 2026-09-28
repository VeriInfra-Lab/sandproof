"""Opt-in stream-limit regression against an isolated real Sandproof VM.

Uses the pinned SDK, creates/kills only its own sandbox, and never targets E2B
cloud by default. Requires the same service setup as sdk_server_smoke.py.
"""

import argparse
import hashlib
import importlib.metadata
from io import RawIOBase
import json
import os
import shlex
import tempfile
import uuid

import httpx
import pyqwest
from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b import Sandbox
from e2b.exceptions import SandboxException, TimeoutException

from sdk_server_smoke import options


MIB = 1024 * 1024
# This is an HTTP response-body budget, including Connect framing/encoding.
# File downloads provide an exact byte boundary; command stdout is not 1:1.
RESPONSE_LIMIT = 32 * MIB


class UnknownLengthFile(RawIOBase):
    """A non-seekable producer: SDK must send HTTP chunks, with bounded memory."""

    def __init__(self, size):
        self.remaining = size

    def read(self, size=-1):
        size = min(self.remaining, 64 * 1024, size if size >= 0 else 64 * 1024)
        self.remaining -= size
        return b"X" * size


def pattern_digest(size):
    digest = hashlib.sha256()
    stream = UnknownLengthFile(size)
    while chunk := stream.read():
        digest.update(chunk)
    return digest.hexdigest()


def upload_boundary(sandbox, directory, limit):
    cases = []
    for known_length in (True, False):
        for size in (0, limit - 1, limit, limit + 1):
            path = f"{directory}/upload-{known_length}-{size}.bin"
            with tempfile.TemporaryFile() as known:
                if known_length:
                    producer = UnknownLengthFile(size)
                    while chunk := producer.read():
                        known.write(chunk)
                    known.seek(0)
                body = known if known_length else UnknownLengthFile(size)
                error_name = None
                try:
                    sandbox.files.write(path, body, request_timeout=60)
                except SandboxException as error:
                    if size <= limit or not str(error).startswith("413:"):
                        raise
                    error_name = type(error).__name__
            digest = None
            if size <= limit:
                assert error_name is None
                assert sandbox.files.get_info(path).size == size
                digest = sandbox.commands.run(f"sha256sum {shlex.quote(path)}", timeout=30).stdout.split()[0]
                assert digest == pattern_digest(size), "uploaded bytes differ in the real guest"
            else:
                assert error_name is not None, "oversized upload returned success"
                if known_length:
                    assert not sandbox.files.exists(path), "rejected known upload reached envd"
                # Unknown length may have created/truncated/written part of the file.
                # Never retry the write or infer that a 413 undid those effects.
            sandbox.commands.run(f"rm -f -- {shlex.quote(path)}", timeout=15)
            healthy(sandbox)
            cases.append({"file_bytes": size, "known_length": known_length,
                          "sha256": digest, "upload_error": error_name})
    return cases


def stream_failure(error):
    # Never turn authentication errors, timeouts, command nonzero exits, or
    # arbitrary test bugs into evidence that a stream limit was enforced.
    if type(error) in (httpx.RemoteProtocolError, httpx.ReadError):
        return True
    if type(error) is Exception and str(error) == "Command ended without an end event":
        return True
    # SDK 2.51.0 maps a chunked-body EOF to TimeoutException via UNAVAILABLE.
    # Accept only its typed transport cause, never a deadline or timeout alone.
    rpc_error = error.__context__ if type(error) is TimeoutException else error
    return (isinstance(rpc_error, ConnectError)
            and rpc_error.code is Code.UNAVAILABLE
            and isinstance(rpc_error.__cause__,
                           (pyqwest.RemoteProtocolError, pyqwest.ReadError, pyqwest.StreamError)))


def healthy(sandbox):
    assert sandbox.get_info().state == "running"
    assert sandbox.commands.run("printf stream-limit-healthy", timeout=15).stdout == "stream-limit-healthy"


def download_boundary(sandbox, directory, limit=RESPONSE_LIMIT):
    path = f"{directory}/download.bin"
    cases = []
    for size in (0, limit - 1, limit, limit + 1):
        result = sandbox.commands.run(
            f"head -c {size} /dev/zero | tr '\\000' X > {shlex.quote(path)}", timeout=30,
        )
        assert result.exit_code == 0
        assert sandbox.files.get_info(path).size == size
        received = 0
        digest = hashlib.sha256()
        error_name = None
        try:
            with sandbox.files.read(path, format="stream", request_timeout=30) as chunks:
                for chunk in chunks:
                    assert chunk == b"X" * len(chunk)
                    received += len(chunk)
                    digest.update(chunk)
        except Exception as error:
            if size <= limit or not stream_failure(error):
                raise
            error_name = type(error).__name__
        if size <= limit:
            assert received == size
            assert digest.hexdigest() == pattern_digest(size)
        else:
            assert error_name is not None, "oversized download ended as a successful response"
            assert 0 < received <= limit
        healthy(sandbox)
        cases.append({"file_bytes": size, "received_bytes": received,
                      "sha256": digest.hexdigest() if error_name is None else None,
                      "stream_error": error_name})
    sandbox.commands.run(f"rm -f -- {shlex.quote(path)}", timeout=15)
    return cases


def command_overflow(sandbox, directory, limit=RESPONSE_LIMIT):
    marker = f"{directory}/overflow-starts"
    # Append before output, so a retransmitted Start leaves a second marker.
    # stdout equal to the budget exceeds it once Connect JSON/base64 is added.
    command = (f"printf 'once\\n' >> {shlex.quote(marker)}; "
               f"head -c {limit} /dev/zero | tr '\\000' X")
    handle = sandbox.commands.run(command, background=True, timeout=30)
    error_name = None
    received = 0

    def observe_output(chunk):
        nonlocal received
        received += len(chunk.encode())

    try:
        handle.wait(on_stdout=observe_output)
    except Exception as error:
        if not stream_failure(error):
            raise
        error_name = type(error).__name__
    finally:
        # Query/terminate the known handle; never issue Start again to learn
        # whether the original command happened. kill() may return False if done.
        handle.kill()
    assert error_name is not None, "oversized command stream reported successful completion"
    assert received > 0, "command stream failed before any output was observed"
    assert sandbox.files.read(marker) == "once\n", "command side effect was replayed"
    healthy(sandbox)
    return {"pid": handle.pid, "stream_error": error_name,
            "stdout_bytes": received, "observed_starts": 1}


def disconnect_and_reconnect(sandbox, directory):
    marker = f"{directory}/disconnect-starts"
    gate = f"{directory}/release.fifo"
    # The FIFO is a deterministic barrier: disconnect only after observing
    # ready, reconnect to the known PID, then release that same running command.
    command = (f"mkfifo {shlex.quote(gate)}; printf 'once\\n' >> {shlex.quote(marker)}; "
               f"printf 'ready\\n'; read -r release < {shlex.quote(gate)}; printf 'done\\n'")
    handle = sandbox.commands.run(command, background=True, timeout=30)
    ready = ""
    for stdout, _stderr, _pty in handle:
        ready += stdout or ""
        if "ready\n" in ready:
            break
    else:
        raise AssertionError("command ended before the disconnect barrier")
    handle.disconnect()
    resumed = sandbox.commands.connect(handle.pid, timeout=30)
    assert resumed.pid == handle.pid
    sandbox.commands.run(f"printf 'release\\n' > {shlex.quote(gate)}", timeout=15)
    assert "done\n" in resumed.wait().stdout
    assert sandbox.files.read(marker) == "once\n", "disconnect caused a new command execution"
    healthy(sandbox)
    return {"pid": handle.pid, "reconnected_pid": resumed.pid, "observed_starts": 1}


def exercise(args):
    assert importlib.metadata.version("e2b") == "2.51.0"
    owner = options(os.environ["SANDPROOF_TEST_API_KEY"], args.api_url, args.sandbox_url)
    sandbox = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **owner)
    try:
        directory = f"/tmp/sandproof-stream-{uuid.uuid4().hex}"
        sandbox.files.make_dir(directory)
        report = {
            "sandbox_id": sandbox.sandbox_id,
            "sdk_version": "2.51.0",
            "configured_budgets": {"file_upload_bytes": args.file_upload_bytes,
                                   "file_download_bytes": args.file_download_bytes,
                                   "command_output_bytes": args.command_output_bytes},
            "download_boundaries": download_boundary(sandbox, directory, args.file_download_bytes),
            "command_overflow": command_overflow(sandbox, directory, args.command_output_bytes),
            "disconnect": disconnect_and_reconnect(sandbox, directory),
        }
        if args.file_upload_bytes is not None:
            report["upload_boundaries"] = upload_boundary(sandbox, directory, args.file_upload_bytes)
    finally:
        assert Sandbox.kill(sandbox.sandbox_id, **owner), "test sandbox destruction was not confirmed"
    report["scope"] = "real gateway/SDK stream checks; host residual audit and full S11/S12 remain separate"
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    def positive_bytes(value):
        size = int(value)
        if size <= 0:
            raise argparse.ArgumentTypeError("byte budget must be positive")
        return size

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api-url", required=True)
    parser.add_argument("--sandbox-url", required=True)
    parser.add_argument("--file-upload-bytes", type=positive_bytes,
                        help="enable streaming upload cases; match deployed data_transfer exactly")
    parser.add_argument("--file-download-bytes", type=positive_bytes, default=RESPONSE_LIMIT)
    parser.add_argument("--command-output-bytes", type=positive_bytes, default=RESPONSE_LIMIT)
    exercise(parser.parse_args())
