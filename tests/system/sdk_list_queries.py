"""Pinned SDK list filters, ordering and keyset pagination on a real service.

Use an isolated Linux/KVM service with two test tenants and at least three
concurrent VMs. Host-side identity and residual auditing is required separately.
"""

import argparse
import asyncio
from pathlib import Path
import sys
import importlib.metadata
import json
import os
from urllib.parse import quote, urlencode
import uuid

import httpx
from e2b import AsyncSandbox, Sandbox, SandboxQuery
from e2b.exceptions import SandboxException, SandboxNotFoundException

from sdk_command_input import call
from sdk_server_smoke import options


sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'compat'))
from sdk_management_samples import SPECIAL_METADATA, query_cases, validate_list, validate_list_detail


def ids(items):
    return [item.sandbox_id for item in items]


async def all_pages(cls, owner, **kwargs):
    paginator = await call(cls.list, **kwargs, **owner)
    items, sizes = [], []
    while paginator.has_next:
        page = await call(paginator.next_items)
        items.extend(page)
        sizes.append(len(page))
        assert len(sizes) <= 10, 'pagination did not terminate'
    assert paginator.next_token is None
    assert len(ids(items)) == len(set(ids(items))), 'duplicate across pages'
    return items, sizes


async def exercise_client(cls, args):
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    foreign = options(os.environ['SANDPROOF_TEST_FOREIGN_API_KEY'], args.api_url, args.sandbox_url)
    run = uuid.uuid4().hex
    special = SPECIAL_METADATA
    common = {'query_run': run, **special}
    known, rows = [], []

    async def create(account, colour):
        # Never replay a create after an ambiguous exception.
        sandbox = await call(cls.create, template='base', timeout=300,
                             allow_internet_access=False, metadata={**common, 'colour': colour},
                             **account)
        known.append((account, sandbox.sandbox_id))
        return sandbox

    async def check(name, expected, query=None, order=None, limit=1):
        kwargs = dict(query=query, limit=limit)
        if order is not None:
            kwargs['order'] = order
        items, sizes = await all_pages(cls, owner, **kwargs)
        validate_list(items, expected)
        for item in items:
            full = await call(cls.get_info, item.sandbox_id, **owner)
            validate_list_detail(item, full)
        rows.append(dict(case=name, pages=sizes, matches=len(items)))
        return items

    try:
        first = await create(owner, 'red')
        other = await create(foreign, 'red')
        second = await create(owner, 'blue')
        first_info = await call(first.get_info)
        second_info = await call(second.get_info)
        assert first_info.started_at < second_info.started_at, 'fixture requires distinct starts'
        query = SandboxQuery(metadata=common)
        for name, expected, kwargs in query_cases(common, first.sandbox_id, second.sandbox_id,
                                                  second_info.started_at):
            await check(name, expected, **kwargs)
        foreign_items, _ = await all_pages(cls, foreign, query=query, limit=1)
        assert ids(foreign_items) == [other.sandbox_id]
        try:
            await call(cls.get_info, other.sandbox_id, **owner)
        except SandboxNotFoundException:
            pass
        else:
            raise AssertionError('foreign instance was visible through get_info')
        rows.append(dict(case='cross_tenant', matches=1))

        # A paginator can be reconstructed using its opaque token. Reversing
        # the order must reject that token, rather than silently repeating rows.
        paginator = await call(cls.list, query=query, limit=1, **owner)
        assert ids(await call(paginator.next_items)) == [second.sandbox_id]
        cursor = paginator.next_token
        assert cursor and paginator.has_next
        resumed, _ = await all_pages(cls, owner, query=query, limit=1, next_token=cursor)
        assert ids(resumed) == [first.sandbox_id]
        try:
            await all_pages(cls, owner, query=query, order='asc', next_token=cursor)
        except SandboxException as error:
            assert type(error) is SandboxException and error.status_code == 400
        else:
            raise AssertionError('cursor order mismatch accepted')
        rows.append(dict(case='resume_and_direction_guard', matches=1))

        # Inspect API pagination headers that the SDK does not expose publicly.
        encoded_metadata = urlencode({quote(k): quote(v) for k, v in common.items()})
        async with httpx.AsyncClient(base_url=args.api_url, trust_env=False, timeout=30) as client:
            headers = {'X-API-Key': owner['api_key']}
            for extra, count, total in [
                ({}, 1, '2'), ({'nextToken': cursor}, 1, '2'),
                ({'template': 'missing'}, 0, '0'), ({'state': 'paused'}, 0, None),
                ({'state': 'paused', 'template': 'missing'}, 0, None),
                ({'startedAfter': second_info.started_at.isoformat()}, 1, '1'),
            ]:
                response = await client.get('/v2/sandboxes', headers=headers, params={'metadata': encoded_metadata, 'limit': 1, **extra})
                assert response.status_code == 200 and len(response.json()) == count
                assert response.headers.get('x-total-running') == total
                if count == 0:
                    assert 'x-next-token' not in response.headers
                rows.append(dict(case='filtered_total_header', filters=sorted(extra), matches=count))
            for params in [{'nextToken':'broken'}, {'order':'sideways'}, {'state':'destroyed'},
                           {'startedAfter':'yesterday'}, {'template':'base:tag'},
                           {'metadata':'broken'}, {'limit':0}, {'limit':101}]:
                response = await client.get('/v2/sandboxes', headers=headers, params=params)
                assert response.status_code == 400 and response.json()['code'] == 400
                rows.append(dict(case='invalid_query', fields=sorted(params), status=400))

        # Delete the cursor anchor, then insert a newer VM. The continuation
        # remains strictly after the old key, so it neither repeats nor skips
        # the surviving older instance.
        assert await call(second.kill)
        replacement = await create(owner, 'green')
        assert ids(await call(paginator.next_items)) == [first.sandbox_id]
        assert not paginator.has_next
        await check('after_anchor_delete_and_insert', [replacement.sandbox_id, first.sandbox_id], query=query)
        assert (await call(replacement.commands.run, 'printf list-ok', timeout=15)).stdout == 'list-ok'
    finally:
        # Discover only this run's original identities if a create response was
        # lost. An incomplete Starting row is still caught by the host auditor.
        for account in (owner, foreign):
            items, _ = await all_pages(cls, account, query=SandboxQuery(metadata={'query_run':run}))
            targets = set(ids(items)) | {identity for candidate, identity in known if candidate == account}
            for identity in targets:
                await call(cls.kill, identity, **account)
            remaining, _ = await all_pages(cls, account, query=SandboxQuery(metadata={'query_run':run}))
            assert remaining == []
    return dict(sdk=cls.__name__, cases=rows)


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    results = []
    for cls in (Sandbox, AsyncSandbox):
        result = await exercise_client(cls, args)
        results.append(result)
        print(json.dumps(dict(progress='client_completed', sdk=cls.__name__, cases=len(result['cases']))), flush=True)
    print(json.dumps(dict(results=results, scope='selected full-service query/pagination contract; host resources audited separately')))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    asyncio.run(exercise(parser.parse_args()))
