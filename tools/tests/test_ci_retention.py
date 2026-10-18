import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import ci_retention as gc


class RetentionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.parent = Path(self.temporary.name).resolve()
        root = self.parent / 'retention'
        root.mkdir(mode=0o700)
        self.store = gc.Store(root)
        self.candidate = 'a' * 40
        self.now = 20 * gc.DAY
        self.evidence = self.parent / 'original-report.json'
        self.evidence.write_text('{"passed": false, "resource_cleanup_confirmed": true}\n')
        self.evidence.chmod(0o600)
        self.receipt = self.parent / 'receipt.json'
        self.receipt.write_text(json.dumps(dict(candidate=self.candidate, status='failed',
            resource_cleanup_confirmed=True, evidence_sha256=gc.digest(self.evidence))))
        self.receipt.chmod(0o600)

    def allocate(self, retire=True, age=8):
        path = self.store.allocate(kind='acceptance', candidate=self.candidate)
        (path / 'generated-disk').write_bytes(b'x' * 8192)
        if retire:
            self.store.retire(path.name, self.receipt, self.evidence, now=self.now - age * gc.DAY)
        return path

    def test_dry_run_then_real_delete_preserves_original_evidence(self):
        path = self.allocate()
        plan = self.store.collect(now=self.now)
        self.assertEqual([row['id'] for row in plan], [path.name])
        self.assertGreaterEqual(plan[0]['allocated_bytes'], 8192)
        self.assertTrue(path.exists())
        self.store.collect(now=self.now, apply=True)
        self.assertFalse(path.exists())
        self.assertTrue(self.evidence.exists())
        self.assertEqual(self.store.read(path.name)[2]['state'], 'removed')
        self.assertEqual(self.store.collect(now=self.now, apply=True), [])

    def test_active_unregistered_and_external_materials_are_never_collected(self):
        active = self.allocate(retire=False)
        orphan = self.store.root / 'payloads' / ('b' * 32)
        orphan.mkdir(mode=0o700)
        proof = self.parent / 'proof' / '.lake'
        proof.mkdir(parents=True)
        self.assertEqual(self.store.collect(now=self.now, apply=True, max_bytes=0, max_count=0), [])
        for path in (active, orphan, proof):
            self.assertTrue(path.is_dir())

    def test_cleanup_unknown_or_nonterminal_does_not_retire(self):
        for field, value in [('resource_cleanup_confirmed', False), ('resource_cleanup_confirmed', 1),
                             ('status', 'running'), ('candidate', 'b' * 40),
                             ('evidence_sha256', 'not-a-hash')]:
            with self.subTest(field=field, value=value):
                path = self.allocate(retire=False)
                receipt = json.loads(self.receipt.read_text())
                receipt[field] = value
                bad = self.parent / 'bad.json'
                bad.write_text(json.dumps(receipt))
                bad.chmod(0o600)
                with self.assertRaises(ValueError):
                    self.store.retire(path.name, bad, self.evidence, now=self.now)
                self.assertEqual(self.store.read(path.name)[2]['state'], 'active')

    def test_all_original_terminal_statuses_remain_unchanged(self):
        for status in ('passed', 'failed', 'cancelled'):
            path = self.allocate(retire=False)
            receipt = json.loads(self.receipt.read_text())
            receipt['status'] = status
            self.receipt.write_text(json.dumps(receipt))
            self.store.retire(path.name, self.receipt, self.evidence, now=self.now - 8 * gc.DAY)
            self.store.collect(now=self.now, apply=True)
            self.assertEqual(self.store.read(path.name)[2]['receipt']['status'], status)

    def test_unknown_record_does_not_authorize_partial_collection(self):
        path = self.allocate()
        (self.store.root / 'records' / 'unrecognized').write_text('{}')
        with self.assertRaises(ValueError):
            self.store.collect(now=self.now, apply=True)
        self.assertTrue((path / 'generated-disk').exists())

    def test_foreign_owner_or_mount_is_rejected_before_deletion(self):
        path = self.allocate()
        original = Path.lstat
        def mismatched_owner(item):
            result = original(item)
            if item.name == 'generated-disk':
                from types import SimpleNamespace
                return SimpleNamespace(st_uid=result.st_uid + 1, st_dev=result.st_dev)
            return result
        with patch.object(Path, 'lstat', mismatched_owner), self.assertRaises(ValueError):
            self.store.collect(now=self.now, apply=True)
        self.assertTrue((path / 'generated-disk').exists())

    def test_evidence_must_be_preserved_independently_and_match_hash(self):
        path = self.allocate(retire=False)
        self.evidence.write_text('changed')
        with self.assertRaises(ValueError):
            self.store.retire(path.name, self.receipt, self.evidence, now=self.now)
        embedded = path / 'report.json'
        embedded.write_bytes(self.evidence.read_bytes())
        embedded.chmod(0o600)
        data = json.loads(self.receipt.read_text())
        data['evidence_sha256'] = gc.digest(embedded)
        self.receipt.write_text(json.dumps(data))
        with self.assertRaises(ValueError):
            self.store.retire(path.name, self.receipt, embedded, now=self.now)

    def test_minimum_age_is_not_bypassed_by_count_or_bytes(self):
        path = self.allocate(age=0)
        self.assertEqual(self.store.collect(now=self.now, apply=True, max_count=0, max_bytes=0), [])
        self.assertTrue(path.exists())
        for policy in ({'keep_days': 0}, {'max_count': -1}, {'max_bytes': True}):
            with self.assertRaises(ValueError):
                self.store.collect(now=self.now, **policy)

    def test_count_keeps_newest_and_bytes_are_allocated_not_sparse_length(self):
        old, fresh = self.allocate(age=3), self.allocate(age=2)
        (fresh / 'generated-disk').unlink()
        with (fresh / 'generated-disk').open('wb') as stream:
            stream.truncate(1024**3)
        plan = self.store.collect(now=self.now, max_count=1)
        self.assertEqual([row['id'] for row in plan], [old.name])
        self.assertLess(gc.allocated_bytes(fresh), 1024**3)
        self.store.collect(now=self.now, apply=True, max_bytes=0)
        self.assertFalse(fresh.exists())
        self.assertFalse(old.exists())

    def test_replacement_or_symlink_retains_every_payload(self):
        path = self.allocate()
        saved = path.with_name(path.name + '-original')
        path.rename(saved)
        path.mkdir(mode=0o700)
        with self.assertRaises(ValueError):
            self.store.collect(now=self.now, apply=True)
        path.rmdir()
        path.symlink_to(saved, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.store.collect(now=self.now, apply=True)
        self.assertTrue((saved / 'generated-disk').exists())

    def test_nested_symlink_is_not_followed_or_partially_deleted(self):
        path = self.allocate()
        (path / 'link').symlink_to(self.evidence)
        with self.assertRaises(ValueError):
            self.store.collect(now=self.now, apply=True)
        self.assertTrue((path / 'generated-disk').exists())
        self.assertTrue(self.evidence.exists())
        self.assertEqual(self.store.read(path.name)[2]['state'], 'retired')

    def test_deletion_failure_retains_intent_and_restart_resumes_same_identity(self):
        path = self.allocate()
        with patch.object(gc, 'remove_private_tree', side_effect=OSError('injected disk error')):
            with self.assertRaises(OSError):
                self.store.collect(now=self.now, apply=True)
        self.assertEqual(self.store.read(path.name)[2]['state'], 'deleting')
        reopened = gc.Store(self.store.root)
        reopened.collect(now=self.now, apply=True)
        self.assertFalse(path.exists())
        self.assertEqual(reopened.read(path.name)[2]['state'], 'removed')

    def test_crash_after_unlink_before_final_receipt_can_finish_without_replay(self):
        path = self.allocate()
        original = gc.atomic_json
        def fail_final(record, value):
            if value['state'] == 'removed':
                raise OSError('injected fsync error')
            original(record, value)
        with patch.object(gc, 'atomic_json', side_effect=fail_final):
            with self.assertRaises(OSError):
                self.store.collect(now=self.now, apply=True)
        self.assertFalse(path.exists())
        self.assertEqual(self.store.read(path.name)[2]['state'], 'deleting')
        with patch.object(gc, 'remove_private_tree') as remove:
            gc.Store(self.store.root).collect(now=self.now, apply=True)
        remove.assert_not_called()
        self.assertEqual(self.store.read(path.name)[2]['state'], 'removed')

    def test_missing_retired_without_intent_is_unknown_not_success(self):
        path = self.allocate()
        gc.remove_private_tree(path, self.store.read(path.name)[2]['identity'])
        with self.assertRaises(ValueError):
            self.store.collect(now=self.now, apply=True)
        self.assertEqual(self.store.read(path.name)[2]['state'], 'retired')

    def test_seal_and_deletion_intent_write_errors_do_not_delete(self):
        path = self.allocate(retire=False)
        with patch.object(gc, 'atomic_json', side_effect=OSError('injected fsync error')):
            with self.assertRaises(OSError):
                self.store.retire(path.name, self.receipt, self.evidence, now=self.now - 8 * gc.DAY)
        self.assertEqual(self.store.read(path.name)[2]['state'], 'active')
        self.store.retire(path.name, self.receipt, self.evidence, now=self.now - 8 * gc.DAY)
        with patch.object(gc, 'atomic_json', side_effect=OSError('injected fsync error')):
            with self.assertRaises(OSError):
                self.store.collect(now=self.now, apply=True)
        self.assertTrue((path / 'generated-disk').exists())

    def test_nonblocking_maintenance_lock_and_one_way_retirement(self):
        path = self.allocate()
        with self.store.lock():
            with self.assertRaises(BlockingIOError):
                with gc.Store(self.store.root).lock():
                    self.fail('second collector acquired the same lock')
        with self.assertRaises(ValueError):
            self.store.retire(path.name, self.receipt, self.evidence, now=self.now)
        with self.assertRaises(ValueError):
            self.store.paths('../proof')

    def test_only_fully_removed_tombstones_are_bounded(self):
        active = self.allocate(retire=False)
        unknown = self.allocate()
        for index in range(42):
            path = self.allocate(age=10)
            payload, record, value = self.store.read(path.name)
            gc.remove_private_tree(payload, value['identity'])
            value.update(state='removed', finished=index + 1)
            gc.atomic_json(record, value)
        self.store.prune_removed()
        self.assertEqual(len(list((self.store.root / 'records').iterdir())), 42)
        self.assertTrue(active.exists())
        self.assertTrue(unknown.exists())

    def test_cli_default_is_inventory_not_deletion(self):
        path = self.allocate()
        with patch.object(gc.time, 'time', return_value=self.now), patch('builtins.print'):
            gc.main(['--root', str(self.store.root), 'collect'])
        self.assertTrue(path.exists())


if __name__ == '__main__':
    unittest.main()
