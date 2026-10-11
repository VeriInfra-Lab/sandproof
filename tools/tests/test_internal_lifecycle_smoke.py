"""Unprivileged checks for the explicit, manual real-lifecycle entrypoint."""
from pathlib import Path
import os
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tests/system/internal_lifecycle_smoke.py"


class InternalLifecycleSmokeTests(unittest.TestCase):
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
