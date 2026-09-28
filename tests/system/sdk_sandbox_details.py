"""Pinned SDK detail fields on real VMs, with an optional supervisor restart.

Run against an isolated service with two test tenants. A supervisor may watch
--restart-dir/request.json, restart the same service/config, and create done.
The supervisor must audit exact live identities and resources independently.
"""

import argparse
import asyncio
import importlib.metadata
import json
import os
from pathlib import Path
import uuid

import httpx
from e2b import AsyncSandbox, Sandbox, SandboxQuery
from e2b.exceptions import SandboxNotFoundException

from sdk_command_input import call
from sdk_server_smoke import options


async def exercise_client(cls, args):
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    foreign = options(os.environ['SANDPROOF_TEST_FOREIGN_API_KEY'], args.api_url, args.sandbox_url)
    run = uuid.uuid4().hex
    rows = []
    async with httpx.AsyncClient(base_url=args.api_url, trust_env=False, timeout=20) as client:
        for network in (None, {'allow_out': [], 'deny_out': ['8.8.8.8', '1.2.3.4/24']}):
            known = []
            metadata = {'detail_run': run, 'label': '雪 &=+'}
            kwargs = {} if network is None else {'network': network}
            expected = None if network is None else {**network, 'allow_public_traffic': True}
            try:
                sandbox = await call(cls.create, template='base', timeout=300,
                                     allow_internet_access=False, metadata=metadata, **kwargs, **owner)
                known.append(sandbox.sandbox_id)
                async def verify():
                    instance = await call(sandbox.get_info)
                    detail = await call(cls.get_info, sandbox.sandbox_id, **owner)
                    assert instance == detail
                    assert detail.allow_internet_access is False
                    assert detail.network == expected, 'original network fields changed'
                    assert detail.sandbox_domain == 'example.invalid'
                    assert detail.lifecycle == {'on_timeout': 'kill', 'auto_resume': False}
                    assert detail.volume_mounts == [] and detail.name is None
                    assert detail.metadata == metadata and detail.template_id == 'base'
                    assert detail.cpu_count == 1 and detail.memory_mb == 512
                    response = await client.get('/sandboxes/'+sandbox.sandbox_id,
                                                headers={'X-API-Key':owner['api_key']})
                    assert response.status_code == 200
                    raw = response.json()
                    assert raw['envdAccessToken'] and raw['diskSizeMB'] == 1024
                    assert raw['volumeMounts'] == [] and 'alias' not in raw
                    listed = await client.get('/v2/sandboxes', headers={'X-API-Key':owner['api_key']})
                    assert listed.status_code == 200
                    matches = [item for item in listed.json() if item['sandboxID'] == sandbox.sandbox_id]
                    assert len(matches) == 1
                    for key in ('envdAccessToken', 'domain', 'network', 'allowInternetAccess', 'lifecycle'):
                        assert key not in matches[0], key
                    assert matches[0]['metadata'] == metadata and matches[0]['volumeMounts'] == []
                    try:
                        await call(cls.get_info, sandbox.sandbox_id, **foreign)
                    except SandboxNotFoundException:
                        pass
                    else:
                        raise AssertionError('foreign detail was visible')
                    result = await call(sandbox.commands.run, 'printf details-ok', timeout=15)
                    assert result.stdout == 'details-ok'
                    return raw
                before = await verify()
                rows.append({'case':'detail_and_list', 'network_supplied':network is not None})
                if network is not None and args.restart_dir:
                    barrier = Path(args.restart_dir)
                    assert not (barrier/'request.json').exists() and not (barrier/'done').exists()
                    (barrier/'request.json').write_text(json.dumps({'sdk':cls.__name__, 'sandbox_id':sandbox.sandbox_id}))
                    async with asyncio.timeout(45):
                        while not (barrier/'done').exists():
                            await asyncio.sleep(0.05)
                    (barrier/'done').unlink()
                    after = await verify()
                    assert after == before, 'service restart changed instance details or credentials'
                    rows.append({'case':'same_details_after_service_restart'})
                assert await call(sandbox.kill)
                try:
                    await call(cls.get_info, sandbox.sandbox_id, **owner)
                except SandboxNotFoundException:
                    pass
                else:
                    raise AssertionError('destroyed detail remained visible')
                rows.append({'case':'destroyed_detail_not_found', 'network_supplied':network is not None})
            finally:
                # Discover this run only if a create response was lost; never replay create.
                paginator = cls.list(query=SandboxQuery(metadata={'detail_run':run}), **owner)
                while paginator.has_next:
                    known.extend(item.sandbox_id for item in await call(paginator.next_items))
                for identity in set(known):
                    await call(cls.kill, identity, **owner)
    return {'sdk':cls.__name__, 'cases':rows}


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    results = []
    for cls in (Sandbox, AsyncSandbox):
        results.append(await exercise_client(cls, args))
        print(json.dumps({'progress':'client_completed', 'sdk':cls.__name__}), flush=True)
    print(json.dumps({'results':results, 'scope':'detail fields on explicit-deny VMs; restart only if supervisor supplied; host audit separate'}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    parser.add_argument('--restart-dir')
    asyncio.run(exercise(parser.parse_args()))
