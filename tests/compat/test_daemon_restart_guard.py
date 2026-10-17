"""The privileged probe must refuse ordinary CI before performing host actions."""
from pathlib import Path
import subprocess
import sys
import unittest


class DaemonRestartGuard(unittest.TestCase):
    def test_direct_invocation_refuses_before_reading_candidate_or_configuring_network(self):
        probe = Path(__file__).resolve().parents[1] / 'system/guest_daemon_restart.py'
        for optimize in ([], ['-O']):
            with self.subTest(optimize=optimize):
                result = subprocess.run([sys.executable, *optimize, str(probe),
                    '--service', '/nonexistent/service', '--worker', '/nonexistent/worker',
                    '--configuration', '/nonexistent/configuration'],
                    capture_output=True, text=True, timeout=10)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, '')
                self.assertNotIn('FileNotFoundError', result.stderr)
                if not optimize:
                    self.assertIn('requires root as PID 1 in a new PID namespace', result.stderr)
                else:
                    self.assertIn('assertions', result.stderr)
