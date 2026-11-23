from pathlib import Path
import runpy
import unittest


MODULE = runpy.run_path(str(Path(__file__).resolve().parents[2] / "tests/system/_hot_fanout_evidence.py"))
capture_live_native_ids = MODULE["capture_live_native_ids"]
automatic_expiry_acceptance = MODULE["automatic_expiry_acceptance"]


class HotFanoutLiveEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.native = dict(namespace="example", node_state_dir="/protected/native",
                           bounded_host={"maintenance": {"max_host_slots": 2}})
        self.claims = [{"resource_id": f"vm-example-{vm}"} for vm in (87, 45)]
        self.rows = {
            Path(f"/protected/native/host-slots/{position}.json"): {
                "position": position,
                "value": {"phase": "Occupied", "context": {
                    "position": position, "binding": {"sandbox": sandbox, "vm": vm}}}}
            for position, (sandbox, vm) in enumerate(((4, 45), (23, 87)))
        }

    def read_slot(self, path, limit):
        self.assertEqual(limit, 65536)
        return self.rows[path]

    def test_identity_survives_window_retirement_without_another_binding_read(self):
        captured = capture_live_native_ids(self.native, self.claims, self.read_slot)
        for slot in self.rows.values():
            slot["value"]["phase"] = "Free"
        self.assertEqual(captured, ("sb-example-4", "sb-example-23"))
        with self.assertRaisesRegex(ValueError, "occupied slot"):
            capture_live_native_ids(self.native, self.claims, self.read_slot)

    def test_first_allocated_vm_id_zero_is_a_valid_original_claim(self):
        slot = self.rows[Path("/protected/native/host-slots/0.json")]
        slot["value"]["context"]["binding"]["vm"] = 0
        claims = [self.claims[0], {"resource_id": "vm-example-0"}]
        self.assertEqual(capture_live_native_ids(self.native, claims, self.read_slot),
                         ("sb-example-4", "sb-example-23"))
        for invalid in (-1, False, 2**64):
            with self.subTest(vm=invalid), self.assertRaisesRegex(ValueError, "original native sandbox"):
                slot["value"]["context"]["binding"]["vm"] = invalid
                capture_live_native_ids(self.native, claims, self.read_slot)

    def test_unknown_or_foreign_original_claim_does_not_complete_cohort(self):
        for claims in ([], self.claims[:1], [self.claims[0], self.claims[0]]):
            with self.subTest(claims=claims), self.assertRaisesRegex(ValueError, "distinct original VM"):
                capture_live_native_ids(self.native, claims, self.read_slot)
        wrong_pool = [self.claims[0], {"resource_id": "vm-other-45"}]
        with self.assertRaisesRegex(ValueError, "original VM claim"):
            capture_live_native_ids(self.native, wrong_pool, self.read_slot)


class HotFanoutAutomaticExpiryTests(unittest.TestCase):
    def setUp(self):
        self.original = dict(session="parent", accepted_version=1,
                             key=dict(deployment="deployment", owner="1", window=1, sequence=1))
        self.key = dict(self.original["key"], window=2, sequence=9)
        self.view = dict(session="parent", current_version=2, phase="terminating", operation=self.key)

    def test_original_automatic_destroy_can_be_seen_in_either_terminal_phase(self):
        expected = dict(session="parent", key=self.key, accepted_version=2)
        for phase in ("terminating", "cleaned"):
            actual = automatic_expiry_acceptance(dict(self.view, phase=phase), self.original)
            self.assertEqual(expected, actual)
            self.assertIsNot(actual["key"], self.key)

    def test_public_request_or_unrelated_identity_cannot_certify_expiry(self):
        wrong = (
            dict(self.view, phase="running"),
            dict(self.view, session="other"),
            dict(self.view, current_version=3),
            dict(self.view, operation=self.original["key"]),
            dict(self.view, operation=dict(self.key, owner="2")),
            dict(self.view, operation=dict(self.key, deployment="other")),
            dict(self.view, operation=dict(self.key, sequence=True)),
        )
        for view in wrong:
            with self.subTest(view=view), self.assertRaisesRegex(ValueError, "automatic Destroy"):
                automatic_expiry_acceptance(view, self.original)
