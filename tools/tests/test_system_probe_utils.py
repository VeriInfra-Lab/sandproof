"""Pure probe helpers and exact-child teardown; no mounts, servers or KVM."""

import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, call, patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests/system"))

import _process_utils as processes
import _storage_probe_utils as storage


class SystemProbeUtilsTests(unittest.TestCase):
    def test_digest_and_git_keep_exact_inputs_and_do_not_refresh_the_index(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "controlled-file"
            path.write_bytes(b"storage-probe\0bytes")
            self.assertEqual(storage.digest(path), hashlib.sha256(path.read_bytes()).hexdigest())
        with patch.object(storage.subprocess, "check_output", return_value=" result\n") as executed:
            self.assertEqual(storage.command("git", "--version"), "result")
        executed.assert_called_once_with(("git", "--version"), text=True)
        with patch.object(storage, "command", return_value="commit") as executed:
            self.assertEqual(storage.git("rev-parse", "HEAD"), "commit")
        executed.assert_called_once_with("git", "--no-optional-locks", "-c",
            f"safe.directory={storage.REPO}", "-C", str(storage.REPO), "rev-parse", "HEAD")

    def test_trusted_requires_canonical_root_owner_and_nonwritable_ancestors(self):
        leaf, parent = Mock(), Mock()
        leaf.is_absolute.return_value = True
        leaf.resolve.return_value = leaf
        leaf.parents = [parent]
        leaf.stat.return_value = SimpleNamespace(st_uid=0, st_mode=0o600)
        parent.stat.return_value = SimpleNamespace(st_uid=0, st_mode=0o755)
        storage.trusted(leaf, private=True)
        for owner, mode in [(1, 0o700), (0, 0o722), (0, 0o702)]:
            parent.stat.return_value = SimpleNamespace(st_uid=owner, st_mode=mode)
            with self.subTest(owner=owner, mode=mode), self.assertRaises(AssertionError):
                storage.trusted(leaf, private=True)
        parent.stat.return_value = SimpleNamespace(st_uid=0, st_mode=0o755)
        leaf.stat.return_value = SimpleNamespace(st_uid=0, st_mode=0o644)
        with self.assertRaises(AssertionError):
            storage.trusted(leaf, private=True)
        leaf.stat.return_value = SimpleNamespace(st_uid=0, st_mode=0o600)
        leaf.resolve.return_value = parent
        with self.assertRaises(AssertionError):
            storage.trusted(leaf)
        leaf.is_absolute.return_value = False
        with self.assertRaises(AssertionError):
            storage.trusted(leaf)

    def test_child_teardown_is_bounded_and_reaps_after_forced_exit(self):
        processes.stop_child(None)
        already_exited = Mock(poll=Mock(return_value=0))
        processes.stop_child(already_exited)
        already_exited.terminate.assert_not_called()
        child = Mock(poll=Mock(return_value=None))
        child.wait.side_effect = [subprocess.TimeoutExpired("controlled-child", 10), 0]
        processes.stop_child(child)
        child.terminate.assert_called_once_with()
        child.kill.assert_called_once_with()
        self.assertEqual(child.wait.call_args_list, [call(timeout=10), call(timeout=10)])
        child.wait.side_effect = subprocess.TimeoutExpired("controlled-child", 10)
        with self.assertRaises(subprocess.TimeoutExpired):
            processes.stop_child(child)

    def test_exact_owned_child_is_terminated_and_reaped_without_a_service_launcher(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            processes.stop_child(child)
            self.assertIsNotNone(child.poll())
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=10)

    def test_storage_checks_refuse_optimized_python_before_any_probe_action(self):
        checked = subprocess.run([sys.executable, "-O", "-c",
            "import runpy; runpy.run_path(" + repr(str(ROOT / "tests/system/_storage_probe_utils.py")) + ")"],
            stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=10)
        self.assertNotEqual(checked.returncode, 0)
        self.assertIn("optimized_python", checked.stderr)


if __name__ == "__main__":
    unittest.main()
