"""New #9/#12 lifetime target through pinned sync/async SDK and real KVM VMs.

Requires a fresh isolated server with default=300, max single and max total
matching the arguments. Physical residuals are audited separately by the host.
"""

import argparse
import asyncio
from datetime import datetime, timedelta, timezone
import importlib.metadata
import json
import os

from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import SandboxException, SandboxNotFoundException

from sdk_command_input import call
from sdk_server_smoke import options


async def info(sandbox):
    return await call(sandbox.get_info)


async def rejected(function, *args, **kwargs):
    try:
        await call(function, *args, **kwargs)
    except SandboxException as error:
        assert type(error) is SandboxException and error.status_code == 400, error
    else:
        raise AssertionError('invalid lifetime was accepted')


async def one_sdk(cls, owner, maximum, total, scanner_seconds):
    assert total >= maximum >= 300
    live = []
    observations = []
    try:
        implicit = await call(cls.create, template='base', allow_internet_access=False, **owner)
        live.append(implicit)
        current = await info(implicit)
        assert (current.end_at - current.started_at).total_seconds() == 300
        observations.append('create omitted: 300 seconds')
        assert await call(cls.kill, implicit.sandbox_id, **owner)
        live.remove(implicit)

        for value in (0, -1, maximum + 1):
            await rejected(cls.create, template='base', timeout=value,
                           allow_internet_access=False, **owner)
        observations.append('create invalid values: HTTP 400 without accepted VM')

        sandbox = await call(cls.create, template='base', timeout=5,
                             allow_internet_access=False, **owner)
        live.append(sandbox)
        before = await info(sandbox)
        started = datetime.now(timezone.utc)
        connected = await call(cls.connect, sandbox.sandbox_id, **owner)
        finished = datetime.now(timezone.utc)
        assert connected.sandbox_id == sandbox.sandbox_id
        after = await info(sandbox)
        assert after.started_at == before.started_at
        assert after.end_at > before.end_at
        assert started + timedelta(seconds=300, milliseconds=-2) <= after.end_at
        assert after.end_at <= finished + timedelta(seconds=300, milliseconds=2)

        shorter = await call(sandbox.connect, timeout=1)
        assert shorter.sandbox_id == sandbox.sandbox_id
        assert (await info(sandbox)).end_at == after.end_at
        for value in (0, -1, maximum + 1):
            await rejected(cls.connect, sandbox.sandbox_id, timeout=value, **owner)
            assert (await info(sandbox)).end_at == after.end_at
        observations.append('connect omitted renews; shorter retains; invalid rejects')

        # The class and instance methods must share the same durable lease.
        await call(cls.set_timeout, sandbox.sandbox_id, 20, **owner)
        shortened = await info(sandbox)
        assert shortened.end_at < after.end_at
        await call(sandbox.set_timeout, total * 2)
        capped = await info(sandbox)
        assert capped.end_at == capped.started_at + timedelta(seconds=total)
        await call(sandbox.connect)
        assert (await info(sandbox)).end_at == capped.end_at
        assert (await call(sandbox.commands.run, 'printf still-live', timeout=15)).stdout == 'still-live'
        observations.append('set_timeout shortens; oversized positive value caps total life')
        assert await call(cls.kill, sandbox.sandbox_id, **owner)
        live.remove(sandbox)

        for value in (0, -1):
            expiring = await call(cls.create, template='base', timeout=300,
                                  allow_internet_access=False, **owner)
            live.append(expiring)
            await call(expiring.set_timeout, value)
            try:
                await info(expiring)
            except SandboxNotFoundException:
                pass
            else:
                raise AssertionError('expired sandbox remained readable')
            # This pause waits for the configured periodic scanner, not a race
            # between two concurrent operations or a guessed safety proof.
            await asyncio.sleep(2 * scanner_seconds + 1)
            assert await call(cls.kill, expiring.sandbox_id, **owner) is False
            live.remove(expiring)
        observations.append('zero/negative stop granting access, then scanner destroys')
        return {'sdk': cls.__name__, 'observations': observations}
    finally:
        for sandbox in reversed(live):
            await call(cls.kill, sandbox.sandbox_id, **owner)


async def main(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    assert args.max_total >= args.max_timeout >= 300
    assert 1 <= args.scanner_seconds <= 10
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    for cls in (Sandbox, AsyncSandbox):
        results.append(await one_sdk(cls, owner, args.max_timeout,
                                     args.max_total, args.scanner_seconds))
    print(json.dumps({'sdk_version': '2.51.0', 'default_seconds': 300,
                      'max_single_seconds': args.max_timeout,
                      'max_total_seconds': args.max_total, 'results': results,
                      'scope': 'real VM selected lifetime semantics; host residual and restart '
                               'competition require separate evidence'}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    parser.add_argument('--max-timeout', type=int, required=True)
    parser.add_argument('--max-total', type=int, required=True)
    parser.add_argument('--scanner-seconds', type=int, required=True)
    asyncio.run(main(parser.parse_args()))
