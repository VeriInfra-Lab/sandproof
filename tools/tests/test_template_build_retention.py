"""Real SQLite + compiled Rust window authority. All budgets are test fixtures."""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch

from tools.template_build_client import BuildClient, saved_intent
from tools.template_build_service import Backend, BuildHTTP, Supervisor
from tools.template_build_store import BuildError, BuildStore, MIB
from tools.template_stage import file_hash, stage_delete, stage_inventory
from tools.tests.test_template_build_service import limits, PowerLoss, FakeBackend

ROOT = Path(__file__).resolve().parents[2]


class RetentionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run(['cargo', 'build', '--locked', '-p', 'sandproof-controller-service',
                        '--bin', 'sandproof-window-reducer'], cwd=ROOT, check=True, timeout=180)
        target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
        if not target.is_absolute():
            target = ROOT / target
        binary = target / 'debug/sandproof-window-reducer'
        cls.helper = {'path': str(binary), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                      'owner_uid': os.geteuid(), 'max_input_bytes': 65536,
                      'max_output_bytes': 65536, 'timeout_seconds': 5}

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.now = 1000
        self.config = {'namespace': 17, 'windows': {'max_open_windows': 4, 'max_duration_ms': 600000},
                       'retention_ms': 1000, 'max_request_bytes': 2048, 'max_result_bytes': 8192,
                       'max_record_bytes': 65536, 'database_bytes': 8 * MIB, 'wal_bytes': 17 * MIB,
                       'recovery_reserve_bytes': 8 * MIB, 'helper': self.helper}
        self.limits = limits()
        self.store = self.open()
        self.addCleanup(lambda: self.store.close() if self.store else None)

    def open(self, **kwargs):
        return BuildStore(self.root, self.limits, bounded=self.config, clock=lambda: self.now,
                          free_bytes=lambda: 1000000 * MIB, **kwargs)

    def test_configuration_reserves_future_encoded_result_and_framing(self):
        from tools.template_build_store import bounded_configuration, future_record_fields
        config = self.config | {'max_request_bytes': 4096, 'max_result_bytes': 32768}
        with self.assertRaisesRegex(ValueError, 'insufficient encoded record'):
            bounded_configuration(config)  # The formerly accepted 64 KiB profile.
        minimum = 2 * config['max_request_bytes'] + sum(future_record_fields(config).values()) + 4096
        bounded_configuration(config | {'max_record_bytes': minimum})
        with self.assertRaisesRegex(ValueError, 'insufficient encoded record'):
            bounded_configuration(config | {'max_record_bytes': minimum - 1})
        self.assertEqual(self.store.db.execute('SELECT count(*) FROM jobs').fetchone()[0], 0)

    def reopen(self):
        self.store.close()
        self.store = None
        self.store = self.open()

    def window(self, owner=1, duration=5000):
        return self.store.open_window(owner, duration)['requestWindow']

    def submit(self, token='key', owner=1, window=None, request=None):
        return self.store.submit(owner, request or {'kind': 'oci', 'image': 'ubuntu:24.04'}, token,
                                 window=window or self.window(owner))['buildID']

    def terminal(self, job):
        self.store.private_checkpoint(job, builder_settled={'requestWindow': None, 'sandboxID': None}, private_ack=1)
        self.store.change(job, 'Failed', failure='Synthetic', release=True)

    def test_build_ids_never_reuse_after_gc_even_when_randomness_collides(self):
        server = BuildHTTP(('127.0.0.1', 0), self.store, [{'tenant': 1, 'secret': 'synthetic'}])
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            client = BuildClient('http://127.0.0.1:' + str(server.server_port), 'synthetic')
            with patch('tools.template_build_store.secrets.token_hex', return_value='a' * 32):
                first_window = self.window()
                first = client.request('POST', '/sandproof/template-builds',
                    {'kind': 'dockerfile', 'requestWindow': first_window}, token='old')['buildID']
                self.assertEqual(first, '00000000000000110000000000000000')
                self.terminal(first)
                self.store.close_window(1, first_window)
                self.now += 2
                self.store.retire_results()
                self.assertEqual(self.store.rows(), [])
                second = client.request('POST', '/sandproof/template-builds',
                    {'kind': 'dockerfile', 'requestWindow': self.window()}, token='new')['buildID']
                self.assertEqual(second, '00000000000000110000000000000001')
                with self.assertRaisesRegex(RuntimeError, 'NotFound'):
                    client.request('GET', '/sandproof/template-builds/' + first)
                with self.assertRaisesRegex(RuntimeError, 'NotFound'):
                    client.request('PUT', '/sandproof/template-builds/' + first + '/context', b'late bytes', size=10)
                self.assertEqual(self.store.get(second)['state'], 'AwaitingInput')
                self.assertIsNone(self.store.get(second)['input_hash'])
                self.assertFalse((self.store.directory(second) / 'source.tar').exists())
                with self.assertRaisesRegex(BuildError, 'ClosedWindow'):
                    self.store.submit(1, {'kind': 'dockerfile'}, 'old', window=first_window)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def test_permanent_frontier_survives_empty_journal_restart(self):
        window = self.window()
        first = self.submit(window=window)
        self.terminal(first)
        self.store.close_window(1, window)
        self.now += 2
        self.store.retire_results()
        self.assertEqual(self.store.rows(), [])
        self.assertEqual(self.store._job_frontier(), 0)
        self.reopen()
        self.assertEqual(self.store._job_frontier(), 0)
        second = self.submit(token='new')
        self.assertEqual(int(second[16:], 16), 1)
        self.assertEqual(self.store._job_frontier(), 1)

    def test_allocator_rollback_and_unknown_commit_keep_admission_atomic(self):
        for point in ('after_job_allocation', 'before_commit'):
            with self.subTest(point=point):
                window = self.window()
                self.fault_once(point)
                with self.assertRaises(PowerLoss):
                    self.submit(token=point, window=window)
                self.assertIsNone(self.store._job_frontier())
                self.assertEqual(self.store.rows(), [])
                self.assertFalse(any(path.is_dir() for path in self.root.iterdir()))
                self.reopen()
                self.assertIsNone(self.store._job_frontier())
                self.store.close_window(1, window)
        window = self.window()
        self.fault_once('after_commit')
        with self.assertRaises(PowerLoss):
            self.submit(token='uncertain', window=window)
        first = self.store.rows()[0]['id']
        self.assertEqual(self.store._job_frontier(), 0)
        self.reopen()
        self.assertEqual(self.submit(token='uncertain', window=window), first)
        self.assertEqual(self.submit(token='later', window=window)[16:], '0000000000000001')

    def test_allocator_exhaustion_never_wraps_or_blocks_existing_tuple(self):
        window = self.window()
        with self.store.transaction():
            self.store.db.execute("UPDATE job_allocator SET issued_through='fffffffffffffffe' WHERE id=1")
        last = self.submit(window=window)
        self.assertEqual(last, '0000000000000011ffffffffffffffff')
        self.assertEqual(self.submit(window=window), last)
        self.terminal(last)
        self.store.close_window(1, window)
        self.now += 2
        self.store.retire_results()
        self.reopen()
        with self.assertRaisesRegex(BuildError, 'BuildIdentityExhausted'):
            self.submit(token='overflow')
        self.assertEqual(self.store._job_frontier(), 2**64 - 1)
        self.assertEqual(self.store.rows(), [])
        self.assertFalse(any(path.is_dir() for path in self.root.iterdir()))

    def test_missing_frontier_is_not_reseeded_from_post_gc_rows(self):
        window = self.window()
        first = self.submit(window=window)
        self.terminal(first)
        self.store.close_window(1, window)
        self.now += 2
        self.store.retire_results()
        with self.store.transaction():
            self.store.db.execute('DROP TABLE job_allocator')
        self.store.close()
        self.store = None
        with self.assertRaisesRegex(ValueError, 'frontier is missing'):
            self.open()
        with sqlite3.connect(self.root / 'builds.db') as db:
            self.assertIsNone(db.execute("SELECT 1 FROM sqlite_master WHERE name='job_allocator'").fetchone())

    def test_frontier_behind_existing_identity_fails_closed(self):
        self.submit()
        with self.store.transaction():
            self.store.db.execute('UPDATE job_allocator SET issued_through=NULL WHERE id=1')
        self.store.close()
        self.store = None
        with self.assertRaisesRegex(ValueError, 'exceeds permanent build frontier'):
            self.open()

    def test_random_bounded_format_cannot_migrate_from_surviving_rows(self):
        with self.store.transaction():
            settings = json.loads(self.store.db.execute('SELECT limits FROM settings WHERE id=1').fetchone()[0])
            settings['schema_version'] = 3
            self.store.db.execute('UPDATE settings SET limits=? WHERE id=1', (json.dumps(settings, sort_keys=True),))
            self.store.db.execute('DROP TABLE job_allocator')
        self.store.close()
        self.store = None
        with self.assertRaisesRegex(ValueError, 'lacks a permanent build frontier'):
            self.open(migrate_legacy=True)
        with sqlite3.connect(self.root / 'builds.db') as db:
            self.assertEqual(json.loads(db.execute('SELECT limits FROM settings WHERE id=1').fetchone()[0])['schema_version'], 3)

    def test_explicit_legacy_migration_seeds_above_matching_random_ids(self):
        self.store.close()
        self.store = None
        self.root = self.root / 'legacy-frontier'
        self.root.mkdir(mode=0o700)
        legacy = BuildStore(self.root, self.limits, clock=lambda: self.now, free_bytes=lambda: 1000000 * MIB)
        request = {'kind': 'oci', 'image': 'ubuntu:24.04'}
        try:
            for suffix in (3, 9):
                with patch('tools.template_build_store.secrets.token_hex', return_value=f"0000000000000011{suffix:016x}"):
                    legacy.submit(1, request, 'old-' + str(suffix))
        finally:
            legacy.close()
        self.store = self.open(migrate_legacy=True)
        self.assertEqual(self.store._job_frontier(), 9)
        self.assertEqual(self.store.submit(1, request, 'old-3')['buildID'], '00000000000000110000000000000003')
        self.assertEqual(self.submit(owner=2), '0000000000000011000000000000000a')
        self.reopen()
        self.assertEqual(self.store._job_frontier(), 10)
        self.assertEqual(sum(not row['window_namespace'] for row in self.store.rows()), 2)

    def test_original_tuple_precedes_closed_expiry_and_reboot(self):
        window = self.window()
        job = self.submit(window=window)
        self.store.close_window(1, window)
        self.now += 10
        self.reopen()
        self.assertEqual(self.submit(window=window), job)
        with self.assertRaisesRegex(BuildError, 'IdempotencyConflict'):
            self.submit(window=window, request={'kind': 'oci', 'image': 'debian:13'})
        with self.assertRaisesRegex(BuildError, 'ClosedWindow'):
            self.submit(token='new', window=window)
        self.assertEqual(len(self.store.rows()), 1)

    def test_expiry_refusal_commits_floor_and_clock_rollback_cannot_reopen(self):
        window = self.window(duration=1000)
        self.now += 2
        with self.assertRaisesRegex(BuildError, 'ClosedWindow'):
            self.submit(window=window)
        floor = self.store._window_state()['time_floor_ms']
        self.assertEqual(floor, 1002000)
        self.now -= 100
        self.reopen()
        with self.assertRaisesRegex(BuildError, 'ClosedWindow'):
            self.submit(window=window)
        self.assertEqual(self.store._window_state()['time_floor_ms'], floor)

    def test_tenant_namespace_and_missing_key_domains(self):
        window = self.window()
        with self.assertRaisesRegex(BuildError, 'WrongTenant'):
            self.submit(owner=2, window=window)
        with self.assertRaisesRegex(BuildError, 'WrongRequestNamespace'):
            self.submit(window=window | {'namespace': 18})
        with self.assertRaisesRegex(BuildError, 'IdempotencyKeyRequired'):
            self.store.submit(1, {'kind': 'dockerfile'}, window=window)
        with self.assertRaisesRegex(BuildError, 'LegacyBuildDomainClosed'):
            self.store.submit(1, {'kind': 'dockerfile'}, 'unseen')
        first = self.submit(window=window)
        second = self.submit(window=self.window())
        self.assertNotEqual(first, second)
        self.store.close()
        self.store = None
        changed = dict(self.config, namespace=18)
        with self.assertRaisesRegex(ValueError, 'migration'):
            BuildStore(self.root, self.limits, bounded=changed)
        self.store = self.open()

    def test_window_capacity_and_frontier_never_reused(self):
        windows = [self.window() for _ in range(4)]
        with self.assertRaisesRegex(BuildError, 'CapacityExceeded'):
            self.window()
        self.store.close_window(1, windows[0])
        self.reopen()
        self.assertGreater(self.window()['sequence'], windows[-1]['sequence'])

    def test_retention_requires_closed_window_time_and_settlement(self):
        window = self.window()
        job = self.submit(window=window)
        self.terminal(job)
        self.now += 2
        self.store.retire_results()
        self.assertEqual(self.store.get(job)['state'], 'Failed')
        self.store.close_window(1, window)
        with self.store.transaction():
            self.store.db.execute('UPDATE jobs SET private_ack=0 WHERE id=?', (job,))
        self.store.retire_results()
        self.assertTrue(self.store.directory(job).exists())
        self.store.private_checkpoint(job, private_ack=1)
        self.store.retire_results()
        self.assertFalse(self.store.directory(job).exists())
        self.reopen()
        with self.assertRaisesRegex(BuildError, 'ClosedWindow'):
            self.submit(window=window)
        with self.assertRaisesRegex(BuildError, 'NotFound'):
            self.store.get(job)

    def test_no_early_eviction_full_promised_history_and_reuse_after_expiry(self):
        self.store.close()
        self.store = None
        # Separate journal because limits are immutable.
        self.root = self.root / 'small'
        self.root.mkdir(mode=0o700)
        self.limits.update(max_jobs=2, max_jobs_per_tenant=2)
        self.store = self.open()
        window = self.window()
        for token in ('a', 'b'):
            self.terminal(self.submit(token=token, window=window))
        self.store.close_window(1, window)
        new = self.window()
        with self.assertRaisesRegex(BuildError, 'BuildBudgetExceeded'):
            self.submit(token='c', window=new)
        self.store.retire_results()
        self.assertEqual(len(self.store.rows()), 2)
        self.now += 2
        self.store.retire_results()
        self.assertEqual(len(self.store.rows()), 0)
        self.submit(token='c', window=new)

    def test_unknown_old_work_does_not_block_other_retirement_or_accumulate_storage(self):
        old_window = self.window()
        old = self.submit(token='old', window=old_window)
        self.store.change(old, 'CleanupUnknown', failure='Unresolved')
        self.store.close_window(1, old_window)
        for i in range(30):
            window = self.window()
            job = self.submit(token=str(i), window=window)
            self.terminal(job)
            self.store.close_window(1, window)
            self.now += 2
            self.store.retire_results()
            self.assertEqual([row['id'] for row in self.store.rows()], [old])
            self.assertLessEqual((self.root / 'builds.db').stat().st_size, self.config['database_bytes'])
            self.assertLessEqual((self.root / 'builds.db-wal').stat().st_size, self.config['wal_bytes'])
        self.reopen()
        self.assertEqual(self.store.get(old)['state'], 'CleanupUnknown')
        self.assertGreater(self.store.get(old)['reserved'], 0)

    def test_ready_artifacts_are_not_deleted_with_public_history(self):
        job = self.submit()
        artifact = self.root.parent / ('artifact-' + job)
        artifact.mkdir()
        self.addCleanup(lambda: artifact.rmdir())
        self.store.private_checkpoint(job, builder_settled={'sandboxID': 'sb-1'}, private_ack=1)
        self.store.change(job, 'Ready', result={'templateId': 'tpl-' + job}, release=True)
        self.now += 10
        self.store.retire_results()
        self.assertTrue(artifact.is_dir())
        self.assertEqual(len(self.store.rows()), 0)

    def test_five_insert_failures_create_no_orphans(self):
        window = self.window()
        self.store.db.execute("CREATE TEMP TRIGGER fail_insert BEFORE INSERT ON jobs BEGIN SELECT RAISE(FAIL,'injected'); END")
        for i in range(5):
            with self.assertRaises(sqlite3.IntegrityError):
                self.submit(token=str(i), window=window)
        self.assertEqual(len(self.store.rows()), 0)
        self.assertFalse(any(path.is_dir() for path in self.root.iterdir()))
        self.reopen()
        self.assertEqual(len(self.store.rows()), 0)

    def fault_once(self, point):
        def fault(current):
            if current == point:
                self.store.fault = lambda _point: None
                raise PowerLoss()
        self.store.fault = fault

    def test_admission_commit_and_directory_fsync_crashes_recover_same_job(self):
        for point in ('after_admission_commit', 'after_mkdir', 'after_marker_fsync', 'after_staging_fsync', 'after_prepare_rename', 'after_directory_fsync'):
            with self.subTest(point=point):
                window = self.window()
                self.fault_once(point)
                with self.assertRaises(PowerLoss):
                    self.submit(token=point, window=window)
                row = next(row for row in self.store.rows() if row['token'] == point)
                self.reopen()
                self.assertEqual(self.submit(token=point, window=window), row['id'])
                self.assertEqual(self.store.get(row['id'])['fs_phase'], 'Ready')
                self.terminal(row['id'])
                self.store.close_window(1, window)
                self.now += 2
                self.store.retire_results()
                self.assertEqual(self.store.rows(), [])
                self.assertFalse(any(path.is_dir() for path in self.root.iterdir()))

    def test_live_supervisor_resumes_committed_preparing_after_io_error(self):
        supervisor = Supervisor(self.store, FakeBackend())
        supervisor.recover()
        window = self.window()
        def fail(point):
            if point == 'after_directory_fsync':
                self.store.fault = lambda _point: None
                raise OSError('transient fsync error')
        self.store.fault = fail
        with self.assertRaises(OSError):
            self.submit(window=window)
        row = self.store.rows()[0]
        self.assertEqual(self.store.submit(1, {'kind': 'oci', 'image': 'ubuntu:24.04'}, 'key', window=window)['state'], 'Preparing')
        self.now += 1000
        supervisor.tick()
        self.assertEqual(self.store.get(row['id'])['state'], 'Failed')
        self.assertEqual(self.store.get(row['id'])['reserved'], 0)

    def test_unknown_final_directory_is_not_adopted_after_preparing_commit(self):
        window = self.window()
        self.fault_once('after_admission_commit')
        with self.assertRaises(PowerLoss):
            self.submit(window=window)
        row = self.store.rows()[0]
        path = self.store.directory(row['id'])
        path.mkdir(mode=0o700)  # No marker: this is not our unpublished stage.
        self.store.close()
        self.store = None
        with self.assertRaises((FileNotFoundError, BuildError)):
            self.open()
        self.assertTrue(path.is_dir())
        with sqlite3.connect(self.root / 'builds.db') as db:
            self.assertEqual(db.execute('SELECT COUNT(*) FROM jobs').fetchone()[0], 1)

    def test_deletion_crashes_resume_exact_manifest_without_freeing_capacity(self):
        for point in ('after_delete_intent', 'after_delete_file', 'after_delete_fsync'):
            with self.subTest(point=point):
                window = self.window()
                job = self.submit(token=point, window=window)
                self.terminal(job)
                self.store.close_window(1, window)
                self.now += 2
                self.fault_once(point)
                with self.assertRaises(PowerLoss):
                    self.store.retire_results()
                self.assertEqual(self.store.get(job)['fs_phase'], 'Deleting')
                self.reopen()
                with self.assertRaisesRegex(BuildError, 'NotFound'):
                    self.store.get(job)

    def test_unknown_entry_and_replaced_inode_are_never_adopted(self):
        job = self.submit()
        unknown = self.store.directory(job) / 'foreign'
        unknown.write_text('preserve')
        self.terminal(job)
        self.now += 10
        with self.assertRaisesRegex(BuildError, 'UnknownEntry'):
            self.store.retire_results()
        self.assertEqual(unknown.read_text(), 'preserve')
        unknown.unlink()
        self.fault_once('after_delete_intent')
        with self.assertRaises(PowerLoss):
            self.store.retire_results()
        marker = self.store.directory(job) / '.owner.json'
        original = marker.rename(marker.with_name('old'))
        marker.write_text('replacement')
        with self.assertRaisesRegex(BuildError, 'UnknownEntry|IdentityUnknown'):
            self.store._delete(self.store.get(job))
        self.assertEqual(marker.read_text(), 'replacement')
        self.assertTrue(original.exists())

    def test_dangling_symlink_replacement_never_clears_deletion_charge(self):
        job = self.submit()
        self.terminal(job)
        self.now += 10
        self.fault_once('after_delete_intent')
        with self.assertRaises(PowerLoss):
            self.store.retire_results()
        original = self.store.directory(job)
        moved = original.rename(original.with_name('moved-' + job))
        original.symlink_to(self.root / 'missing-target', target_is_directory=True)
        with self.assertRaises((ValueError, OSError, BuildError)):
            self.store._delete(self.store.get(job))
        self.assertEqual(self.store.get(job)['fs_phase'], 'Deleting')
        self.assertTrue(original.is_symlink())
        self.assertTrue(moved.is_dir())

    def test_staging_delete_fsync_crash_never_restages_or_pins_completed_result(self):
        job = self.submit()
        row = self.store.get(job)
        self.store.private_checkpoint(job, builder_settled={'sandboxID': 'sb-1'}, private_ack=1)
        self.store.change(job, 'Publishing', candidate={'state': 'candidate'})
        with tempfile.TemporaryDirectory() as temporary:
            registry = Path(temporary)
            inbox = registry / 'inbox'
            inbox.mkdir(mode=0o700)
            staged = inbox / ('.stage-' + job)
            staged.mkdir(mode=0o700)
            (staged / '.build-owner.json').write_text(json.dumps({'id': job, 'nonce': row['fs_nonce']}, sort_keys=True))
            (staged / 'kernel').write_bytes(b'kernel')
            backend = object.__new__(Backend)
            backend.config = {'runtime': {'template_registry': {'root': str(registry)}}}
            backend.request = Mock(side_effect=BuildError(404, 'NotFound'))
            self.fault_once('after_staging_delete_fsync')
            with self.assertRaises(PowerLoss):
                backend.discard_unpublished(self.store, self.store.get(job))
            self.assertIsNotNone(self.store.get(job)['staging_cleanup'])
            self.assertFalse(staged.exists())
            self.reopen()
            with patch('tools.template_build_service.stage') as staging:
                Supervisor(self.store, backend).tick()
                staging.assert_not_called()
            self.assertEqual(self.store.get(job)['state'], 'Failed')
            self.assertIsNone(self.store.get(job)['staging_cleanup'])
            self.assertTrue(all(call.args[1] == 'GET' for call in backend.request.call_args_list))
            self.now += 10
            self.store.retire_results()
            self.assertEqual(self.store.rows(), [])

    def handoff_publication(self):
        job = self.submit()
        # The production workspace is exclusively the bounded job inventory;
        # kernel/registry are independent roots, not unexplained workspace files.
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        base = Path(temporary.name).resolve()
        registry = base / 'registry'
        for path in (registry, registry / 'inbox', registry / 'artifacts'):
            path.mkdir(mode=0o700)
        kernel = base / 'kernel'
        kernel.write_bytes(b'kernel')
        disk = self.store.directory(job) / 'candidate.ext4'
        disk.write_bytes(b'opaque disk' + bytes(65536))
        disk.chmod(0o600)
        status = dict(state='candidate', ready=False, startup_applied=False,
            disk_bytes=disk.stat().st_size, min_free_bytes=1,
            image_digest='sha256:' + 'b' * 64, rootfs_sha256=file_hash(disk),
            runtime_defaults=dict(env_vars={}, user='root', workdir='/', argv=[]))
        self.store.private_checkpoint(job, builder_settled={'sandboxID': 'sb-original'}, private_ack=1)
        self.store.change(job, 'Publishing', candidate=status)
        backend = object.__new__(Backend)
        backend.config = {'runtime': {'template_registry': {'root': str(registry)}, 'kernel': str(kernel)}}
        return job, backend, registry, disk

    def test_bounded_handoff_keeps_original_inode_and_unknown_publication_after_restart(self):
        job, backend, registry, disk = self.handoff_publication()
        inode = disk.stat().st_ino
        backend.request = Mock(side_effect=[BuildError(404, 'NotFound'), TimeoutError('lost reply')])
        with self.assertRaises(TimeoutError):
            backend.publish(self.store, self.store.get(job))
        final = registry / 'inbox' / job
        self.assertFalse(disk.exists())
        self.assertEqual((final / 'rootfs.ext4').stat().st_ino, inode)
        ownership = {'id': job, 'nonce': self.store.get(job)['fs_nonce']}
        stage_inventory(final, ownership)
        self.reopen()
        backend.request = Mock(side_effect=BuildError(404, 'NotFound'))
        with patch('tools.template_build_service.stage', side_effect=AssertionError('must not restage')):
            Supervisor(self.store, backend).tick()
        self.assertEqual(self.store.get(job)['state'], 'Publishing')
        self.assertGreater(self.store.get(job)['reserved'], 0)
        self.assertTrue(self.store.get(job)['publication_attempted'])
        self.assertEqual((final / 'rootfs.ext4').stat().st_ino, inode)
        self.assertTrue(all(call.args[1] == 'GET' for call in backend.request.call_args_list))

    def test_bounded_handoff_refusal_deletes_only_committed_inode_inventory(self):
        job, backend, registry, disk = self.handoff_publication()
        backend.request = Mock(side_effect=[BuildError(404, 'NotFound'), BuildError(400, 'Rejected')])
        with self.assertRaisesRegex(BuildError, 'Rejected'):
            backend.publish(self.store, self.store.get(job))
        self.assertFalse(disk.exists())
        self.assertFalse(self.store.get(job)['publication_attempted'])
        final = registry / 'inbox' / job
        unrelated = registry / 'inbox' / ('f' * 32)
        unrelated.mkdir(mode=0o700)
        (unrelated / 'keep').write_text('foreign')
        backend.request = Mock(side_effect=BuildError(404, 'NotFound'))
        self.fault_once('after_staging_delete_intent')
        with self.assertRaises(PowerLoss):
            backend.discard_unpublished(self.store, self.store.get(job))
        self.assertIsNotNone(self.store.get(job)['staging_cleanup'])
        self.reopen()
        old = final / 'kernel'
        replacement = final / 'replacement'
        replacement.write_bytes(b'foreign replacement')
        os.replace(replacement, old)
        with self.assertRaisesRegex(ValueError, 'identity changed'):
            backend.discard_unpublished(self.store, self.store.get(job))
        self.assertEqual(old.read_bytes(), b'foreign replacement')
        self.assertTrue((final / 'rootfs.ext4').exists())
        self.assertIsNotNone(self.store.get(job)['staging_cleanup'])
        self.assertGreater(self.store.get(job)['reserved'], 0)
        self.assertEqual((unrelated / 'keep').read_text(), 'foreign')

    def test_whole_record_bound_rejects_candidate_and_failure_growth(self):
        job = self.submit()
        for values in ({'candidate': {'data': 'x' * 65536}}, {'failure': 'x' * 65536}, {'result': {'x': 'y' * 8192}}):
            with self.subTest(values=list(values)), self.assertRaisesRegex(BuildError, 'TooLarge'):
                self.store.change(job, 'Failed', **values)
        self.assertEqual(self.store.get(job)['state'], 'Queued')

    def test_held_reader_blocks_new_writes_without_early_release(self):
        window = self.window()
        job = self.submit(window=window)
        reader = sqlite3.connect(self.root / 'builds.db')
        self.addCleanup(reader.close)
        reader.execute('BEGIN')
        reader.execute('SELECT * FROM jobs').fetchall()
        self.store.change(job, 'Cleaning')
        self.store.db.execute('PRAGMA busy_timeout=0')
        before = (self.root / 'builds.db-wal').stat().st_size
        with self.assertRaisesRegex(BuildError, 'CheckpointBlocked'):
            self.store.change(job, 'Failed', release=True)
        self.assertGreater(self.store.get(job)['reserved'], 0)
        self.assertEqual((self.root / 'builds.db-wal').stat().st_size, before)
        self.store.free_bytes = lambda: 0
        self.assertEqual(self.submit(window=window), job)
        self.store.free_bytes = lambda: 1000000 * MIB
        self.assertLessEqual(before, self.config['wal_bytes'])
        reader.rollback()
        self.store.change(job, 'Failed', release=True)

    def test_legacy_migration_is_explicit_disjoint_and_pins_unknown_rows(self):
        self.store.close()
        self.store = None
        self.root = self.root / 'legacy'
        self.root.mkdir(mode=0o700)
        legacy = BuildStore(self.root, self.limits, clock=lambda: self.now)
        job = legacy.submit(1, {'kind': 'oci', 'image': 'ubuntu:24.04'}, 'old')['buildID']
        legacy.change(job, 'Building')
        legacy.close()
        with self.assertRaisesRegex(ValueError, 'migration'):
            self.open()
        self.store = self.open(migrate_legacy=True)
        self.assertEqual(self.store.submit(1, {'kind': 'oci', 'image': 'ubuntu:24.04'}, 'old')['buildID'], job)
        with self.assertRaisesRegex(BuildError, 'LegacyBuildDomainClosed'):
            self.store.submit(1, {'kind': 'dockerfile'}, 'new')
        fresh = self.submit(token='old')
        self.assertNotEqual(job, fresh)
        backend = object.__new__(Backend)
        backend.request = Mock()
        self.assertFalse(backend.cleanup_bounded(self.store, self.store.get(job)))
        backend.request.assert_not_called()

    def test_private_fence_before_absence_and_settlement_commit_before_ack(self):
        job = self.submit()
        window = 'bw-private-1-0'
        self.store.private_checkpoint(job, private_window=window)
        backend = object.__new__(Backend)
        calls = []
        def request(pool, method, path, body=None, **kwargs):
            calls.append(path)
            if path.endswith('/close'):
                return {'requestWindow': window, 'closed': True}
            if method == 'GET':
                raise BuildError(404, 'NotFound')
            with sqlite3.connect(self.root / 'builds.db') as db:
                saved = json.loads(db.execute('SELECT builder_settled FROM jobs WHERE id=?', (job,)).fetchone()[0])
            self.assertEqual(saved, {'requestWindow': window, 'sandboxID': None})
            self.assertEqual(body, saved)
            raise TimeoutError('ack lost')
        backend.request = request
        with self.assertRaises(TimeoutError):
            backend.cleanup_bounded(self.store, self.store.get(job))
        self.assertTrue(calls[0].endswith('/close'))
        self.assertIn('?requestWindow=', calls[1])
        self.reopen()
        backend.request = Mock(return_value={'requestWindow': window, 'settled': True})
        self.assertTrue(backend.cleanup_bounded(self.store, self.store.get(job)))
        backend.request.assert_called_once()
        self.assertTrue(backend.request.call_args.args[2].endswith('/settle'))
        self.assertEqual(self.store.get(job)['private_ack'], 1)

    def test_private_delayed_post_is_fenced_before_negative_observation(self):
        job = self.submit()
        window = 'bw-private-1-0'
        self.store.private_checkpoint(job, private_window=window)
        barrier = threading.Barrier(2)
        fenced = threading.Event()
        accepted = []
        def delayed_post():
            barrier.wait(timeout=5)
            fenced.wait(timeout=5)
            if not fenced.is_set():
                accepted.append(job)
        thread = threading.Thread(target=delayed_post)
        thread.start()
        backend = object.__new__(Backend)
        def request(pool, method, path, body=None, **kwargs):
            if path.endswith('/close'):
                barrier.wait(timeout=5)
                fenced.set()
                return {'requestWindow': window, 'closed': True}
            if method == 'GET':
                self.assertTrue(fenced.is_set())
                raise BuildError(404, 'NotFound')
            return {'requestWindow': window, 'settled': True}
        backend.request = request
        self.assertTrue(backend.cleanup_bounded(self.store, self.store.get(job)))
        thread.join(timeout=5)
        self.assertFalse(thread.is_alive())
        self.assertEqual(accepted, [])

    def test_http_client_persists_original_window_key_before_uncertain_post(self):
        server = BuildHTTP(('127.0.0.1', 0), self.store, [{'tenant': 1, 'secret': 'synthetic'}])
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            client = BuildClient('http://127.0.0.1:' + str(server.server_port), 'synthetic')
            request = {'kind': 'oci', 'image': 'ubuntu:24.04', 'timeoutSeconds': 30}
            path = self.root.parent / ('intent-' + self.root.name + '.json')
            self.addCleanup(lambda: path.unlink(missing_ok=True))
            intent = saved_intent(path, client, request, build_mode='windowed', token='original')
            result = client.request('POST', '/sandproof/template-builds', request | {'requestWindow': intent['requestWindow']}, token='original')
            self.store.close_window(1, intent['requestWindow'])
            recovered = saved_intent(path, client, request, build_mode='windowed')
            duplicate = client.request('POST', '/sandproof/template-builds', request | {'requestWindow': recovered['requestWindow']}, token=recovered['idempotency_key'])
            self.assertEqual(result['buildID'], duplicate['buildID'])
            self.assertEqual(intent, recovered)
            with self.assertRaisesRegex(ValueError, 'conflicts'):
                saved_intent(path, client, request, build_mode='legacy-unbounded')
            with self.assertRaisesRegex(RuntimeError, 'LegacyBuildDomainClosed'):
                client.request('POST', '/sandproof/template-builds', request, token='new-legacy')
            self.assertEqual(len(self.store.rows()), 1)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def test_staging_manifest_rejects_unknown_entries_and_resumes_exact_deletion(self):
        path = self.root.parent / ('stage-' + self.root.name)
        path.mkdir(mode=0o700)
        self.addCleanup(lambda: path.exists() and __import__('shutil').rmtree(path))
        ownership = {'id': 'a' * 32, 'nonce': 'b' * 32}
        (path / '.build-owner.json').write_text(json.dumps(ownership, sort_keys=True))
        (path / 'kernel').write_bytes(b'kernel')
        manifest = stage_inventory(path, ownership)
        foreign = path / 'foreign'
        foreign.write_text('keep')
        with self.assertRaisesRegex(ValueError, 'identity changed'):
            stage_delete(path, manifest)
        self.assertEqual(foreign.read_text(), 'keep')
        foreign.unlink()
        (path / 'kernel').unlink()  # Crash after first exact deletion.
        moved = path.rename(path.with_name(path.name + '-moved'))
        path.symlink_to(path.with_name(path.name + '-missing'), target_is_directory=True)
        with self.assertRaises((ValueError, OSError)):
            stage_delete(path, manifest)
        self.assertTrue(moved.is_dir())
        path.unlink()
        moved.rename(path)
        stage_delete(path, manifest)
        self.assertFalse(path.exists())


if __name__ == '__main__':
    unittest.main()
