"""Pinned sync/async E2B list contract through the real management service.

Run only on an isolated KVM host with room for two owner VMs and one foreign VM.
This probe does not start a public E2B cloud sandbox.
"""

import argparse
import asyncio
import importlib.metadata
import json
import os

from e2b import AsyncSandbox, Sandbox, SandboxQuery, SandboxState
from e2b.exceptions import SandboxException

from sdk_command_input import call
from sdk_server_smoke import options


async def page(cls, owner, **kwargs):
    paginator = await call(cls.list, **kwargs, **owner)
    items = await call(paginator.next_items)
    return paginator, items


async def collect(cls, owner, **kwargs):
    paginator = await call(cls.list, **kwargs, **owner)
    pages = []
    while paginator.has_next:
        assert len(pages) < 10, 'pagination did not terminate'
        pages.append(await call(paginator.next_items))
    return pages


async def expect_bad_cursor(cls, owner, token, **kwargs):
    try:
        await page(cls, owner, next_token=token, **kwargs)
    except SandboxException as error:
        assert error.status_code == 400, error
    else:
        raise AssertionError('invalid cursor was accepted')


async def exercise_one(cls, owner, foreign):
    live = []
    try:
        first = await call(
            cls.create, template='base', timeout=300, allow_internet_access=False,
            metadata={'job name': 'batch 12', 'slot': 'first'}, **owner,
        )
        live.append((first, owner))
        second = await call(
            cls.create, template='base', timeout=300, allow_internet_access=False,
            metadata={'job name': 'batch 12', 'slot': 'second'}, **owner,
        )
        live.append((second, owner))
        outsider = await call(
            cls.create, template='base', timeout=300, allow_internet_access=False,
            metadata={'job name': 'batch 12'}, **foreign,
        )
        live.append((outsider, foreign))

        infos = [await call(sandbox.get_info) for sandbox in (first, second)]
        for sandbox, info in zip((first, second), infos):
            assert info.sandbox_id == sandbox.sandbox_id
            assert info.template_id == 'base' and info.state == SandboxState.RUNNING
            assert info.metadata['job name'] == 'batch 12'
            assert info.started_at < info.end_at
            assert info.cpu_count > 0 and info.memory_mb > 0
            assert info.allow_internet_access is False
            assert info.sandbox_domain
            assert info.volume_mounts == []

        metadata = SandboxQuery(metadata={'job name': 'batch 12'}, template='base')
        pages = await collect(cls, owner, query=metadata, limit=1)
        assert [len(items) for items in pages] == [1, 1]
        got = [item.sandbox_id for items in pages for item in items]
        expected = sorted(
            infos,
            key=lambda info: (-int(info.started_at.timestamp() * 1_000_000), info.sandbox_id),
        )
        assert got == [info.sandbox_id for info in expected]
        assert outsider.sandbox_id not in got
        ascending = await collect(cls, owner, query=metadata, limit=1, order='asc')
        assert [item.sandbox_id for items in ascending for item in items] == list(reversed(got))

        assert [item.sandbox_id for items in await collect(
            cls, owner, query=SandboxQuery(metadata={'slot': 'first'}), limit=1,
        ) for item in items] == [first.sandbox_id]
        assert await collect(cls, owner, query=SandboxQuery(state=[SandboxState.PAUSED])) == [[]]
        both = await collect(cls, owner, query=SandboxQuery(state=[SandboxState.RUNNING, SandboxState.PAUSED]))
        assert {item.sandbox_id for items in both for item in items} == {first.sandbox_id, second.sandbox_id}
        assert await collect(cls, owner, query=SandboxQuery(template='not-a-template')) == [[]]
        inclusive = await collect(cls, owner, query=SandboxQuery(started_after=infos[0].started_at))
        assert first.sandbox_id in {item.sandbox_id for items in inclusive for item in items}
        assert [item.sandbox_id for items in await collect(cls, foreign) for item in items] == [outsider.sandbox_id]

        paginator, page_one = await page(cls, owner, query=metadata, limit=1)
        assert len(page_one) == 1 and paginator.next_token
        token = paginator.next_token
        await expect_bad_cursor(cls, owner, 'not-a-cursor')
        await expect_bad_cursor(cls, foreign, token, query=metadata, limit=1)
        await expect_bad_cursor(cls, owner, token, query=SandboxQuery(metadata={'slot': 'first'}), limit=1)
        await expect_bad_cursor(cls, owner, token, query=metadata, limit=1, order='asc')
        return {'sdk': cls.__name__, 'pages': len(pages), 'sandbox_ids': got}
    finally:
        for sandbox, credential in reversed(live):
            assert await call(cls.kill, sandbox.sandbox_id, **credential)


async def main(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    foreign = options(os.environ['SANDPROOF_TEST_FOREIGN_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    for cls in (Sandbox, AsyncSandbox):
        results.append(await exercise_one(cls, owner, foreign))
    print(json.dumps({'sdk_version': '2.51.0', 'results': results,
                      'scope': 'real VM management get/list, filters, order, cursor and tenant boundary; '
                               'exact host residual audit is a separate harness step'}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    asyncio.run(main(parser.parse_args()))
