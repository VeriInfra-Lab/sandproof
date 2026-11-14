"""Original Unload/Destroy contract at the dispatched/pre-Pause crash cut.

Keep Freeze unknown. Running now and exit87 do not constitute NoRequest. Only
the original terminal Stop/kernel exit and product cleanup can release backing.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

from lifecycle_freeze_destroy import FreezeDestroyProbe, paused_source


class FreezeDispatchDestroyProbe(FreezeDestroyProbe):
    def _fault_environment(self):
        return {'SANDPROOF_TEST_EXIT_BEFORE_HOST_ACTION':'freeze_vm'}

    def _cut_exit_code(self):
        return 87

    def _cut_stage_label(self):
        return 'freeze-before-effect'

    def _paused_cut(self):
        return False

    def _observe_cut_source(self,deadline):
        return paused_source(self.runtime,self.prior,deadline,expected_state='Running')

    def _require_cut_source(self,deadline):
        if self._source_state(deadline) in ('T','t'):
            raise ValueError('unexpected externally stopped pre-Pause source')
        self._observe_cut_source(deadline)

    def _hold_source(self,deadline):
        # No SIGSTOP/SIGCONT, pause marker, or fabricated Frozen receipt.
        self._require_cut_source(deadline)

    def _release_source_hold(self,deadline):
        self._require_cut_source(deadline)
        return self._observe_cut_source(deadline)

    def _inventory(self,snapshot):
        rows=super()._inventory(snapshot)
        freeze=[row for row in snapshot['original_actions']
                if 'FreezeVm' in row[0]['command']['logical']['command']]
        if self._success_observation(freeze[0],'Frozen') is not None:
            raise ValueError('pre-Pause recovery must not execute a new Freeze')
        return rows
