"""Cross-VM admission at the fixed local task transfer budgets.

The supervisor must enable three concurrent VMs (two in tenant one, one in
tenant two). All data_transfer values stay identical to the serial task file.
Run both original SDK clients; stdin readiness is the admission barrier.
"""

import asyncio
import hashlib
import shlex

from e2b import AsyncSandbox
from e2b.sandbox.commands.command_handle import CommandExitException

from gateway_pressure import (
    cleanup_handles, end_input, health, healthy, limited, start_input, wait_capacity,
)
from gateway_stream_limits import UnknownLengthFile, pattern_digest
from sdk_command_input import call


async def admission(sandboxes, url, limits, observe):
    per_vm = limits['max_transfers_per_vm'] - limits['control_reserve']
    global_slots = limits['max_transfers'] - limits['control_reserve']
    # With the locked 16/8/1 budget this is 7 + 7 + 1; VM three still has
    # six free local slots when its ordinary requests are globally refused.
    last = global_slots - 2 * per_vm
    assert 0 < last < per_vm
    directory = '/tmp/task-global-pressure'
    for sandbox in sandboxes:
        await call(sandbox.files.make_dir, directory)
    tracked = []
    refused = directory + '/refused'
    try:
        for vm_index, count in enumerate((per_vm, per_vm, last)):
            sandbox = sandboxes[vm_index]
            for index in range(count):
                await start_input(sandbox, f'{directory}/start-{vm_index}-{index}', tracked)
            if vm_index == 0:
                await limited(sandbox.files.write, refused, b'not admitted')
                # Global capacity remains: another tenant can still use it.
                await healthy(sandboxes[2], directory + '/other-tenant')
        await observe('global-full')
        for sandbox in sandboxes:
            await limited(sandbox.files.write, refused, b'not admitted')
            await limited(sandbox.commands.run, 'touch ' + shlex.quote(refused))
            await health(sandbox, url)
        await end_input(tracked[0], b'control input under all fifteen data slots')
        await wait_capacity(sandboxes[2], directory)
        await healthy(sandboxes[2], directory + '/after-eof')
        await start_input(sandboxes[2], directory + '/refill', tracked)
        await limited(sandboxes[2].files.read, directory + '/other-tenant')
        victim = tracked[per_vm]
        assert await call(victim['handle'].kill)
        try:
            await call(victim['handle'].wait)
        except CommandExitException as error:
            assert error.exit_code != 0
        else:
            raise AssertionError('signalled command reported success')
        victim['done'] = True
        await wait_capacity(sandboxes[2], directory)
        await healthy(sandboxes[2], directory + '/after-signal')
        for item in tracked:
            if not item['done']:
                await end_input(item)
        for sandbox in sandboxes:
            await wait_capacity(sandbox, directory)
            assert not await call(sandbox.files.exists, refused)
            await healthy(sandbox, directory + '/recovered')
        for item in tracked:
            assert await call(item['sandbox'].files.read, item['marker']) == 'once\n'
        await observe('all-slots-released')
        return {'held_data_streams_per_vm': [per_vm, per_vm, last],
                'global_data_slots': global_slots, 'per_vm_and_global_429': True,
                'reserved_health_input_eof_signal': True,
                'capacity_reusable_after_eof_and_signal': True,
                'rejected_side_effects_absent': True, 'all_start_markers_once': True}
    finally:
        await cleanup_handles(tracked)


async def transfer(sandbox, size):
    """Bounded producer/consumer; do not allocate a whole file in the client."""
    path = f'/tmp/task-global-pressure/stream-{size}'
    with UnknownLengthFile(size) as source:
        await call(sandbox.files.write, path, source, request_timeout=120)
    assert (await call(sandbox.files.get_info, path)).size == size
    expected = pattern_digest(size)
    guest = await call(sandbox.commands.run, 'sha256sum ' + shlex.quote(path), timeout=30)
    assert guest.stdout.split()[0] == expected
    received, digest = 0, hashlib.sha256()
    if isinstance(sandbox, AsyncSandbox):
        async with await sandbox.files.read(path, format='stream', request_timeout=120) as stream:
            async for chunk in stream:
                received += len(chunk)
                digest.update(chunk)
    else:
        def consume():
            nonlocal received
            with sandbox.files.read(path, format='stream', request_timeout=120) as stream:
                for chunk in stream:
                    received += len(chunk)
                    digest.update(chunk)
        await asyncio.to_thread(consume)
    assert received == size and digest.hexdigest() == expected
    await call(sandbox.commands.run, 'rm -- ' + shlex.quote(path), timeout=15)
    return {'sandbox_id': sandbox.sandbox_id, 'bytes': size, 'sha256': expected}


async def exercise(sandboxes, url, limits, observe):
    result = {'admission': await admission(sandboxes, url, limits, observe), 'transfers': []}
    for size in (8 * 1024 * 1024, limits['file_upload_bytes']):
        assert size <= limits['file_download_bytes']
        await observe(f'transfer-{size}-start')
        # Keep all task outcomes, so a failed sibling is not left running when
        # the supervisor starts destruction. No failed write is retried.
        outcomes = await asyncio.gather(*(transfer(vm, size) for vm in sandboxes),
                                        return_exceptions=True)
        failures = [item for item in outcomes if isinstance(item, BaseException)]
        if failures:
            raise BaseExceptionGroup('concurrent transfers failed', failures)
        result['transfers'].extend(outcomes)
        await observe(f'transfer-{size}-complete')
    return result
