"""Original sync/async SDK creation context on two independent real VMs.

Requires an isolated test service and a supervisor watching --restart-dir.
For each request.json it must restart the same service/config, independently
verify both VM PID/start identities, then create done. Final host audit must
find exactly four admitted sandboxes: rejected requests must add no records.
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
from sdk_management_samples import CONTEXT_KEYS as KEYS, creation_contexts, validate_context

INSPECT = 'python3 -c ' + shlex.quote(
    'import json,os; print(json.dumps({key:os.environ.get(key) for key in ' + repr(KEYS) + '}))')


async def listed(cls, owner, metadata):
    paginator = cls.list(query=SandboxQuery(metadata=metadata), limit=1, **owner)
    result = {}
    while paginator.has_next:
        for item in await call(paginator.next_items):
            assert item.sandbox_id not in result, 'duplicate pagination item'
            result[item.sandbox_id] = item.metadata
    return result


async def inspect_environment(sandbox, expected, user, overrides=None):
    kwargs = {} if overrides is None else {'envs': overrides}
    result = await call(sandbox.commands.run, INSPECT, user=user, timeout=15, **kwargs)
    assert result.exit_code == 0 and result.stderr == ''
    actual = json.loads(result.stdout)
    validate_context(actual, expected)


async def invalid_contexts(cls, owner):
    cases = [
        ('env_empty_name', {'envs': {'': 'rejected'}}),
        ('env_equals_name', {'envs': {'BAD=NAME': 'rejected'}}),
        ('env_nul_name', {'envs': {'BAD\0NAME': 'rejected'}}),
        ('env_nul_value', {'envs': {'SP_CTX_BAD': 'bad\0value'}}),
        ('metadata_empty_key', {'metadata': {'': 'rejected'}}),
        ('metadata_nul_key', {'metadata': {'bad\0key': 'rejected'}}),
        ('metadata_nul_value', {'metadata': {'bad': 'bad\0value'}}),
        ('oversize_context', {'metadata': {'large': 'x' * (64 * 1024)}}),
    ]
    result = []
    for name, kwargs in cases:
        try:
            sandbox = await call(cls.create, template='base', timeout=300,
                                 allow_internet_access=False, **kwargs, **owner)
        except Exception as error:
            assert type(error) is SandboxException and error.status_code == 400, (
                name, type(error).__name__, str(error))
        else:
            await call(sandbox.kill)
            raise AssertionError(f'{name} was accepted')
        result.append(dict(case=name, status=400, exception='SandboxException'))
    return result


async def exercise_client(cls, args):
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    foreign = options(os.environ['SANDPROOF_TEST_FOREIGN_API_KEY'], args.api_url, args.sandbox_url)
    rows = await invalid_contexts(cls, owner)
    run = uuid.uuid4().hex
    live = []
    envs, metadata = zip(*creation_contexts(run))
    async with httpx.AsyncClient(base_url=args.api_url, trust_env=False, timeout=20) as client:
        try:
            for values, tags in zip(envs, metadata):
                live.append(await call(cls.create, template='base', timeout=300,
                                       allow_internet_access=False, envs=values, metadata=tags, **owner))
            ids = [sandbox.sandbox_id for sandbox in live]
            assert len(set(ids)) == 2
            before = None
            for stage in ('before_restart', 'after_restart'):
                identities = []
                for index, sandbox in enumerate(live):
                    expected = envs[index]
                    for user in ('root', 'ubuntu'):
                        await inspect_environment(sandbox, expected, user)
                        rows.append(dict(case='create_environment', stage=stage, vm=index, user=user))
                        overrides = {'SP_CTX_COMMON': 'command override', 'SP_CTX_TRANSIENT': 'one command'}
                        await inspect_environment(sandbox, {**expected, **overrides}, user, overrides)
                        await inspect_environment(sandbox, expected, user)
                        # Also check the other live VM after the override.
                        await inspect_environment(live[1-index], envs[1-index], user)
                        rows.append(dict(case='command_override_is_local', stage=stage, vm=index, user=user))
                    detail = await call(sandbox.get_info)
                    assert detail.metadata == metadata[index]
                    assert (await call(cls.get_info, sandbox.sandbox_id, **owner)).metadata == metadata[index]
                    raw = await client.get('/sandboxes/' + sandbox.sandbox_id,
                                           headers={'X-API-Key': owner['api_key']})
                    assert raw.status_code == 200
                    body = raw.json()
                    assert body['metadata'] == metadata[index] and 'envVars' not in body
                    identities.append((body['sandboxID'], body['startedAt'], body['envdAccessToken']))
                    assert await listed(cls, owner, metadata[index]) == {ids[index]: metadata[index]}
                    assert await listed(cls, foreign, metadata[index]) == {}
                    try:
                        await call(cls.get_info, sandbox.sandbox_id, **foreign)
                    except SandboxNotFoundException:
                        pass
                    else:
                        raise AssertionError('foreign metadata was visible')
                    rows.append(dict(case='metadata_query_and_owner', stage=stage, vm=index))
                assert await listed(cls, owner, {'context_run':run}) == dict(zip(ids, metadata))
                assert await listed(cls, owner, {'context_run':run, 'label':'missing'}) == {}
                rows.append(dict(case='combined_paginated_metadata', stage=stage))
                if stage == 'before_restart':
                    before = identities
                    barrier = Path(args.restart_dir)
                    assert not (barrier/'request.json').exists() and not (barrier/'done').exists()
                    (barrier/'request.json').write_text(json.dumps({'sdk':cls.__name__, 'sandbox_ids':ids}))
                    async with asyncio.timeout(45):
                        while not (barrier/'done').exists():
                            await asyncio.sleep(0.05)
                    (barrier/'done').unlink()
                else:
                    assert identities == before, 'restart changed instance identity or credentials'
                    rows.append(dict(case='restart_preserved_both_instances'))
        finally:
            # Discover only this run if an original create response was lost.
            # Never issue a replacement create to resolve an unknown outcome.
            known = {sandbox.sandbox_id for sandbox in live}
            known.update(await listed(cls, owner, {'context_run':run}))
            for identity in known:
                await call(cls.kill, identity, **owner)
            assert await listed(cls, owner, {'context_run':run}) == {}
    return dict(sdk=cls.__name__, cases=rows)


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    results = [await exercise_client(cls, args) for cls in (Sandbox, AsyncSandbox)]
    print(json.dumps(dict(sdk_version='2.51.0', results=results,
                          scope='two-VM create env/metadata isolation and same-instance restart per '
                                'original SDK client; selected invalid inputs; supervisor must '
                                'audit exactly four admissions and all physical resources'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    parser.add_argument('--restart-dir', required=True)
    asyncio.run(exercise(parser.parse_args()))
