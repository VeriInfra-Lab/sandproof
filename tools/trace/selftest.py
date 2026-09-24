#!/usr/bin/env python3
"""Self-tests for the trace checker rules. Run: python3 -m tools.trace.selftest"""
import unittest

from tools.trace.checker import Checker
from tools.trace.trace_model import Record


def evt(seq, name, result="success"):
    return Record(seq=seq, ts=seq, source="client", kind="event",
                  sandbox="s1", event=name, attrs={"result": result})


def obs(seq, **kwargs):
    defaults = {"running": False, "snapshot_present": False,
                "snapshot_deleted": False, "phase_hint": "absent"}
    defaults.update(kwargs)
    return Record(seq=seq, ts=seq, source="observer", kind="observe",
                  sandbox="s1", obs=defaults)


class CheckerTests(unittest.TestCase):
    def verdict(self, records):
        return Checker().run(records).status

    def test_normal_lifecycle_conforms(self):
        records = [
            evt(1, "acceptCreate"),
            obs(2, running=True, phase_hint="running"),
            evt(3, "acceptPause"),
            obs(4, snapshot_present=True, phase_hint="paused"),
            evt(5, "acceptResume"),
            obs(6, running=True, snapshot_present=True, phase_hint="running"),
            evt(7, "acceptKill"),
            obs(8, snapshot_present=True, snapshot_deleted=True, phase_hint="deleted"),
        ]
        self.assertEqual(self.verdict(records), "Conforms")

    def test_f1_race_violates(self):
        records = [
            evt(1, "acceptCreate"),
            obs(2, running=True, phase_hint="running"),
            evt(3, "acceptPause"),
            obs(4, snapshot_present=True, phase_hint="paused"),
            evt(5, "acceptKill"),
            obs(6, snapshot_present=True, snapshot_deleted=True, phase_hint="deleted"),
            evt(7, "acceptResume"),
            obs(8, running=True, snapshot_present=True, snapshot_deleted=True,
                phase_hint="running"),
        ]
        self.assertEqual(self.verdict(records), "Violates")

    def test_kill_unknown_is_inconclusive(self):
        records = [
            evt(1, "acceptCreate"),
            obs(2, running=True, phase_hint="running"),
            evt(3, "acceptKill", result="unknown"),
            obs(4, running=True, phase_hint="running"),
        ]
        self.assertEqual(self.verdict(records), "Inconclusive")

    def test_running_without_start_violates(self):
        records = [obs(1, running=True, phase_hint="running")]
        self.assertEqual(self.verdict(records), "Violates")

    def test_resume_without_snapshot_violates(self):
        records = [
            evt(1, "acceptCreate"),
            obs(2, running=True, phase_hint="running"),
            evt(3, "acceptPause"),
            obs(4, phase_hint="paused"),
            evt(5, "acceptResume"),
        ]
        self.assertEqual(self.verdict(records), "Violates")

    def test_paused_with_deleted_snapshot_violates(self):
        records = [
            evt(1, "acceptCreate"),
            evt(2, "acceptPause"),
            obs(3, snapshot_present=True, snapshot_deleted=True, phase_hint="paused"),
        ]
        self.assertEqual(self.verdict(records), "Violates")

    def test_workspace_identity_change_violates(self):
        records = [obs(1, workspace="fs1"), obs(2, workspace="fs2")]
        self.assertEqual(self.verdict(records), "Violates")

    def test_generation_decrease_violates(self):
        records = [obs(1, generation=2), obs(2, generation=1)]
        self.assertEqual(self.verdict(records), "Violates")

    def test_epoch_increase_conforms(self):
        records = [obs(1, epoch=1), obs(2, epoch=2)]
        self.assertEqual(self.verdict(records), "Conforms")

    def test_two_executing_vms_violate(self):
        records = [obs(1, executing_vms=["vm1", "vm2"])]
        self.assertEqual(self.verdict(records), "Violates")

    def test_reclaimed_current_vm_violates(self):
        records = [obs(1, current_vm="vm1", reclaimed_vms=["vm1"])]
        self.assertEqual(self.verdict(records), "Violates")

    def test_operation_final_result_rewrite_violates(self):
        records = [
            obs(1, operations=[{"id": "op1", "kind": "kill", "epoch": 1, "result": "destroyed"}]),
            obs(2, operations=[{"id": "op1", "kind": "kill", "epoch": 1, "result": "running"}]),
        ]
        self.assertEqual(self.verdict(records), "Violates")

    def test_effect_status_regression_violates(self):
        records = [
            obs(1, effects=[{"id": "e1", "status": "acknowledged", "result": "ok"}]),
            obs(2, effects=[{"id": "e1", "status": "committed", "result": None}]),
        ]
        self.assertEqual(self.verdict(records), "Violates")

    def test_running_with_stale_applied_policy_violates(self):
        records = [obs(1, running=True, applied_policy="p1", file_policy="p2")]
        self.assertEqual(self.verdict(records), "Violates")

    def test_rules_skip_when_data_absent(self):
        # No rule may fire on records that carry none of their observations.
        records = [obs(1, phase_hint="running")]
        self.assertEqual(self.verdict(records), "Conforms")

    def test_paused_with_executing_vm_violates(self):
        records = [obs(1, phase_hint="paused", executing_vms=["vm1"])]
        self.assertEqual(self.verdict(records), "Violates")

    def test_active_slot_not_current_violates(self):
        records = [obs(1, active_slots=["slot1"], current_slot="slot2")]
        self.assertEqual(self.verdict(records), "Violates")

    def test_durable_but_mutable_snapshot_violates(self):
        records = [obs(1, snapshot_stage="durable", snapshot_immutable=False)]
        self.assertEqual(self.verdict(records), "Violates")

    def test_fence_beyond_epoch_violates(self):
        records = [obs(1, fence_epoch=5, epoch=3)]
        self.assertEqual(self.verdict(records), "Violates")

    def test_egress_outside_allowlist_violates(self):
        records = [obs(1, egress_destination="evil.example", egress_allowlist=["good.example"])]
        self.assertEqual(self.verdict(records), "Violates")

    def test_file_event_wrong_workspace_violates(self):
        records = [obs(1, file_event_workspace="fsA", workspace="fsB")]
        self.assertEqual(self.verdict(records), "Violates")

    def test_stale_index_violates(self):
        records = [obs(1, index_members=["s1", "s2"], existing_sandboxes=["s1"])]
        self.assertEqual(self.verdict(records), "Violates")

    def test_execution_reuse_violates(self):
        records = [obs(1, execution_id="e1"), obs(2, execution_id="e2"),
                   obs(3, execution_id="e1")]
        self.assertEqual(self.verdict(records), "Violates")

    def test_execution_change_conforms(self):
        records = [obs(1, execution_id="e1"), obs(2, execution_id="e2")]
        self.assertEqual(self.verdict(records), "Conforms")

    def test_transient_phase_inconclusive(self):
        records = [obs(1, phase_hint="pausing", snapshot_present=True)]
        self.assertEqual(self.verdict(records), "Inconclusive")

    def test_stable_phase_conforms(self):
        records = [obs(1, phase_hint="paused", snapshot_present=True)]
        self.assertEqual(self.verdict(records), "Conforms")

    def test_pause_after_kill_violates(self):
        records = [evt(1, "acceptCreate"), evt(2, "acceptKill"), evt(3, "acceptPause")]
        self.assertEqual(self.verdict(records), "Violates")

    def test_node_orphan_violates(self):
        records = [obs(1, node_sandboxes=["x"], existing_sandboxes=[])]
        self.assertEqual(self.verdict(records), "Violates")

    def test_node_store_consistent_conforms(self):
        records = [obs(1, node_sandboxes=["x"], existing_sandboxes=["x"])]
        self.assertEqual(self.verdict(records), "Conforms")


if __name__ == "__main__":
    unittest.main()
