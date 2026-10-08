"""Outer supervisor guards and independent cleanup, never KVM acceptance."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / 'tests/system/server_slow_boundaries.py'
SPEC = importlib.util.spec_from_file_location('server_slow_probe', SOURCE)
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


class ConfigurationTests(unittest.TestCase):
    def test_final_transfer_limits_unchanged_and_two_isolated_shapes(self):
        budgets = json.loads((probe.ROOT / 'benchmarks/local-service-budgets.json').read_bytes())
        original = copy.deepcopy(budgets)
        artifacts = {key: {'path': '/trusted/' + key, 'sha256': 'a' * 64} for key in probe.host.ARTIFACTS}
        base = {'template_id': 'base-id', 'owner': None, 'kernel': {'path': '/trusted/kernel'}}
        config = probe.configuration(budgets, Path('/root/run'), '0123456789abcdef', artifacts,
                                     uid=10000, port=3000, cgroup=Path('/sys/fs/cgroup/runtime'),
                                     keys=['one', 'two'], base=base, file_size=lambda path: 100)
        self.assertEqual(budgets, original)
        self.assertEqual(config['data_transfer'], budgets['data_transfer'])
        self.assertEqual(config['max_active_global'], 2)
        self.assertEqual(config['max_active_per_tenant'], 1)
        self.assertEqual(config['memory_mib'], 512)
        self.assertEqual(config['host_resources']['memory_max_bytes'], 768 * probe.MIB)
        self.assertFalse(config['public_ipv4_enabled'])
        self.assertEqual([row['tenant'] for row in config['credentials']], [1, 2])
        self.assertEqual(config['templates']['compute_limits']['max_memory_bytes_global'], 2 * 768 * probe.MIB)
        artifacts['firecracker']['path'] = '/trusted/' + 'long' * 32
        with self.assertRaises(ValueError):
            probe.configuration(budgets, Path('/root/run'), '0123456789abcdef', artifacts,
                                uid=10000, port=3000, cgroup=Path('/sys/fs/cgroup/runtime'), keys=['one', 'two'], base=base,
                                file_size=lambda path: 100)

    def test_service_oom_hard_ceiling_and_identity_cannot_be_ignored(self):
        baseline = {'identity': {'device': 1, 'inode': 2}, 'limits': {'memory.max': '1024'},
                    'memory_current': 100, 'memory_peak': 200,
                    'memory_events': {'oom': 0, 'oom_kill': 0, 'oom_group_kill': 0}}
        probe.assert_no_oom(baseline, copy.deepcopy(baseline))
        for key, value in (('memory_current', 1025), ('memory_peak', 1025),
                           ('identity', {'device': 1, 'inode': 3}), ('limits', {'memory.max': '2048'})):
            with self.subTest(key=key), self.assertRaises(AssertionError):
                probe.assert_no_oom(baseline, baseline | {key: value})
        for event in ('oom', 'oom_kill', 'oom_group_kill'):
            altered = copy.deepcopy(baseline)
            altered['memory_events'][event] += 1
            with self.assertRaises(AssertionError):
                probe.assert_no_oom(baseline, altered)

    def test_delegated_parents_must_be_empty_and_nonoverlapping(self):
        with patch.object(probe.host, 'trusted_path'), patch.object(probe.service_cgroup, 'filesystem'), \
                patch.object(Path, 'iterdir', return_value=iter([])):
            with self.assertRaises(ValueError):
                probe.service_groups(Path('/sys/fs/cgroup/outer'), Path('/sys/fs/cgroup/outer/inner'))

    def test_child_green_report_must_match_actual_inputs_and_both_cases(self):
        report = {'result': 'passed', 'config_sha256': 'a' * 64,
                  'server': {'executable_sha256': 'b' * 64}, 'sandbox_ids': ['one', 'two'],
                  'cases': {'file_idle_and_progress': {}, 'paused_download_memory': {}}}
        with patch.object(Path, 'read_bytes', return_value=json.dumps(report).encode()):
            self.assertEqual(probe.check_child_report(Path('/private/report'), 0, 'a' * 64, 'b' * 64, ['one', 'two']), report)
            with self.assertRaises(AssertionError):
                probe.check_child_report(Path('/private/report'), 1, 'a' * 64, 'b' * 64, ['one', 'two'])
        variants = [{'config_sha256': 'c' * 64}, {'sandbox_ids': ['two', 'one']},
                    {'server': {'executable_sha256': 'c' * 64}}, {'cases': {'file_idle_and_progress': {}}}]
        for altered in variants:
            with self.subTest(altered=altered), patch.object(Path, 'read_bytes', return_value=json.dumps(report | altered).encode()):
                with self.assertRaises(AssertionError):
                    probe.check_child_report(Path('/private/report'), 0, 'a' * 64, 'b' * 64, ['one', 'two'])


class CleanupTests(unittest.TestCase):
    def test_namespace_scan_sees_fd_only_holder_and_compares_device_and_inode(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = root / 'original'
            original.write_bytes(b'original namespace fixture')
            process = root / 'proc/123'
            (process / 'fd').mkdir(parents=True)
            (process / 'ns').mkdir()
            (process / 'ns/net').write_bytes(b'different current namespace')
            (process / 'fd/4').symlink_to(original)
            identity = original.stat()
            path_type = Path
            with patch.object(probe, 'Path', side_effect=lambda value: root / 'proc' if value == '/proc' else path_type(value)):
                self.assertEqual(probe.namespace_holders(identity.st_dev, identity.st_ino), [123])
                self.assertEqual(probe.namespace_holders(identity.st_dev + 1, identity.st_ino), [])

    def test_unknown_namespace_fd_inspection_never_reports_absence(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            process = root / 'proc/123'
            (process / 'fd').mkdir(parents=True)
            path_type, original_iterdir = Path, Path.iterdir

            def iterdir(path):
                if path == process / 'fd':
                    raise PermissionError('unavailable descriptor inventory')
                return original_iterdir(path)

            with patch.object(probe, 'Path', side_effect=lambda value: root / 'proc' if value == '/proc' else path_type(value)), \
                    patch.object(Path, 'iterdir', iterdir):
                with self.assertRaises(PermissionError):
                    probe.namespace_holders(1, 2)

    def test_service_cleanup_waits_for_observed_empty_leaf_without_killing(self):
        baseline = {'identity': {'device': 1, 'inode': 2}, 'limits': {'memory.max': '1024'},
                    'memory_current': 100, 'memory_peak': 200, 'memory_events': {}}
        occupied, empty = baseline | {'pids': [10], 'populated': 1}, baseline | {'pids': [], 'populated': 0}
        with patch.object(probe.service_cgroup, 'observe', side_effect=[occupied, empty]), \
                patch.object(probe.time, 'sleep') as unused:
            result = probe.wait_service_empty(Path('/private/record'), baseline,
                                              clock=lambda: 1, sleep=lambda _: None)
            self.assertEqual(result, empty)
            unused.assert_not_called()
        with patch.object(probe.service_cgroup, 'observe', return_value=occupied):
            with self.assertRaises(AssertionError):
                probe.wait_service_empty(Path('/private/record'), baseline, timeout=0,
                                         clock=lambda: 1, sleep=lambda _: None)

    def test_first_failed_delete_never_skips_second(self):
        calls = []

        def first():
            calls.append('first')
            raise TimeoutError()

        def second():
            calls.append('second')
            return True

        outcomes = probe.kill_each([SimpleNamespace(sandbox_id='one', kill=first),
                                    SimpleNamespace(sandbox_id='two', kill=second)], lambda fn: fn())
        self.assertEqual(calls, ['first', 'second'])
        self.assertFalse(outcomes[0]['confirmed'])
        self.assertEqual(outcomes[0]['failure_type'], 'TimeoutError')
        self.assertTrue(outcomes[1]['confirmed'])

    def test_unconfirmed_delete_is_not_success(self):
        outcome = probe.kill_each([SimpleNamespace(sandbox_id='one', kill=lambda: False)], lambda fn: fn())
        self.assertFalse(outcome[0]['confirmed'])
        self.assertEqual(outcome[0]['failure_type'], 'AssertionError')


if __name__ == '__main__':
    unittest.main()
