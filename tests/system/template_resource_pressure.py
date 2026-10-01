"""Post-task acceptance on two genuinely published templates.

This is a separate engineering probe: it never modifies original task files or
scores. Four private VMs coexist with one interrupted build. Failure retains
identities before attempting exact cleanup; no create or build POST is retried.
"""
import asyncio
import hashlib
import json
from pathlib import Path
import secrets
import shlex
import time
from types import SimpleNamespace
import urllib.error
import urllib.request

from e2b import AsyncSandbox
from e2b.exceptions import RateLimitException
from gateway_pressure import health, limited, wait_capacity
from sdk_private_root import clean, install, installed, CONFIG
from sdk_server_smoke import options
from tools.template_build_client import BuildClient

MIB = 1024**2


def validate_cohort(requirements, deployment):
    if len(requirements) != 2:
        raise ValueError('exactly two published template shapes required')
    count = 2 * len(requirements)
    cpu = 2 * sum(r['cpus'] for r in requirements)
    memory = 2 * sum((r['memory_mib'] + 256) * MIB for r in requirements)
    disk = 2 * sum(r['disk_mib'] * MIB for r in requirements)
    r, compute = deployment['runtime'], deployment['runtime_compute']
    if (count != r['max_active_global'] or count > r['max_active_per_tenant']
            or cpu > compute['max_vcpus_per_tenant'] or cpu > compute['max_vcpus_global']
            or memory > compute['max_memory_bytes_per_tenant'] or memory > compute['max_memory_bytes_global']
            or disk > r['max_disk_bytes_per_tenant']
            or disk + r['cleanup_reserve_bytes'] > r['max_disk_bytes_global']):
        raise ValueError('four-instance cohort exceeds the shared acceptance profile')
    transfer = r['data_transfer']
    ordinary = transfer['max_transfers'] - transfer['control_reserve']
    per_vm = transfer['max_transfers_per_vm'] - transfer['control_reserve']
    if not 1 <= ordinary <= 3 * per_vm:
        raise ValueError('three VMs must be able to saturate data while the fourth is stopped')
    if deployment['limits']['max_pending'] != 1:
        raise ValueError('probe requires a single admitted build reservation')


async def hold_input(sandbox, marker, tracked):
    ready, output = asyncio.Event(), ''
    def received(chunk):
        nonlocal output
        output += chunk
        if 'ready\n' in output:
            ready.set()
    # POSIX tools only; neither a benchmark recipe nor a Python installation.
    command = "printf 'once\\n' >> " + shlex.quote(marker) + "; printf 'ready\\n'; exec cat"
    handle = await sandbox.commands.run(command, user='root', stdin=True, background=True,
                                       timeout=180, on_stdout=received)
    item = {'handle': handle, 'sandbox': sandbox, 'marker': marker, 'done': False}
    tracked.append(item)
    await asyncio.wait_for(ready.wait(), timeout=15)
    return item


async def finish_input(item):
    await item['handle'].send_stdin('reserved-control\n', request_timeout=10)
    await item['handle'].close_stdin(request_timeout=10)
    result = await item['handle'].wait()
    assert (result.stdout, result.stderr, result.exit_code) == ('ready\nreserved-control\n', '', 0)
    item['done'] = True


async def verify_no_replay(tracked):
    for item in tracked:
        assert await item['sandbox'].files.read(item['marker'], user='root') == 'once\n'


