"""Opt-in stream-limit regression against an isolated real Sandproof VM.

Uses the pinned SDK, creates/kills only its own sandbox, and never targets E2B
cloud by default. Requires the same service setup as sdk_server_smoke.py.
"""

import argparse
import importlib.metadata
import json
import os
import shlex
import uuid

import httpx
from e2b import Sandbox
from e2b.exceptions import SandboxException

from sdk_server_smoke import options


MIB = 1024 * 1024
# This is an HTTP response-body budget, including Connect framing/encoding.
# File downloads provide an exact byte boundary; command stdout is not 1:1.
RESPONSE_LIMIT = 32 * MIB


def stream_failure(error):
    # Never turn authentication errors, timeouts, command nonzero exits, or
    # arbitrary test bugs into evidence that a stream limit was enforced.
    return (type(error) in (httpx.RemoteProtocolError, httpx.ReadError, SandboxException)
            or (type(error) is Exception
                and str(error) == "Command ended without an end event"))


def healthy(sandbox):
    assert sandbox.get_info().state == "running"
    assert sandbox.commands.run("printf stream-limit-healthy", timeout=15).stdout == "stream-limit-healthy"


def download_boundary(sandbox, directory):
    path = f"{directory}/download.bin"
    cases = []
    for size in (RESPONSE_LIMIT - 1, RESPONSE_LIMIT, RESPONSE_LIMIT + 1):
        result = sandbox.commands.run(
            f"head -c {size} /dev/zero | tr '\\000' X > {shlex.quote(path)}", timeout=30,
        )
        assert result.exit_code == 0
        assert sandbox.files.get_info(path).size == size
        received = 0
        error_name = None
        try:
            with sandbox.files.read(path, format="stream", request_timeout=30) as chunks:
                for chunk in chunks:
                    assert chunk == b"X" * len(chunk)
                    received += len(chunk)
        except Exception as error:
            if size <= RESPONSE_LIMIT or not stream_failure(error):
                raise
            error_name = type(error).__name__
        if size <= RESPONSE_LIMIT:
            assert received == size
        else:
            assert error_name is not None, "oversized download ended as a successful response"
            assert 0 < received <= RESPONSE_LIMIT
        healthy(sandbox)
        cases.append({"file_bytes": size, "received_bytes": received, "stream_error": error_name})
    return cases


def command_overflow(sandbox, directory):
    marker = f"{directory}/overflow-starts"
    # Append before output, so a retransmitted Start leaves a second marker.
    # 32 MiB of stdout exceeds the wire budget once Connect JSON/base64 is added.
    command = (f"printf 'once\\n' >> {shlex.quote(marker)}; "
               f"head -c {RESPONSE_LIMIT} /dev/zero | tr '\\000' X")
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
            "download_boundaries": download_boundary(sandbox, directory),
            "command_overflow": command_overflow(sandbox, directory),
            "disconnect": disconnect_and_reconnect(sandbox, directory),
        }
    finally:
        assert Sandbox.kill(sandbox.sandbox_id, **owner), "test sandbox destruction was not confirmed"
    report["scope"] = "real gateway/SDK stream checks; host residual audit and full S11/S12 remain separate"
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api-url", required=True)
    parser.add_argument("--sandbox-url", required=True)
    exercise(parser.parse_args())
