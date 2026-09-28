"""Original SDK sandbox lifetime values against the isolated local KVM server.

Deployment default/max lifetimes are explicit inputs, separate from request
and command stream timeouts. The client and local server share the host clock.
"""

import argparse
import asyncio
from datetime import datetime, timedelta, timezone
import importlib.metadata
import json
import os
from pathlib import Path

from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import AuthenticationException, SandboxException, SandboxNotFoundException

from sdk_command_input import call
from sdk_server_smoke import options


async def listed_ids(cls, owner):
    paginator = await call(cls.list, **owner)
    return [item.sandbox_id for item in await call(paginator.next_items)]


def arguments(case, value):
    return {} if case == 'omitted' else {'timeout': value}


def bad_lifetime(error):
    # This management API maps HTTP 400 to SandboxException, distinct from
    # the envd RPC InvalidArgumentException checked in the other probes.
    assert type(error) is SandboxException and error.status_code == 400


async def healthy(sandbox):
    assert (await call(sandbox.get_info)).state == 'running'
    assert (await call(sandbox.commands.run, 'printf healthy', timeout=15)).stdout == 'healthy'


async def create_case(cls, owner, case, value, default):
    assert await listed_ids(cls, owner) == []
    sandbox = None
    rejected = case in ('zero', 'negative', 'above_maximum')
    try:
        try:
            sandbox = await call(cls.create, template='base', allow_internet_access=False,
                                 **arguments(case, value), **owner)
        except Exception as error:
            assert rejected
            bad_lifetime(error)
        else:
            assert not rejected, 'invalid lifetime created a sandbox'
            info = await call(sandbox.get_info)
            expected = default if case == 'omitted' else value
            assert (info.end_at - info.started_at).total_seconds() == expected
            assert await listed_ids(cls, owner) == [sandbox.sandbox_id]
            await healthy(sandbox)
    finally:
        if sandbox is not None:
            assert await call(sandbox.kill)
    assert await listed_ids(cls, owner) == []
    return dict(operation='create', case=case, outcome='http_400' if rejected else 'completed')


async def supervisor(args, kind, sandbox_id):
    barrier = Path(args.supervisor_dir)
    assert not (barrier/'request.json').exists() and not (barrier/'done').exists()
    (barrier/'request.json').write_text(json.dumps({'kind':kind, 'sandbox_id':sandbox_id}))
    async with asyncio.timeout(45):
        while not (barrier/'done').exists():
            await asyncio.sleep(0.05)
    (barrier/'done').unlink()


async def lifetime_case(cls, sandbox, owner, operation, static, case, value, args):
    before = await call(sandbox.get_info)
    rejected = operation == 'connect' and case in ('zero', 'negative', 'above_maximum')
    missing = operation == 'set_timeout' and case == 'omitted'
    function = getattr(cls if static else sandbox, operation)
    positional = (sandbox.sandbox_id,) if static else ()
    opts = owner if static else {}
    started = datetime.now(timezone.utc)
    try:
        result = await call(function, *positional, **arguments(case, value), **opts)
    except Exception as error:
        if missing:
            assert type(error) is TypeError and 'timeout' in str(error)
        else:
            assert rejected
            bad_lifetime(error)
    else:
        assert not rejected and not missing
        if operation == 'connect':
            assert result.sandbox_id == sandbox.sandbox_id
            await healthy(result)
        else:
            assert result is None
    finished = datetime.now(timezone.utc)
    after = await call(sandbox.get_info)
    assert before.sandbox_id == after.sandbox_id and before.started_at == after.started_at
    if rejected or missing:
        assert after.end_at == before.end_at
    else:
        duration = 300 if case == 'omitted' else value
        cap = before.started_at + timedelta(seconds=args.max_lifetime)
        # Compare without constructing a timedelta from an arbitrarily large
        # request. The server must clip i64::MAX before any narrow arithmetic.
        remaining = max(0, (cap-started).total_seconds())
        if duration >= remaining + 1:
            lower = upper = cap
        else:
            lower = min(cap, started + timedelta(seconds=duration, milliseconds=-1))
            upper = min(cap, finished + timedelta(seconds=duration, milliseconds=1))
        if operation == 'connect':
            lower, upper = max(before.end_at, lower), max(before.end_at, upper)
        assert lower <= after.end_at <= upper, (case, after.end_at, lower, upper)
    assert await listed_ids(cls, owner) == [sandbox.sandbox_id]
    await healthy(sandbox)
    return dict(operation=operation, form='class' if static else 'instance', case=case,
                outcome='missing_argument' if missing else 'http_400' if rejected else 'completed')


async def assert_data_revoked(sandbox):
    try:
        await call(sandbox.is_running)
    except AuthenticationException as error:
        # The pinned envd HTTP mapping creates this exact class only for 401.
        assert type(error) is AuthenticationException
    else:
        raise AssertionError('revoked credentials were accepted or mapped to an unrelated result')


