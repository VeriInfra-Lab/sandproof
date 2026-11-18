"""Close an original SDK command subscription, never the guest process.

The existing scenario starts the RAM witness once. This observer only connects
to that public PID, closes each returned subscription once on its owner thread,
and observes its exact host relay through a pidfd. It sends no process signal,
changes no SQL, and creates no replacement guest command on a lost Connect.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import copy
import hashlib
import os
import time

from lifecycle_unload_data_drain import InflightDataDrain, _live_routes
from lifecycle_unload_runtime import (
    open_original_candidate_pidfd, original_json, pidfd_exited, remaining,
)


class CommandDisconnectProbe:
    def __init__(self, runtime, client):
        self.runtime, self.client = runtime, client
        self.native = runtime.services.native_pools[0]
        self.deadline = client.deadline
        self.guest = copy.deepcopy(client.witness)
        self.handles = []
        self.pidfds = []
        self.rows = []
        self.closed = False
        self.failures = []
        self._guest_identity(self.guest)

    def _guest_identity(self, witness):
        if (type(witness) is not dict or set(witness) != {
                'pid', 'start_time_ticks', 'counter', 'metadata_generation',
                'nonce', 'activation_nonce'}):
            raise ValueError('complete original RAM witness required')
        for key in ('pid', 'start_time_ticks', 'counter', 'metadata_generation'):
            if type(witness[key]) is not int or not 0 <= witness[key] < 2**64:
                raise ValueError('original complete RAM witness required')
        if witness['pid'] == 0 or witness['start_time_ticks'] == 0:
            raise ValueError('positive original guest process identity required')
        for key in ('nonce', 'activation_nonce'):
            value = witness[key]
            if (type(value) is not str or len(value) != 64
                    or any(char not in '0123456789abcdef' for char in value)):
                raise ValueError('original RAM-only witness nonce required')
        if any(witness[key] != self.guest[key] for key in ('pid', 'start_time_ticks', 'nonce')):
            raise ValueError('original guest process or RAM identity changed')

    def _disconnect(self, owned):
        if not owned['attempted']:
            owned['attempted'] = True  # An unknown close is never retried.
            try:
                owned['handle'].disconnect()
            except BaseException as error:
                self.failures.append(type(error).__name__)
                raise

    def _connect(self, running, sdk):
        baseline = _live_routes(self.native, running)
        # SDK 2.51.0 returns after the original Start event. Its event generator
        # is suspended here; no cross-thread close of an executing generator.
        handle = sdk.commands.connect(self.guest['pid'],
            timeout=min(30, remaining(self.deadline)))
        owned = dict(handle=handle, attempted=False)
        self.handles.append(owned)  # Even an invalid reply owns a subscription.
        if type(handle.pid) is not int or handle.pid != self.guest['pid']:
            raise ValueError('Connect must refer to the single original guest PID')
        deadline = min(self.deadline, time.monotonic() + 30)
        while True:
            remaining(deadline)
            routes = {key: value for key, value in _live_routes(self.native, running).items()
                      if key not in baseline}
            if len(routes) > 1:
                raise ValueError('ambiguous newly admitted command relay')
            if routes:
                marker, route = next(iter(routes.values()))
                return owned, marker, route, deadline
            time.sleep(min(.005, remaining(deadline)))

    def _subscription(self, running, sdk):
        owned, marker, route, deadline = self._connect(running, sdk)
        source = copy.deepcopy(route['process'])
        descriptor = open_original_candidate_pidfd(source)
        self.pidfds.append(descriptor)
        if (pidfd_exited(descriptor) or original_json(marker, 16384) != route):
            raise ValueError('original command relay changed before disconnect')
        self._disconnect(owned)
        while not pidfd_exited(descriptor, min(1000, max(1, int(remaining(deadline) * 1000)))):
            remaining(deadline)
        # A successful client close alone is not a kernel exit observation.
        # Nor does this exit attest retirement of every server-side route cell.
        return dict(guest_pid=self.guest['pid'], original_route=route,
            original_relay_kernel_exit=True, subscription_close_attempts=1,
            guest_signal_sent=False, start_replayed=False,
            output_scope='quiet original command; Start frame only, no output replay claim')

    def observe(self, running, physical_running, *, cycle):
        if self.closed or type(cycle) is not int or cycle != len(self.rows):
            raise ValueError('one ordered observation per original execution required')
        if (running['session'] != self.client.session
                or physical_running['session'] != self.client.session):
            raise ValueError('original public session required')
        previous = copy.deepcopy(self.client.witness)
        self._guest_identity(previous)
        if self.rows:
            last = self.rows[-1]
            if (previous['metadata_generation'] != last['metadata_generation'] + 1
                    or previous['activation_nonce'] == last['_activation_nonce']
                    or previous['counter'] != last['counter'] + 1
                    or self.client.data_token == last['_token']):
                raise ValueError('original Resume RAM advance and new authorization required')
        sdk = self.client.sdk()
        token = self.client.data_token
        if self.rows and token == self.rows[-1]['_token']:
            raise ValueError('reconnect reused old execution authorization')
        self.client.assert_previous_tokens_closed()
        row = self._subscription(physical_running, sdk)
        # Independent finite public commands demonstrate that closing the
        # subscription did not kill or restart the original RAM process.
        observed = self.client.witness_command(sdk, 'read')
        if observed != previous:
            raise ValueError('disconnect changed the original RAM process state')
        current = self.client.witness_command(sdk, 'advance')
        self._guest_identity(current)
        if (current['counter'] != previous['counter'] + 1
                or any(current[key] != previous[key] for key in previous if key != 'counter')):
            raise ValueError('original guest process did not make exactly one RAM advance')
        if sdk.files.read('/tmp/sandproof-unload-marker') != 'disk-counter:' + str(current['counter']):
            raise ValueError('original process disk marker differs after subscription close')
        self.client.witness = current
        if self.client.query() != running:
            raise ValueError('subscription close changed the public running execution')
        physical = self.runtime.confirm_running(self.client.session, physical_running, self.deadline)
        row.update(cycle=cycle, guest_start_time_ticks=current['start_time_ticks'],
            ram_nonce_sha256=hashlib.sha256(current['nonce'].encode()).hexdigest(),
            counter=current['counter'], metadata_generation=current['metadata_generation'],
            physical=physical, old_authorizations_rejected=True,
            _token=token, _activation_nonce=current['activation_nonce'])
        self.rows.append(row)
        # Never disclose bearer credentials or RAM nonce values in reports.
        return {key: value for key, value in row.items() if not key.startswith('_')}

    def close(self):
        if not self.closed:
            for owned in self.handles:
                try:
                    self._disconnect(owned)
                except BaseException:
                    pass  # All failures retained below; continue other closes.
            for descriptor in self.pidfds:
                try:
                    os.close(descriptor)
                except OSError as error:
                    self.failures.append(type(error).__name__)
            self.pidfds.clear()
            self.closed = True
        return dict(subscription_close_attempts=sum(row['attempted'] for row in self.handles),
            cleanup_unknown=bool(self.failures), failure_types=list(self.failures),
            guest_signal_sent=False, replacement_start_sent=False)


class InflightCommandDrain(InflightDataDrain):
    """Keep one SDK subscription open until original Unload revokes its relay."""
    def __init__(self, runtime, client, running, gate_directory):
        super().__init__(client, running, runtime.services.native_pools[0],
                         runtime.services.host_configs[0], gate_directory, runtime.lease.identity)
        self.command = CommandDisconnectProbe(runtime, client)
        self.subscription = None
        self.original_token = None
        self.report = dict(completed=False, relay_control=self._relay_control)

    def begin(self):
        sdk = self.client.sdk()
        self.original_token = self.client.data_token
        self.subscription, self.marker, self.route, _ = self.command._connect(self.running, sdk)
        if original_json(self.marker, 16384) != self.route:
            raise ValueError('original command relay changed before Unload')
        self._hold_original_route()
        self.report.update(guest_pid=self.command.guest['pid'], sdk_start_received=True,
                           client_disconnected_before_unload=False)
        return self.report.copy()

    def release(self):
        self._release_relay()
        self.command._disconnect(self.subscription)
        self.report['client_disconnected_after_relay_exit'] = True
        return self.report.copy()

    def assert_resumed(self, physical_running):
        if (self.report.get('client_disconnected_after_relay_exit') is not True
                or physical_running['session'] != self.client.session):
            raise ValueError('original command relay must have drained before reconnect')
        witness = copy.deepcopy(self.client.witness)
        self.command._guest_identity(witness)
        if (witness['metadata_generation'] != self.command.guest['metadata_generation'] + 1
                or witness['counter'] != self.command.guest['counter'] + 1
                or witness['activation_nonce'] == self.command.guest['activation_nonce']):
            raise ValueError('original command reconnect requires the resumed execution')
        sdk = self.client.sdk()
        if self.client.data_token == self.original_token:
            raise ValueError('original command reconnect requires new data authorization')
        self.client.assert_previous_tokens_closed()
        receipt = self.command._subscription(physical_running, sdk)
        if self.client.witness_command(sdk, 'read') != witness:
            raise ValueError('resumed command subscription changed the original RAM process')
        self.report.update(resumed_command_subscription=receipt, completed=True)
        return self.report.copy()

    def close(self):
        relay = super().close()
        subscription = self.command.close()
        relay.update(command_subscription=subscription,
                     cleanup_unknown=relay['cleanup_unknown'] or subscription['cleanup_unknown'])
        return relay
