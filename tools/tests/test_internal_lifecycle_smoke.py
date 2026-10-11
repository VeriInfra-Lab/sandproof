"""Unprivileged checks for the explicit, manual real-lifecycle entrypoint."""
from pathlib import Path
import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tests/system/internal_lifecycle_smoke.py"


class InternalLifecycleSmokeTests(unittest.TestCase):
    def load(self):
        spec = importlib.util.spec_from_file_location("manual_smoke", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_parsed_fingerprints_match_all_consumed_argument_names(self):
        module = self.load()
        args = module.arguments(["--host-config", "/config", "--expected-commit", "a" * 40,
                                 "--binary", "/binary", "--binary-sha256", "b" * 64,
                                 "--redis-server", "/redis", "--redis-server-sha256", "c" * 64,
                                 "--relay", "/relay", "--relay-sha256", "d" * 64])
        self.assertEqual(args.redis_server_sha256, "c" * 64)
        self.assertEqual(args.binary_sha256, "b" * 64)
        self.assertEqual(args.relay_sha256, "d" * 64)

    def test_cleanup_removes_only_private_owned_test_file(self):
        module = self.load()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "credential"
            path.write_text("test-only")
            path.chmod(0o600)
            with self.assertRaises(RuntimeError):
                module.remove_test_credential(path, os.geteuid() + 1)
            self.assertTrue(path.exists())
            link = Path(directory) / "link"
            link.symlink_to(path)
            with self.assertRaises(RuntimeError):
                module.remove_test_credential(link, os.geteuid())
            self.assertTrue(path.exists())
            self.assertTrue(link.is_symlink())
            module.remove_test_credential(path, os.geteuid())
            self.assertFalse(path.exists())

    def test_normal_help_loads_without_root_or_starting_resources(self):
        env = {key: value for key, value in os.environ.items()
               if key not in {"PYTHONOPTIMIZE", "PYTHONPATH"}}
        result = subprocess.run([sys.executable, str(SCRIPT), "--help"],
                                capture_output=True, text=True, env=env, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--expected-commit", result.stdout)
        self.assertIn("--binary-sha256", result.stdout)
        self.assertIn("--redis-server-sha256", result.stdout)

    def test_required_inputs_cannot_implicitly_use_installed_credentials(self):
        result = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True,
                                text=True, timeout=10)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("required", result.stderr)


if __name__ == "__main__":
    unittest.main()
