"""Original sync/async SDK network options and detail roundtrips on real VMs.

Requires the isolated Outside echo namespace from sdk_public_network.py,
public-IPv4-enabled test service, and fixed base with a guest default route.
No Internet endpoint is contacted. A supervisor handles --restart-dir exactly
as sdk_create_context.py and audits twelve admissions plus host resources.
"""

import argparse
import asyncio
import importlib.metadata
import json
import os
from pathlib import Path
import shlex
import sys
import uuid

import httpx
from e2b import AsyncSandbox, Sandbox, SandboxQuery
from e2b.exceptions import SandboxException, SandboxNotFoundException

from sdk_command_input import call
from sdk_server_smoke import options


sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'compat'))
from sdk_network_samples import PROFILES, validate_network


async def echo(sandbox, address, protocol):
    payload = uuid.uuid4().hex.encode()
    program = f'''
import socket
try:
    with socket.socket(socket.AF_INET, socket.{'SOCK_STREAM' if protocol == 'tcp' else 'SOCK_DGRAM'}) as stream:
        stream.settimeout(1)
        if {protocol!r} == 'tcp':
            stream.connect(({address!r}, 443))
            stream.sendall({payload!r})
            reply = b''
            while len(reply) < {len(payload)}:
                block = stream.recv({len(payload)} - len(reply))
                if not block:
                    break
                reply += block
        else:
            stream.sendto({payload!r}, ({address!r}, 443))
            reply, peer = stream.recvfrom(256)
            assert peer == ({address!r}, 443)
    print('allowed' if reply == {payload!r} else 'wrong-response')
except OSError:
    print('denied')
'''
    result = await call(sandbox.commands.run, 'python3 -c ' + shlex.quote(program), timeout=10)
    assert result.stderr == '' and result.stdout.strip() in ('allowed', 'denied')
    return result.stdout.strip() == 'allowed'


async def list_run(cls, owner, run):
    paginator = cls.list(query=SandboxQuery(metadata={'network_run':run}), **owner)
    items = []
    while paginator.has_next:
        items.extend(await call(paginator.next_items))
    return {item.sandbox_id for item in items}


async def invalid_networks(cls, owner):
    cases = [('domain', {'allow_out':['example.com']}),
             ('ipv6', {'deny_out':['::1']}),
             ('prefix_33', {'allow_out':['1.2.3.4/33']}),
             ('empty_address', {'allow_out':['']}),
             ('unspecified_address', {'allow_out':['0.0.0.0']}),
             ('public_traffic_false', {'allow_public_traffic':False}),
             ('too_many_rules', {'allow_out':['11.22.33.1'] * 129})]
    results = []
    for name, network in cases:
        try:
            sandbox = await call(cls.create, template='base', timeout=300,
                                 network=network, **owner)
        except Exception as error:
            assert type(error) is SandboxException and error.status_code == 400, (
                name, type(error).__name__, str(error))
        else:
            await call(sandbox.kill)
            raise AssertionError(f'{name} was accepted')
        results.append(dict(case='invalid_network', input=name, status=400))
    return results


