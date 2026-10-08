"""Unprivileged source/observation regressions; no SDK import, VM or network."""

import ast
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock


SOURCE = Path(__file__).resolve().parents[2] / "tests/system/harbor_diagnostic_chain.py"
TREE = ast.parse(SOURCE.read_text())
FUNCTIONS = {node.name: node for node in TREE.body if isinstance(node, ast.FunctionDef)}


def observation_functions(**overrides):
    # Execute the real observation bodies with explicit inert dependencies. The
    # resource entry and its platform/SDK imports are never executed here.
    names = ("save_identity", "no_exact_residual", "wait_destroyed", "run_original_task")
    namespace = {"Path": Path, "Sandbox": object, "json": json, "os": os,
                 "time": SimpleNamespace(monotonic=Mock(), sleep=Mock())}
    namespace.update(overrides)
    selected = ast.Module(body=[FUNCTIONS[name] for name in names], type_ignores=[])
    exec(compile(selected, str(SOURCE), "exec"), namespace)
    return namespace


class HarborDiagnosticCompleteTests(unittest.TestCase):
    def test_entry_declares_one_complete_slot_and_provisions_before_start(self):
        source = SOURCE.read_text()
        self.assertNotIn("MAX_VM_ID", source)
        self.assertNotIn('"max_vm_id"', source)
        calls = [node for node in ast.walk(FUNCTIONS["exercise"]) if isinstance(node, ast.Call)]
        complete = next(node for node in calls if isinstance(node.func, ast.Name) and node.func.id == "complete_runtime")
        self.assertEqual({value.arg: ast.literal_eval(value.value) for value in complete.keywords}, {"slots": 1})
        positions = {name: next(node.lineno for node in calls if isinstance(node.func, ast.Name) and node.func.id == name)
                     for name in ("complete_runtime", "provision_runtime", "start_server")}
        self.assertLess(positions["complete_runtime"], positions["provision_runtime"])
        self.assertLess(positions["provision_runtime"], positions["start_server"])
        self.assertIn('wait_destroyed(root / "db/controller.db", result["sandbox_id"])', source)
        self.assertIn('"cgroup-parent"', source)

    def test_original_public_sdk_create_immediately_saves_identity_before_assertions(self):
        body = FUNCTIONS["run_original_task"].body
        trial = next(node for node in body if isinstance(node, ast.Try))
        create, save, first_assert = trial.body[:3]
        self.assertEqual(ast.unparse(create.value.func), "Sandbox.create")
        self.assertEqual({keyword.arg: ast.literal_eval(keyword.value)
                          for keyword in create.value.keywords if keyword.arg is not None},
                         {"template": "base", "timeout": 86400, "allow_internet_access": True})
        self.assertEqual(ast.unparse(save.value.func), "save_identity")
        self.assertIsInstance(first_assert, ast.Assert)
        # Preserve the reviewed initializer and phase-local original files.
        source = SOURCE.read_text()
        for command in ("apt update -y && apt install -y gcc", "bash /tmp/sandproof-oracle.sh", "bash /tests/test.sh"):
            self.assertIn(command, source)

    def test_saved_original_identity_is_private_and_survives_claim_retirement(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            name = "vm-" + "a" * 16 + "-9"
            claim = root / "jails/.claims" / name
            claim.mkdir(parents=True)
            (claim / "network.json").write_text(json.dumps({"resource_id": name, "namespace_inode": 31}))
            sandbox = SimpleNamespace(sandbox_id="sb-" + "a" * 16 + "-17")
            record = {"resource_id": name, "pid": 47, "start_time_ticks": 59}
            functions = observation_functions(record_for=Mock(return_value=({"secret": "never-save"}, name, record)))
            identities = []
            saved = functions["save_identity"](root, sandbox, identities)
            evidence = root / (name + ".identity.json")
            self.assertEqual(evidence.stat().st_mode & 0o777, 0o600)
            self.assertEqual(json.loads(evidence.read_text()), saved)
            self.assertEqual(saved, record | {"network_inode": 31, "sandbox_id": sandbox.sandbox_id})
            self.assertEqual(identities, [saved])
            self.assertNotIn("never-save", evidence.read_text())
            (claim / "network.json").unlink()
            claim.rmdir()
            self.assertEqual(json.loads(evidence.read_text())["network_inode"], 31)
            # Original observation cannot silently be overwritten by a retry.
            claim.mkdir()
            (claim / "network.json").write_text(json.dumps({"resource_id": name, "namespace_inode": 32}))
            with self.assertRaises(FileExistsError):
                functions["save_identity"](root, sandbox, identities)
            self.assertEqual(json.loads(evidence.read_text())["network_inode"], 31)

    def test_saved_identity_reaches_exact_audit_and_only_one_physical_uid(self):
        identity = {"resource_id": "vm-" + "a" * 16 + "-9", "pid": 47,
                    "start_time_ticks": 59, "network_inode": 31, "sandbox_id": "sb-a-17"}
        audit = Mock()
        uid = Mock(return_value=[])
        # All host path enumeration is inert, including the global cgroup scan.
        path = Mock()
        path.__truediv__ = Mock(return_value=path)
        path.glob.return_value = []
        path.rglob.return_value = []
        firecracker = SimpleNamespace(name="firecracker")
        functions = observation_functions(Path=Mock(return_value=path), audit_gone=audit, processes_in_uid_range=uid)
        functions["no_exact_residual"](path, "a" * 16, 10000, [identity], firecracker, path)
        audit.assert_called_once_with(path, firecracker, path, identity["resource_id"], identity=identity)
        uid.assert_called_once_with(10000, 10000)
        with self.assertRaisesRegex(AssertionError, "not saved"):
            functions["no_exact_residual"](path, "a" * 16, 10000, [], firecracker, path)
        audit.side_effect = AssertionError("original network inode still held")
        with self.assertRaisesRegex(AssertionError, "network inode"):
            functions["no_exact_residual"](path, "a" * 16, 10000, [identity], firecracker, path)

    def test_wait_requires_terminal_witness_for_the_result_sandbox_not_row_count(self):
        terminal = Mock(side_effect=[False, True])
        functions = observation_functions(terminal_cleanup_confirmed=terminal)
        functions["time"].monotonic.side_effect = [0, 0, 1]
        database, sandbox = Path("/private/controller.db"), "sb-" + "a" * 16 + "-17"
        functions["wait_destroyed"](database, sandbox)
        self.assertEqual(terminal.call_args_list, [unittest.mock.call(database, sandbox)] * 2)
        terminal.side_effect = None
        terminal.return_value = False
        functions["time"].monotonic.side_effect = [0, 0, 40]
        with self.assertRaisesRegex(AssertionError, sandbox):
            functions["wait_destroyed"](database, sandbox)

    def test_sdk_assertion_failure_kills_only_after_original_identity_was_saved(self):
        events = []
        sandbox = SimpleNamespace(sandbox_id="sb-" + "a" * 16 + "-17")
        sdk = SimpleNamespace(create=Mock(return_value=sandbox), connect=Mock(return_value=SimpleNamespace(sandbox_id="wrong")),
                              kill=Mock(side_effect=lambda *args, **kwargs: events.append("delete") or True))
        functions = observation_functions(Sandbox=sdk, options=Mock(return_value={}),
                                         process_start_ticks=Mock(return_value=59))
        identities = []
        def save(*args):
            events.append("saved")
            identity = {"resource_id": "vm-a-9", "pid": 47, "start_time_ticks": 59, "network_inode": 31}
            identities.append(identity)
            return identity
        functions["save_identity"] = save
        with self.assertRaises(AssertionError):
            functions["run_original_task"]({"name": "regex-log", "source_path": "regex-log"}, {},
                                            Path("/task"), "http://loopback", "key", Path("/evidence"), Path("/python"), identities)
        self.assertEqual(events, ["saved", "delete"])
        sdk.kill.assert_called_once_with(sandbox.sandbox_id)

    def test_failed_identity_save_retains_exact_sdk_sandbox_without_delete(self):
        sandbox = SimpleNamespace(sandbox_id="sb-" + "a" * 16 + "-17")
        sdk = SimpleNamespace(create=Mock(return_value=sandbox), kill=Mock())
        functions = observation_functions(Sandbox=sdk, options=Mock(return_value={}), sys=SimpleNamespace(stderr=Mock()))
        functions["save_identity"] = Mock(side_effect=OSError("evidence fsync failed"))
        with self.assertRaisesRegex(OSError, "evidence fsync"):
            functions["run_original_task"]({"name": "regex-log", "source_path": "regex-log"}, {},
                                            Path("/task"), "http://loopback", "key", Path("/evidence"), Path("/python"), [])
        sdk.kill.assert_not_called()
        retained_message = "".join(call.args[0] for call in functions["sys"].stderr.write.call_args_list)
        self.assertIn(sandbox.sandbox_id, retained_message)
        self.assertIn("retained", retained_message)


if __name__ == "__main__":
    unittest.main()
