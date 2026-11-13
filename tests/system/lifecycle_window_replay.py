"""E2 extension of the original normal scenario, under its original deadline.

Waits for product window retirement, never changes the clock or the databases.
The existing runtime supplies all physical/accounting evidence and cleanup.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import copy
import time

PREFIX = '/sandproof/lifecycle'


def current_window(client):
    client.step('e2-current-request-window', 200)
    value = client.read_json('GET', PREFIX + '/request-window', key=client.owner_key)
    if (set(value) != {'window'} or type(value['window']) is not int
            or not 0 < value['window'] < 2**64 or value['window'] % 2 != 1):
        raise ValueError('one current unexhausted public window required')
    return value['window']


def wait_retired_window(client):
    original = client.create_request['window']
    started = time.monotonic()
    polls = 0
    while True:
        window = current_window(client)
        polls += 1
        if window < original:
            raise ValueError('public request window moved backwards')
        if window > original:
            return dict(original_window=original, current_window=window, polls=polls,
                        wait_seconds=time.monotonic() - started)
        # Read-only polling stays under the original scenario deadline. No new
        # key, wall-clock adjustment, reduced retention or write retry is used.
        time.sleep(min(.25, client.remaining()))


def rejected_original_requests(client):
    original = copy.deepcopy(client.create_request)
    changed = copy.deepcopy(original)
    if type(changed['parameters']['timeout_ms']) is not int or changed['parameters']['timeout_ms'] <= 1:
        raise ValueError('a valid altered original timeout is required')
    changed['parameters']['timeout_ms'] -= 1
    cases = [('original-create', PREFIX + '/create', original, 409),
             ('altered-create', PREFIX + '/create', changed, 409)]
    for operation in ('unload', 'resume', 'destroy'):
        request = getattr(client, operation + '_request')
        cases.append(('original-' + operation, client.path() + '/' + operation,
                      copy.deepcopy(request), 404))
    results = []
    for name, path, body, expected in cases:
        client.step('e2-expired-' + name, expected)
        status, _ = client.call('POST', path, key=client.owner_key, body=body)
        if status != expected:
            raise ValueError('expired request was not rejected with its exact original classification')
        results.append(dict(operation=name, status=status))
    for label, secret in [('owner', client.owner_key), ('foreign-owner', client.foreign_key)]:
        client.step('e2-retired-query-' + label, 404)
        status, _ = client.call('GET', client.path(), key=secret)
        if status != 404:
            raise ValueError('retired session became queryable')
        results.append(dict(operation=label + '-query', status=status))
    client.assert_data_closed()
    client.assert_previous_tokens_closed()
    return results


def check_fresh_acceptance(original, fresh, window):
    expected = dict(original['key'], window=window)
    if (set(fresh) != {'session', 'key', 'accepted_version'}
            or type(fresh['session']) is not str
            or fresh['session'] == original['session'] or not fresh['session']
            or type(fresh['key']) is not dict or set(fresh['key']) != set(expected)
            or any(type(fresh['key'][name]) is not type(value) for name, value in expected.items())
            or fresh['key'] != expected or type(fresh['accepted_version']) is not int
            or fresh['accepted_version'] != 1 or window <= original['key']['window']):
        raise ValueError('same owner/sequence in the new window must have one fresh session')


def exercise(client, runtime):
    if runtime.gc_complete is not True:
        raise ValueError('original product settlement must precede window retirement checks')
    retirement = wait_retired_window(client)
    rejected = rejected_original_requests(client)
    client.step('e2-expired-replay-physical-settlement')
    after_rejection = runtime.settlement_evidence(client.deadline)
    client.step('e2-retired-window-manager-reopen')
    reopened = runtime.restart_managers(client.deadline)
    if current_window(client) != retirement['current_window']:
        raise ValueError('manager reopen changed the durable request window')
    rejected_after_reopen = rejected_original_requests(client)
    after_reopen = runtime.settlement_evidence(client.deadline)

    # A rejection-only test could hide a permanently unusable authority. Use
    # the new window with the SAME original owner/sequence and parameters.
    fresh = type(client)(client.base_url, client.owner_key, client.foreign_key,
                         client.deployment, client.deadline, owner=client.owner)
    runtime.gc_complete = False
    runtime.physical_complete = False
    client.step('e2-fresh-window-create', 202)
    accepted = fresh.create(copy.deepcopy(client.create_request['parameters']),
                            sequence=client.create_request['sequence'])
    check_fresh_acceptance(client.create_acceptance, accepted, retirement['current_window'])
    running = fresh.wait_running()
    physical = runtime.record_running(fresh.session, client.deadline)
    fresh.operation_evidence_completed()
    workload = fresh.run_public_workload(running)
    client.step('e2-fresh-window-full-replay', 202)
    replay = fresh.read_json('POST', PREFIX + '/create', key=fresh.owner_key,
                             body=copy.deepcopy(fresh.create_request), status=202)
    if replay != accepted or fresh.query() != running:
        raise ValueError('new-window replay changed its original acceptance or current result')
    same_candidate = runtime.confirm_running(fresh.session, physical, client.deadline)
    rejected_with_successor = rejected_original_requests(client)
    if fresh.query() != running:
        raise ValueError('retired request changed the successor session')
    successor_after_rejection = runtime.confirm_running(fresh.session, physical, client.deadline)
    client.step('e2-fresh-window-product-cleanup')
    fresh.destroy(running)
    fresh.assert_data_closed()
    fresh.wait_cleaned()
    cleanup = runtime.destroy_evidence(fresh.session, client.deadline)
    runtime.physical_complete = True
    fresh.operation_evidence_completed()
    settlement = runtime.settlement_evidence(client.deadline)
    return dict(scope='natural retired window, original requests, reopen and one fresh successor',
                retirement=retirement, rejected=rejected,
                rejected_after_reopen=rejected_after_reopen,
                rejected_with_successor=rejected_with_successor,
                after_rejection=after_rejection, manager_reopen=reopened, after_reopen=after_reopen,
                fresh_acceptance=accepted, fresh_workload=workload,
                fresh_physical=physical, replay_candidate=same_candidate,
                successor_after_rejection=successor_after_rejection,
                fresh_cleanup=cleanup, final_settlement=settlement,
                fresh_operation_timings=fresh.timing_report())
