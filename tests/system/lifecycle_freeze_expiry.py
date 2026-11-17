"""One frozen-Unload expiry observation using the original Rust audit.

The Host is down at Freeze-success/pre-SQL and the checked VMM is held by the
inherited pidfd. Center stays live across the real persisted deadline. Its
existing writer-lease audit is read only while Center is stopped, then that
same configured service is restarted. This is a finite test consumer, not a
second lifecycle implementation or an authority to free resources.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name('_assertions_required.py'))
)["require_assertions"]
_require_assertions()

import copy
import json
import time

from _internal_fault_relay import _fields, _u64, accepted_request
from lifecycle_freeze_destroy import FreezeDestroyProbe, OrderedFreezeDestroyProbe
from lifecycle_unload_expiry import (_check_audited_history, expiry_resume_witness,
                                     read_expiry_history)
from lifecycle_unload_runtime import remaining
from tools.check_kvm_host import unique_keys
from tools.template_build_guest import equal


def now_ms():
    return time.time_ns() // 1_000_000


def require_before_expiry(lifetime):
    if now_ms() >= lifetime['end_at_ms']:
        raise ValueError('real expiry preceded the selected frozen boundary')


def _row(audit, session):
    matches = [r for r in audit['sandboxes'] if r['create']['binding']['session'] == session]
    if len(matches) != 1:
        raise ValueError('one original frozen session in Rust audit required')
    row = matches[0]
    _fields(row, 'create private operations terminating released phase last_running terminal_confirmation')
    return row


def _intent(operation):
    # These operations are exported in full by OperationAudit. User Create
    # options are redacted there and are explicitly outside this projection.
    _fields(operation, 'center binding retention accepted_version acceptance_time operation running create_refusal terminal unloaded')
    if operation['operation']['kind'] not in ('unload', 'resume', 'destroy'):
        raise ValueError('complete original non-Create audit projection required')
    return dict(request=dict(binding=operation['binding'], operation=operation['operation']),
                center=operation['center'], retention=operation['retention'],
                accepted_version=operation['accepted_version'])


def frozen_baseline(audit, create_acceptance, intents, node, lifetime):
    """Bind audit to the three original Host intents before TTL, not samples."""
    _fields(create_acceptance, 'session key accepted_version')
    _fields(lifetime, 'started_at_ms end_at_ms max_end_at_ms startup_deadline_ms')
    for value in lifetime.values():
        _u64(value)
    if len(intents) != 3:
        raise ValueError('original Unload/Resume/Unload intents required')
    session = create_acceptance['session']
    row = _row(audit, session)
    operations = row['operations']
    if (len(operations) != 4
            or [o['operation']['kind'] for o in operations] != ['create', 'unload', 'resume', 'unload']
            or row['terminating'] is not False or row['released'] is not False
            or row['phase'] != 'unloading' or row['terminal_confirmation'] is not None):
        raise ValueError('original pending frozen history required before expiry')
    first = operations[0]
    if (not equal(create_acceptance, dict(session=first['binding']['session'],
            key=first['binding']['key'], accepted_version=first['accepted_version']))
            or not equal(first['binding'], row['create']['binding'])
            or not equal({k:row['last_running'][k] for k in lifetime}, lifetime)
            or not equal(row['create']['timeout_ms'], lifetime['end_at_ms']-lifetime['started_at_ms'])):
        raise ValueError('original public Create and committed lifetime differ')
    for operation, intent in zip(operations[1:], intents, strict=True):
        accepted_request(intent, node)
        if (not equal(intent['request']['binding']['node'], node)
                or intent['request']['binding']['session'] != session
                or not equal(_intent(operation), intent)):
            raise ValueError('center history differs from complete original Host intent')
    pending = operations[-1]
    if any(pending[k] is not None for k in ('running', 'create_refusal', 'terminal', 'unloaded')):
        raise ValueError('frozen Unload acquired a result before the boundary')
    return copy.deepcopy(row)


def expiry_acceptance(audit, baseline, node, lifetime, sampled_now):
    """Only one appended full internal Destroy; all original history is exact."""
    if _u64(sampled_now) < lifetime['end_at_ms']:
        raise ValueError('real persisted deadline has not arrived')
    row = _row(audit, baseline['create']['binding']['session'])
    if equal(row, baseline):
        return None
    if len(row['operations']) != len(baseline['operations'])+1:
        raise ValueError('expiry must append exactly one terminal request')
    terminal = row['operations'][-1]
    actual = _intent(terminal)
    accepted_request(actual, node)
    prior = _intent(baseline['operations'][-1])
    key = terminal['binding']['key']
    expected = copy.deepcopy(prior)
    expected['accepted_version'] += 1
    expected['request']['binding'].update(key=copy.deepcopy(key),
        expected=dict(state='version', version=prior['accepted_version']))
    expected['request']['operation'] = {'kind':'destroy'}
    if (not equal(actual, expected) or not equal(key['window'], 2)
            or equal(key, prior['request']['binding']['key'])
            or any(not equal(key[k], prior['request']['binding']['key'][k]) for k in ('owner', 'deployment'))
            or not equal(terminal['acceptance_time'], {'kind':'unsampled'})
            or any(terminal[k] is not None for k in ('running', 'create_refusal', 'terminal', 'unloaded'))):
        raise ValueError('complete automatic terminal intent or unsampled time differs')
    expected_row = copy.deepcopy(baseline)
    expected_row.update(operations=expected_row['operations']+[copy.deepcopy(terminal)],
                        terminating=True, phase='terminating')
    if not equal(row, expected_row):
        raise ValueError('expiry changed original history, lifetime or material responsibility')
    return dict(session=terminal['binding']['session'], key=copy.deepcopy(key),
                accepted_version=terminal['accepted_version'])


def wait_expired_cleaned(client, deadline):
    """Query original terminal work without a synthetic Destroy timing sample."""
    while True:
        remaining(deadline)
        status, raw = client.call('GET', client.path(), key=client.owner_key)
        if status == 503:
            time.sleep(min(.1, remaining(deadline)))
            continue
        if status != 200:
            raise ValueError('original automatic terminal query required')
        view = json.loads(raw, object_pairs_hook=unique_keys)
        client.check_view(view, client.current_acceptance)
        if view['phase'] == 'cleaned':
            return view
        if view['phase'] != 'terminating':
            raise ValueError('automatic terminal decision was revived or replaced')
        time.sleep(min(.1, remaining(deadline)))


class FreezeExpiryProbe(FreezeDestroyProbe):
    """Reuse exact Freeze/Stop/receipt/pin/exit checks; change only authority."""
    def _audit(self, deadline):
        remaining(deadline)
        processes = self.runtime.services.processes
        processes.stop('center')
        try:
            return processes.audit('center')
        finally:
            processes.start('center')

    def after_acceptance(self, accepted, deadline):
        result = super().after_acceptance(accepted, deadline)
        lifetime = self.prior['lifetime']
        require_before_expiry(lifetime)
        backing = self._backing(self.initial_sources)
        intents = [backing['work']['anchor']['request'], backing['restore']['intent'], self.unload_entry['intent']]
        node = self.runtime.services.host_configs[0]['inbox']['node']
        deadline = min(deadline, self.deadline)
        resume = expiry_resume_witness(self.running_acceptance, intents[1], node)
        history = read_expiry_history(self.runtime, self.client, resume, intents[0], deadline,
                                      current_unload=intents[2])
        baseline = frozen_baseline(self._audit(deadline), self.client.create_acceptance,
                                   intents, node, lifetime)
        _check_audited_history(baseline, history)
        require_before_expiry(lifetime)
        self.expiry_baseline = baseline
        self.automatic_acceptance = None
        return dict(result, original_center_frozen_baseline=copy.deepcopy(self.expiry_baseline),
                    boundary_before_real_expiry=True)

    def observe_expiry(self, deadline):
        if (self.client.destroy_request is not None or self.automatic_acceptance is not None
                or not equal(self.client.current_acceptance, self.accepted)):
            raise ValueError('unmodified original Unload and no explicit Destroy required')
        deadline = min(deadline, self.deadline)
        lifetime = self.prior['lifetime']
        require_before_expiry(lifetime)
        processes = self.runtime.services.processes
        # Pausing has no Host-native Running expiry scan. Keep Center live
        # across TTL; stopping it until after TTL would test a different path.
        while True:
            left = remaining(deadline)
            if processes.children['center'].poll() is not None:
                raise ValueError('original Center exited before autonomous expiry observation')
            delay = (lifetime['end_at_ms']-now_ms())/1000
            if delay <= 0:
                break
            time.sleep(min(.1, left, delay))
        polls = 0
        while True:
            if self.host.poll() != 86 or self._source_state(deadline) != 'T':
                raise ValueError('original pre-receipt Host and held frozen source required')
            accepted = expiry_acceptance(self._audit(deadline), self.expiry_baseline,
                self.runtime.services.host_configs[0]['inbox']['node'], lifetime, now_ms())
            polls += 1
            if accepted is not None:
                break
            time.sleep(min(1, remaining(deadline)))
        self.automatic_acceptance = copy.deepcopy(accepted)
        self.client.current_acceptance = copy.deepcopy(accepted)  # Observe actual key; allocate nothing.
        return dict(acceptance=accepted, end_at_ms=lifetime['end_at_ms'], observed_at_ms=now_ms(),
            observed_monotonic=time.monotonic(),
            center_audits=polls, explicit_destroy_sent=False, public_query_before_durable_expiry=False,
            acceptance_time='unsampled: no invented expiry acceptance timestamp')

    def _bind_destroy(self, accepted):
        if (self.client.destroy_request is not None or self.terminal_accepted is not None or self.recovered
                or self.automatic_acceptance is None or not equal(accepted, self.automatic_acceptance)
                or not equal(accepted, self.client.current_acceptance)):
            raise ValueError('only the audited automatic terminal acceptance may reach Stop')
        self.terminal_accepted = copy.deepcopy(accepted)

    def recover_after_expiry(self, accepted, deadline, *, observe_peer):
        result = super().recover_after_destroy(accepted, deadline, observe_peer=observe_peer)
        result['automatic_terminal_reached_host'] = result.pop('public_destroy_reached_host')
        return result

    def terminal_failure_state(self):
        result = super().terminal_failure_state()
        result['automatic_terminal'] = result.pop('destroy')
        result['observed_automatic_acceptance'] = copy.deepcopy(getattr(self, 'automatic_acceptance', None))
        result['explicit_destroy_sent'] = self.client.destroy_request is not None
        return result


class UnresponsiveFreezeExpiryProbe(FreezeExpiryProbe):
    """Same automatic terminal identity, with no test-side VMM continuation."""
    continue_source_before_stop = False

    def _prepare_source_for_stop(self,deadline):
        from lifecycle_freeze_destroy import UnresponsiveFreezeDestroyProbe
        return UnresponsiveFreezeDestroyProbe._prepare_source_for_stop(self,deadline)


class OrderedFreezeExpiryProbe(FreezeExpiryProbe, OrderedFreezeDestroyProbe):
    """Keep audited expiry authority; reuse the original ordered recovery hook.

    FreezeExpiryProbe binds only the observed automatic acceptance and keeps
    the first session free of public Destroy calls. Its recovery then enters
    OrderedFreezeDestroyProbe: restore the same VMM's response after exact
    durable Host Kill, before waiting for the original fence or Stop.
    """


def expiry_probe_for_scenario(scenario):
    """Keep each response experiment on its own original expiry consumer."""
    probes={
        'capture-freeze-expiry':FreezeExpiryProbe,
        'capture-freeze-receipt-expiry':OrderedFreezeExpiryProbe,
        'capture-freeze-unresponsive-expiry':UnresponsiveFreezeExpiryProbe,
    }
    if scenario not in probes:
        raise ValueError('one explicit frozen-Unload expiry scenario required')
    return probes[scenario]
