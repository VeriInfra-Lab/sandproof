"""Post-task acceptance on two genuinely published templates.

This is a separate engineering probe: it never modifies original task files or
scores. Four private VMs coexist with one interrupted build. Failure retains
identities before attempting exact cleanup; no create or build POST is retried.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import asyncio
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import shlex
import time
from types import SimpleNamespace
import urllib.error
import urllib.parse
import urllib.request

from e2b import AsyncSandbox
from e2b.exceptions import RateLimitException
from gateway_pressure import health, limited, wait_capacity
from sdk_private_root import clean, install, installed, CONFIG
from sdk_server_smoke import options
from tools.template_build_client import BuildClient, saved_intent
from tools.bounded_acceptance import wait_private_build_window

MIB = 1024**2


def ready_for_pressure(rows):
    """Independent resource probe, not a replacement for the original score.

    Only known Ready templates after exact per-task reclaim are eligible.
    Benchmark failure remains a failure in the enclosing report/exit status.
    """
    return len(rows) == 2 and all(
        (row.get('build') or {}).get('state') == 'Ready'
        and row.get('template') and row.get('exact_cleanup') == 'passed'
        for row in rows)


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


async def submit_build(root, client, request, build_mode, label, checkpoint):
    """Persist the ordinary client's original window/key before the only POST."""
    if build_mode != 'windowed':
        raise ValueError('only windowed builds are supported')
    intent = await asyncio.to_thread(saved_intent, root / ('pressure-' + label + '-intent.json'),
                                     client, request)
    checkpoint({'token': intent['idempotency_key'], 'request': request, 'build_mode': build_mode,
                'requestWindow': intent['requestWindow']})
    body = request | {'requestWindow': intent['requestWindow']}
    return await asyncio.to_thread(client.request, 'POST', '/sandproof/template-builds',
                                   body, token=intent['idempotency_key'])


def private_vm_path(job, window):
    path = '/sandproof/internal-build-vms/' + job
    return path + ('?' + urllib.parse.urlencode({'requestWindow': window}) if window else '')


def private_settlement(database, job, window, sandbox):
    """Read the committed acknowledgement after the private anchor is retired.

    A successful bounded cleanup removes the private lookup anchor. Its 404 is
    not a new failure; require the original window and sandbox in the durable
    public settlement, plus the supervisor's acknowledgement, before reporting.
    The enclosing probe still independently audits actual resources.
    """
    with closing(sqlite3.connect(Path(database).as_uri() + '?mode=ro', uri=True)) as connection:
        row = connection.execute(
            'SELECT state,private_window,builder_settled,private_ack FROM jobs WHERE id=?', (job,)).fetchone()
    expected = {'requestWindow': window, 'sandboxID': sandbox}
    if (row is None or row[0] != 'Failed' or row[3] != 1
            or json.loads(row[1] or 'null') != window or json.loads(row[2] or 'null') != expected):
        raise ValueError('original private build settlement is incomplete or mismatched')
    return expected


async def wait_builder_session(read_vm, read_build, deadline, supervisor):
    """Observe the original admission; a slow read cannot authorize a new POST.

    Private controller reads can wait behind a bounded disk preparation. Retry
    only read timeouts, within the original build deadline; terminal builds,
    other errors and cancellation still fail the interruption probe.
    """
    while True:
        assert time.time() < deadline, 'original builder missed the interruption barrier'
        assert supervisor.poll() is None, 'build supervisor exited before the barrier'
        try:
            vm = await read_vm()
        except TimeoutError:
            vm = None
        if vm and vm.get('session'):
            assert time.time() < deadline, 'builder observation exceeded original deadline'
            return vm
        assert time.time() < deadline, 'original builder missed the interruption barrier'
        view = await read_build()
        assert view['state'] not in ('Ready', 'Failed', 'CleanupUnknown'), view['state']
        await asyncio.sleep(0.2)


async def exercise(*, root, urls, key, builder_key, rows, deployment, supervisor,
                   restart, source_entry, audit_shape, completion_audit, build_mode, service_audit=None):
    if build_mode != 'windowed':
        raise ValueError('only windowed builds are supported')
    templates = [r['template'] for r in rows]
    assert len({t['templateId'] for t in templates}) == 2
    validate_cohort([{'cpus': t['cpuCount'], 'memory_mib': t['memoryMib'],
                     'disk_mib': t['diskBytes'] // MIB} for t in templates], deployment)
    owner = options(key, urls[0], urls[0]) | {'request_timeout': 600, 'retries': 0}
    client = BuildClient(urls[2], key)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    live, tracked, identities, cases = [], [], [], []
    build_id, build_view, private_window = None, None, None
    stopped = False
    report = {'status': 'running', 'build_mode': build_mode,
              'scope': 'published-template package isolation, shared admission and interrupted build',
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
        request = urllib.request.Request(urls[1] + private_vm_path(build_id, private_window),
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
        request = {'kind': 'oci', 'image': source_entry['upstream_image_reference'], 'allowInternetAccess': True,
                   'timeoutSeconds': int(source_entry['requirements']['build_timeout_sec']),
                   'resources': {'cpuCount': 1, 'memoryMiB': 256, 'diskMiB': 1024, 'minFreeMiB': 64}}
        def save_intent(field, intent):
            report[field] = intent
            checkpoint()
        build_view = await submit_build(root, client, request, build_mode, 'interruption',
                                        lambda intent: save_intent('build_intent', intent))
        build_id = build_view['buildID']
        report['build_id'] = build_id
        checkpoint()
        # Pending capacity must be the active constraint, not exhausted history.
        assert deployment['limits']['max_jobs'] >= len(rows) + 2
        try:
            unexpected = await submit_build(root, client, request, build_mode, 'expected-rejection',
                                             lambda intent: save_intent('expected_rejection_intent', intent))
        except RuntimeError as error:
            assert 'BuildBudgetExceeded' in str(error)
        else:
            report['unexpected_build_response'] = unexpected
            checkpoint()
            raise AssertionError('full build reservation admitted another job')
        assert (await build_get('/sandproof/template-builds/' + build_id))['buildID'] == build_id
        await build_get('/sandproof/template-builds/' + build_id + '/logs?offset=0&limit=1024')
        cases.append('full-build-still-serves-status-and-logs')
        remaining = build_view['deadline'] - time.time()
        assert remaining > 0, 'original build deadline elapsed before window observation'
        private_window = await asyncio.to_thread(wait_private_build_window,
            root / 'builds/builds.db', build_id, timeout=remaining)
        vm = await wait_builder_session(build_vm,
            lambda: build_get('/sandproof/template-builds/' + build_id), build_view['deadline'], supervisor)
        report['original_builder_sandbox'] = vm['sandboxID']
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
        _kill_confirmed = (await cohort[3].kill(request_timeout=60))
        assert _kill_confirmed
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
        report['private_settlement'] = await asyncio.to_thread(private_settlement,
            root / 'builds/builds.db', build_id, private_window, report['original_builder_sandbox'])
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
                _kill_confirmed = (await sandbox.kill(request_timeout=60))
                assert _kill_confirmed
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
