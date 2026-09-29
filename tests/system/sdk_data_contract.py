"""Sync/async original SDK data calls through an isolated real controller/VM.

Requires pinned SDK 2.51.0, fixed base with bash/coreutils and the selected
non-root user. Creates and destroys only its own sandbox. Not a synthetic test.
"""

import argparse
import asyncio
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import shlex
import sys
import uuid

import httpx
from e2b import AsyncSandbox, Sandbox
from e2b.sandbox.commands.command_handle import CommandExitException

from sdk_server_smoke import options


# Import the same input bytes and assertions as the synthetic/gateway tests.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "compat"))
from sdk_data_samples import (
    COMMAND, ENV, FILES, NONZERO_COMMAND, file_entries, validate_command,
    validate_content, validate_entries, validate_file_info, validate_nonzero,
)


def barrier_command(directory):
    gate = shlex.quote(f"{directory}/release.fifo")
    marker = shlex.quote(f"{directory}/starts")
    return (f"mkfifo {gate}; printf 'once\\n' >> {marker}; printf 'ready\\n'; "
            f"read -r value < {gate}; printf 'done\\n'")


def sync_case(sandbox, directory, user):
    home = sandbox.commands.run("printf '%s' \"$HOME\"", user=user).stdout
    default_cwd = sandbox.commands.run("pwd", user=user)
    assert (default_cwd.stdout, default_cwd.stderr) == (home + "\n", "")
    assert sandbox.files.make_dir(directory, user=user)
    written = sandbox.files.write_files(file_entries(directory), user=user)
    assert {entry.path for entry in written} == {f"{directory}/{name}" for name in FILES}
    sandbox.files.write(f"{directory}/text.txt", FILES["text.txt"].decode(), user=user)
    validate_entries(sandbox.files.list(directory, depth=4, user=user), directory)
    hashes = {}
    for name, expected in FILES.items():
        path = f"{directory}/{name}"
        info = sandbox.files.get_info(path, user=user)
        validate_file_info(info, path, expected)
        content = sandbox.files.read(path, format="bytes", user=user)
        validate_content(content, expected)
        with sandbox.files.read(path, format="stream", user=user) as chunks:
            streamed = b"".join(chunks)
        validate_content(streamed, expected)
        hashes[name] = hashlib.sha256(content).hexdigest()
    validate_command(sandbox.commands.run(COMMAND, cwd=directory, envs=ENV, user=user), directory, user)
    try:
        sandbox.commands.run(NONZERO_COMMAND, cwd=directory, user=user)
    except CommandExitException as error:
        validate_nonzero(error)
    else:
        raise AssertionError("nonzero command reported success")
    handle = sandbox.commands.run(barrier_command(directory), cwd=directory, user=user,
                                  background=True, timeout=30)
    try:
        ready = ""
        for stdout, _stderr, _pty in handle:
            ready += stdout or ""
            if "ready\n" in ready:
                break
        else:
            raise AssertionError("background command missed barrier")
        handle.disconnect()
        resumed = sandbox.commands.connect(handle.pid, timeout=30)
        assert resumed.pid == handle.pid
        sandbox.commands.run(f"printf 'release\\n' > {shlex.quote(directory + '/release.fifo')}", user=user)
        assert "done\n" in resumed.wait().stdout
        assert sandbox.files.read(f"{directory}/starts", user=user) == "once\n"
    finally:
        handle.kill()  # Only the known PID, never another Start to discover its result.
    killed = sandbox.commands.run("sleep 60", user=user, background=True, timeout=30)
    assert killed.kill()
    try:
        killed.wait()
    except CommandExitException as error:
        assert error.exit_code != 0
    else:
        raise AssertionError("killed command reported success")
    assert sandbox.commands.run("printf healthy", user=user).stdout == "healthy"
    return {"user": user, "sha256": hashes, "reconnected_pid": handle.pid, "killed_pid": killed.pid}


