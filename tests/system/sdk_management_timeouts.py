"""Original SDK management request deadlines against an isolated KVM service.

An observation-only HTTPX send wrapper records numeric deadline extensions and
then calls the original method unchanged. Responses and transport behavior are
never mocked. This is separate from sandbox lifetime and command/RPC deadlines.
"""

import argparse
import asyncio
from contextlib import contextmanager
import importlib.metadata
import json
import os
import uuid

import httpx
from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import TimeoutException

from sdk_command_input import call as sdk_call
from sdk_file_timeouts import verify_deadline
from sdk_server_smoke import options


VALUES = [('omitted', None), ('zero', 0), ('negative', -1), ('large', 10_000_000)]


async def call(function, *args, **kwargs):
    # Separate fixture watchdog: zero disables the SDK deadline, not the
    # finite acceptance run. An outer watchdog is never counted as SDK timeout.
    return await asyncio.wait_for(sdk_call(function, *args, **kwargs), timeout=45)


def arguments(case, value):
    return {} if case == 'omitted' else {'request_timeout': value}


@contextmanager
def observe_timeout(expected):
    observed = []
    original_sync, original_async = httpx.Client.send, httpx.AsyncClient.send

    def record(request):
        assert request.url.host == '127.0.0.1', 'probe must stay on the isolated local service'
        timeout = request.extensions['timeout']
        assert timeout == dict(connect=expected, read=expected, write=expected, pool=expected), timeout
        # Do not collect headers, tokens, URLs, request bodies or response data.
        observed.append(dict(timeout))

    def send(client, request, *args, **kwargs):
        record(request)
        return original_sync(client, request, *args, **kwargs)

    async def asend(client, request, *args, **kwargs):
        record(request)
        return await original_async(client, request, *args, **kwargs)

    httpx.Client.send, httpx.AsyncClient.send = send, asend
    try:
        yield observed
    finally:
        httpx.Client.send, httpx.AsyncClient.send = original_sync, original_async


def expected_timeout(form, case, value):
    if case == 'omitted':
        return 60 if form == 'class' else 30
    if case == 'zero':
        # Instance management methods rebuild ConnectionConfig after converting
        # zero to None. Health and paginator overrides do not take that path.
        return 60 if form == 'instance' else None
    return value


async def invoke(function, positional, kwargs, form, case, value, health=False):
    print(json.dumps(dict(progress='start', operation=function.__name__, form=form, case=case)), flush=True)
    result, outcome = None, 'completed'
    expected = expected_timeout(form, case, value)
    with observe_timeout(expected) as observed:
        try:
            result = await call(function, *positional, **kwargs)
        except Exception as error:
            assert case == 'negative', type(error).__name__
            if health:
                assert type(error) is TimeoutException
                verify_deadline(error.__context__)
            else:
                verify_deadline(error)
            outcome = 'deadline'
    # A negative deadline can finish before expiry; then its complete result is
    # checked below. A deadline never proves that the operation had no effect.
    assert len(observed) == 1, 'expected exactly one original SDK HTTP send'
    print(json.dumps(dict(progress='done', operation=function.__name__, form=form, case=case, outcome=outcome)), flush=True)
    return result, dict(case=case, outcome=outcome, effective_timeout_seconds=expected)


async def items(cls, owner):
    paginator = await call(cls.list, **owner)
    return await call(paginator.next_items)


async def healthy(sandbox):
    info = await call(sandbox.get_info, request_timeout=30)
    assert info.state == 'running' and info.sandbox_id == sandbox.sandbox_id
    assert (await call(sandbox.commands.run, 'printf management-healthy', timeout=15,
                       request_timeout=30)).stdout == 'management-healthy'
    return info


async def cleanup_observed(cls, owner, run):
    found = await items(cls, owner)
    for info in found:
        assert info.metadata.get('deadline_run') == run, 'foreign sandbox in exclusive fixture'
        await call(cls.kill, info.sandbox_id, **owner)
    assert await items(cls, owner) == []
    return len(found)


async def new_sandbox(cls, owner, run):
    return await call(cls.create, template='base', timeout=300, allow_internet_access=False,
                      metadata={'deadline_run': run}, **owner)


async def create_cases(cls, owner, run):
    results = []
    for case, value in VALUES:
        opts = {k: v for k, v in owner.items() if k != 'request_timeout'}
        opts.update(arguments(case, value))
        opts.update(template='base', timeout=300, allow_internet_access=False,
                    metadata={'deadline_run': run})
        sandbox = None
        try:
            sandbox, row = await invoke(cls.create, (), opts, 'class', case, value)
            if sandbox is not None:
                await healthy(sandbox)
        finally:
            if sandbox is not None:
                await call(cls.kill, sandbox.sandbox_id, **owner)
            # Discover only original identities; never replay a timed-out create.
            # The host harness must also audit durable rows/claims at completion:
            # list shows Running only, so an empty page is not a no-effect claim.
            remaining = await cleanup_observed(cls, owner, run)
        row.update(operation='create', form='class', recovered_running_ids=remaining)
        results.append(row)
    return results


