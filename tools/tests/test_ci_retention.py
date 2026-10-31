import json
from contextlib import ExitStack
import os
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import Mock, patch

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

    def test_same_device_bind_mount_is_refused_before_any_deletion(self):
        path = self.allocate()
        row = f'123 1 0:1 / {path}/mounted rw - ext4 /dev/example rw\n'
        with self.assertRaises(ValueError):
            gc.reject_mounts(path, row)
        with patch.object(gc, 'reject_mounts', side_effect=ValueError('mount remains')):
            with self.assertRaises(ValueError):
                self.store.collect(now=self.now, apply=True)
        self.assertTrue((path / 'generated-disk').exists())
        self.assertEqual(self.store.read(path.name)[2]['state'], 'retired')

    def test_mount_view_decodes_spaces_and_refuses_unknown_observation(self):
        path = Path('/private/path with space')
        with self.assertRaises(ValueError):
            gc.reject_mounts(path, r'123 1 0:1 / /private/path\040with\040space/sub rw - ext4 /dev/x rw')
        gc.reject_mounts(path, '123 1 0:1 / /private rw - ext4 /dev/x rw')
        with self.assertRaises(ValueError):
            gc.reject_mounts(path, 'truncated')

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
            # XFS can store this directory inline and leave a pure hole at
            # zero allocated bytes. Give the byte-budget eviction a real
            # positive allocation while keeping the 1 GiB logical sparse file.
            stream.write(b'x' * 8192)
            stream.flush()
            os.fsync(stream.fileno())
        self.assertEqual((fresh / 'generated-disk').stat().st_size, 1024**3)
        self.assertGreater(gc.allocated_bytes(fresh), 0)
        plan = self.store.collect(now=self.now, max_count=1)
        self.assertEqual([row['id'] for row in plan], [old.name])
        self.assertLess(gc.allocated_bytes(fresh), 1024**3)
        self.store.collect(now=self.now, apply=True, max_bytes=0)
        self.assertFalse(fresh.exists())
        self.assertFalse(old.exists())

    def test_hardlinks_in_one_artifact_are_not_counted_twice(self):
        path = self.allocate()
        size = gc.allocated_bytes(path)
        os.link(path / 'generated-disk', path / 'another-name')
        self.assertEqual(gc.allocated_bytes(path), size)
        self.store.collect(now=self.now, apply=True)
        self.assertFalse(path.exists())

    def test_uncommitted_atomic_scratch_never_retires_active_record(self):
        path = self.allocate(retire=False)
        _, record, value = self.store.read(path.name)
        value.update(state='retired', finished=1, receipt=json.loads(self.receipt.read_text()))
        scratch = record.with_suffix('.tmp')
        scratch.write_text(json.dumps(value))
        scratch.chmod(0o600)
        self.assertEqual(self.store.collect(now=self.now, apply=True), [])
        self.assertTrue(path.exists())
        self.assertEqual(self.store.read(path.name)[2]['state'], 'active')

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

    def test_explicit_current_and_rollback_floor_survives_age_and_byte_pressure(self):
        old, rollback, current = [self.allocate(age=age) for age in (12, 11, 10)]
        plan = self.store.collect(now=self.now, apply=True, keep_days=1,
            max_count=0, max_bytes=0, keep_latest=2)
        self.assertEqual([row['id'] for row in plan], [old.name])
        self.assertFalse(old.exists())
        self.assertTrue(rollback.exists())
        self.assertTrue(current.exists())
        for bad in (-1, True, 1001):
            with self.assertRaises(ValueError):
                self.store.collect(now=self.now, keep_latest=bad)

    def native_fixture(self, number=0):
        work = self.parent / ('native-run-' + str(number))
        work.mkdir(mode=0o700)
        run = gc.NativeBuildRun(work, self.candidate)
        folder = work / 'correct'
        folder.mkdir(mode=0o700)
        target = folder / 'target'
        target.mkdir(mode=0o700)
        (target / 'CACHEDIR.TAG').write_text('Signature: 8a477f597d28d172789f06886806bc55\n')
        (target / '.rustc_info.json').write_text('{}\n')
        (target / 'rebuildable-binary').write_bytes(b'x' * 8192)
        (folder / 'source.rs').write_text('// preserved fixture source\n')
        (folder / 'original.log').write_text('synthetic native fixture, not a proof\n')
        uses = [dict(identity='fixture-command', sha256='b' * 64)]
        (work / 'commands-used.jsonl').write_text(json.dumps(uses[0]) + '\n')
        (work / 'commands-used.jsonl').chmod(0o600)
        evidence = work / 'native-complete.json'
        gc.write_once(evidence, dict(complete=True, exit_code=0, scope_complete=True, uses=uses))
        return run, target, evidence

    def native_store(self):
        root = self.parent / 'native-builds'
        root.mkdir(mode=0o700, exist_ok=True)
        return gc.Store(root)

    def native_handoff(self, run, completion, *, now, observation=None, manifest=None, status='passed'):
        manifest = manifest or run.work / 'build-targets.json'
        if not manifest.exists():
            manifest = run.finish(completion)
        number = getattr(self, 'handoff_count', 0)
        self.handoff_count = number + 1
        evidence = self.parent / ('operator-observation-' + str(number) + '.json')
        # Synthetic trusted-operator observations exercise the handoff contract;
        # these fixtures do not claim a real host-wide process/resource audit.
        report = dict(manifest_sha256=gc.digest(manifest), references_absent=True,
            mounts_absent=True, observation_complete=True, observation_error=None)
        report.update(observation or {})
        gc.write_once(evidence, report)
        receipt = self.parent / ('operator-receipt-' + str(number) + '.json')
        gc.write_once(receipt, dict(candidate=self.candidate, status=status,
            resource_cleanup_confirmed=True, evidence_sha256=gc.digest(evidence)))
        store = self.native_store()
        with store.lock():
            return gc.handoff_native_builds(store, manifest, receipt, evidence, now=now)

    def test_native_handoff_retains_inputs_logs_and_original_completion(self):
        run, target, evidence = self.native_fixture()
        before = evidence.read_bytes()
        manifest = run.finish(evidence)
        self.assertTrue(target.exists())  # Publishing native success never moves/deletes it.
        self.assertFalse((self.parent / 'native-builds').exists())
        payload = self.native_handoff(run, evidence, now=self.now)
        self.assertFalse(target.exists())
        self.assertEqual(evidence.read_bytes(), before)
        self.assertTrue((target.parent / 'source.rs').is_file())
        self.assertTrue((target.parent / 'original.log').is_file())
        store = self.native_store()
        row, = list(store.record_paths())
        payload, _, value = store.read(row.stem)
        self.assertEqual(value['state'], 'retired')
        self.assertTrue((payload / '0/rebuildable-binary').exists())
        original = json.loads(manifest.read_text())
        self.assertFalse(original['product_resource_cleanup_claimed'])
        self.assertEqual(original['completion_sha256'], gc.digest(evidence))

    def test_native_three_successes_collect_only_old_builds_and_keep_every_report(self):
        runs = [self.native_fixture(n) for n in range(3)]
        for n, (run, _, evidence) in enumerate(runs):
            self.native_handoff(run, evidence, now=self.now + n * 2 * gc.DAY)
        store = self.native_store()
        self.assertTrue(all(store.read(path.stem)[2]['state']=='retired' for path in store.record_paths()))
        # Collection remains a separate explicit operator action.
        store.collect(now=self.now + 4 * gc.DAY, apply=True, keep_latest=2, max_count=2)
        states = [store.read(path.stem)[2]['state'] for path in store.record_paths()]
        self.assertEqual(sorted(states), ['removed', 'retired', 'retired'])
        for run, _, evidence in runs:
            self.assertTrue(evidence.exists())
            self.assertTrue((run.work / 'correct/source.rs').exists())
            self.assertTrue((run.work / 'correct/original.log').exists())

    def terminal_fixture(self, number=0, status='failed'):
        run, target, success = self.native_fixture(number)
        # This fixture models a failed run, not a previously sealed success.
        success.unlink()
        record = run.work / 'executor-exit.json'
        gc.write_once(record, dict(kind='native-process-exit-v1', candidate=self.candidate,
            identity=run.identity, status=status, exit_code=1 if status == 'failed' else -15,
            processes_exited=True, observation_complete=True, observation_error=None))
        return run, target, record

    def test_failed_and_cancelled_manual_cli_preserves_status_and_all_original_evidence(self):
        from tools.acceptance_receipts import current_native_seal
        for n, status in enumerate(('failed', 'cancelled')):
            with self.subTest(status=status):
                run, target, record = self.terminal_fixture(n, status)
                before = record.read_bytes()
                store = self.native_store()
                with patch('builtins.print'):
                    gc.main(['--root', str(store.root), 'record-native-terminal',
                        '--work', str(run.work), '--exit-record', str(record)])
                manifest = run.work / 'build-terminal-targets.json'
                value = json.loads(manifest.read_text())
                self.assertEqual(value['kind'], 'native-terminal-cargo-targets-v1')
                self.assertEqual(value['terminal_status'], status)
                self.assertTrue(target.exists())  # Explicit inventory never moves anything.
                self.assertFalse(current_native_seal(value, {}))
                for name in ('native-complete.json', 'complete.json', 'build-targets.json'):
                    self.assertFalse((run.work / name).exists())
                with self.assertRaisesRegex(ValueError, 'cannot become native success'):
                    run.finish(record)
                payload = self.native_handoff(run, record, now=self.now + n,
                    manifest=manifest, status=status)
                store.collect(now=self.now + 9 * gc.DAY, apply=True)
                self.assertEqual(store.read(payload.name)[2]['receipt']['status'], status)
                self.assertFalse(payload.exists())
                self.assertEqual(record.read_bytes(), before)
                self.assertTrue(manifest.exists())
                self.assertTrue((run.work / 'commands-used.jsonl').exists())
                self.assertTrue((target.parent / 'source.rs').exists())
                self.assertTrue((target.parent / 'original.log').exists())

    def test_failure_before_any_target_or_command_is_an_empty_inventory_not_success(self):
        work = self.parent / 'early-failure'
        work.mkdir(mode=0o700)
        run = gc.NativeBuildRun(work, self.candidate)
        record = work / 'executor-exit.json'
        gc.write_once(record, dict(kind='native-process-exit-v1', candidate=self.candidate,
            identity=run.identity, status='failed', exit_code=2, processes_exited=True,
            observation_complete=True, observation_error=None))
        manifest = gc.record_native_terminal(work, record)
        value = json.loads(manifest.read_text())
        self.assertEqual(value['targets'], [])
        self.assertIsNone(value['commands_used_sha256'])
        with self.assertRaisesRegex(ValueError, 'nonempty build list'):
            self.native_handoff(run, record, now=self.now, manifest=manifest, status='failed')
        self.assertEqual(list(self.native_store().record_paths()), [])
        self.assertFalse((work / 'complete.json').exists())

    def test_terminal_exit_requires_exact_status_nonzero_integer_and_complete_observation(self):
        mutations = [('exit_code', 0), ('exit_code', True), ('exit_code', '1'),
            ('exit_code', None), ('status', 'passed'), ('status', 'running'),
            ('processes_exited', False), ('processes_exited', 1),
            ('observation_complete', False), ('observation_error', 'PermissionError'),
            ('candidate', 'b' * 40), ('identity', [0, 0])]
        for n, (key, value) in enumerate(mutations):
            with self.subTest(key=key, value=value):
                run, target, record = self.terminal_fixture(n)
                changed = json.loads(record.read_text())
                changed[key] = value
                record.write_text(json.dumps(changed))
                with self.assertRaises(ValueError):
                    gc.record_native_terminal(run.work, record)
                self.assertTrue(target.exists())
                self.assertFalse((run.work / 'build-terminal-targets.json').exists())

    def test_cancelled_parent_with_live_child_cannot_register_terminal_builds(self):
        run, target, record = self.terminal_fixture(status='cancelled')
        with (target / 'rebuildable-binary').open('rb') as stream:
            child = subprocess.Popen([sys.executable, '-B', '-c',
                "import sys; print('ready',flush=True); sys.stdin.readline()"],
                pass_fds=(stream.fileno(),), stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
            try:
                self.assertEqual(child.stdout.readline().strip(), 'ready')
                changed = json.loads(record.read_text())
                changed['processes_exited'] = False
                record.write_text(json.dumps(changed))
                with self.assertRaisesRegex(ValueError, 'complete child termination'):
                    gc.record_native_terminal(run.work, record)
                self.assertIsNone(child.poll())
                self.assertTrue(target.exists())
            finally:
                child.communicate('\n', timeout=10)

    def test_terminal_handoff_rechecks_original_exit_uses_and_exact_failed_status(self):
        for n, mutation in enumerate(('exit', 'uses', 'receipt', 'unknown')):
            with self.subTest(mutation=mutation):
                run, target, record = self.terminal_fixture(n)
                manifest = gc.record_native_terminal(run.work, record)
                if mutation == 'exit':
                    record.write_text(record.read_text() + '\n')
                elif mutation == 'uses':
                    (run.work / 'commands-used.jsonl').write_text('{}\n')
                with self.assertRaises(ValueError):
                    self.native_handoff(run, record, now=self.now, manifest=manifest,
                        status='passed' if mutation == 'receipt' else 'failed',
                        observation={'references_absent': None} if mutation == 'unknown' else None)
                self.assertTrue(target.exists())
                self.assertEqual(list(self.native_store().record_paths()), [])

    def test_terminal_inventory_refuses_unregistered_success_or_incomplete_cargo_target(self):
        run, target, record = self.terminal_fixture()
        (target / '.rustc_info.json').unlink()
        with self.assertRaises(FileNotFoundError):
            gc.record_native_terminal(run.work, record)
        self.assertFalse((run.work / 'build-terminal-targets.json').exists())
        self.assertTrue(target.exists())
        (run.work / 'build-allocation.json').unlink()
        with self.assertRaises(FileNotFoundError):
            gc.record_native_terminal(run.work, record)
        run, target, success = self.native_fixture(1)
        record = run.work / 'executor-exit.json'
        gc.write_once(record, dict(kind='native-process-exit-v1', candidate=self.candidate,
            identity=run.identity, status='failed', exit_code=1, processes_exited=True,
            observation_complete=True, observation_error=None))
        with self.assertRaisesRegex(ValueError, 'success and failure records'):
            gc.record_native_terminal(run.work, record)
        self.assertTrue(success.exists())

    def test_nonprivate_command_record_cannot_authorize_terminal_inventory_or_handoff(self):
        run, target, record = self.terminal_fixture()
        used = run.work / 'commands-used.jsonl'
        used.chmod(0o644)
        with self.assertRaisesRegex(ValueError, 'owner-only'):
            gc.record_native_terminal(run.work, record)
        used.chmod(0o600)
        manifest = gc.record_native_terminal(run.work, record)
        used.chmod(0o644)
        with self.assertRaisesRegex(ValueError, 'owner-only'):
            self.native_handoff(run, record, now=self.now, manifest=manifest, status='failed')
        self.assertTrue(target.exists())
        self.assertFalse((run.work / 'build-handoff.json').exists())

    def test_native_failure_changed_uses_and_changed_allocation_never_move_builds(self):
        for n, mutation in enumerate(('incomplete', 'failed', 'bool_exit', 'changed_uses', 'allocation')):
            with self.subTest(mutation=mutation):
                run, target, evidence = self.native_fixture(n)
                value = json.loads(evidence.read_text())
                if mutation == 'incomplete': value['complete'] = False
                elif mutation == 'failed': value['exit_code'] = 1
                elif mutation == 'bool_exit': value['exit_code'] = False
                elif mutation == 'changed_uses': value['uses'] = []
                else: run.allocation.write_text('{}\n')
                evidence.write_text(json.dumps(value))
                with self.assertRaises(ValueError):
                    run.finish(evidence)
                self.assertTrue(target.exists())
                self.assertFalse((run.work / 'build-handoff.json').exists())
        old = self.parent / 'old-unregistered'
        old.mkdir(mode=0o700)
        (old / 'original.log').write_text('unfinished\n')
        with self.assertRaises(ValueError):
            gc.NativeBuildRun(old, self.candidate)

    def test_live_child_and_rejected_operator_observation_preserve_builds(self):
        run, target, evidence = self.native_fixture()
        with (target / 'rebuildable-binary').open('rb') as stream:
            child = subprocess.Popen([sys.executable, '-B', '-c',
                "import sys; print('ready',flush=True); sys.stdin.readline()"],
                pass_fds=(stream.fileno(),), stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
            try:
                self.assertEqual(child.stdout.readline().strip(), 'ready')
                stream.close()  # Only the live child's inherited fd retains it.
                # The new default path just inventories targets, even when a
                # nonprivileged host-wide /proc observation is unavailable.
                run.finish(evidence)
                with self.assertRaisesRegex(ValueError, 'unknown references'):
                    self.native_handoff(run, evidence, now=self.now,
                        observation={'references_absent': False})
                self.assertIsNone(child.poll())
                self.assertTrue(target.exists())
            finally:
                child.communicate('\n', timeout=10)
        self.native_handoff(run, evidence, now=self.now)
        self.assertFalse(target.exists())

    def test_unknown_process_observation_symlink_or_mount_never_hands_off(self):
        run, target, evidence = self.native_fixture()
        run.finish(evidence)
        for report in ({'observation_error':'PermissionError: /proc/1494/fd'},
                       {'observation_complete':False}, {'mounts_absent':False},
                       {'references_absent':None}, {'manifest_sha256':'a'*64}):
            with self.subTest(report=report), self.assertRaises(ValueError):
                self.native_handoff(run, evidence, now=self.now, observation=report)
        with patch.object(gc, 'reject_mounts', side_effect=ValueError('live mount')):
            with self.assertRaises(ValueError): self.native_handoff(run, evidence, now=self.now)
        saved = target.with_name('saved-build')
        target.rename(saved)
        target.symlink_to(saved, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.native_handoff(run, evidence, now=self.now)
        self.assertTrue((saved / 'rebuildable-binary').exists())

    def test_manual_same_second_handoff_cannot_guess_latest_and_rollback(self):
        first, _, first_evidence = self.native_fixture(0)
        second, target, second_evidence = self.native_fixture(1)
        self.native_handoff(first, first_evidence, now=self.now)
        with self.assertRaisesRegex(ValueError, 'time must follow'):
            self.native_handoff(second, second_evidence, now=self.now)
        self.assertTrue(target.exists())

    def test_interrupted_native_target_move_remains_active_and_cannot_be_collected(self):
        run, target, evidence = self.native_fixture()
        with patch.object(Path, 'rename', side_effect=OSError('injected handoff interruption')):
            with self.assertRaises(OSError): self.native_handoff(run, evidence, now=self.now)
        store = self.native_store()
        row, = list(store.record_paths())
        self.assertEqual(store.read(row.stem)[2]['state'], 'active')
        self.assertEqual(store.collect(now=self.now + 10 * gc.DAY, apply=True,
            max_count=0, max_bytes=0), [])
        self.assertTrue(target.exists())
        self.assertTrue((run.work / 'build-handoff.json').exists())

    def test_invalid_native_inventory_does_not_publish_outer_success(self):
        from tools import proof_native_suite as suite
        run, _, _ = self.native_fixture()
        output = self.parent / 'native-evidence'
        entry = output / 'commands' / ('a' * 64)
        entry.mkdir(parents=True, mode=0o700)
        gc.write_once(entry / 'receipt.json', {'key': {'fixture': True}})
        used = dict(identity=entry.name, sha256=gc.digest(entry / 'receipt.json'))
        (run.work / 'commands-used.jsonl').write_text(json.dumps(used) + '\n')
        names = ('rust_inputs', 'rules', 'runtime', 'repository_inputs',
                 'contract_inputs', 'source_archive_inputs')
        initial = dict(rust={}, rules={}, runtime={}, repository={}, contracts={}, source_archives={})
        builds = Mock()
        builds.finish.side_effect = ValueError('changed Cargo target identity')
        with ExitStack() as stack:
            for name in names:
                stack.enter_context(patch.object(suite, name, return_value={}))
            stack.enter_context(patch.object(suite, 'trusted_checkout'))
            stack.enter_context(patch.object(suite, 'load_success', return_value={'key': {'fixture': True}}))
            stack.enter_context(patch.object(suite, 'check_native_components'))
            with self.assertRaisesRegex(ValueError, 'changed Cargo target identity'):
                suite.publish_suite(output, run.work, initial, 0, builds=builds)
        self.assertFalse((output / 'complete.json').exists())
        self.assertTrue(json.loads((run.work / 'native-complete.json').read_text())['complete'])


if __name__ == '__main__':
    unittest.main()
