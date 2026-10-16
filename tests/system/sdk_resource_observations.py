"""Optional, finite resource observations alongside the paired SDK calls."""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import copy
import hashlib
import time

from sdk_command_input import call
from sdk_subscriber_compare import MODES, RESOURCE_PHASES


def phases(inputs):
    assert inputs['users'] == ['root', 'ubuntu']
    return ['before-cases', *[
        f'subscribers/{user}/{mode}/{phase}'
        for user in inputs['users'] for mode in MODES for phase in RESOURCE_PHASES
    ], 'after-cases']


class ResourceSamples:
    """One read-only command per declared phase; failures cannot be retried."""
    def __init__(self, sandbox, expected, daemon, inputs):
        from tools import guest_probe
        self.probe, self.sandbox = guest_probe, sandbox
        self.expected, self.daemon = copy.deepcopy(expected), copy.deepcopy(daemon)
        self.command = guest_probe.resource_command(self.expected, self.daemon)
        self.phases = phases(inputs)
        self.busy = self.failed = False
        self.report = dict(budget_verdict='not_evaluated',
            command_sha256=hashlib.sha256(self.command.encode()).hexdigest(), observations=[])

    async def take(self, phase):
        rows = self.report['observations']
        assert not self.busy and not self.failed, 'resource observation already pending or failed'
        assert len(rows) < len(self.phases) and phase == self.phases[len(rows)], 'resource phase order'
        self.busy = True
        row = dict(phase=phase, status='incomplete')
        rows.append(row)
        started = time.monotonic_ns()
        stdout, size = [], 0
        def on_stdout(part):
            nonlocal size
            assert type(part) is str
            size += len(part.encode())
            assert size <= self.probe.REPORT_LIMIT, 'resource reply byte budget'
            if part:
                stdout.append(part)
        def on_stderr(part):
            assert type(part) is str and not part, 'resource observer emitted stderr'
        try:
            result = await call(self.sandbox.commands.run, self.command,
                user='root', timeout=20, request_timeout=20,
                on_stdout=on_stdout, on_stderr=on_stderr)
            assert result.stdout == ''.join(stdout), 'resource callback/result mismatch'
            row['receipt'] = self.probe.resource_receipt(result, self.expected, self.daemon)
            row['status'] = 'complete'
        except BaseException as error:
            self.failed = True
            row['error'] = type(error).__name__
            raise
        finally:
            row['request_elapsed_ns'] = time.monotonic_ns() - started
            self.busy = False

    def complete(self):
        assert not self.busy and not self.failed
        rows = self.report['observations']
        assert [row['phase'] for row in rows] == self.phases, 'missing resource phase'
        assert all(row['status'] == 'complete' for row in rows)