async def live_cases(cls, owner, run):
    results = []
    sandbox = await new_sandbox(cls, owner, run)
    try:
        for operation in ('connect', 'get_info', 'set_timeout'):
            for form in ('class', 'instance'):
                for case, value in VALUES:
                    before = await healthy(sandbox)
                    opts = ({k: v for k, v in owner.items() if k != 'request_timeout'}
                            if form == 'class' else {})
                    opts.update(arguments(case, value))
                    positional = (sandbox.sandbox_id,) if form == 'class' else ()
                    if operation == 'set_timeout':
                        positional += (300,)
                    function = getattr(cls if form == 'class' else sandbox, operation)
                    result, row = await invoke(function, positional, opts, form, case, value)
                    if row['outcome'] == 'completed':
                        if operation == 'set_timeout':
                            assert result is None
                        else:
                            assert result.sandbox_id == sandbox.sandbox_id
                    after = await healthy(sandbox)
                    assert after.started_at == before.started_at
                    if operation == 'get_info':
                        assert after.end_at == before.end_at
                    row.update(operation=operation, form=form,
                               deadline_changed=after.end_at != before.end_at)
                    results.append(row)
        for operation in ('list', 'next_items', 'is_running'):
            form = {'list': 'class', 'next_items': 'paginator', 'is_running': 'health'}[operation]
            for case, value in VALUES:
                opts = arguments(case, value)
                if operation == 'list':
                    config = {k: v for k, v in owner.items() if k != 'request_timeout'}
                    # Construction is lazy and performs no HTTP request.
                    with observe_timeout(None) as sends:
                        paginator = await call(cls.list, **config, **opts)
                    assert sends == []
                    function, kwargs = paginator.next_items, {}
                elif operation == 'next_items':
                    paginator = await call(cls.list, **owner)
                    function, kwargs = paginator.next_items, opts
                else:
                    function, kwargs = sandbox.is_running, opts
                result, row = await invoke(function, (), kwargs, form, case, value,
                                           health=operation == 'is_running')
                if row['outcome'] == 'completed':
                    if operation == 'is_running':
                        assert result is True
                    else:
                        assert [entry.sandbox_id for entry in result] == [sandbox.sandbox_id]
                await healthy(sandbox)
                row.update(operation=operation, form=form)
                results.append(row)
    finally:
        await call(cls.kill, sandbox.sandbox_id, **owner)
    return results


async def kill_cases(cls, owner, run):
    results = []
    for form in ('class', 'instance'):
        for case, value in VALUES:
            sandbox = await new_sandbox(cls, owner, run)
            try:
                await healthy(sandbox)
                opts = ({k: v for k, v in owner.items() if k != 'request_timeout'}
                        if form == 'class' else {})
                opts.update(arguments(case, value))
                positional = (sandbox.sandbox_id,) if form == 'class' else ()
                function = cls.kill if form == 'class' else sandbox.kill
                result, row = await invoke(function, positional, opts, form, case, value)
                if row['outcome'] == 'completed':
                    assert result is True
                visible = [entry.sandbox_id for entry in await items(cls, owner)]
                assert set(visible) <= {sandbox.sandbox_id}
                if row['outcome'] == 'completed':
                    assert visible == []
                row.update(operation='kill', form=form, running_after_call=bool(visible))
                results.append(row)
            finally:
                # Explicit reconciliation/cleanup of a known ID, not SDK retry.
                await call(cls.kill, sandbox.sandbox_id, **owner)
            assert await items(cls, owner) == []
    return results


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    run = uuid.uuid4().hex
    results = []
    classes = {'both': (Sandbox, AsyncSandbox), 'sync': (Sandbox,), 'async': (AsyncSandbox,)}[args.sdk]
    for cls in classes:
        assert await items(cls, owner) == []
        try:
            rows = await create_cases(cls, owner, run)
            rows += await live_cases(cls, owner, run)
            rows += await kill_cases(cls, owner, run)
        finally:
            await cleanup_observed(cls, owner, run)
        assert len(rows) == 48
        results.append(dict(sdk=cls.__name__, cases=rows))
    print(json.dumps(dict(sdk_version='2.51.0', results=results,
                          scope='selected management/health HTTP request timeout cases; '
                                'observation-only send wrapper delegates unchanged; '
                                'not lifetime values, forced response loss, or final budgets')))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk', choices=('both', 'sync', 'async'), default='both')
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    asyncio.run(exercise(parser.parse_args()))
