"""Original sync/async command stream and opening timeout argument matrix.

Each public run/connect interface receives omitted, zero, negative and a value
above Connect's ten-digit millisecond header limit. Request/opening deadlines
are checked separately from the command stream deadline and sandbox lifetime.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import asyncio
from contextlib import AsyncExitStack
import importlib.metadata
import json
import os
import shlex
import time
import uuid

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b import AsyncSandbox, Sandbox
from e2b.envd.client_async import _request_timeout_exception
from e2b.envd.rpc import handle_rpc_exception
from e2b.exceptions import InvalidArgumentException, TimeoutException
from e2b.sandbox.commands.command_handle import CommandExitException

from sdk_command_input import KnownCommand, call, destroy_probes, public_pid
from sdk_data_contract import guest_arguments, guest_candidate, observe_candidate
from sdk_server_smoke import options


VALUES = [('omitted', None), ('zero', 0), ('negative', -1), ('above_rpc_limit', 10_000_000)]
DEADLINE_SUFFIX = str(handle_rpc_exception(ConnectError(Code.DEADLINE_EXCEEDED, '')))
QUIET = "import sys; sys.stdin.buffer.read(); print('finished')"


def expected_outcome(asynchronous, field, case):
    if field == 'timeout' and case == 'above_rpc_limit':
        return 'invalid_rpc_timeout'
    if case == 'negative':
        if field == 'timeout':
            return 'stream_deadline'
        if asynchronous:
            return 'opening_deadline'
    return 'completed'


def verify_failure(error, expected):
    if expected == 'invalid_rpc_timeout':
        assert type(error) is InvalidArgumentException and 'has >10 digits' in str(error)
    elif expected == 'opening_deadline':
        assert type(error) is TimeoutException
        assert str(error) == str(_request_timeout_exception(-1))
        assert isinstance(error.__cause__, TimeoutError)
    elif expected == 'stream_deadline':
        assert type(error) is TimeoutException
        context = error.__context__
        if isinstance(context, ConnectError):
            assert context.code is Code.DEADLINE_EXCEEDED
        else:
            # Async handle.wait may lose the original exception chain. Only
            # the pinned DEADLINE_EXCEEDED mapping is valid here, not its
            # UNAVAILABLE/EOF mapping or a generic TimeoutException.
            assert context is None and error.__cause__ is None
            assert str(error).endswith(DEADLINE_SUFFIX)
    else:
        raise AssertionError(f'unexpected failure for {expected}: {error!r}')


def arguments(field, case, value):
    kwargs = {'timeout': 15} if field == 'request_timeout' else {}
    if case != 'omitted':
        kwargs[field] = value
    return kwargs


async def marker_count(sandbox, marker):
    if not await call(sandbox.files.exists, marker):
        return 0
    content = await call(sandbox.files.read, marker)
    assert content == 'once\n', 'a Start side effect was repeated'
    return 1


def expected_entry(command, pid):
    return dict(pid=pid, tag=None, cmd='/bin/bash', args=['-l', '-c', command], envs={}, cwd=None)


async def listed_entries(sandbox, timeout):
    listed = await call(sandbox.commands.list, request_timeout=timeout)
    assert len(listed) <= 128, 'command observation count budget'
    entries = [dict(pid=public_pid(item.pid), tag=item.tag, cmd=item.cmd, args=list(item.args),
                    envs=dict(item.envs), cwd=item.cwd) for item in listed]
    assert len({entry['pid'] for entry in entries}) == len(entries), 'duplicate listed PID'
    assert len(json.dumps(entries).encode()) <= 128 * 1024, 'command observation byte budget'
    return entries


async def unacknowledged_candidates(sandbox, command):
    # Exact public metadata is evidence of a possible original Start, not a
    # returned Start identity. Never adopt or signal these PIDs. The outer
    # exercise still destroys its own VM, including any unacknowledged process.
    entries = await listed_entries(sandbox, 15)
    return [entry for entry in entries if entry == expected_entry(command, entry['pid'])]


async def observe_absent(sandbox, pid, command, *, clock=time.monotonic, pause=asyncio.sleep):
    deadline, observations = clock() + 15, []
    for _ in range(128):
        remaining = deadline - clock()
        assert remaining > 0, ('List removal observation deadline', observations)
        entries = await listed_entries(sandbox, remaining)
        found = [entry for entry in entries if entry['pid'] == pid]
        observations.append(dict(request_timeout=remaining, original_entry=found))
        assert clock() < deadline, ('List removal observation deadline', observations)
        if not found:
            return observations
        assert found == [expected_entry(command, pid)], 'original PID List metadata changed'
        remaining = deadline - clock()
        assert remaining > 0, ('List removal observation deadline', observations)
        await pause(min(0.02, remaining))
    raise AssertionError('List removal observation count exceeded', observations)


async def run_case(sandbox, asynchronous, field, case, value, background):
    expected = expected_outcome(asynchronous, field, case)
    marker = '/tmp/run-timeout-' + uuid.uuid4().hex
    command = "printf 'once\\n' >> " + shlex.quote(marker) + '; '
    if expected == 'stream_deadline':
        # No successful end event is possible before an explicit stdin close.
        # This prevents a quick command from racing the one-millisecond limit.
        command += 'exec python3 -c ' + shlex.quote(QUIET)
    else:
        command += "printf 'finished\\n'"
    owned, terminal_observed, candidates = None, False, None
    outcome, terminal = expected, None
    async with AsyncExitStack() as cleanup:
        try:
            result = await call(sandbox.commands.run, command, stdin=True,
                                background=background, **arguments(field, case, value))
            if background:
                owned = await cleanup.enter_async_context(KnownCommand(sandbox.commands, result))
                result = await owned.wait(result)
            terminal_observed = True
            terminal = terminal_fields(result)
        except Exception as error:
            if isinstance(error, CommandExitException):
                terminal_observed = True
                terminal = terminal_fields(error)
            if expected == 'stream_deadline':
                outcome = deadline_outcome(error, execution_deadline=True)
            else:
                verify_failure(error, expected)
        else:
            assert expected == 'completed', 'deadline or malformed header was ignored'
            assert (result.stdout, result.stderr, result.exit_code, result.error) == ('finished\n', '', 0, None)
        count = await marker_count(sandbox, marker)
        if expected == 'completed':
            assert count == 1
        if expected in ('invalid_rpc_timeout', 'opening_deadline'):
            assert count == 0
        if owned is None and not terminal_observed and expected in ('stream_deadline', 'opening_deadline'):
            candidates = await unacknowledged_candidates(sandbox, command)
    return dict(operation='run', parameter=field, case=case, background=background,
                expected_outcomes=([expected, 'command_terminated'] if expected == 'stream_deadline' else [expected]),
                outcome=outcome, terminal_result=terminal,
                observed_start_markers=count, terminal_observed=terminal_observed,
                original_pid=owned.pid if owned is not None else None,
                unacknowledged_candidates=candidates)


async def connect_case(sandbox, asynchronous, field, case, value):
    expected = expected_outcome(asynchronous, field, case)
    marker = '/tmp/connect-timeout-' + uuid.uuid4().hex
    command = "printf 'once\\n' >> " + shlex.quote(marker)
    command += '; exec python3 -c ' + shlex.quote(QUIET)
    original = await call(sandbox.commands.run, command, stdin=True,
                          background=True, timeout=60)
    connected = None
    recovery = None
    async with KnownCommand(sandbox.commands, original) as owned:
        await owned.disconnect(original)
        try:
            connected = owned.add(await call(sandbox.commands.connect, owned.pid,
                                             **arguments(field, case, value)))
            if expected == 'stream_deadline':
                await owned.wait(connected)
        except Exception as error:
            verify_failure(error, expected)
        else:
            assert expected == 'completed', 'invalid connect timeout was ignored'
        listed = await call(sandbox.commands.list, request_timeout=15)
        assert owned.pid in [item.pid for item in listed]
        if expected != 'completed':
            if connected is not None:
                await owned.disconnect(connected)
            recovery = owned.add(await call(sandbox.commands.connect, owned.pid, timeout=15))
        active = recovery or connected
        await call(active.close_stdin, request_timeout=15)
        result = await owned.wait(active)
        assert (result.stdout, result.stderr, result.exit_code, result.error) == ('finished\n', '', 0, None)
        count = await marker_count(sandbox, marker)
        assert count == 1
    return dict(operation='connect', parameter=field, case=case,
                outcome=expected, same_pid_recovered=True, start_markers=1)


def terminal_fields(result):
    return dict(exit_code=result.exit_code, stdout=result.stdout, stderr=result.stderr, error=result.error)


def verify_deadline_exit(error):
    assert type(error) is CommandExitException
    actual = terminal_fields(error)
    assert actual == dict(exit_code=-1, stdout='', stderr='', error='signal: killed'), (
        'unexpected deadline terminal', actual)


def deadline_outcome(error, *, execution_deadline):
    # Start creates a process deadline separately from the RPC stream's.
    # Connect only bounds the new subscription, never the original process.
    if execution_deadline and type(error) is CommandExitException:
        verify_deadline_exit(error)
        return 'command_terminated'
    verify_failure(error, 'stream_deadline')
    return 'stream_deadline'


async def wait_deadline_case(sandbox, started_with_deadline):
    marker = '/tmp/wait-timeout-' + uuid.uuid4().hex
    command = "printf 'once\\n' >> " + shlex.quote(marker)
    command += '; exec python3 -c ' + shlex.quote(QUIET)
    original = await call(sandbox.commands.run, command, stdin=True,
                          background=True, timeout=1 if started_with_deadline else 60)
    waiting = original
    recovery = None
    removal = None
    async with KnownCommand(sandbox.commands, original) as owned:
        if not started_with_deadline:
            await owned.disconnect(original)
            waiting = owned.add(await call(sandbox.commands.connect, owned.pid, timeout=1))
        # Start has returned a known PID, but stdin stays open, so wait cannot
        # get a successful end event. Its deadline is the existing stream's.
        try:
            await owned.wait(waiting)
        except Exception as error:
            initial_outcome = deadline_outcome(error, execution_deadline=started_with_deadline)
        else:
            raise AssertionError('wait ignored the command stream deadline')
        await owned.disconnect(waiting)
        recovery = owned.add(await call(sandbox.commands.connect, owned.pid, timeout=15))
        if started_with_deadline:
            # Start copies its header deadline to exec.CommandContext. A late
            # Connect observes the retained exit, never a restarted command.
            try:
                await owned.wait(recovery)
            except Exception as error:
                verify_deadline_exit(error)
                terminal = terminal_fields(error)
            else:
                raise AssertionError('command deadline did not terminate the process')
            removal = await observe_absent(sandbox, owned.pid, command)
        else:
            listed = await call(sandbox.commands.list, request_timeout=15)
            assert owned.pid in [item.pid for item in listed]
            await call(recovery.close_stdin, request_timeout=15)
            result = await owned.wait(recovery)
            assert (result.stdout, result.stderr, result.exit_code, result.error) == ('finished\n', '', 0, None)
            terminal = terminal_fields(result)
        count = await marker_count(sandbox, marker)
        if not started_with_deadline:
            assert count == 1
        healthy = await call(sandbox.commands.run, 'printf healthy', timeout=15)
        assert healthy.stdout == 'healthy'
    return dict(operation='wait', parameter='inherited_stream_deadline',
                case='start' if started_with_deadline else 'connect',
                outcome='command_terminated' if started_with_deadline else 'stream_deadline',
                initial_wait_outcome=initial_outcome, terminal_result=terminal,
                same_pid_observed=True, start_markers=count, post_end_list_observations=removal)


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    template, candidate = guest_candidate(args)
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    for cls in (Sandbox, AsyncSandbox):
        sandbox = await call(cls.create, template=template, timeout=300,
                             allow_internet_access=False, **owner)
        try:
            before = await observe_candidate(sandbox, candidate, asynchronous=cls is AsyncSandbox)
            rows = []
            for field in ('timeout', 'request_timeout'):
                for case, value in VALUES:
                    for background in (False, True):
                        rows.append(await run_case(sandbox, cls is AsyncSandbox,
                                                   field, case, value, background))
                    rows.append(await connect_case(sandbox, cls is AsyncSandbox,
                                                   field, case, value))
                    assert (await call(sandbox.commands.run, 'printf healthy', timeout=15)).stdout == 'healthy'
            for started_with_deadline in (True, False):
                rows.append(await wait_deadline_case(sandbox, started_with_deadline))
            after = await observe_candidate(sandbox, candidate, asynchronous=cls is AsyncSandbox)
            assert before == after, 'Rust guest identity changed during command timeout cases'
            item = dict(sdk=cls.__name__, cases=rows)
            if candidate is not None:
                item.update(sandbox_id=sandbox.sandbox_id, rust_guest=before)
            results.append(item)
        finally:
            await destroy_probes([sandbox])
    report = dict(sdk_version='2.51.0', results=results,
                  scope='run/connect stream and opening timeout values; wait uses '
                        'the existing stream deadline, no separate timeout argument; '
                        'not file/unary RPC or sandbox lifetime timeout matrices')
    if getattr(args, 'template', None) is not None:
        report['template'] = template
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    guest_arguments(parser)
    asyncio.run(exercise(parser.parse_args()))
