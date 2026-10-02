"""Filesystem/identity regressions; actual enforcement uses the Linux probe."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import service_cgroup as cg


class ServiceCgroupTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.mount = self.root / 'cgroup'
        self.parent = self.mount / 'delegated'
        self.parent.mkdir(parents=True, mode=0o700)
        self.populate(self.parent, delegated=True)
        self.record = self.root / 'services.json'
        boot = self.root / 'boot-id'
        boot.write_text('fixture-boot')
        for context in (patch.object(cg, 'ROOT', self.mount), patch.object(cg, 'OWNER_UID', os.geteuid()),
                        patch.object(cg, 'BOOT', boot),
                        patch.object(cg, 'require_mount'), patch.object(cg, 'create_leaf', side_effect=self.create)):
            context.start()
            self.addCleanup(context.stop)

    def populate(self, path, delegated=False):
        for name, value in {'cgroup.type': 'domain', 'cgroup.subtree_control': 'cpu memory' if delegated else '',
                'cgroup.procs': '', 'cgroup.events': 'populated 0\n', 'cpu.stat': 'usage_usec 0\n',
                'memory.current': '0', 'memory.peak': '0', 'memory.events': 'oom 0\noom_kill 0\n'}.items():
            (path / name).write_text(value)

    def create(self, path):
        path.mkdir(mode=0o700)
        self.populate(path)

    def prepare(self):
        return cg.prepare(self.parent, 'svc-test', self.record, cpu_quota_us=100000, memory_bytes=64 * 1024**2)

    def test_limits_and_intent_are_durable_before_use(self):
        value = self.prepare()
        self.assertEqual(cg.observe(self.record)['limits'], {'cpu.max': '100000 100000',
            'cpu.max.burst': '0', 'memory.max': '67108864', 'memory.swap.max': '0', 'memory.oom.group': '1'})
        intent = json.loads(self.record.with_suffix('.intent.json').read_text())
        self.assertEqual(intent, {k: v for k, v in value.items() if k != 'identity'})
        self.assertEqual(self.record.stat().st_mode & 0o777, 0o600)
        with self.assertRaises(ValueError):
            self.prepare()

    def test_refuses_limits_tampering_children_and_parent_replacement(self):
        value = self.prepare()
        leaf = Path(value['path'])
        for name in ('cpu.max', 'cpu.max.burst', 'memory.max', 'memory.swap.max', 'memory.oom.group'):
            with self.subTest(name=name):
                (leaf / name).write_text('1' if value['limits'][name] == '0' else '0')
                with self.assertRaisesRegex(ValueError, 'limits'):
                    cg.checked(self.record)
                (leaf / name).write_text(value['limits'][name])
        (leaf / 'unexpected').mkdir()
        with self.assertRaisesRegex(ValueError, 'topology'):
            cg.checked(self.record)
        (leaf / 'unexpected').rmdir()
        self.parent.rename(self.mount / 'original')
        self.parent.mkdir(mode=0o700)
        self.populate(self.parent, delegated=True)
        with self.assertRaisesRegex(ValueError, 'identity'):
            cg.remove(self.record)

    def test_replacement_leaf_is_never_removed(self):
        value = self.prepare()
        leaf = Path(value['path'])
        leaf.rename(self.parent / 'original')
        self.create(leaf)
        for name, setting in value['limits'].items():
            (leaf / name).write_text(setting)
        with self.assertRaisesRegex(ValueError, 'identity'):
            cg.remove(self.record)
        self.assertTrue(leaf.exists())

    def test_populated_group_refuses_removal_even_with_empty_pid_snapshot(self):
        leaf = Path(self.prepare()['path'])
        for name, value in (('cgroup.procs', '123'), ('cgroup.events', 'populated 1')):
            with self.subTest(name=name):
                (leaf / name).write_text(value)
                with self.assertRaisesRegex(ValueError, 'contains processes'):
                    cg.remove(self.record)
                (leaf / name).write_text('' if name.endswith('procs') else 'populated 0')

    def test_invalid_delegation_paths_and_budgets_fail_before_mutation(self):
        for cpu, memory in ((True, 64 * 1024**2), (0, 64 * 1024**2), (100000, 12345), (100000, 1)):
            with self.assertRaises(ValueError):
                cg.prepare(self.parent, 'svc-test', self.record, cpu_quota_us=cpu, memory_bytes=memory)
        (self.parent / 'cgroup.subtree_control').write_text('cpu')
        with self.assertRaisesRegex(ValueError, 'delegated'):
            self.prepare()
        self.assertFalse(self.record.with_suffix('.intent.json').exists())
        self.assertFalse((self.parent / 'svc-test').exists())

    def test_partial_creation_is_retained_and_not_adopted(self):
        with patch.object(cg, 'create_leaf', side_effect=OSError('injected create failure')):
            with self.assertRaises(OSError):
                self.prepare()
        self.assertTrue(self.record.with_suffix('.intent.json').exists())
        with self.assertRaises(FileExistsError):
            self.prepare()
        self.assertFalse((self.parent / 'svc-test').exists())

    def test_wrong_boot_and_symlink_records_reject(self):
        self.prepare()
        with patch.object(cg, 'BOOT', self.root / 'other-boot'):
            cg.BOOT.write_text('other')
            with self.assertRaisesRegex(ValueError, 'boot'):
                cg.checked(self.record)
        link = self.root / 'linked.json'
        link.symlink_to(self.record)
        with self.assertRaises(ValueError):
            cg.checked(link)


if __name__ == '__main__':
    unittest.main()