async def expire_case(cls, sandbox, owner, static, value, args):
    function = cls.set_timeout if static else sandbox.set_timeout
    await call(function, *((sandbox.sandbox_id, value) if static else (value,)),
               **(owner if static else {}))
    try:
        await call(sandbox.get_info)
    except SandboxNotFoundException:
        pass
    else:
        raise AssertionError('expired sandbox was still queryable')
    await assert_data_revoked(sandbox)
    assert await listed_ids(cls, owner) == []
    # The supervisor observes durable Destroyed and the exact VM exit without
    # sending kill; successful return alone is not proof of synchronous cleanup.
    await supervisor(args, 'expired', sandbox.sandbox_id)
    assert not await call(sandbox.kill)
    return dict(operation='set_timeout', form='class' if static else 'instance',
                case='zero' if value == 0 else 'negative', outcome='expired_and_reclaimed')


async def short_total_case(cls, owner, args):
    try:
        await call(cls.create, template='base', allow_internet_access=False, **owner)
    except SandboxException as error:
        bad_lifetime(error)
    else:
        raise AssertionError('default create silently clipped to short total cap')
    sandbox = await call(cls.create, template='base', timeout=args.max_lifetime,
                         allow_internet_access=False, **owner)
    try:
        original = await call(sandbox.get_info)
        cap = original.started_at + timedelta(seconds=args.max_lifetime)
        assert original.end_at == cap
        await call(sandbox.connect)
        await call(sandbox.set_timeout, 2**63-1)
        assert (await call(sandbox.get_info)).end_at == cap
        await supervisor(args, 'restart', sandbox.sandbox_id)
        await call(sandbox.set_timeout, 2**63-1)
        assert (await call(sandbox.get_info)).end_at == cap
        # The supervisor uses a 20-second expiry scan and a 12-second total
        # cap, so this observes the Running-but-over-total error before cleanup.
        remaining = (cap-datetime.now(timezone.utc)).total_seconds()
        assert remaining > 0, 'fixture exhausted before the boundary check'
        await asyncio.sleep(remaining + 0.05)
        for operation in ('connect', 'set_timeout'):
            try:
                await call(getattr(sandbox, operation), timeout=300)
            except SandboxException as error:
                bad_lifetime(error)
            else:
                raise AssertionError('renewal escaped total lifetime')
        await assert_data_revoked(sandbox)
        await supervisor(args, 'expired', sandbox.sandbox_id)
        assert not await call(sandbox.kill)
    finally:
        await call(sandbox.kill)
    return dict(operation='total_lifetime', case='live_boundary_and_restart', outcome='cap_preserved_and_reclaimed')


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    assert args.default_timeout == 300
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    values = [('omitted', None), ('zero', 0), ('negative', -1),
              ('below_maximum', args.max_timeout - 1), ('at_maximum', args.max_timeout),
              ('above_maximum', args.max_timeout + 1)]
    results = []
    for cls in (Sandbox, AsyncSandbox):
        rows = []
        if args.mode == 'total':
            rows.append(await short_total_case(cls, owner, args))
        else:
            assert 60 < args.default_timeout < args.max_timeout - 1
            assert args.max_lifetime >= args.max_timeout
            for case, value in values:
                rows.append(await create_case(cls, owner, case, value, args.default_timeout))
            for operation in ('connect', 'set_timeout'):
                for static in (False, True):
                    sandbox = await call(cls.create, template='base', timeout=300,
                                         allow_internet_access=False, **owner)
                    try:
                        cases = values[:3] + [('shorter', 30)] + values[3:]
                        if operation == 'set_timeout':
                            cases = [item for item in cases if item[0] not in ('zero', 'negative')]
                            cases += [('largest_signed', 2**63-1)]
                        for case, value in cases:
                            await call(sandbox.set_timeout, 60)
                            rows.append(await lifetime_case(cls, sandbox, owner, operation, static, case, value, args))
                        if operation == 'set_timeout':
                            rows.append(await expire_case(cls, sandbox, owner, static, 0, args))
                    finally:
                        await call(sandbox.kill)
                    if operation == 'set_timeout':
                        sandbox = await call(cls.create, template='base', timeout=300,
                                             allow_internet_access=False, **owner)
                        try:
                            rows.append(await expire_case(cls, sandbox, owner, static, -1, args))
                        finally:
                            await call(sandbox.kill)
        assert await listed_ids(cls, owner) == []
        results.append(dict(sdk=cls.__name__, cases=rows))
        print(json.dumps(dict(progress='client_completed', sdk=cls.__name__, cases=len(rows))), flush=True)
    print(json.dumps(dict(sdk_version='2.51.0', default_timeout=300, max_timeout=args.max_timeout,
                          max_lifetime=args.max_lifetime, results=results,
                          scope='target lifetime contract; exact resource cleanup observed by supervisor; not final deployment budgets')))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    parser.add_argument('--default-timeout', type=int, default=300)
    parser.add_argument('--max-timeout', type=int, required=True)
    parser.add_argument('--max-lifetime', type=int, required=True)
    parser.add_argument('--supervisor-dir', required=True)
    parser.add_argument('--mode', choices=('values', 'total'), default='values')
    asyncio.run(exercise(parser.parse_args()))
