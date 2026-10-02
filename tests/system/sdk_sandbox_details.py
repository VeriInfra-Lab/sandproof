"""Pinned SDK detail fields on real VMs, with an optional supervisor restart.

Run against an isolated service with two test tenants. A supervisor may watch
--restart-dir/request.json, restart the same service/config, and create done.
The supervisor must audit exact live identities and resources independently.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import asyncio
import importlib.metadata
import json
import os
from pathlib import Path
import uuid
import sys

import httpx
from e2b import AsyncSandbox, Sandbox, SandboxQuery
from e2b.exceptions import SandboxNotFoundException

from sdk_command_input import call
from sdk_server_smoke import options


sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'compat'))
from sdk_network_samples import DETAIL_PROFILES, validate_base_details, validate_network


async def exercise_client(cls, args):
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    foreign = options(os.environ['SANDPROOF_TEST_FOREIGN_API_KEY'], args.api_url, args.sandbox_url)
    run = uuid.uuid4().hex
    rows = []
    async with httpx.AsyncClient(base_url=args.api_url, trust_env=False, timeout=20) as client:
        for profile in DETAIL_PROFILES:
            network = profile.sdk_network
            known = []
            metadata = {'detail_run': run, 'label': '雪 &=+'}
            try:
                sandbox = await call(cls.create, template='base', timeout=300,
                                     metadata=metadata, **profile.options, **owner)
                known.append(sandbox.sandbox_id)
                async def verify():
                    instance = await call(sandbox.get_info)
                    detail = await call(cls.get_info, sandbox.sandbox_id, **owner)
                    assert instance == detail
                    assert detail.template_id == 'base'
                    response = await client.get('/sandboxes/'+sandbox.sandbox_id,
                                                headers={'X-API-Key':owner['api_key']})
                    assert response.status_code == 200
                    raw = response.json()
                    validate_network(detail, raw, profile)
                    validate_base_details(detail, raw, metadata)
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
                _kill_confirmed = (await call(sandbox.kill))
                assert _kill_confirmed
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
