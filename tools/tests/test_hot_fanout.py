import ctypes
import json
from pathlib import Path
import tempfile
import socket
import time
from concurrent.futures import ThreadPoolExecutor
import unittest

from tools.hot_fanout.control import GuestFanout
from tools.hot_fanout.linux import CLONE_INTO_CGROUP, CloneArgs
from tools.hot_fanout.service import _dispatch, _firecracker_exchange, _read_frame, _write_frame


class FakeGuest:
    def __init__(self):
        self.active = set()
        self.cleaned = []
        self.stop_after = None
        self.cleanup_failure = None
        self.starts = 0
        self.fail_presence = False

    def start(self, plan):
        self.starts += 1
        for ordinal, _ in enumerate(plan.branches):
            if ordinal == self.stop_after:
                raise RuntimeError("unknown partial fork")
            self.active.add((plan.batch_id, ordinal))

    def release(self, batch_id, ordinal):
        if ordinal == self.cleanup_failure:
            raise OSError("cleanup not yet confirmed")
        self.active.discard((batch_id, ordinal))
        self.cleaned.append((batch_id, ordinal))

    def present(self, batch_id, ordinal):
        if self.fail_presence:
            raise OSError("observation unavailable")
        return (batch_id, ordinal) in self.active


def plan(batch_id="batchA", ports=(31000, 31001), prefix="branch"):
    return {"batch_id": batch_id, "checkpoint_id": "outer",
            "branches": [{"branch_id": f"{prefix}{n}", "port": port,
                          "access_token": f"secret_by_test_{batch_id}_{n}"}
                         for n, port in enumerate(ports)]}


class HotFanoutTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.backend = FakeGuest()

    def own(self, max_branches=4):
        return GuestFanout(Path(self.tmp.name) / "journal", "outer", self.backend,
                           max_branches=max_branches, max_batches=4, ports=range(31000, 31005))

    def test_control_stream_frames_are_bounded_and_reject_duplicate_fields(self):
        sender, receiver = socket.socketpair()
        with sender, receiver:
            until = time.monotonic() + 1
            _write_frame(sender, {"operation": "observe", "batch_id": "batchA"})
            self.assertEqual(_read_frame(receiver, until),
                             {"operation": "observe", "batch_id": "batchA"})
            raw = b'{"operation":"observe","operation":"release"}'
            sender.sendall(len(raw).to_bytes(4, "big") + raw)
            with self.assertRaises(ValueError):
                _read_frame(receiver, until)
            sender.sendall(b"\xff\xff\xff\xff")
            with self.assertRaises(ValueError):
                _read_frame(receiver, until)

    def test_firecracker_host_handshake_precedes_the_batch_frame(self):
        client, server = socket.socketpair()
        with client, server:
            client.settimeout(1)
            server.settimeout(1)

            def fake_firecracker():
                command = bytearray()
                while not command.endswith(b"\n"):
                    byte = server.recv(1)
                    if not byte or len(command) >= 16:
                        raise ValueError("invalid Firecracker connect line")
                    command.extend(byte)
                self.assertEqual(command, b"CONNECT 770\n")
                server.sendall(b"OK 1073741824\n")
                self.assertEqual(_read_frame(server), {"operation": "observe", "batch_id": "batchA"})
                _write_frame(server, {"batch_id": "batchA", "status": "pending"})

            with ThreadPoolExecutor(max_workers=1) as executor:
                peer = executor.submit(fake_firecracker)
                response = _firecracker_exchange(
                    client, {"operation": "observe", "batch_id": "batchA"}, time.monotonic() + 1,
                )
                peer.result(timeout=2)
            self.assertEqual(response, {"batch_id": "batchA", "status": "pending"})

    def test_linux_clone_layout_matches_version_two_abi(self):
        self.assertEqual(ctypes.sizeof(CloneArgs), 88)
        self.assertEqual(CloneArgs.exit_signal.offset, 32)
        self.assertEqual(CloneArgs.cgroup.offset, 80)
        self.assertEqual(CLONE_INTO_CGROUP, 1 << 33)

    def test_same_plan_replays_once_and_redacts_credentials(self):
        request = plan()
        with self.own() as owner:
            result = _dispatch(owner, {"operation": "start", "plan": request})
            self.assertEqual(result, {"batch_id": "batchA", "status": "ready", "released": [],
                                      "branches": [{"branch_id": "branch0", "port": 31000},
                                                   {"branch_id": "branch1", "port": 31001}]})
            self.assertEqual(owner.start(request), result)
            self.assertEqual(self.backend.starts, 1)
            self.assertNotIn("secret_by_test", json.dumps(result))
            self.assertNotIn("secret_by_test", (owner.state / "batchA.json").read_text())
            request["branches"][1]["access_token"] = "another_test_access_token"
            with self.assertRaises(ValueError):
                owner.start(request)

    def test_partial_failure_only_cleans_exact_batch_and_returns_no_endpoints(self):
        with self.own() as owner:
            self.assertEqual(owner.start(plan("peer", (31003,), "peer"))["status"], "ready")
            self.backend.stop_after = 1
            result = owner.start(plan())
            self.assertEqual(result, {"batch_id": "batchA", "status": "failed_clean"})
            self.assertEqual(self.backend.active, {("peer", 0)})
            self.assertEqual(self.backend.cleaned, [("batchA", 0), ("batchA", 1)])
            self.assertEqual(owner.start(plan()), result)

    def test_inconclusive_cleanup_blocks_reuse_and_restart_resumes_cleanup(self):
        self.backend.stop_after = 1
        self.backend.cleanup_failure = 0
        with self.own() as owner:
            self.assertEqual(owner.start(plan()), {"batch_id": "batchA", "status": "pending"})
            self.assertEqual(self.backend.active, {("batchA", 0)})
            with self.assertRaises(ValueError):
                owner.start(plan("other", prefix="new"))
        with self.own() as owner:
            self.backend.cleanup_failure = None
            self.assertEqual(owner.observe("batchA")["status"], "failed_clean")
            self.assertEqual(self.backend.active, set())
            self.backend.stop_after = None
            self.assertEqual(owner.start(plan("other", prefix="new"))["status"], "ready")

    def test_crash_after_effect_before_ready_publication_never_redispatches(self):
        with self.own() as owner:
            start = self.backend.start

            def crash(request):
                start(request)
                raise KeyboardInterrupt()

            self.backend.start = crash
            with self.assertRaises(KeyboardInterrupt):
                owner.start(plan())
        self.backend.start = start
        with self.own() as owner:
            self.assertEqual(owner.start(plan()), {"batch_id": "batchA", "status": "failed_clean"})
            self.assertEqual(self.backend.starts, 1)
            self.assertEqual(self.backend.active, set())

    def test_individual_release_only_fences_its_ordinal_and_is_idempotent(self):
        with self.own() as owner:
            owner.start(plan())
            with self.assertRaises(ValueError):
                owner.release("batchA", "foreign")
            result = owner.release("batchA", "branch0")
            self.assertEqual(result["released"], ["branch0"])
            self.assertEqual(result["branches"], [{"branch_id": "branch1", "port": 31001}])
            self.assertEqual(owner.release("batchA", "branch0"), result)
            self.assertEqual(self.backend.cleaned, [("batchA", 0)])
            self.assertEqual(owner.start(plan("new", (31000,), "next"))["status"], "ready")
            self.assertEqual(owner.release("batchA")["status"], "released")
            self.assertEqual(self.backend.active, {("new", 0)})

    def test_lost_and_unknown_ready_checks_expose_no_endpoint(self):
        with self.own() as owner:
            owner.start(plan())
            self.backend.fail_presence = True
            self.assertEqual(owner.observe("batchA"), {"batch_id": "batchA", "status": "pending"})
            self.backend.fail_presence = False
            self.backend.active.discard(("batchA", 1))
            self.assertEqual(owner.observe("batchA"), {"batch_id": "batchA", "status": "lost"})

    def test_invalid_plan_limits_and_single_owner(self):
        bad_count = plan(ports=())
        wrong_checkpoint = dict(plan(), checkpoint_id="old")
        request_with_alias = plan(ports=(31000, 31000))
        bool_port = plan(ports=(True,))
        with self.own(max_branches=2) as owner:
            for request in (bad_count, wrong_checkpoint, request_with_alias, bool_port):
                with self.subTest(request=request), self.assertRaises(ValueError):
                    owner.start(request)
            with self.assertRaises(BlockingIOError):
                self.own()
            self.assertEqual(owner.start(plan())["status"], "ready")
            with self.assertRaises(ValueError):
                owner.start(plan("overflow", (31004,), "also"))
            self.assertEqual(self.backend.starts, 1)
