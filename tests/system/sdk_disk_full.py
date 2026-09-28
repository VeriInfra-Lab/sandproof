"""Real guest ENOSPC with original sync/async SDK and a concurrent control VM.

Run only against an isolated, fixed-base Sandproof test service. The probe fills
its own guest root filesystem, never a host path. Exact host residual checks
remain the responsibility of the invoking KVM harness.
"""

import argparse
import asyncio
import errno
import importlib.metadata
import inspect
from io import BytesIO
import json
import os
import shlex
import uuid

from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import NotEnoughSpaceException

from gateway_stream_limits import UnknownLengthFile
from sdk_server_smoke import options


MIB = 1024 * 1024
PAYLOAD = b"X" * (256 * 1024)


async def call(function, *args, **kwargs):
    """Exercise both original SDK variants with exactly the same operations."""
    result = function(*args, **kwargs)
    return await result if inspect.isawaitable(result) else result


def fill_command(directory, max_disk_bytes):
    # fallocate consumes real filesystem blocks without buffering a disk-sized
    # Python object or doing unnecessary guest writes. Shrink to single blocks
    # after ENOSPC so an SDK upload cannot fit in the last allocation fragment.
    program = f"""
import errno, json, os
path = {directory!r} + '/fill'
root = os.statvfs('/')
assert 0 < root.f_blocks * root.f_frsize <= {max_disk_bytes}, 'unexpected guest disk capacity'
assert os.stat({directory!r}).st_dev == os.stat('/').st_dev, 'probe is not on the guest root disk'
initial = root.f_bfree * root.f_frsize
allocated = 0
errors = []
with open(path, 'xb') as stream:
    for chunk in (1024 * 1024, root.f_frsize):
        while allocated + chunk <= {max_disk_bytes}:
            try:
                os.posix_fallocate(stream.fileno(), allocated, chunk)
            except OSError as error:
                if error.errno != errno.ENOSPC:
                    raise
                errors.append(error.errno)
                break
            allocated += chunk
        else:
            raise AssertionError('allocation cap reached without ENOSPC')
    os.fsync(stream.fileno())
assert errors and errors[-1] == errno.ENOSPC
remaining = os.statvfs('/')
print(json.dumps(dict(errno=errors[-1], allocated_bytes=allocated, initial_free_bytes=initial,
    remaining_free_bytes=remaining.f_bfree * remaining.f_frsize,
    filesystem_bytes=root.f_blocks * root.f_frsize)))
"""
    return 'python3 -c ' + shlex.quote(program)


async def healthy(sandbox, marker, expected):
    assert (await call(sandbox.get_info)).state == 'running'
    result = await call(sandbox.commands.run, "printf enospc-control-healthy", timeout=15)
    assert result.stdout == 'enospc-control-healthy'
    assert await call(sandbox.files.read, marker, format='bytes') == expected
    await call(sandbox.files.write, marker, BytesIO(expected), request_timeout=15)


def body(kind):
    if kind == 'multipart-text':
        return PAYLOAD.decode()
    if kind == 'octet-stream':
        return BytesIO(PAYLOAD)
    return UnknownLengthFile(len(PAYLOAD))


async def case(cls, owner, max_disk_bytes):
    live = []
    directory = f'/var/tmp/sandproof-enospc-{uuid.uuid4().hex}'
    marker = directory + '/control'
    expected = b'independent guest remains writable\n'
    report = {'client': cls.__name__, 'errors': []}
    try:
        victim = await call(cls.create, template='base', timeout=300, allow_internet_access=False, **owner)
        live.append(victim)
        control = await call(cls.create, template='base', timeout=300, allow_internet_access=False, **owner)
        live.append(control)
        report.update(victim=victim.sandbox_id, control=control.sandbox_id)
        for sandbox in (victim, control):
            assert await call(sandbox.files.make_dir, directory)
        await call(control.files.write, marker, BytesIO(expected))
        kinds = ('multipart-text', 'octet-stream', 'nonseekable-stream')
        # Existing empty inodes distinguish data capacity from inode creation.
        for kind in kinds:
            await call(victim.files.write, directory + '/' + kind, b'')
        await healthy(control, marker, expected)
        result = await call(victim.commands.run, fill_command(directory, max_disk_bytes), timeout=60)
        report['fill'] = json.loads(result.stdout)
        assert report['fill']['errno'] == errno.ENOSPC
        for kind in kinds:
            try:
                await call(victim.files.write, directory + '/' + kind, body(kind), request_timeout=30)
            except NotEnoughSpaceException as error:
                report['errors'].append({'input': kind, 'exception': type(error).__name__})
            else:
                raise AssertionError(f'{kind} write returned success on the full disk')
            await healthy(control, marker, expected)
        # Release only this test's file, then prove the same sandbox can write.
        await call(victim.commands.run, 'rm -- ' + shlex.quote(directory + '/fill'), timeout=15)
        recovery = directory + '/recovered'
        await call(victim.files.write, recovery, BytesIO(PAYLOAD), request_timeout=30)
        assert await call(victim.files.read, recovery, format='bytes') == PAYLOAD
        report['write_after_release'] = True
        # Kill while the guest disk is full, not only after manually freeing it.
        result = await call(victim.commands.run, fill_command(directory, max_disk_bytes), timeout=60)
        report['refill'] = json.loads(result.stdout)
        assert report['refill']['errno'] == errno.ENOSPC
        assert await call(cls.kill, victim.sandbox_id, **owner), 'full guest destruction was not confirmed'
        live.remove(victim)
        report['killed_while_full'] = True
        await healthy(control, marker, expected)
        fresh = await call(cls.create, template='base', timeout=300, allow_internet_access=False, **owner)
        live.append(fresh)
        assert not await call(fresh.files.exists, directory), 'next guest inherited test data'
        await call(fresh.files.write, '/var/tmp/enospc-fresh', BytesIO(PAYLOAD), request_timeout=30)
        assert await call(fresh.files.read, '/var/tmp/enospc-fresh', format='bytes') == PAYLOAD
        report['fresh'] = fresh.sandbox_id
        return report
    finally:
        errors = []
        for sandbox in reversed(live):
            try:
                assert await call(cls.kill, sandbox.sandbox_id, **owner), 'test guest destruction was not confirmed'
            except Exception as error:
                errors.append(error)
        if errors:
            raise ExceptionGroup('exact test guest cleanup failed', errors)


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    cases = []
    for cls in (Sandbox, AsyncSandbox):
        cases.append(await case(cls, owner, args.max_guest_disk_bytes))
    print(json.dumps({'sdk_version': '2.51.0', 'cases': cases,
        'scope': 'real guest block ENOSPC, typed SDK error, another VM remains writable, same-VM recovery and full-disk destruction; excludes host filesystem exhaustion and full fault matrix'}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    parser.add_argument('--max-guest-disk-bytes', type=int, default=2 * 1024 * MIB,
                        help='refuse to fill a guest filesystem larger than this safety cap')
    args = parser.parse_args()
    assert args.max_guest_disk_bytes > 0
    asyncio.run(exercise(args))
