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

from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import SandboxException

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


async def lifetime_case(cls, sandbox, owner, operation, static, case, value):
    before = await call(sandbox.get_info)
    rejected = case in ('zero', 'negative', 'above_maximum')
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
    if rejected or missing or (operation == 'connect' and case in ('omitted', 'shorter')):
        assert after.end_at == before.end_at, 'rejected/omitted/short connect changed lifetime'
    else:
        # Local server and SDK run on the same host; account only for the
        # protocol's millisecond precision, never wait for a guessed duration.
        lower = started + timedelta(seconds=value, milliseconds=-1)
        upper = finished + timedelta(seconds=value, milliseconds=1)
        assert lower <= after.end_at <= upper
        if operation == 'connect':
            assert after.end_at >= before.end_at
        elif case == 'shorter':
            assert after.end_at < before.end_at
    assert await listed_ids(cls, owner) == [sandbox.sandbox_id]
    await healthy(sandbox)
    return dict(operation=operation, form='class' if static else 'instance', case=case,
                outcome='missing_argument' if missing else 'http_400' if rejected else 'completed')


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    assert 60 < args.default_timeout < args.max_timeout - 1
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    values = [('omitted', None), ('zero', 0), ('negative', -1),
              ('below_maximum', args.max_timeout - 1), ('at_maximum', args.max_timeout),
              ('above_maximum', args.max_timeout + 1)]
    results = []
    for cls in (Sandbox, AsyncSandbox):
        rows = []
        for case, value in values:
            rows.append(await create_case(cls, owner, case, value, args.default_timeout))
        sandbox = await call(cls.create, template='base', timeout=args.default_timeout,
                             allow_internet_access=False, **owner)
        try:
            for operation in ('connect', 'set_timeout'):
                for static in (False, True):
                    await call(sandbox.set_timeout, args.default_timeout)
                    for case, value in values[:3] + [('shorter', 30)] + values[3:]:
                        rows.append(await lifetime_case(cls, sandbox, owner, operation, static, case, value))
        finally:
            assert await call(sandbox.kill)
        assert await listed_ids(cls, owner) == []
        results.append(dict(sdk=cls.__name__, cases=rows))
    print(json.dumps(dict(sdk_version='2.51.0', default_timeout=args.default_timeout,
                          max_timeout=args.max_timeout, results=results,
                          scope='sandbox lifetime values for create and class/instance connect/set_timeout; '
                                'not HTTP request deadlines, expiry races or final deployment budgets'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    parser.add_argument('--default-timeout', type=int, required=True)
    parser.add_argument('--max-timeout', type=int, required=True)
    asyncio.run(exercise(parser.parse_args()))
