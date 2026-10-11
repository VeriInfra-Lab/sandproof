"""Real Linux subprocess identity, with a controlled cgroup observation boundary."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SYSTEM = Path(__file__).resolve().parents[2] / 'tests/system'
sys.path.insert(0, str(SYSTEM))
try:
    import harbor_generic_chain as chain
finally:
    sys.path.remove(str(SYSTEM))


@unittest.skipUnless(sys.platform == 'linux', 'Linux /proc child identity required')
class ServiceRelayObservationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.children = {}
        for mode in ('relay', 'unrelated'):
            (self.root / mode).write_text("import sys\nprint('ready', flush=True)\nsys.stdin.read()\n")
            child = subprocess.Popen([sys.executable, mode], cwd=self.root,
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            self.children[mode] = child
            self.addCleanup(self.close_child, child)
            self.assertEqual(child.stdout.readline(), b'ready\n')
        self.output = self.root / 'resources.json'
        self.samples = []

    @staticmethod
    def close_child(child):
        child.stdin.close()
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=5)
        child.stdout.close()
        child.stderr.close()

    def snapshot(self, record, *, pids):
        self.assertEqual(record, self.root / 'service.json')
        self.assertTrue(set(pids) <= {p.pid for p in self.children.values()})
        return {'pids': [p.pid for p in self.children.values()], 'memory_events': {'oom_kill': 0}}

    def observe(self, *, binary=None):
        return chain.observe_service_resources(binary or Path(sys.executable), self.root / 'service.json',
            [], self.samples, self.output, minimum_relays=1)

    def test_counts_only_the_configured_protocol_from_actual_child_cmdlines(self):
        with patch.object(chain.service_cgroup, 'observe', self.snapshot):
            result = self.observe()
            expected = self.children['relay'].pid
            self.assertEqual(result['relay_mode'], 'relay')
            self.assertEqual(result['relays'], [{'pid': expected,
                'start_time_ticks': chain.process_start_ticks(expected)}])
            self.assertEqual(json.loads(self.output.read_text())[-1], result)

    def test_same_mode_from_another_executable_does_not_count_and_failure_is_saved(self):
        with patch.object(chain.service_cgroup, 'observe', self.snapshot):
            with self.assertRaisesRegex(RuntimeError, 'blocked data relays'):
                self.observe(binary=self.root / 'different-server')
        self.assertEqual(json.loads(self.output.read_text())[-1]['relays'], [])

    def test_matching_cmdline_cannot_bypass_service_cgroup_membership(self):
        def moved(record, *, pids):
            if pids:
                raise ValueError('service process outside its recorded cgroup')
            return self.snapshot(record, pids=pids)
        with patch.object(chain.service_cgroup, 'observe', moved):
            with self.assertRaisesRegex(ValueError, 'outside its recorded cgroup'):
                self.observe()

    def test_pid_reuse_between_cmdline_and_membership_is_not_counted(self):
        original = chain.process_start_ticks
        calls = 0
        def ticks(pid):
            nonlocal calls
            value = original(pid)
            if pid == self.children['relay'].pid:
                calls += 1
                return value + (calls - 1)
            return value
        with patch.object(chain.service_cgroup, 'observe', self.snapshot), \
                patch.object(chain, 'process_start_ticks', ticks):
            with self.assertRaisesRegex(RuntimeError, 'blocked data relays'):
                self.observe()
        self.assertEqual(calls, 2)
        self.assertEqual(json.loads(self.output.read_text())[-1]['relays'], [])
