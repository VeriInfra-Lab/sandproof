import ctypes
import json
import os
from pathlib import Path
import tempfile
import socket
import time
from concurrent.futures import ThreadPoolExecutor
import unittest
from unittest.mock import patch

from tools.hot_fanout.control import GuestFanout
from tools.hot_fanout.linux import CLONE_INTO_CGROUP, CloneArgs, _owns_tcp_listener
from tools.hot_fanout.prepared import validate_frozen_mounts
from tools.hot_fanout.service import _dispatch, _firecracker_exchange, _read_frame, _write_frame


class FakeGuest:
    def __init__(self):
        self.active = set()
        self.cleaned = []
        self.stop_after = None
        self.cleanup_failure = None
        self.starts = 0
        self.fail_presence = False
        self.fail_scope = False
        self.has_checkpoint = True

    def checkpoint_available(self):
        return self.has_checkpoint

    def start(self, plan):
        self.starts += 1
        for ordinal, _ in enumerate(plan.branches):
            if ordinal == self.stop_after:
                raise RuntimeError("unknown partial fork")
            self.active.add((plan.batch_id, ordinal))

    def has_batch(self, batch_id):
        if self.fail_scope:
            raise OSError("presence unavailable")
        return any(existing == batch_id for existing, _ in self.active)

    def release(self, batch_id, ordinal):
        if ordinal == self.cleanup_failure:
            raise OSError("cleanup not yet confirmed")
        self.active.discard((batch_id, ordinal))
        self.cleaned.append((batch_id, ordinal))

    def present(self, batch_id, ordinal, port):
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

    def test_prepared_root_requires_readonly_superblock_without_writable_alias(self):
        sealed = "100 90 7:2 / /run/frozen ro - squashfs /dev/loop2 ro"
        validate_frozen_mounts([Path("/run/frozen")], sealed)
        for broken in [
            sealed.replace("/dev/loop2 ro", "/dev/loop2 rw"),
            sealed + "\n101 90 7:2 / /run/alias rw - squashfs /dev/loop2 ro",
            sealed.replace("7:2 / /run/frozen", "7:2 /child /run/frozen"),
            sealed.replace("/run/frozen", "/run/wrong"),
        ]:
            with self.assertRaises(ValueError):
                validate_frozen_mounts([Path("/run/frozen")], broken)

    def test_probe_requires_original_live_checkpoint(self):
        with self.own() as owner:
            self.assertEqual(_dispatch(owner, {"operation": "probe"}),
                             {"status": "checkpoint_ready", "checkpoint_id": "outer",
                              "max_branches": 4})
            self.backend.has_checkpoint = False
            self.assertEqual(_dispatch(owner, {"operation": "probe"}), {"status": "unavailable"})
            with self.assertRaises(ValueError):
                _dispatch(owner, {"operation": "probe", "checkpoint_id": "outer"})

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

    def test_full_release_before_start_fences_delayed_dispatch_across_reopen(self):
        with self.own() as owner:
            self.assertEqual(owner.start(plan("peer", (31003,), "peer"))["status"], "ready")
            self.assertEqual(owner.release("batchA", "branch0")["status"], "lost")
            self.assertEqual(owner.observe("batchA")["status"], "lost")
            self.assertEqual(owner.release("batchA")["status"], "released")
        with self.own() as owner:
            self.assertEqual(owner.observe("batchA")["status"], "released")
            self.assertEqual(owner.release("batchA")["status"], "released")
            with self.assertRaises(ValueError):
                owner.start(plan())
            self.assertEqual(self.backend.starts, 1)
            self.assertEqual(self.backend.active, {("peer", 0)})

    def test_full_monotonic_history_proves_exact_absence_after_reopen(self):
        ready = plan("peer", (31003,), "peer")
        with self.own() as owner:
            owner.start(ready)
            self.assertEqual(owner.observe("batchA")["status"], "lost")
            for index in range(3):
                self.assertEqual(owner.release(f"closed{index}")["status"], "released")
            self.assertEqual(owner.start(ready)["status"], "ready")
        with self.own() as owner:
            result = {"batch_id": "batchA", "status": "never_started"}
            self.assertEqual(_dispatch(owner, {"operation": "start", "plan": plan()}), result)
            self.assertEqual(_dispatch(owner, {"operation": "observe", "batch_id": "batchA"}), result)
            self.assertEqual(owner.release("batchA"), {"batch_id": "batchA", "status": "released"})
            self.assertEqual(owner.release("batchA", "branch0")["status"], "lost")
            self.assertFalse((owner.state / "batchA.json").exists())
            self.assertEqual(self.backend.active, {("peer", 0)})
            self.assertEqual(self.backend.starts, 1)

    def test_full_history_with_residue_or_unknown_inspection_never_proves_absence(self):
        self.backend.active.add(("batchA", 0))
        with self.own() as owner:
            self.assertEqual(owner.release("batchA")["status"], "pending")
            for index in range(4):
                self.assertEqual(owner.release(f"closed{index}")["status"], "released")
            for fail_scope in (False, True):
                self.backend.fail_scope = fail_scope
                self.assertEqual(owner.start(plan())["status"], "pending")
                self.assertEqual(owner.observe("batchA")["status"], "lost")
                self.assertEqual(owner.release("batchA")["status"], "pending")
            self.assertEqual(self.backend.active, {("batchA", 0)})
            self.assertEqual(self.backend.starts, 0)

    def test_definite_lost_member_reclaims_only_its_original_batch(self):
        with self.own() as owner:
            owner.start(plan())
            owner.start(plan("peer", (31003,), "peer"))
            self.backend.fail_presence = True
            self.assertEqual(owner.observe("batchA"), {"batch_id": "batchA", "status": "pending"})
            self.assertFalse(self.backend.cleaned)
            self.backend.fail_presence = False
            self.backend.active.discard(("batchA", 1))
            result = {"batch_id": "batchA", "status": "failed_clean"}
            self.assertEqual(owner.observe("batchA"), result)
            self.assertEqual(owner.start(plan()), result)
            self.assertEqual(self.backend.active, {("peer", 0)})
            self.assertEqual(self.backend.cleaned, [("batchA", 0), ("batchA", 1)])

    def test_liveness_cleanup_keeps_failure_reason_across_restart_and_release_replay(self):
        with self.own() as owner:
            owner.start(plan())
            owner.release("batchA", "branch0")
            self.backend.active.discard(("batchA", 1))
            self.backend.cleanup_failure = 1
            self.assertEqual(owner.observe("batchA"), {"batch_id": "batchA", "status": "pending"})
            self.assertEqual(self.backend.cleaned, [("batchA", 0)])
            with self.assertRaises(ValueError):
                owner.start(plan("collision", (31001,), "other"))
        with self.own() as owner:
            self.backend.cleanup_failure = None
            result = {"batch_id": "batchA", "status": "failed_clean"}
            self.assertEqual(owner.start(plan()), result)
            self.assertEqual(owner.release("batchA"), result)
            self.assertEqual(owner.observe("batchA"), result)
            self.assertEqual(self.backend.cleaned, [("batchA", 0), ("batchA", 1)])
            self.assertEqual(owner.start(plan("replacement", (31001,), "other"))["status"], "ready")

    def test_failure_retirement_waits_until_exact_scope_is_absent(self):
        with self.own() as owner:
            owner.start(plan())
            self.backend.active.discard(("batchA", 1))
            self.backend.active.add(("batchA", 2))
            self.assertEqual(owner.observe("batchA")["status"], "pending")
            self.assertEqual(self.backend.active, {("batchA", 2)})
            self.backend.active.discard(("batchA", 2))
            self.assertEqual(owner.observe("batchA")["status"], "failed_clean")

    def test_unattributed_listener_is_unknown_in_established_batch(self):
        root = Path(self.tmp.name)
        (root / "cgroup.procs").write_text("201\n")
        proc = root / "proc"
        (proc / "201/fd").mkdir(parents=True)
        (proc / "net").mkdir()
        tcp = proc / "net/tcp"
        with patch("tools.hot_fanout.linux.Path",
                   side_effect=lambda path: proc / Path(path).relative_to("/proc")):
            tcp.write_text("header\n")
            self.assertFalse(_owns_tcp_listener(root, 31000, observe=True))
            tcp.write_text("header\n0: 00000000:7918 x 0A x x x x x 42\n")
            self.assertFalse(_owns_tcp_listener(root, 31000))
            with self.assertRaises(OSError):
                _owns_tcp_listener(root, 31000, observe=True)
            os.symlink("socket:[42]", proc / "201/fd/9")
            self.assertTrue(_owns_tcp_listener(root, 31000, observe=True))

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
