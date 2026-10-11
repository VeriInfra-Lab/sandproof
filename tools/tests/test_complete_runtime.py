"""Unprivileged complete-fixture checks; these tests never start a VM."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / "tests/system/_complete_runtime.py"
SPEC = importlib.util.spec_from_file_location("complete_runtime_fixture", SOURCE)
fixture = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fixture)


class CompleteFixtureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir="/tmp")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for name in ("db", "state", "jails"):
            (self.root / name).mkdir(mode=0o700)
        for name, size in (("kernel", 50), ("firecracker", 100), ("rootfs", 200)):
            (self.root / name).write_bytes(bytes(size))
        self.settings = dict(namespace="a" * 16, database=str(self.root / "db/controller.db"),
                             node_state_dir=str(self.root / "state"), chroot_base=str(self.root / "jails"),
                             kernel=str(self.root / "kernel"), rootfs=str(self.root / "rootfs"),
                             firecracker=str(self.root / "firecracker"), uid_base=10000,
                             max_active_global=2, max_active_per_tenant=1,
                             max_disk_bytes_global=700, cleanup_reserve_bytes=300,
                             max_disk_bytes_per_tenant=200)
        self.free = SimpleNamespace(f_bavail=2**40, f_frsize=4096, f_favail=2**30)

    def complete(self):
        return fixture.complete_runtime(self.settings, slots=2)

    def test_explicit_single_runtime_policy_is_pure_and_has_complete_cleanup_budget(self):
        original = copy.deepcopy(self.settings)
        files = sorted(self.root.rglob("*"))
        result = self.complete()
        self.assertEqual(self.settings, original)
        self.assertEqual(sorted(self.root.rglob("*")), files)
        self.assertNotIn("max_vm_id", result)
        host = result["bounded_host"]
        pool = host["physical_storage"]["pools"][0]
        cap = result["maintenance"]["capacity"]
        self.assertEqual(pool["max_host_slots"], 2)
        self.assertEqual(host["maintenance"]["windows"], dict(max_open_windows=2, max_duration_ms=1000))
        self.assertEqual(pool["rootfs_budget_bytes"], 400)
        self.assertEqual(result["max_disk_bytes_per_tenant"], 200)
        self.assertEqual(pool["runtime_copy_bytes"], 100)
        self.assertEqual(pool["kernel_copy_bytes"], 50)
        self.assertEqual(result["cleanup_reserve_bytes"], pool["host_metadata_bytes"] + cap["max_online_storage_bytes"])
        self.assertEqual(cap["max_online_storage_bytes"], sum(cap[key] for key in (
            "max_database_bytes", "max_wal_bytes", "max_shm_bytes", "checkpoint_scratch_bytes")))
        self.assertEqual(cap["max_sql_value_bytes"],cap["max_sql_bytes"])
        self.assertEqual(cap["max_sql_bytes"],16*fixture.MIB)
        self.assertEqual(cap["max_online_storage_bytes"],fixture.ONLINE_BYTES)
        self.assertEqual(result["max_disk_bytes_global"], pool["rootfs_budget_bytes"] + result["cleanup_reserve_bytes"])
        self.assertEqual(pool["node_state_dir"], result["node_state_dir"])
        self.assertEqual(host["console_policy"], "discard")
        self.assertGreater(fixture._envelope(host["physical_storage"])[0], result["max_disk_bytes_global"])

    def test_constructor_refuses_old_or_existing_configuration_and_missing_inputs(self):
        for key, value in (("max_vm_id", 2), ("bounded_host", None), ("maintenance", None),
                           ("internal_build_vms", True), ("template_registry", {"enabled": True})):
            with self.subTest(key=key), self.assertRaises(ValueError):
                fixture.complete_runtime(self.settings | {key: value}, slots=2)
        for key in ("database", "kernel", "uid_base", "max_active_global"):
            value = self.settings.copy()
            del value[key]
            with self.subTest(missing=key), self.assertRaises(KeyError):
                fixture.complete_runtime(value, slots=2)
        for slots in (0, True, 65):
            with self.subTest(slots=slots), self.assertRaises(ValueError):
                fixture.complete_runtime(self.settings, slots=slots)
        with self.assertRaises(ValueError):
            fixture.complete_runtime(self.settings, slots=1)

    def test_existing_explicit_data_budgets_are_preserved_not_replaced(self):
        budgets = self.complete()["data_transfer"]
        budgets["file_upload_bytes"] = 1234
        result = fixture.complete_runtime(self.settings | {"data_transfer": budgets}, slots=2)
        self.assertEqual(result["data_transfer"], budgets)
        self.assertIsNot(result["data_transfer"], budgets)
        with self.assertRaises(ValueError):
            fixture.complete_runtime(self.settings | {"data_transfer": budgets | {"max_transfers_per_vm": 129}}, slots=2)

    def test_provision_is_once_only_and_keeps_exact_coordination_identity(self):
        result = self.complete()
        with patch.object(fixture.os, "statvfs", return_value=self.free):
            fixture.provision_runtime(result)
            coordination = self.root / "physical-profile"
            identity = coordination.stat()
            self.assertEqual(coordination.read_bytes(), b"")
            self.assertEqual(identity.st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                fixture.provision_runtime(result)
            self.assertEqual(coordination.stat().st_ino, identity.st_ino)

    def test_insufficient_bytes_or_inodes_is_failure_without_creating_a_profile(self):
        result = self.complete()
        for free in (SimpleNamespace(f_bavail=0, f_frsize=4096, f_favail=2**30),
                     SimpleNamespace(f_bavail=2**40, f_frsize=4096, f_favail=0)):
            with self.subTest(free=free), patch.object(fixture.os, "statvfs", return_value=free), self.assertRaises(RuntimeError):
                fixture.provision_runtime(result)
            self.assertFalse((self.root / "physical-profile").exists())

    def test_provision_never_adopts_busy_or_nonprivate_roots(self):
        result = self.complete()
        (self.root / "state/foreign").write_bytes(b"unowned")
        with self.assertRaises(ValueError):
            fixture.provision_runtime(result)
        (self.root / "state/foreign").unlink()
        (self.root / "state").chmod(0o755)
        with self.assertRaises(ValueError):
            fixture.provision_runtime(result)
        self.assertFalse((self.root / "physical-profile").exists())


class RetirementObservationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.database = Path(self.temporary.name) / "controller.db"
        self.writer = sqlite3.connect(self.database)
        self.addCleanup(self.writer.close)
        self.writer.executescript("""
            CREATE TABLE sandboxes(sandbox_id TEXT,state_blob BLOB);
            CREATE TABLE outbox(sandbox_id TEXT,dispatch_state TEXT,id INTEGER,action_json BLOB,receipt_json BLOB);
            CREATE TABLE controller_checkpoint(id INTEGER,input_cut INTEGER,payload BLOB);
            CREATE TABLE controller_inputs(sequence INTEGER,input_json BLOB);
            CREATE TABLE controller_id_highwaters(kind TEXT,scope TEXT,high_water TEXT);
            CREATE TABLE controller_host_slots(position INTEGER,context_json BLOB);
        """)
        self.sandbox = "sb-" + "a" * 16 + "-1"
        self.checkpoint([])

    def checkpoint(self, retained):
        self.writer.execute("DELETE FROM controller_checkpoint")
        self.writer.execute("INSERT INTO controller_checkpoint VALUES(1,5,?)", (
            json.dumps({"core": {"maintenance": {"retained": retained}}}).encode(),))
        self.writer.commit()

    def test_absence_alone_or_running_with_no_outbox_never_means_cleanup(self):
        self.assertFalse(fixture.terminal_cleanup_confirmed(self.database, self.sandbox))
        self.writer.execute("INSERT INTO sandboxes VALUES(?,?)", (self.sandbox, b'{"phase":"Running"}'))
        self.writer.commit()
        self.assertFalse(fixture.terminal_cleanup_confirmed(self.database, self.sandbox))
        self.writer.execute("UPDATE sandboxes SET state_blob=?", (b'{"phase":"Destroyed"}',))
        self.writer.execute("INSERT INTO outbox VALUES(?,'dispatched',1,NULL,NULL)", (self.sandbox,))
        self.writer.commit()
        self.assertFalse(fixture.terminal_cleanup_confirmed(self.database, self.sandbox))
        self.writer.execute("UPDATE outbox SET dispatch_state='receipted'")
        self.writer.commit()
        self.assertTrue(fixture.terminal_cleanup_confirmed(self.database, self.sandbox))
        self.writer.execute("UPDATE outbox SET dispatch_state='pending'")
        self.writer.commit()
        self.assertFalse(fixture.terminal_cleanup_confirmed(self.database, self.sandbox),
                         'Destroyed cannot cover pending cleanup work')

    def test_retired_terminal_result_in_checkpoint_is_evidence_but_another_identity_is_not(self):
        result = {"ledger": {"result": {"Failed": "DeadlineExceeded"}}}
        self.checkpoint([[{"sandbox": 2, "operation": 0}, result]])
        self.assertFalse(fixture.terminal_cleanup_confirmed(self.database, self.sandbox))
        self.checkpoint([[{"sandbox": 1, "operation": 0}, result]])
        self.assertTrue(fixture.terminal_cleanup_confirmed(self.database, self.sandbox))

    def test_atomic_retirement_tail_requires_exact_identity_and_all_quiescence_fields(self):
        certificate = {key: "Confirmed" for key in (
            "worker_drained", "data_connections_drained", "host_resources_reclaimed", "external_pins_released")}
        certificate["sandbox"] = 1
        retirement = {"sandbox": 1, "quiescence": certificate}
        def tail(value):
            self.writer.execute("DELETE FROM controller_inputs")
            self.writer.execute("INSERT INTO controller_inputs VALUES(6,?)", (
                json.dumps({"input": {"Maintenance": {"Retire": value}}}).encode(),))
            self.writer.commit()
        tail(retirement)
        self.assertTrue(fixture.terminal_cleanup_confirmed(self.database, self.sandbox))
        tail(retirement | {"sandbox": 2})
        self.assertFalse(fixture.terminal_cleanup_confirmed(self.database, self.sandbox))
        tail(retirement | {"quiescence": certificate | {"external_pins_released": "Unconfirmed"}})
        self.assertFalse(fixture.terminal_cleanup_confirmed(self.database, self.sandbox))

    def test_unknown_activation_is_terminal_only_with_exact_durable_stop_coverage(self):
        activation = {"logical": {"command": {"ActivateVm": {"sandbox": 1, "vm": 0}}}, "attempt": 0}
        self.writer.execute("INSERT INTO sandboxes VALUES(?,?)", (self.sandbox, b'{"phase":"Destroyed"}'))
        self.writer.execute("INSERT INTO outbox VALUES(?,'dispatched',1,?,NULL)", (
            self.sandbox, json.dumps({"command": activation}).encode()))
        self.writer.commit()
        self.assertFalse(fixture.terminal_cleanup_confirmed(self.database, self.sandbox))
        self.writer.execute("INSERT INTO controller_inputs VALUES(6,?)", (
            json.dumps({"input": {"Effect": {"RetireActivation": {"activation": activation}}}}).encode(),))
        self.writer.commit()
        self.assertTrue(fixture.terminal_cleanup_confirmed(self.database, self.sandbox))
        self.writer.execute("UPDATE outbox SET action_json=?", (json.dumps({"command": activation | {"attempt": 1}}).encode(),))
        self.writer.commit()
        self.assertFalse(fixture.terminal_cleanup_confirmed(self.database, self.sandbox))
    def test_idle_reopen_allows_maintenance_clock_and_terminal_retirement_only(self):
        before = dict(frontiers={("input", ""): 5, ("sandbox", ""): 1, ("operation", "1"): 1}, contexts=[(0, b'identity')],
                      states=[(self.sandbox, b'{"phase":"Destroyed"}')], actions=[(1, "receipted", b'work', b'ok')])
        after = copy.deepcopy(before)
        after["frontiers"][("input", "")] = 6
        del after["frontiers"][("operation", "1")]
        after.update(states=[], actions=[])
        fixture.assert_idle_reopen(before, after)
        for key in ("frontiers", "contexts", "states", "actions"):
            broken = copy.deepcopy(after)
            if key == "frontiers":
                broken[key][("sandbox", "")] = 2
            else:
                broken[key].append(("new", b'new'))
            with self.subTest(key=key), self.assertRaises(AssertionError):
                fixture.assert_idle_reopen(before, broken)
        running = before | {"states": [(self.sandbox, b'{"phase":"Running"}')]}
        with self.assertRaises(AssertionError):
            fixture.assert_idle_reopen(running, after)


if __name__ == "__main__":
    unittest.main()
