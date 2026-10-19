"""Engineering inventories only; no Redis, rewrite, product VM or hard quota."""
import copy
import ast
import json
import os
from pathlib import Path
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
from tools import service_storage_observation as storage


class ServiceStorageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.root.chmod(0o700)
        self.record = dict(root=str(self.root), owner_uid=os.geteuid(),
                           peak_bytes=1024, inode_budget=12)

    def write(self, name, content):
        path = self.root / name
        path.write_bytes(content)
        path.chmod(0o600)
        return path

    def test_aof_base_incremental_and_rewrite_temp_are_all_charged_before_replacement(self):
        sampler = storage.ServiceStoragePeaks([self.record])
        self.write('appendonly.aof.1.base.rdb', b'a' * 11)
        self.write('appendonly.aof.1.incr.aof', b'b' * 13)
        temporary = self.write('temp-rewriteaof-123.aof', b'c' * 17)
        report = sampler.report()
        self.assertEqual(report['roots'][0]['observed_peak']['logical_file_bytes'], 41)
        self.assertEqual(report['roots'][0]['observed_peak']['entries'], 4)
        temporary.unlink()
        self.assertEqual(sampler.report()['roots'][0]['last_stable']['logical_file_bytes'], 24)
        self.assertEqual(sampler.report()['roots'][0]['observed_peak']['logical_file_bytes'], 41)
        self.assertEqual(self.record['peak_bytes'], 1024)
        self.assertEqual(report['observation'], 'sampled_pathname_peak_not_hard_quota')
        self.assertNotIn(str(self.root), str(report))

    def test_logs_across_restarts_share_one_original_peak_and_logical_budget(self):
        path = self.write('host-0.log', b'first')
        sampler = storage.ServiceStoragePeaks([self.record])
        self.assertEqual(sampler.report()['roots'][0]['last_stable']['logical_file_bytes'], 5)
        with path.open('ab') as output:
            output.write(b'-restart')
        self.assertEqual(sampler.report()['roots'][0]['observed_peak']['logical_file_bytes'], 13)
        path.write_bytes(b'')
        self.assertEqual(sampler.report()['roots'][0]['observed_peak']['logical_file_bytes'], 13)

    def test_over_budget_is_latched_and_cannot_be_erased_by_shrinking_or_deleting(self):
        sampler = storage.ServiceStoragePeaks([self.record])
        path = self.write('redis.log', b'x' * 1025)
        with self.assertRaises(ValueError):
            sampler.sample()
        path.unlink()
        sampler.stop()
        with self.assertRaises(storage.ObservationUnknown):
            sampler.report()
        with patch.object(storage, 'inventory', side_effect=AssertionError('no new scan')):
            diagnostic = sampler.diagnostics()
        self.assertEqual(diagnostic['failure'], dict(kind='inventory_rejected', root_index=0,
                                                    reason='bytes_exceeded'))
        self.assertTrue(diagnostic['sampler_exited'])
        self.assertNotIn(str(self.root), json.dumps(diagnostic))
        with self.assertRaises(ValueError):
            storage.require_storage_observation(diagnostic, expected=sampler.declaration())
        diagnostic['failure']['reason'] = 'changed'
        self.assertEqual(sampler.diagnostics()['failure']['reason'], 'bytes_exceeded')

    def test_directory_change_is_unknown_not_empty_and_a_later_stable_sample_recovers(self):
        self.write('original', b'old')
        sampler = storage.ServiceStoragePeaks([self.record])
        sampler.sample()
        original = storage.os.scandir

        class Changed:
            def __init__(inner, fd):
                inner.iterator = original(fd)
            def __enter__(inner):
                return inner
            def __iter__(inner):
                yield from inner.iterator
                before = self.root.stat()
                self.write('arrived-after-enumeration', b'new')
                # Deterministically change the original stamp, including on
                # filesystems where two writes can share a timestamp tick.
                # This test does not claim an atomic pathname snapshot.
                os.utime(self.root, ns=(before.st_atime_ns, before.st_mtime_ns + 1_000_000_000))
            def __exit__(inner, *_):
                inner.iterator.close()

        with patch.object(storage.os, 'scandir', Changed):
            self.assertFalse(sampler.sample())
        self.assertIsNone(sampler.latest)
        self.assertEqual(sampler.peaks[0]['logical_file_bytes'], 3)
        self.assertEqual(sampler.unknown_samples, 1)
        self.assertEqual(sampler.report()['roots'][0]['last_stable']['logical_file_bytes'], 6)

    def test_no_successful_final_observation_cannot_seal_a_previous_good_sample(self):
        self.write('original', b'content')
        sampler = storage.ServiceStoragePeaks([self.record])
        sampler.sample()
        with patch.object(storage, 'inventory', side_effect=storage.ObservationUnknown):
            self.assertFalse(sampler.sample())
        sampler.closed = True
        with self.assertRaises(storage.ObservationUnknown):
            sampler.report()

    def test_unlinked_open_file_during_replacement_is_unknown_not_a_permanent_alias_fault(self):
        self.write('pending', b'old')
        sampler = storage.ServiceStoragePeaks([self.record])
        sampler.sample()
        original = storage.os.open
        replaced = False

        def open_then_replace(path, flags, *args, **kwargs):
            nonlocal replaced
            fd = original(path, flags, *args, **kwargs)
            if path == 'pending' and not replaced:
                replaced = True
                (self.root/'pending').unlink()
                self.write('pending', b'new-content')
                self.assertEqual(os.fstat(fd).st_nlink, 0)
            return fd

        with patch.object(storage.os, 'open', side_effect=open_then_replace):
            self.assertFalse(sampler.sample())
        self.assertTrue(replaced)
        self.assertEqual(sampler.unknown_samples, 1)
        self.assertEqual(sampler.peaks[0]['logical_file_bytes'], 3)
        self.assertIsNone(sampler.failure)
        self.assertIsNone(sampler.latest)
        sampler.stop()
        self.assertEqual(sampler.report()['roots'][0]['last_stable']['logical_file_bytes'], 11)

    def test_atomic_rename_between_opened_names_is_unknown_and_does_not_latch_alias_failure(self):
        pending = self.write('a-pending', b'new-content')
        published = self.write('z-published', b'old')
        original_inode = pending.stat().st_ino
        sampler = storage.ServiceStoragePeaks([self.record])
        self.assertTrue(sampler.sample())
        original_scan, renamed = storage.os.scandir, False

        class OrderedScan:
            def __init__(inner, fd):
                inner.iterator = original_scan(fd)
            def __enter__(inner):
                return inner
            def __iter__(inner):
                nonlocal renamed
                for entry in sorted(inner.iterator, key=lambda entry: entry.name):
                    yield entry
                    # The consumer has checked and queued the first actual
                    # descriptor before it opens the second enumerated name.
                    if entry.name == pending.name and not renamed:
                        renamed = True
                        pending.replace(published)
                        self.assertEqual(published.stat().st_ino, original_inode)
                        self.assertEqual(published.stat().st_nlink, 1)
            def __exit__(inner, *_):
                inner.iterator.close()

        with patch.object(storage.os, 'scandir', OrderedScan):
            self.assertFalse(sampler.sample())
        self.assertTrue(renamed)
        self.assertIsNone(sampler.failure)
        self.assertIsNone(sampler.latest)
        self.assertEqual(sampler.unknown_samples, 1)
        self.assertEqual(sampler.peaks[0]['logical_file_bytes'], 14)
        sampler.stop()
        final = storage.require_storage_observation(sampler.report(), expected=sampler.declaration())
        self.assertEqual(final['roots'][0]['last_stable']['logical_file_bytes'], 11)
        self.assertEqual(final['roots'][0]['observed_peak']['logical_file_bytes'], 14)
        self.assertTrue(final['sampler_exited'])

    def test_unknown_peer_does_not_erase_an_independently_observed_original_root_peak(self):
        peer = tempfile.TemporaryDirectory()
        self.addCleanup(peer.cleanup)
        peer_root = Path(peer.name).resolve()
        peer_root.chmod(0o700)
        sampler = storage.ServiceStoragePeaks([self.record, self.record | {'root': str(peer_root)}])
        self.write('redis-temp-rewrite', b'rewrite')
        original = storage.inventory

        def observe(record, *args):
            if record['root'] == str(peer_root):
                raise storage.ObservationUnknown('peer directory changing')
            return original(record, *args)

        with patch.object(storage, 'inventory', side_effect=observe):
            self.assertFalse(sampler.sample())
        self.assertIsNone(sampler.latest)
        (self.root/'redis-temp-rewrite').unlink()
        sampler.stop()
        final = storage.require_storage_observation(sampler.report(), expected=sampler.declaration())
        self.assertEqual(final['roots'][0]['observed_peak']['logical_file_bytes'], 7)
        self.assertEqual(final['roots'][0]['last_stable']['logical_file_bytes'], 0)
        self.assertEqual(final['unknown_samples'], 1)

    def test_changed_root_identity_is_not_adopted_even_when_empty_and_within_budget(self):
        sampler = storage.ServiceStoragePeaks([self.record])
        old = self.root / 'previous-root'
        moved = self.root.with_name(self.root.name + '-moved')
        self.root.rename(moved)
        self.root.mkdir(mode=0o700)
        self.addCleanup(lambda: moved.rename(old))
        with self.assertRaises(ValueError):
            sampler.sample()

    def test_missing_original_root_does_not_seal_previous_success(self):
        sampler = storage.ServiceStoragePeaks([self.record])
        sampler.sample()
        self.root.rmdir()
        self.addCleanup(lambda: self.root.mkdir(mode=0o700))
        self.assertFalse(sampler.sample())
        sampler.closed = True
        with self.assertRaises(storage.ObservationUnknown):
            sampler.report()

    def test_final_summary_survives_json_roundtrip_without_scanning_removed_roots(self):
        self.write('appendonly.aof', b'original-bytes')
        sampler = storage.ServiceStoragePeaks([self.record])
        sampler.stop()
        original = sampler.report()
        persisted = json.loads(json.dumps(original))
        with patch.object(storage, 'inventory', side_effect=AssertionError('no report IO')):
            checked = storage.require_storage_observation(persisted, expected=sampler.declaration())
        self.assertEqual(checked, original)
        checked['roots'][0]['budget']['entries'] += 1
        self.assertNotEqual(checked, persisted)

    def test_persisted_missing_unknown_unbounded_and_identity_evidence_rejects(self):
        sampler = storage.ServiceStoragePeaks([self.record])
        sampler.stop()
        report = sampler.report()
        changes = [(('sampler_exited',), False), (('successful_samples',), 0),
                   (('successful_samples',), True), (('unknown_samples',), storage.U64_MAX + 1),
                   (('roots',), []), (('roots',), report['roots'] * 9),
                   (('observation',), 'hard_quota'), (('limitations',), 'bounded disk'),
                   (('sample_interval_ms',), 0), (('roots', 0, 'identity'), [1, 0]),
                   (('roots', 0, 'index'), 1), (('roots', 0, 'owner_uid'), True),
                   (('roots', 0, 'budget', 'entries'), 0),
                   (('roots', 0, 'observed_peak', 'logical_file_bytes'), 1025),
                   (('roots', 0, 'last_stable', 'entries'), 13),
                   (('roots', 0, 'last_stable', 'logical_file_bytes'), 1)]
        for path, value in changes:
            changed = json.loads(json.dumps(report))
            parent = changed
            for key in path[:-1]:
                parent = parent[key]
            parent[path[-1]] = value
            with self.subTest(path=path), self.assertRaises(ValueError):
                storage.require_storage_observation(changed, expected=sampler.declaration())
        for missing in report:
            changed = copy.deepcopy(report)
            del changed[missing]
            with self.subTest(missing=missing), self.assertRaises(ValueError):
                storage.require_storage_observation(changed, expected=sampler.declaration())
        for changed in (None, dict(report, private_path=str(self.root))):
            with self.assertRaises(ValueError):
                storage.require_storage_observation(changed, expected=sampler.declaration())
        for foreign in (False, True):
            changed = copy.deepcopy(report)
            peer = copy.deepcopy(changed['roots'][0])
            peer['index'] = 1
            if foreign:
                peer['identity'][0] += 1
                peer['identity'][1] += 1
            changed['roots'].append(peer)
            with self.assertRaises(ValueError):
                storage.require_storage_observation(changed, expected=sampler.declaration())

    def test_summary_cannot_replace_original_receipt_root_owner_or_budget(self):
        sampler = storage.ServiceStoragePeaks([self.record])
        sampler.stop()
        expected = json.loads(json.dumps(sampler.declaration()))
        self.assertEqual(storage.require_storage_observation(sampler.report(), expected=expected), sampler.report())
        for key, value in (('identity', [expected[0]['identity'][0], expected[0]['identity'][1] + 1]),
                           ('owner_uid', self.record['owner_uid'] + 1),
                           ('budget', dict(logical_file_bytes=1025, entries=12))):
            report = sampler.report()
            report['roots'][0][key] = value
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'original execution declaration'):
                storage.require_storage_observation(report, expected=expected)
        # The first observation remains fixed; caller mutation cannot retarget
        # future execution receipts, and no paths or file content are exported.
        copied = sampler.declaration()
        copied[0]['identity'][1] += 1
        self.assertEqual(sampler.declaration(), expected)
        self.assertNotIn(str(self.root), json.dumps(expected))

    def test_original_declaration_is_bounded_exact_and_independent_of_report_io(self):
        sampler = storage.ServiceStoragePeaks([self.record])
        original = sampler.declaration()
        with patch.object(storage.os, 'lstat', side_effect=AssertionError('no receipt IO')):
            self.assertEqual(storage.require_storage_declaration(original), original)
        invalid = [None, [], original * 9, [dict(original[0], root=str(self.root))],
                   [dict(original[0], index=True)], [dict(original[0], owner_uid=True)],
                   [dict(original[0], identity=[original[0]['identity'][0], 0])],
                   [dict(original[0], budget=dict(logical_file_bytes=0, entries=12))]]
        for value in invalid:
            with self.subTest(value=value), self.assertRaises(ValueError):
                storage.require_storage_declaration(value)

    def test_unexpected_background_observation_failure_cannot_seal_old_sample(self):
        sampler = storage.ServiceStoragePeaks([self.record])
        sampler.sample()
        with patch.object(storage, 'inventory', side_effect=RuntimeError('unavailable')):
            sampler.thread.start()
            sampler.thread.join(2)
            self.assertFalse(sampler.thread.is_alive())
            sampler.stop()
        with self.assertRaises(storage.ObservationUnknown):
            sampler.report()
        self.assertEqual(sampler.diagnostics()['failure'],
            dict(kind='background_observation_failed', error_type='RuntimeError'))
        self.assertNotIn('unavailable', json.dumps(sampler.diagnostics()))

    def test_unexpected_inventory_error_text_is_not_exported_as_a_diagnostic(self):
        sampler = storage.ServiceStoragePeaks([self.record])
        with patch.object(storage, 'inventory', side_effect=ValueError('private path and credential')):
            with self.assertRaises(ValueError): sampler.sample()
        self.assertEqual(sampler.diagnostics()['failure'],
            dict(kind='inventory_rejected', root_index=0, reason='unexpected_value_error'))
        self.assertNotIn('credential', json.dumps(sampler.diagnostics()))

    def test_symlink_hardlink_fifo_and_foreign_owner_are_never_counted_as_safe_files(self):
        path = self.write('regular', b'x')
        for kind in ('symlink', 'hardlink', 'fifo'):
            alias = self.root / kind
            if kind == 'symlink':
                alias.symlink_to(path)
            elif kind == 'hardlink':
                os.link(path, alias)
            else:
                os.mkfifo(alias, 0o600)
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                storage.ServiceStoragePeaks([self.record]).sample()
            alias.unlink()
        wrong = copy.deepcopy(self.record)
        wrong['owner_uid'] = os.geteuid() + 1
        with self.assertRaises(ValueError):
            storage.ServiceStoragePeaks([wrong])

    def test_original_inode_budget_includes_directories_and_sparse_files_count_logical_bytes(self):
        sub = self.root / 'sub'
        sub.mkdir(mode=0o700)
        path = sub / 'sparse'
        with path.open('wb') as output:
            output.truncate(1024)
        path.chmod(0o600)
        record = self.record | {'inode_budget': 3}
        observed = storage.ServiceStoragePeaks([record]).report()['roots'][0]['last_stable']
        self.assertEqual(observed['entries'], 3)
        self.assertEqual(observed['logical_file_bytes'], 1024)
        with self.assertRaises(ValueError):
            storage.ServiceStoragePeaks([record | {'inode_budget': 2}]).sample()
        with self.assertRaises(ValueError):
            storage.ServiceStoragePeaks([record | {'peak_bytes': 1023}]).sample()

    def test_inventory_is_stat_only_without_hashing_or_reading_file_contents(self):
        self.write('test-only-secret', b'never-serialize-this')
        sampler = storage.ServiceStoragePeaks([self.record])
        with patch('builtins.open', side_effect=AssertionError('no content reads')):
            report = sampler.report()
        self.assertNotIn('never-serialize-this', str(report))
        self.assertNotIn('test-only-secret', str(report))
        self.assertGreater(report['roots'][0]['last_stable']['allocated_bytes'], 0)

    def test_sampler_spans_original_lifetime_and_stops_before_staging_removal(self):
        sampler = storage.ServiceStoragePeaks([self.record])
        original, observed = storage.inventory, threading.Event()
        calls = 0

        def capture(*args):
            nonlocal calls
            result = original(*args)
            calls += 1
            if calls >= 2:
                observed.set()
            return result

        with patch.object(storage, 'inventory', side_effect=capture):
            sampler.start()
            self.assertTrue(observed.wait(2), 'original background sampler did not execute')
            sampler.stop()
        self.assertFalse(sampler.thread.is_alive())
        report = sampler.report()
        self.assertTrue(report['sampler_exited'])
        with patch.object(storage, 'inventory', side_effect=AssertionError('removed roots never rescanned')):
            self.assertEqual(sampler.report(), report)
        with self.assertRaises(ValueError):
            sampler.start()

    def test_deadline_and_unknown_os_observation_are_not_budget_success(self):
        sampler = storage.ServiceStoragePeaks([self.record])
        with self.assertRaises(storage.ObservationUnknown):
            storage.inventory(self.record, sampler.identities[0], time.monotonic() - 1)
        with patch.object(storage.os, 'open', side_effect=PermissionError):
            self.assertFalse(sampler.sample())
        self.assertIsNone(sampler.latest)
        sampler.closed = True
        with self.assertRaises(storage.ObservationUnknown):
            sampler.report()

    def test_missing_excessive_or_overlapping_original_declarations_are_rejected(self):
        for records in ([], [self.record] * 9, [self.record, self.record],
                        [self.record | {'root': 'relative'}],
                        [self.record | {'peak_bytes': True}], [self.record | {'inode_budget': 0}],
                        [self.record | {'owner_uid': True}], [self.record | {'extra': 1}]):
            with self.subTest(records=records), self.assertRaises(ValueError):
                storage.ServiceStoragePeaks(records)
        nested = self.root / 'nested'
        nested.mkdir(mode=0o700)
        with self.assertRaises(ValueError):
            storage.ServiceStoragePeaks([self.record, self.record | {'root': str(nested)}])

    def test_two_question_and_churn_reports_use_the_same_original_sampler(self):
        def calls(node, method):
            return [item for item in ast.walk(node) if isinstance(item, ast.Call)
                    and isinstance(item.func, ast.Attribute) and item.func.attr == method]
        for filename in ('harbor_generic_chain.py', 'server_bounded_churn.py'):
            tree = ast.parse((ROOT / 'tests/system' / filename).read_text())
            with self.subTest(filename=filename):
                self.assertTrue(calls(tree, 'storage_peaks'))
                self.assertFalse(any(isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                                     and node.func.id == 'ServiceStoragePeaks' for node in ast.walk(tree)))
        tree = ast.parse((ROOT / 'tests/system/harbor_generic_chain.py').read_text())
        finish = next(item for item in ast.walk(tree) if isinstance(item, ast.Try)
                      and any(calls(statement, 'cleanup_staging') for statement in item.body))
        stop_index = next(index for index, statement in enumerate(finish.body) if calls(statement, 'stop_all'))
        sample_index = next(index for index, statement in enumerate(finish.body) if calls(statement, 'storage_peaks'))
        remove_index = next(index for index, statement in enumerate(finish.body) if calls(statement, 'cleanup_staging'))
        self.assertLess(stop_index, sample_index)
        self.assertLess(sample_index, remove_index)


if __name__ == '__main__':
    unittest.main()