async def async_case(sandbox, directory, user):
    home = (await sandbox.commands.run("printf '%s' \"$HOME\"", user=user)).stdout
    default_cwd = await sandbox.commands.run("pwd", user=user)
    assert (default_cwd.stdout, default_cwd.stderr) == (home + "\n", "")
    assert await sandbox.files.make_dir(directory, user=user)
    written = await sandbox.files.write_files(file_entries(directory), user=user)
    assert {entry.path for entry in written} == {f"{directory}/{name}" for name in FILES}
    await sandbox.files.write(f"{directory}/text.txt", FILES["text.txt"].decode(), user=user)
    validate_entries(await sandbox.files.list(directory, depth=4, user=user), directory)
    hashes = {}
    for name, expected in FILES.items():
        path = f"{directory}/{name}"
        info = await sandbox.files.get_info(path, user=user)
        validate_file_info(info, path, expected)
        content = await sandbox.files.read(path, format="bytes", user=user)
        validate_content(content, expected)
        async with await sandbox.files.read(path, format="stream", user=user) as chunks:
            streamed = b"".join([chunk async for chunk in chunks])
        validate_content(streamed, expected)
        hashes[name] = hashlib.sha256(content).hexdigest()
    validate_command(await sandbox.commands.run(COMMAND, cwd=directory, envs=ENV, user=user), directory, user)
    try:
        await sandbox.commands.run(NONZERO_COMMAND, cwd=directory, user=user)
    except CommandExitException as error:
        validate_nonzero(error)
    else:
        raise AssertionError("async nonzero command reported success")
    ready = asyncio.Event()
    ready_output = ""

    def on_stdout(chunk):
        nonlocal ready_output
        ready_output += chunk
        if "ready\n" in ready_output:
            ready.set()

    handle = await sandbox.commands.run(barrier_command(directory), cwd=directory, user=user,
                                        background=True, timeout=30, on_stdout=on_stdout)
    try:
        # Unlike sync handles, the pinned async SDK delivers output callbacks.
        await asyncio.wait_for(ready.wait(), timeout=15)
        await handle.disconnect()
        resumed = await sandbox.commands.connect(handle.pid, timeout=30)
        assert resumed.pid == handle.pid
        await sandbox.commands.run(f"printf 'release\\n' > {shlex.quote(directory + '/release.fifo')}", user=user)
        assert "done\n" in (await resumed.wait()).stdout
        assert await sandbox.files.read(f"{directory}/starts", user=user) == "once\n"
    finally:
        await handle.kill()
    killed = await sandbox.commands.run("sleep 60", user=user, background=True, timeout=30)
    assert await killed.kill()
    try:
        await killed.wait()
    except CommandExitException as error:
        assert error.exit_code != 0
    else:
        raise AssertionError("async killed command reported success")
    assert (await sandbox.commands.run("printf healthy", user=user)).stdout == "healthy"
    return {"user": user, "sha256": hashes, "reconnected_pid": handle.pid, "killed_pid": killed.pid}


async def async_service_cases(args, owner):
    sandbox = await AsyncSandbox.create(template="base", timeout=300, allow_internet_access=False, **owner)
    try:
        connected = await AsyncSandbox.connect(sandbox.sandbox_id, **owner)
        assert connected.sandbox_id == sandbox.sandbox_id
        cases = []
        for user in ("root", args.unprivileged_user):
            directory = f"/tmp/sandproof-async-data-{uuid.uuid4().hex}"
            cases.append(await async_case(connected, directory, user))
        return {"sandbox_id": sandbox.sandbox_id, "cases": cases}
    finally:
        assert await AsyncSandbox.kill(sandbox.sandbox_id, **owner), "async destruction not confirmed"
        async with httpx.AsyncClient(base_url=args.sandbox_url, trust_env=False, timeout=10) as client:
            assert (await client.get("/health", headers={"X-Access-Token": sandbox._envd_access_token})).status_code == 401


def exercise(args):
    assert importlib.metadata.version("e2b") == "2.51.0"
    assert args.unprivileged_user not in ("root", "0", ""), "select a real non-root base user"
    owner = options(os.environ["SANDPROOF_TEST_API_KEY"], args.api_url, args.sandbox_url)
    sandbox = Sandbox.create(template="base", timeout=300, allow_internet_access=False, **owner)
    report = {"sdk_version": "2.51.0"}
    try:
        connected = Sandbox.connect(sandbox.sandbox_id, **owner)
        assert connected.sandbox_id == sandbox.sandbox_id
        cases = []
        for user in ("root", args.unprivileged_user):
            directory = f"/tmp/sandproof-sync-data-{uuid.uuid4().hex}"
            cases.append(sync_case(connected, directory, user))
        report["sync"] = {"sandbox_id": sandbox.sandbox_id, "cases": cases}
    finally:
        assert Sandbox.kill(sandbox.sandbox_id, **owner), "sync destruction not confirmed"
        with httpx.Client(base_url=args.sandbox_url, trust_env=False, timeout=10) as client:
            assert client.get("/health", headers={"X-Access-Token": sandbox._envd_access_token}).status_code == 401
    report["async"] = asyncio.run(async_service_cases(args, owner))
    report["scope"] = "real controller/gateway/VM data calls; resource residuals and timeout/fault matrices require separate evidence"
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api-url", required=True)
    parser.add_argument("--sandbox-url", required=True)
    parser.add_argument("--unprivileged-user", default="ubuntu")
    exercise(parser.parse_args())
