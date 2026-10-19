"""Sync/async original SDK data calls through an isolated real controller/VM.

Requires pinned SDK 2.51.0, fixed base with bash/coreutils and the selected
non-root user. Creates and destroys only its own sandbox. Not a synthetic test.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import asyncio
import hashlib
import importlib.metadata
import inspect
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


async def call(function, *args, **kwargs):
    if inspect.iscoroutinefunction(function):
        return await function(*args, **kwargs)
    result = await asyncio.to_thread(function, *args, **kwargs)
    return await result if inspect.isawaitable(result) else result


def public_pid(pid):
    assert type(pid) is int and 0 < pid <= 4194304, 'invalid original command PID'
    return pid


class KnownCommand:
    """One returned Start identity; subscription close and process End are distinct."""
    def __init__(self, commands, original):
        self.commands, self.original = commands, original
        self.handles, self.closed = [original], set()
        self.pid, self.finished, self.signal_attempted = None, False, False

    async def __aenter__(self):
        try:
            self.pid = public_pid(self.original.pid)
        except BaseException as error:
            await self.cleanup(error)
            raise
        return self

    async def __aexit__(self, _kind, error, _traceback):
        await self.cleanup(error)
        return False

    def add(self, handle):
        # Retain even a wrong-PID reply so its subscription can be closed.
        if all(handle is not known for known in self.handles):
            self.handles.append(handle)
        assert public_pid(handle.pid) == self.pid, 'reconnect changed original PID'
        return handle

    async def wait(self, handle, **kwargs):
        assert any(handle is known for known in self.handles)
        assert public_pid(handle.pid) == self.pid
        try:
            result = await call(handle.wait, **kwargs)
        except CommandExitException:
            self.finished = True
            raise
        self.finished = True
        return result

    async def disconnect(self, handle):
        assert any(handle is known for known in self.handles)
        if id(handle) not in self.closed:
            self.closed.add(id(handle))  # Unknown replies never cause a retry.
            await call(handle.disconnect)

    async def kill(self, **kwargs):
        assert self.pid is not None and not self.finished, 'no unfinished original command'
        assert not self.signal_attempted, 'original signal was already attempted'
        self.signal_attempted = True
        return await call(self.commands.kill, self.pid, **kwargs)

    async def cleanup(self, primary):
        errors = []
        if self.pid is not None and not self.finished and not self.signal_attempted:
            try:
                await self.kill(request_timeout=15)
            except BaseException as error:
                errors.append(error)
        for handle in reversed(self.handles):
            try:
                await self.disconnect(handle)
            except BaseException as error:
                errors.append(error)
        if errors:
            raise BaseExceptionGroup('known command and subscription cleanup failed',
                                     ([primary] if primary is not None else []) + errors)


def guest_candidate(args):
    """Validate all protected Rust material before creating either test VM."""
    names = ('guest_release', 'guest_manifest_sha256', 'guest_configuration',
             'guest_configuration_sha256')
    values = [getattr(args, name, None) for name in names]
    template = getattr(args, 'template', None)
    if template is not None and (type(template) is not str or not template):
        raise ValueError('explicit template must be nonempty')
    if not any(value is not None for value in values):
        return template or 'base', None
    if not template or any(value is None for value in values):
        raise ValueError('Rust guest observation requires explicit template and all four pinned inputs')
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
    from tools import guest_probe
    identity, expected = guest_probe.load_expected(*values)
    return template, (identity, expected, guest_probe.command(expected))


def observe_guest(sandbox, candidate):
    if candidate is None:
        return None
    from tools import guest_probe
    identity, expected, command = candidate
    result = sandbox.commands.run(command, user='root', timeout=20)
    return {'identity': identity, 'observation': guest_probe.receipt(result, expected)}


async def observe_guest_async(sandbox, candidate):
    if candidate is None:
        return None
    from tools import guest_probe
    identity, expected, command = candidate
    result = await sandbox.commands.run(command, user='root', timeout=20)
    return {'identity': identity, 'observation': guest_probe.receipt(result, expected)}


async def observe_candidate(sandbox, candidate, *, asynchronous):
    """The same observation for the existing coroutine-based system probes."""
    if candidate is None:
        return None
    if asynchronous:
        return await observe_guest_async(sandbox, candidate)
    return await asyncio.to_thread(observe_guest, sandbox, candidate)


def guest_arguments(parser, *, required=False):
    """One opt-in candidate/pin CLI shared by the original SDK probes."""
    parser.add_argument("--template", required=required, help=("explicit Rust candidate template" if required
                        else "explicit candidate template; defaults to base without Rust observation"))
    parser.add_argument("--guest-release", required=required, help="protected release directory with audited source and binaries")
    parser.add_argument("--guest-manifest-sha256", required=required, help="independently pinned release manifest hash")
    parser.add_argument("--guest-configuration", required=required, help="protected exact installed daemon configuration")
    parser.add_argument("--guest-configuration-sha256", required=required, help="independently pinned configuration hash")


def barrier_command(directory):
    gate = shlex.quote(f"{directory}/release.fifo")
    marker = shlex.quote(f"{directory}/starts")
    return (f"mkfifo {gate}; printf 'once\\n' >> {marker}; printf 'ready\\n'; "
            f"read -r value < {gate}; printf 'done\\n'")


async def command_cases(sandbox, directory, user, *, asynchronous):
    ready = asyncio.Event()
    ready_output = ""

    def on_stdout(chunk):
        nonlocal ready_output
        ready_output += chunk
        if "ready\n" in ready_output:
            ready.set()

    def read_ready(handle):
        for stdout, _stderr, _pty in handle:
            on_stdout(stdout or "")
            if ready.is_set():
                return True
        return False

    handle = await call(sandbox.commands.run, barrier_command(directory),
                        cwd=directory, user=user, background=True, timeout=30,
                        **({'on_stdout': on_stdout} if asynchronous else {}))
    async with KnownCommand(sandbox.commands, handle) as owned:
        if asynchronous:
            # The pinned async handle consumes output through its callbacks.
            await asyncio.wait_for(ready.wait(), timeout=15)
        elif not await call(read_ready, handle):
            await owned.wait(handle)  # Record any actual End before rejecting.
            raise AssertionError("background command missed barrier")
        await owned.disconnect(handle)
        resumed = owned.add(await call(sandbox.commands.connect, owned.pid, timeout=30))
        await call(sandbox.commands.run,
                   f"printf 'release\\n' > {shlex.quote(directory + '/release.fifo')}", user=user)
        result = await owned.wait(resumed)
        assert (result.stdout, result.stderr, result.exit_code, result.error) == ('done\n', '', 0, None)
        assert await call(sandbox.files.read, f"{directory}/starts", user=user) == "once\n"

    killed = await call(sandbox.commands.run, "sleep 60", user=user, background=True, timeout=30)
    async with KnownCommand(sandbox.commands, killed) as owned:
        confirmed = await owned.kill(request_timeout=15)
        assert confirmed, "command kill not confirmed"
        try:
            await owned.wait(killed)
        except CommandExitException as error:
            assert error.exit_code != 0
        else:
            raise AssertionError("killed command reported success")
    return {"reconnected_pid": handle.pid, "killed_pid": killed.pid}


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
    commands = asyncio.run(command_cases(sandbox, directory, user, asynchronous=False))
    assert sandbox.commands.run("printf healthy", user=user).stdout == "healthy"
    return {"user": user, "sha256": hashes, **commands}


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
    commands = await command_cases(sandbox, directory, user, asynchronous=True)
    assert (await sandbox.commands.run("printf healthy", user=user)).stdout == "healthy"
    return {"user": user, "sha256": hashes, **commands}


async def async_service_cases(args, owner, template, candidate):
    sandbox = await AsyncSandbox.create(template=template, timeout=300, allow_internet_access=False, **owner)
    try:
        _runpy.run_path(str(_GuardPath(__file__).with_name("_resource_evidence.py")))["record_resource_identity"](sandbox.sandbox_id)
        connected = await AsyncSandbox.connect(sandbox.sandbox_id, **owner)
        assert connected.sandbox_id == sandbox.sandbox_id
        before = await observe_guest_async(connected, candidate)
        cases = []
        for user in ("root", args.unprivileged_user):
            directory = f"/tmp/sandproof-async-data-{uuid.uuid4().hex}"
            cases.append(await async_case(connected, directory, user))
        after = await observe_guest_async(connected, candidate)
        assert before == after, 'Rust guest identity or listening process changed during SDK cases'
        result = {"sandbox_id": sandbox.sandbox_id, "cases": cases}
        if candidate is not None:
            result['rust_guest'] = before
        return result
    finally:
        _kill_confirmed = (await AsyncSandbox.kill(sandbox.sandbox_id, **owner))
        assert _kill_confirmed, "async destruction not confirmed"
        async with httpx.AsyncClient(base_url=args.sandbox_url, trust_env=False, timeout=10) as client:
            assert (await client.get("/health", headers={"X-Access-Token": sandbox._envd_access_token})).status_code == 401


def exercise(args):
    assert importlib.metadata.version("e2b") == "2.51.0"
    assert args.unprivileged_user not in ("root", "0", ""), "select a real non-root base user"
    template, candidate = guest_candidate(args)
    owner = options(os.environ["SANDPROOF_TEST_API_KEY"], args.api_url, args.sandbox_url)
    sandbox = Sandbox.create(template=template, timeout=300, allow_internet_access=False, **owner)
    report = {"sdk_version": "2.51.0"}
    if candidate is not None or getattr(args, 'template', None) is not None:
        report['template'] = template
    try:
        _runpy.run_path(str(_GuardPath(__file__).with_name("_resource_evidence.py")))["record_resource_identity"](sandbox.sandbox_id)
        connected = Sandbox.connect(sandbox.sandbox_id, **owner)
        assert connected.sandbox_id == sandbox.sandbox_id
        before = observe_guest(connected, candidate)
        cases = []
        for user in ("root", args.unprivileged_user):
            directory = f"/tmp/sandproof-sync-data-{uuid.uuid4().hex}"
            cases.append(sync_case(connected, directory, user))
        after = observe_guest(connected, candidate)
        assert before == after, 'Rust guest identity or listening process changed during SDK cases'
        report["sync"] = {"sandbox_id": sandbox.sandbox_id, "cases": cases}
        if candidate is not None:
            report['sync']['rust_guest'] = before
    finally:
        _kill_confirmed = (Sandbox.kill(sandbox.sandbox_id, **owner))
        assert _kill_confirmed, "sync destruction not confirmed"
        with httpx.Client(base_url=args.sandbox_url, trust_env=False, timeout=10) as client:
            assert client.get("/health", headers={"X-Access-Token": sandbox._envd_access_token}).status_code == 401
    report["async"] = asyncio.run(async_service_cases(args, owner, template, candidate))
    report["scope"] = "real controller/gateway/VM data calls; resource residuals and timeout/fault matrices require separate evidence"
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api-url", required=True)
    parser.add_argument("--sandbox-url", required=True)
    parser.add_argument("--unprivileged-user", default="ubuntu")
    guest_arguments(parser)
    exercise(parser.parse_args())
