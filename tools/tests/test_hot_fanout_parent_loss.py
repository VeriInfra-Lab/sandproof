"""Controlled checks of the parent-loss acceptance driver; no process effects."""

from contextlib import ExitStack, redirect_stderr
from io import StringIO
from pathlib import Path
import sys
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests/system"))
import hot_fanout_product as product
import lifecycle_unload_runtime as runtime


class ParentLossFixtureTests(unittest.TestCase):
    def setUp(self):
        self.client = SimpleNamespace(session="parent", destroy_request=None, deadline=time.monotonic() + 100,
                                      query=lambda: {"phase": "running"})
        self.client.remaining = lambda: self.client.deadline - time.monotonic()
        self.running = {"phase": "running"}
        self.info = SimpleNamespace(sandbox_id="parent", state="running",
                                    end_at=SimpleNamespace(timestamp=lambda: self.end_at))
        self.end_at = time.time() + 300
        self.parent = SimpleNamespace(get_info=lambda: self.info)
        self.claim = dict(resource_id="vm-fixture-6", pid=123, start_time_ticks=456, boot_id="fixture")
        self.slot = ("slot-path", {"position": 0}, self.claim)
        self.report = {}
        self.calls = []

    def drive(self):
        return product.verify_public_branch_rejection_after_parent_exit(
            self.client, self.running, self.parent, {}, "warm0", 2, "fixture",
            {"namespace": "fixture"}, self.slot, self.report,
        )

    def fixture(self, stack):
        ids = ["branch-a", "branch-b"]
        sandboxes = [SimpleNamespace(sandbox_id=branch) for branch in ids]
        stack.enter_context(patch.object(product, "start_batch", return_value=("batch-path", "batch-key", ids)))
        stack.enter_context(patch.object(product, "concurrent_sdk", return_value=sandboxes))
        stack.enter_context(patch.object(product, "slot_and_claim", side_effect=[self.slot, self.slot]))
        stack.enter_context(patch.object(runtime, "open_original_candidate_pidfd", return_value=91))
        stack.enter_context(patch.object(runtime, "exit_original_candidate",
                                         side_effect=lambda *args: self.calls.append(("exit", args[:2]))))
        stack.enter_context(patch.object(runtime, "pidfd_exited", return_value=True))
        stack.enter_context(patch.object(product, "os", SimpleNamespace(
            close=lambda descriptor: self.calls.append(("close", descriptor)))))
        stack.enter_context(patch.object(product, "get_batch", return_value={"state": "cleaning_up"}))
        stack.enter_context(patch.object(product, "check_sdk_not_found",
                                         side_effect=lambda branch, _owner: self.calls.append(("connect", branch))))
        stack.enter_context(patch.object(product, "check_stale_data_route",
                                         side_effect=lambda _client, sdk: self.calls.append(("data", sdk.sandbox_id))))

    def test_denial_checks_follow_certified_exit_without_sending_destroy(self):
        with ExitStack() as stack:
            self.fixture(stack)
            self.assertEqual(self.drive(), ("batch-path", "batch-key"))
        self.assertEqual(self.calls, [("exit", (91, self.claim)), ("connect", "branch-a"),
                                     ("data", "branch-a"), ("connect", "branch-b"),
                                     ("data", "branch-b"), ("close", 91)])
        evidence = self.report["parent_vmm_loss"]
        self.assertIsNone(self.client.destroy_request)
        self.assertLess(evidence["denied_before_destroy_at_ms"], evidence["end_at_ms"])

    def test_replaced_slot_after_pidfd_open_never_dispatches_exit(self):
        with ExitStack() as stack:
            self.fixture(stack)
            product.slot_and_claim.side_effect = [self.slot, ("foreign", self.slot[1], self.claim)]
            with self.assertRaises(AssertionError):
                self.drive()
        self.assertEqual(self.calls, [("close", 91)])

    def test_short_lease_does_not_open_a_fault_handle(self):
        self.end_at = time.time() + 20
        with ExitStack() as stack:
            self.fixture(stack)
            with self.assertRaisesRegex(AssertionError, "original live lease"):
                self.drive()
            runtime.open_original_candidate_pidfd.assert_not_called()
        self.assertEqual(self.calls, [])


    def test_missing_interpreter_api_stops_before_sdk_or_resources(self):
        stderr = StringIO()
        with (
            patch.object(product, "checked_args", return_value=SimpleNamespace(fault_case="parent-vmm-loss")),
            patch.object(product, "os", SimpleNamespace()),
            patch.object(product.importlib.metadata, "distribution") as sdk,
            redirect_stderr(stderr),
        ):
            with self.assertRaises(SystemExit) as failure:
                product.main()
            self.assertEqual(failure.exception.code, 2)
            self.assertIn("os.pidfd_open and signal.pidfd_send_signal", stderr.getvalue())
            sdk.assert_not_called()
