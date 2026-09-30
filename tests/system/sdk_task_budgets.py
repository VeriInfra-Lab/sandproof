"""Fixed local task budgets through original SDKs and a real VM.

Requires benchmarks/local-service-budgets.json, a Python base and an external
host-resource supervisor. Admission is serial (one sandbox); only the per-VM
transfer limit can be saturated. Global multi-VM pressure is a separate test.
"""

import argparse
import asyncio
import importlib.metadata
import json
import os
from pathlib import Path

from e2b import AsyncSandbox, Sandbox
from e2b.sandbox.commands.command_handle import CommandExitException

from gateway_pressure import (
    cleanup_handles, end_input, health, healthy, limited, start_input, wait_capacity,
)
from gateway_stream_limits import (
    command_overflow, disconnect_and_reconnect, download_boundary, upload_boundary,
)
from sdk_command_input import call
from sdk_server_smoke import options


async def admission(sandbox, url, transfer):
    directory = '/tmp/sandproof-task-admission'
    await call(sandbox.files.make_dir, directory)
    tracked = []
    allowance = transfer['max_transfers_per_vm'] - transfer['control_reserve']
    assert allowance >= 2
    try:
        for index in range(allowance):
            await start_input(sandbox, f'{directory}/start-{index}', tracked)
        refused_file = directory + '/refused-file'
        refused_start = directory + '/refused-start'
        await limited(sandbox.files.write, refused_file, b'must not arrive')
        await limited(sandbox.commands.run, 'touch ' + refused_start)
        await health(sandbox, url)
        await end_input(tracked[0], b'control input under full data admission')
        await wait_capacity(sandbox, directory)
        await healthy(sandbox, directory + '/after-eof')
        # Refill the released slot so SendSignal also runs while data is full.
        await start_input(sandbox, directory + '/refill', tracked)
        await limited(sandbox.files.read, directory + '/start-0')
        victim = tracked[1]
        assert await call(victim['handle'].kill)
        try:
            await call(victim['handle'].wait)
        except CommandExitException as error:
            assert error.exit_code != 0
        else:
            raise AssertionError('killed command reported success')
        victim['done'] = True
        for item in tracked:
            if not item['done']:
                await end_input(item)
        await wait_capacity(sandbox, directory)
        assert not await call(sandbox.files.exists, refused_file)
        assert not await call(sandbox.files.exists, refused_start)
        for item in tracked:
            assert await call(sandbox.files.read, item['marker']) == 'once\n'
        await healthy(sandbox, directory + '/recovered')
        return dict(data_slots=allowance, rejected_file_and_start=True,
                    reserved_health_input_eof_signal=True, no_replay=True)
    finally:
        await cleanup_handles(tracked)


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    budgets = json.loads((Path(__file__).resolve().parents[2] /
                          'benchmarks/local-service-budgets.json').read_text())
    transfer = budgets['data_transfer']
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    rows = []
    for cls in (Sandbox, AsyncSandbox):
        sandbox = await call(cls.create, template='base', timeout=600,
                             allow_internet_access=False, **owner)
        row = dict(sdk=cls.__name__, sandbox_id=sandbox.sandbox_id)
        try:
            if cls is Sandbox:
                directory = '/tmp/sandproof-task-stream'
                await call(sandbox.files.make_dir, directory)
                row['uploads'] = await call(upload_boundary, sandbox, directory,
                                            transfer['file_upload_bytes'])
                row['downloads'] = await call(download_boundary, sandbox, directory,
                                              transfer['file_download_bytes'])
                row['command_overflow'] = await call(command_overflow, sandbox, directory,
                                                     transfer['command_output_bytes'])
                row['disconnect'] = await call(disconnect_and_reconnect, sandbox, directory)
            row['admission'] = await admission(sandbox, args.sandbox_url, transfer)
        finally:
            assert await call(sandbox.kill), 'destruction not confirmed'
        rows.append(row)
    print(json.dumps(dict(sdk_version='2.51.0', budgets=budgets, results=rows,
                          scope='Original sync file/output boundaries and sync/async '
                                'per-VM admission at task budgets; host audit required. '
                                'No global saturation, long idle timeout or full fault matrix.'),
                     indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    asyncio.run(exercise(parser.parse_args()))
