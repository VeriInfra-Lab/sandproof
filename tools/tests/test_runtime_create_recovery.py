import copy
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import Mock

from tools.runtime_create_recovery import load_runtime_intent, matching_runtime, recover_runtime_create


class RuntimeCreateRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.database = Path(self.tmp.name).resolve() / 'controller.db'
        self.template = json.loads((Path(__file__).resolve().parents[2] /
                                    'tests/compat/template-view.json').read_text())
        self.intent = {'metadata': {'benchmark': 'terminal-bench-2.0', 'task': 'sample',
            'build': 'a' * 32, 'revision': 'r1', 'runtime_attempt': 'e' * 32}, 'template': self.template}
        self.options = {'metadata': copy.deepcopy(self.intent['metadata']), 'template': {
            'owner': 1, 'template_id': self.template['templateId'], 'revision': 'r1',
            'kernel': {'sha256': self.template['kernelSha256']},
            'rootfs': {'sha256': self.template['rootfsSha256']},
            'resources': {'cpu_count': 2, 'memory_mib': 4096, 'disk_bytes': 10737418240,
                          'host_memory_bytes': self.template['hostMemoryBytes']}}}
        self.now = 0
        with closing(sqlite3.connect(self.database)) as db, db:
            db.executescript('CREATE TABLE sandboxes(sandbox_id TEXT PRIMARY KEY,state_blob BLOB);'
                             'CREATE TABLE sandbox_create_options(sandbox_id TEXT PRIMARY KEY,options_json BLOB);')

    def insert(self, identity='sb-test-1', *, phase='Starting', owner=1, options=None):
        with closing(sqlite3.connect(self.database)) as db, db:
            db.execute('INSERT OR REPLACE INTO sandboxes VALUES (?,?)',
                       (identity, json.dumps({'owner': owner, 'phase': phase})))
            db.execute('INSERT OR REPLACE INTO sandbox_create_options VALUES (?,?)',
                       (identity, json.dumps(options or self.options)))

    def tick(self, seconds):
        self.now += seconds

    def recover(self, delete, *, sleep=None):
        return recover_runtime_create(self.database, self.intent, owner=1, namespace='test',
            delete=delete, deadline=2, monotonic=lambda: self.now, sleep=sleep or self.tick)

    def test_late_starting_identity_deleted_once_then_observed_without_public_list(self):
        def advance(seconds):
            self.tick(seconds)
            if self.now == .5:
                self.insert()
            elif self.now == 1:
                delete.assert_not_called()
                self.insert(phase='Running')
            elif self.now == 1.5:
                self.insert(phase='Destroyed')
        delete = Mock(return_value=204)
        result = self.recover(delete, sleep=advance)
        delete.assert_called_once_with('sb-test-1')
        self.assertEqual(result['status'], 'destroyed')
        self.assertTrue(result['delete_acknowledged'])
        self.assertTrue(result['pending_create_observed'])
        self.assertEqual(result['delete_phase'], 'Running')

    def test_pending_create_uses_original_deadline_without_delete(self):
        self.insert()
        delete = Mock()
        with self.assertRaises(TimeoutError):
            self.recover(delete)
        delete.assert_not_called()
        self.assertEqual(self.now, 2)

    def test_failed_pending_create_can_reach_tombstone_without_delete(self):
        self.insert()
        def advance(seconds):
            self.tick(seconds)
            self.insert(phase='Destroyed')
        delete = Mock()
        result = self.recover(delete, sleep=advance)
        delete.assert_not_called()
        self.assertEqual(result['status'], 'destroyed')
        self.assertTrue(result['pending_create_observed'])
        self.assertFalse(result['delete_acknowledged'])

    def test_lost_delete_receipt_is_not_repeated_and_does_not_become_sdk_ack(self):
        self.insert(phase='Running')
        def delete(identity):
            self.insert(identity, phase='Destroyed')
            raise OSError('private credential-bearing transport text')
        request = Mock(side_effect=delete)
        result = self.recover(request)
        request.assert_called_once_with('sb-test-1')
        self.assertEqual(result['status'], 'destroyed')
        self.assertFalse(result['delete_acknowledged'])
        self.assertEqual(result['delete_error_type'], 'OSError')
        self.assertNotIn('private', json.dumps(result))

    def test_no_match_remains_unknown_and_never_deletes(self):
        other = copy.deepcopy(self.options)
        other['metadata']['runtime_attempt'] = 'f' * 32
        self.insert(options=other)
        delete = Mock()
        with self.assertRaises(TimeoutError):
            self.recover(delete)
        delete.assert_not_called()

    def test_404_or_ack_is_insufficient_without_destroyed_state(self):
        for status in (404, 204):
            with self.subTest(status=status):
                self.now = 0
                self.insert(phase='Running')
                delete = Mock(return_value=status)
                with self.assertRaises(TimeoutError):
                    self.recover(delete)
                delete.assert_called_once_with('sb-test-1')

    def test_existing_tombstone_needs_no_delete_but_still_requires_external_physical_audit(self):
        self.insert(phase='Destroyed')
        delete = Mock()
        result = self.recover(delete)
        delete.assert_not_called()
        self.assertEqual(result['status'], 'destroyed')
        self.assertNotIn('exact_cleanup', result)

    def test_duplicate_identities_refuse_before_any_delete(self):
        self.insert()
        self.insert('sb-test-2')
        delete = Mock()
        with self.assertRaisesRegex(RuntimeError, 'ambiguous'):
            self.recover(delete)
        delete.assert_not_called()

    def test_identity_mismatch_refuses_before_any_delete(self):
        variants = []
        for section, field, value in [('metadata', 'build', 'b' * 32), ('metadata', 'task', 'other'),
                ('template', 'owner', 2), ('template', 'template_id', 'other'),
                ('template', 'revision', 'r2')]:
            options = copy.deepcopy(self.options)
            options[section][field] = value
            variants.append((options, 1))
        for section, field, value in [('rootfs', 'sha256', 'f' * 64), ('kernel', 'sha256', 'f' * 64),
                                       ('resources', 'memory_mib', 8192)]:
            options = copy.deepcopy(self.options)
            options['template'][section][field] = value
            variants.append((options, 1))
        variants.append((self.options, 2))
        for options, owner in variants:
            with self.subTest(options=options, owner=owner):
                self.insert(options=options, owner=owner)
                delete = Mock()
                with self.assertRaisesRegex(RuntimeError, 'mismatch'):
                    self.recover(delete)
                delete.assert_not_called()

    def test_namespace_or_received_identity_mismatch_refuses(self):
        self.insert()
        for extra in ({'namespace': 'other'}, {'namespace': 'test', 'sandbox_id': 'sb-test-2'}):
            with self.subTest(extra=extra), self.assertRaisesRegex(RuntimeError, 'mismatch'):
                matching_runtime(self.database, self.intent, owner=1, **extra)

    def test_missing_database_does_not_create_an_empty_file(self):
        missing = self.database.with_name('missing.db')
        with self.assertRaises(sqlite3.OperationalError):
            matching_runtime(missing, self.intent, owner=1, namespace='test')
        self.assertFalse(missing.exists())

    def load_checkpoint(self):
        return load_runtime_intent(self.database.parent, task='sample', runtime_attempt='e' * 32,
                                   build_id='a' * 32, template=self.template)

    def test_checkpoint_requires_exact_live_task_attempt_build_and_template(self):
        checkpoint = self.database.parent / 'runtime-intent.json'
        checkpoint.write_text(json.dumps(self.intent))
        self.assertEqual(self.load_checkpoint(), self.intent)
        variants = []
        for key, value in (('task', 'previous-task'), ('runtime_attempt', 'f' * 32),
                           ('build', 'b' * 32), ('revision', 'r2')):
            changed = copy.deepcopy(self.intent)
            changed['metadata'][key] = value
            variants.append(changed)
        changed = copy.deepcopy(self.intent)
        changed['template']['rootfsSha256'] = 'f' * 64
        variants.append(changed)
        for changed in variants:
            with self.subTest(changed=changed):
                checkpoint.write_text(json.dumps(changed))
                with self.assertRaisesRegex(ValueError, 'differs from live trial'):
                    self.load_checkpoint()
        checkpoint.write_text(json.dumps(self.intent))
        with self.assertRaisesRegex(ValueError, 'missing live runtime correlation'):
            load_runtime_intent(self.database.parent, task='sample', runtime_attempt=None,
                                build_id='a' * 32, template=self.template)

    def test_checkpoint_rejects_partial_duplicate_and_oversized_json(self):
        checkpoint = self.database.parent / 'runtime-intent.json'
        for raw in ('{', '{"metadata":{},"metadata":{}}', ' ' * (256 * 1024 + 1)):
            with self.subTest(size=len(raw)):
                checkpoint.write_text(raw)
                with self.assertRaises(ValueError):
                    self.load_checkpoint()

    def test_checkpoint_rejects_symlink_hardlink_fifo_and_nonprivate_directory(self):
        directory = self.database.parent
        checkpoint = directory / 'runtime-intent.json'
        original = directory / 'original.json'
        original.write_text(json.dumps(self.intent))
        checkpoint.symlink_to(original)
        with self.assertRaises(OSError):
            self.load_checkpoint()
        checkpoint.unlink()
        os.link(original, checkpoint)
        with self.assertRaises(ValueError):
            self.load_checkpoint()
        checkpoint.unlink()
        os.mkfifo(checkpoint)
        with self.assertRaises(ValueError):
            self.load_checkpoint()
        checkpoint.unlink()
        checkpoint.write_text(json.dumps(self.intent))
        directory.chmod(0o755)
        try:
            with self.assertRaises(ValueError):
                self.load_checkpoint()
        finally:
            directory.chmod(0o700)


if __name__ == '__main__':
    unittest.main()
