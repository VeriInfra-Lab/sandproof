"""Filesystem/identity regressions; actual enforcement uses the Linux probe."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from contextlib import ExitStack
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

    def test_service_identity_requires_complete_non_system_integer_ids_before_effects(self):
        self.assertIsNone(cg.service_identity(None, None))
        self.assertEqual(cg.service_identity(3100000, 3100001), (3100000, 3100001))
        self.assertEqual(cg.prefix(self.record)[-1], '--')
        self.assertEqual(cg.prefix(self.record, uid=3100000, gid=3100001)[-5:],
                         ['--uid', '3100000', '--gid', '3100001', '--'])
        for uid, gid in ((0, 1000), (1000, 0), (999, 1000), (1000, 999),
                         (1000, None), (None, 1000), (True, 1000), (1000, 1.0),
                         (-1, 1000), (2**32 - 1, 1000), ('1000', 1000)):
            with self.subTest(uid=uid, gid=gid), patch.object(cg, 'checked') as checked, \
                    patch.object(cg, 'observe') as observe, patch.object(cg, '_drop_identity') as drop, \
                    patch.object(cg.os, 'execv') as execute:
                with self.assertRaises(ValueError):
                    cg.prefix(self.record, uid=uid, gid=gid)
                with self.assertRaises(ValueError):
                    cg.join_exec(self.record, ['/approved/center'], uid=uid, gid=gid)
                for effect in (checked, observe, drop, execute):
                    effect.assert_not_called()

    def test_service_drop_refuses_non_root_or_threaded_launch_before_record_access(self):
        for real, effective, tasks in ((1000, 0, ['1']), (0, 1000, ['1']), (0, 0, ['1', '2'])):
            with self.subTest(real=real, effective=effective, tasks=tasks), \
                    patch.object(cg.os, 'getuid', return_value=real), \
                    patch.object(cg.os, 'geteuid', return_value=effective), \
                    patch.object(cg.os, 'listdir', return_value=tasks), \
                    patch.object(cg, 'checked') as checked, patch.object(cg.os, 'execv') as execute:
                with self.assertRaisesRegex(ValueError, 'single-threaded root'):
                    cg.join_exec(self.record, ['/approved/center'], uid=1000, gid=1000)
                checked.assert_not_called()
                execute.assert_not_called()

    def test_original_membership_and_pid_observation_precede_drop_and_single_exec(self):
        value = self.prepare()
        leaf = Path(value['path'])
        calls = []
        def observe(record, *, pids):
            self.assertEqual(record, self.record)
            self.assertEqual(pids, [os.getpid()])
            self.assertEqual((leaf / 'cgroup.procs').read_bytes(), b'0')
            calls.append('observe')
        def drop(uid, gid, membership):
            self.assertEqual((uid, gid), (1000, 1001))
            self.assertEqual(membership, '0::/' + str(leaf.relative_to(self.mount)))
            calls.append('drop')
        def execute(binary, argv):
            self.assertEqual((binary, argv), ('/approved/center', ['/approved/center', 'serve']))
            calls.append('exec')
            raise OSError('injected exec refusal')
        with patch.object(cg, 'checked', return_value=value), \
                patch.object(cg.os, 'getuid', return_value=0), \
                patch.object(cg.os, 'geteuid', return_value=0), \
                patch.object(cg.os, 'listdir', return_value=['1']), \
                patch.object(cg.os, 'sched_getscheduler', return_value=cg.os.SCHED_OTHER, create=True), \
                patch.object(Path, 'read_text', return_value='0'), \
                patch.object(cg, 'observe', side_effect=observe), \
                patch.object(cg, '_drop_identity', side_effect=drop), \
                patch.object(cg.os, 'execv', side_effect=execute):
            with self.assertRaisesRegex(OSError, 'exec refusal'):
                cg.join_exec(self.record, ['/approved/center', 'serve'], uid=1000, gid=1001)
        self.assertEqual(calls, ['observe', 'drop', 'exec'])  # No root fallback/retry.

    def dropped_status(self):
        return '\n'.join(['Uid:\t1000\t1000\t1000\t1000', 'Gid:\t1001\t1001\t1001\t1001',
                          'Groups:\t', *(name + ':\t0000000000000000'
                          for name in ('CapInh', 'CapPrm', 'CapEff', 'CapAmb'))])

    def mocked_drop(self, *, status=None, uid=(1000, 1000, 1000), gid=(1001, 1001, 1001),
                    groups=(), membership='0::/original', failure=None):
        calls = []
        def effect(name, *args):
            calls.append((name, args))
            if failure == name:
                raise PermissionError('injected syscall refusal')
        with ExitStack() as stack:
            for name in ('setgroups', 'setgid', 'setuid'):
                stack.enter_context(patch.object(cg.os, name, side_effect=lambda *args, name=name: effect(name, *args), create=True))
            stack.enter_context(patch.object(cg.os, 'getresuid', return_value=uid, create=True))
            stack.enter_context(patch.object(cg.os, 'getresgid', return_value=gid, create=True))
            stack.enter_context(patch.object(cg.os, 'getgroups', return_value=list(groups)))
            stack.enter_context(patch.object(Path, 'read_text', side_effect=[
                status if status is not None else self.dropped_status(), membership]))
            try:
                cg._drop_identity(1000, 1001, '0::/original')
            finally:
                self.last_drop_calls = calls

    def test_drop_clears_groups_then_all_ids_and_checks_saved_filesystem_capabilities(self):
        self.mocked_drop()
        self.assertEqual(self.last_drop_calls,
                         [('setgroups', ([],)), ('setgid', (1001,)), ('setuid', (1000,))])
        for change in ({'uid': (1000, 0, 1000)}, {'gid': (1001, 1001, 0)}, {'groups': [99]},
                       {'membership': '0::/foreign'},
                       {'status': self.dropped_status().replace('Uid:\t1000\t1000\t1000\t1000', 'Uid:\t1000\t1000\t1000\t0')},
                       {'status': self.dropped_status().replace('Groups:\t', 'Groups:\t0')}):
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, 'after privilege drop'):
                self.mocked_drop(**change)
        for name in ('CapInh', 'CapPrm', 'CapEff', 'CapAmb'):
            for value in ('0000000000000001', ''):
                with self.subTest(capability=name, value=value), self.assertRaises(ValueError):
                    self.mocked_drop(status=self.dropped_status().replace(name + ':\t0000000000000000', name + ':\t' + value))

    def test_failed_drop_syscall_never_attempts_later_privilege_steps(self):
        for index, name in enumerate(('setgroups', 'setgid', 'setuid')):
            with self.subTest(syscall=name), self.assertRaisesRegex(PermissionError, 'syscall refusal'):
                self.mocked_drop(failure=name)
            self.assertEqual([call[0] for call in self.last_drop_calls],
                             ['setgroups', 'setgid', 'setuid'][:index + 1])

    def test_existing_root_exec_does_not_gain_drop_or_identity_requirements(self):
        value = self.prepare()
        with patch.object(cg, 'checked', return_value=value), patch.object(cg, 'observe'), \
                patch.object(cg.os, 'sched_getscheduler', return_value=cg.os.SCHED_OTHER, create=True), \
                patch.object(Path, 'read_text', return_value='0'), \
                patch.object(cg.os, 'getuid') as identity, patch.object(cg, '_drop_identity') as drop, \
                patch.object(cg.os, 'execv', side_effect=OSError('single exact exec')) as execute:
            with self.assertRaisesRegex(OSError, 'single exact exec'):
                cg.join_exec(self.record, ['/approved/host', 'relay'])
            identity.assert_not_called()
            drop.assert_not_called()
            execute.assert_called_once_with('/approved/host', ['/approved/host', 'relay'])


if __name__ == '__main__':
    unittest.main()