async def verify(cls, sandbox, profile, owner, foreign, client, stage, rows):
    name, reachability = profile.name, profile.reachable
    info = await call(sandbox.get_info)
    assert await call(cls.get_info, sandbox.sandbox_id, **owner) == info
    raw = await client.get('/sandboxes/' + sandbox.sandbox_id, headers={'X-API-Key':owner['api_key']})
    assert raw.status_code == 200
    detail = raw.json()
    validate_network(info, detail, profile)
    listed = await client.get('/v2/sandboxes', headers={'X-API-Key':owner['api_key']})
    assert listed.status_code == 200
    matching = [item for item in listed.json() if item['sandboxID'] == sandbox.sandbox_id]
    assert len(matching) == 1 and 'envdAccessToken' not in matching[0]
    try:
        await call(cls.get_info, sandbox.sandbox_id, **foreign)
    except SandboxNotFoundException:
        pass
    else:
        raise AssertionError('foreign detail was visible')
    rows.append(dict(case='original_network_detail', profile=name, stage=stage))
    route = await call(sandbox.commands.run, 'ip -4 route get 11.22.33.1')
    assert 'via 172.16.0.1' in route.stdout, 'test base lacks a guest default route'
    for address, expected in zip(('11.22.33.1','11.22.34.1'), reachability):
        for protocol in ('tcp','udp'):
            assert await echo(sandbox, address, protocol) == expected, (name, address, protocol, stage)
            rows.append(dict(case='echo_path', profile=name, stage=stage,
                             address=address, protocol=protocol, allowed=expected))
    host = await call(sandbox.commands.run,
                      'ping -n -c 1 -W 1 11.22.33.2 >/dev/null 2>&1; printf "%s" "$?"')
    assert host.stdout != '0', 'user rules overrode the host-local boundary'
    rows.append(dict(case='host_local_denied', profile=name, stage=stage))
    return detail


async def exercise_client(cls, args):
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    foreign = options(os.environ['SANDPROOF_TEST_FOREIGN_API_KEY'], args.api_url, args.sandbox_url)
    rows = await invalid_networks(cls, owner)
    run = uuid.uuid4().hex
    known = set()
    async with httpx.AsyncClient(base_url=args.api_url, trust_env=False, timeout=20) as client:
        try:
            paired = []
            snapshots = []
            for profile in PROFILES[:2]:
                sandbox = await call(cls.create, template='base', timeout=300,
                                     metadata={'network_run':run, 'profile':profile.name}, **profile.options, **owner)
                known.add(sandbox.sandbox_id)
                paired.append(sandbox)
                snapshots.append(await verify(cls, sandbox, profile, owner, foreign, client,
                                              'before_restart', rows))
            ids = [sandbox.sandbox_id for sandbox in paired]
            barrier = Path(args.restart_dir)
            assert not (barrier/'request.json').exists() and not (barrier/'done').exists()
            (barrier/'request.json').write_text(json.dumps({'sdk':cls.__name__, 'sandbox_ids':ids}))
            async with asyncio.timeout(45):
                while not (barrier/'done').exists():
                    await asyncio.sleep(0.05)
            (barrier/'done').unlink()
            for sandbox, profile, before in zip(paired, PROFILES, snapshots):
                after = await verify(cls, sandbox, profile, owner, foreign, client, 'after_restart', rows)
                assert after == before, 'restart changed original network details or identity'
            rows.append(dict(case='restart_preserved_policies_and_details'))
            assert await call(paired[1].kill)
            assert await echo(paired[0], '11.22.33.1', 'tcp')
            rows.append(dict(case='cleanup_preserved_other_vm_egress'))
            assert await call(paired[0].kill)
            for profile in PROFILES[2:]:
                sandbox = await call(cls.create, template='base', timeout=300,
                                     metadata={'network_run':run, 'profile':profile.name}, **profile.options, **owner)
                known.add(sandbox.sandbox_id)
                await verify(cls, sandbox, profile, owner, foreign, client, 'single_vm', rows)
                assert await call(sandbox.kill)
        finally:
            # Discover only this run if an original response was lost; never
            # issue another create to resolve an unknown creation result.
            known.update(await list_run(cls, owner, run))
            for identity in known:
                await call(cls.kill, identity, **owner)
            assert await list_run(cls, owner, run) == set()
    return dict(sdk=cls.__name__, cases=rows)


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    results = [await exercise_client(cls, args) for cls in (Sandbox, AsyncSandbox)]
    print(json.dumps(dict(sdk_version='2.51.0', results=results,
                          scope='original network fields and isolated IPv4 echo paths; no Internet '
                                'endpoint, DNS query contents or advanced IPv6 claim; supervisor '
                                'must audit twelve admissions and exact host resources'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    parser.add_argument('--restart-dir', required=True)
    asyncio.run(exercise(parser.parse_args()))