async def exercise(*, root, urls, key, builder_key, rows, deployment, supervisor,
                   restart, source_entry, audit_shape, completion_audit, service_audit=None):
    templates = [r['template'] for r in rows]
    assert len({t['templateId'] for t in templates}) == 2
    validate_cohort([{'cpus': t['cpuCount'], 'memory_mib': t['memoryMib'],
                     'disk_mib': t['diskBytes'] // MIB} for t in templates], deployment)
    owner = options(key, urls[0], urls[0]) | {'request_timeout': 600, 'retries': 0}
    client = BuildClient(urls[2], key)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    live, tracked, identities, cases = [], [], [], []
    build_id, build_view = None, None
    stopped = False
    report = {'status': 'running', 'scope': 'published-template package isolation, shared admission and interrupted build',
              'identities': identities, 'cases': cases, 'build_id': None, 'exact_cleanup': 'unknown',
              'host_budget': deployment['host']}
    def checkpoint():
        (root / 'resource-pressure.json').write_text(json.dumps(report, indent=2) + '\n')
    checkpoint()
    async def create(template):
        attempt = {'template_id': template['templateId'], 'state': 'dispatched'}
        report.setdefault('create_attempts', []).append(attempt)
        checkpoint()
        sandbox = await AsyncSandbox.create(template=template['templateId'], timeout=1800,
                                             allow_internet_access=False, **owner)
        live.append(sandbox)
        attempt.update(state='known', sandbox_id=sandbox.sandbox_id)
        identity = {'sandbox_id': sandbox.sandbox_id, 'template_id': template['templateId']}
        identities.append(identity)
        checkpoint()
        identity['host'] = await asyncio.to_thread(audit_shape,
            SimpleNamespace(template=template, _sandbox=sandbox, sandbox_id=sandbox.sandbox_id))
        checkpoint()
        return sandbox
    async def build_get(path):
        return await asyncio.to_thread(client.request, 'GET', path)
    async def build_vm():
        request = urllib.request.Request(urls[1] + '/sandproof/internal-build-vms/' + build_id,
                                         headers={'X-API-Key': builder_key})
        def read():
            try:
                with opener.open(request, timeout=10) as response:
                    return json.loads(response.read(65536))
            except urllib.error.HTTPError as error:
                if error.code == 404:
                    return None
                raise
        return await asyncio.to_thread(read)
    failure = None
    try:
        cohort = [await create(t) for t in templates for _ in range(2)]
        assert len({i['host']['disk_inode'] for i in identities}) == 4
        for sandbox in cohort:
            await clean(sandbox)
        for index, sandbox in enumerate(cohort):
            await install(sandbox, f'{index + 1}.0', sandbox.sandbox_id)
        async def verify_packages():
            for index, sandbox in enumerate(cohort):
                if sandbox not in live:
                    continue
                t = templates[index // 2]
                await installed(sandbox, f'{index + 1}.0', sandbox.sandbox_id,
                                resources=(t['cpuCount'], t['memoryMib']))
        await verify_packages()
        await cohort[0].files.write(CONFIG, 'private-change\n', user='root')
        for sandbox in cohort[1:]:
            assert await sandbox.files.read(CONFIG, user='root') == sandbox.sandbox_id + '\n'
        await cohort[0].files.write(CONFIG, cohort[0].sandbox_id + '\n', user='root')
        cases.append('four-independent-package-databases-and-system-files')
        try:
            await create(templates[0])
        except RateLimitException:
            cases.append('full-runtime-refuses-new-create')
        else:
            raise AssertionError('full runtime admitted a fifth instance')
        token = secrets.token_hex(16)
        request = {'kind': 'oci', 'image': source_entry['upstream_image_reference'], 'allowInternetAccess': True,
                   'timeoutSeconds': int(source_entry['requirements']['build_timeout_sec']),
                   'resources': {'cpuCount': 1, 'memoryMiB': 256, 'diskMiB': 1024, 'minFreeMiB': 64}}
        report['build_intent'] = {'token': token, 'request': request}
        checkpoint()
        build_view = await asyncio.to_thread(client.request, 'POST', '/sandproof/template-builds', request, token=token)
        build_id = build_view['buildID']
        report['build_id'] = build_id
        checkpoint()
        # Pending capacity must be the active constraint, not exhausted history.
        assert deployment['limits']['max_jobs'] >= len(rows) + 2
        rejected_token = secrets.token_hex(16)
        report['expected_rejection_intent'] = {'token': rejected_token, 'request': request}
        checkpoint()
        try:
            unexpected = await asyncio.to_thread(client.request, 'POST', '/sandproof/template-builds', request,
                                                 token=rejected_token)
        except RuntimeError as error:
            assert 'BuildBudgetExceeded' in str(error)
        else:
            report['unexpected_build_response'] = unexpected
            checkpoint()
            raise AssertionError('full build reservation admitted another job')
        assert (await build_get('/sandproof/template-builds/' + build_id))['buildID'] == build_id
        await build_get('/sandproof/template-builds/' + build_id + '/logs?offset=0&limit=1024')
        cases.append('full-build-still-serves-status-and-logs')
        while True:
            vm = await build_vm()
            if vm and vm.get('session'):
                report['original_builder_sandbox'] = vm['sandboxID']
                break
            assert time.time() < build_view['deadline'], 'original builder failed to reach the interruption barrier'
            build_view = await build_get('/sandproof/template-builds/' + build_id)
            assert build_view['state'] not in ('Ready', 'Failed', 'CleanupUnknown'), build_view['state']
            await asyncio.sleep(0.2)
        supervisor.kill()
        await asyncio.to_thread(supervisor.wait, timeout=10)
        stopped = True
        checkpoint()
        transfer = deployment['runtime']['data_transfer']
        remaining = transfer['max_transfers'] - transfer['control_reserve']
        per_vm = transfer['max_transfers_per_vm'] - transfer['control_reserve']
        for sandbox in cohort[:3]:
            for number in range(min(per_vm, remaining)):
                await hold_input(sandbox, '/tmp/pressure-' + str(number), tracked)
                remaining -= 1
        assert remaining == 0
        if service_audit is not None:
            report['service_pressure'] = await asyncio.to_thread(
                service_audit, transfer['max_transfers'] - transfer['control_reserve'])
            checkpoint()
        await limited(cohort[2].files.write, '/tmp/refused-pressure-file', b'must-not-arrive')
        await health(cohort[2], urls[0])
        # The fourth VM is independent of the 15 blocked data streams. Its
        # management stop must work while both pools still own their quotas.
        assert await cohort[3].kill(request_timeout=60)
        live.remove(cohort[3])
        cases.append('runtime-stop-survives-full-data-and-interrupted-builder')
        for item in tracked:
            await finish_input(item)
        await wait_capacity(cohort[2], '/tmp')
        assert not await cohort[2].files.exists('/tmp/refused-pressure-file')
        await verify_no_replay(tracked)
        await verify_packages()
        cases.append('reserved-stdin-eof-no-command-replay')
        supervisor = restart()
        stopped = False
        deadline = time.monotonic() + deployment['limits']['timeout_seconds'] + 90
        while True:
            try:
                build_view = await build_get('/sandproof/template-builds/' + build_id)
            except OSError:
                assert supervisor.poll() is None and time.monotonic() < deadline
                await asyncio.sleep(0.2)
                continue
            if build_view['state'] in ('Ready', 'Failed'):
                break
            assert time.monotonic() < deadline
            await asyncio.sleep(0.2)
        assert build_view['state'] == 'Failed' and build_view['failure'] == 'InterruptedBuild', build_view
        vm = await build_vm()
        assert vm['sandboxID'] == report['original_builder_sandbox'] and vm['cleanupComplete']
        cases.append('restart-reclaims-original-builder-without-rebuild')
        replacement = await create(templates[1])
        await clean(replacement)
        await verify_packages()
        # Exercise the same transfer policy with a bounded SDK round trip.
        payload = b'bounded-template-transfer\0' * (8 * MIB // 26)
        await replacement.files.write('/tmp/roundtrip', payload, user='root')
        received = bytes(await replacement.files.read('/tmp/roundtrip', format='bytes', user='root'))
        assert hashlib.sha256(received).digest() == hashlib.sha256(payload).digest()
        cases.append('replacement-private-and-transfer-hash-matches')
    except BaseException as error:
        failure = error
        report['status'] = 'failed'
        report['error_type'] = type(error).__name__
    finally:
        errors = []
        # Keep recovery available even when pressure assertions fail.
        if stopped:
            try:
                supervisor = restart()
            except BaseException as error:
                errors.append(error)
        for item in tracked:
            try:
                if not item['done']:
                    await asyncio.wait_for(item['handle'].kill(), timeout=10)
                await item['handle'].disconnect()
            except BaseException as error:
                errors.append(error)
        for sandbox in reversed(live):
            try:
                assert await sandbox.kill(request_timeout=60)
            except BaseException as error:
                errors.append(error)
        try:
            row = {'build_dispatched': build_id is not None, 'build_id': build_id, 'build': build_view}
            if report.get('build_intent') and build_id is None:
                raise RuntimeError('lost build admission: reconcile original checkpoint token')
            audit = await asyncio.to_thread(completion_audit, row)
            report['exact_cleanup'] = audit['exact_cleanup']
            report['build'] = audit.get('build')
        except BaseException as error:
            errors.append(error)
        report['cleanup_error_types'] = [type(error).__name__ for error in errors]
        if errors:
            report['status'] = 'failed' if report['exact_cleanup'] == 'passed' else 'cleanup_unknown'
        elif failure is None:
            report['status'] = 'passed'
        checkpoint()
        if errors:
            raise BaseExceptionGroup('resource pressure or cleanup failed',
                                     ([failure] if failure else []) + errors)
    if failure is not None:
        raise failure
    return report
