"""Private-journal/probe regressions only; no SDK calls, host mutations or KVM."""
from contextlib import closing
import copy
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch


SOURCE = Path(__file__).resolve().parents[2] / 'tests/system/benchmark_proxy_policy.py'
SPEC = importlib.util.spec_from_file_location('benchmark_proxy_policy_component', SOURCE)
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


class ProbeResultTests(unittest.TestCase):
    def test_only_successful_fixed_results_are_policy_observations(self):
        for output, expected in [('allowed\n', True), ('denied\n', False)]:
            with self.subTest(output=output):
                self.assertIs(probe.probe_result(SimpleNamespace(
                    exit_code=0, stdout=output, stderr='')), expected)

    def test_execution_errors_and_malformed_output_are_not_denial_passes(self):
        results = [(127, '', 'missing executable'),
                   (2, 'denied\n', 'probe-error:socket'),
                   (0, 'denied\n', 'probe-error:poll'),
                   (0, '', ''), (0, 'denied\nallowed\n', ''),
                   (0, 'probe-error:arguments', '')]
        for code, stdout, stderr in results:
            with self.subTest(code=code, stdout=stdout, stderr=stderr):
                with self.assertRaises(RuntimeError):
                    probe.probe_result(SimpleNamespace(
                        exit_code=code, stdout=stdout, stderr=stderr))


class JournalAndSettlementTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='proxy-policy-db-')
        self.addCleanup(self.directory.cleanup)
        self.database = Path(self.directory.name) / 'controller.db'
        self.namespace = 'a' * 16
        self.identity = 'sb-' + self.namespace + '-1'
        self.attempt = 'b' * 32
        self.intents = {self.attempt: {'proxy_policy_probe': self.namespace,
                                     'attempt': self.attempt, 'case': 'public'}}
        self.template = {
            'template_id': 'proxy-policy', 'revision': 'r1', 'owner': 1,
            'kernel': {'path': '/private/kernel', 'sha256': 'c' * 64, 'size_bytes': 10},
            'rootfs': {'path': '/private/rootfs', 'sha256': 'd' * 64, 'size_bytes': 100},
            'resources': {'cpu_count': 1, 'memory_mib': 512,
                          'disk_bytes': 100, 'host_memory_bytes': 768 * 1024**2},
            'envd_version': '0.9.0',
        }
        self.options = {'metadata': copy.deepcopy(self.intents[self.attempt]),
                        'template': copy.deepcopy(self.template)}
        self.now, self.sleeps = 0.0, 0
        with closing(sqlite3.connect(self.database)) as db, db:
            db.executescript('''
                CREATE TABLE sandboxes(sandbox_id TEXT PRIMARY KEY,state_blob BLOB);
                CREATE TABLE sandbox_create_options(sandbox_id TEXT PRIMARY KEY,options_json BLOB);
                CREATE TABLE outbox(id INTEGER PRIMARY KEY,dispatch_state TEXT);
            ''')

    def insert(self, identity=None, *, phase='Starting', owner=1, options=None):
        identity = identity or self.identity
        with closing(sqlite3.connect(self.database)) as db, db:
            db.execute('INSERT OR REPLACE INTO sandboxes VALUES (?,?)',
                       (identity, json.dumps({'owner': owner, 'phase': phase})))
            db.execute('INSERT OR REPLACE INTO sandbox_create_options VALUES (?,?)',
                       (identity, json.dumps(options or self.options)))

    def outbox(self, state):
        with closing(sqlite3.connect(self.database)) as db, db:
            db.execute('INSERT OR REPLACE INTO outbox VALUES(1,?)', (state,))

    def observe(self):
        return probe.journal(self.database, self.intents, self.namespace, self.template)

    def settle(self, delete, *, advance=None, dispatched=None):
        def tick(seconds):
            self.now += seconds
            self.sleeps += 1
            if advance is not None:
                advance(self.sleeps)
        with patch.object(probe.time, 'monotonic', side_effect=lambda: self.now), \
                patch.object(probe.time, 'sleep', side_effect=tick):
            return probe.settle(self.database, self.intents, self.namespace,
                                self.template, delete, deadline=1, dispatched=dispatched)

    def test_starting_is_visible_and_reader_releases_its_snapshot(self):
        self.insert()
        found, pending = self.observe()
        self.assertEqual(found, {self.attempt: {'id': self.identity, 'phase': 'Starting'}})
        self.assertEqual(pending, 0)
        with closing(sqlite3.connect(self.database)) as db:
            self.assertEqual(db.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()[0], 0)

    def test_owner_template_identity_and_full_correlation_are_exact(self):
        variants = []
        for field, value in [('template_id', 'other'), ('revision', 'r2'), ('owner', 2)]:
            options = copy.deepcopy(self.options)
            options['template'][field] = value
            variants.append((self.identity, 1, options))
        for section, field, value in [('kernel', 'sha256', 'e' * 64),
                                      ('rootfs', 'path', '/replacement'),
                                      ('resources', 'memory_mib', 1024)]:
            options = copy.deepcopy(self.options)
            options['template'][section][field] = value
            variants.append((self.identity, 1, options))
        for field, value in [('attempt', 'f' * 32), ('case', 'closed'),
                              ('proxy_policy_probe', '0' * 16), ('extra', 'new')]:
            options = copy.deepcopy(self.options)
            options['metadata'][field] = value
            variants.append((self.identity, 1, options))
        variants.extend([(self.identity, 2, self.options),
                         ('sb-' + '0' * 16 + '-1', 1, self.options),
                         ('sb-' + self.namespace + '-01', 1, self.options)])
        for identity, owner, options in variants:
            with self.subTest(identity=identity, owner=owner, options=options):
                with closing(sqlite3.connect(self.database)) as db, db:
                    db.execute('DELETE FROM sandboxes')
                    db.execute('DELETE FROM sandbox_create_options')
                self.insert(identity, owner=owner, options=options)
                with self.assertRaises(RuntimeError):
                    self.observe()

    def test_two_sandboxes_cannot_claim_one_create_correlation(self):
        self.insert()
        self.insert('sb-' + self.namespace + '-2')
        with self.assertRaises(RuntimeError):
            self.observe()

    def test_missing_options_or_orphan_options_are_not_silently_joined_away(self):
        self.insert(phase='Destroyed')
        for missing in ('options', 'sandbox'):
            with self.subTest(missing=missing):
                identity = 'sb-' + self.namespace + '-2'
                with closing(sqlite3.connect(self.database)) as db, db:
                    if missing == 'options':
                        db.execute('INSERT INTO sandboxes VALUES (?,?)',
                                   (identity, json.dumps({'owner': 1, 'phase': 'Starting'})))
                    else:
                        db.execute('INSERT INTO sandbox_create_options VALUES (?,?)',
                                   (identity, json.dumps(self.options)))
                with self.assertRaises(RuntimeError):
                    self.observe()
                with closing(sqlite3.connect(self.database)) as db, db:
                    db.execute('DELETE FROM sandboxes WHERE sandbox_id=?', (identity,))
                    db.execute('DELETE FROM sandbox_create_options WHERE sandbox_id=?', (identity,))

    def test_both_ready_and_dispatched_work_prevent_cleanup_confirmation(self):
        self.insert(phase='Destroyed')
        for state in ('ready', 'dispatched'):
            with self.subTest(state=state):
                self.now = self.sleeps = 0
                self.outbox(state)
                self.assertEqual(self.observe()[1], 1)
                delete = Mock()
                with self.assertRaises(TimeoutError):
                    self.settle(delete)
                delete.assert_not_called()
        self.outbox('receipted')
        self.assertEqual(self.observe()[1], 0)

    def test_starting_and_absent_creates_remain_unknown_without_delete(self):
        for present in (False, True):
            with self.subTest(present=present):
                self.now = self.sleeps = 0
                if present:
                    self.insert()
                delete = Mock()
                with self.assertRaises(TimeoutError):
                    self.settle(delete)
                delete.assert_not_called()

    def test_late_create_and_late_destruction_are_observed_not_recreated(self):
        delete = Mock()
        def advance(tick):
            if tick == 1:
                self.insert()
            elif tick == 2:
                delete.assert_not_called()
                self.insert(phase='Running')
            elif tick == 3:
                self.insert(phase='Destroyed')
        found = self.settle(delete, advance=advance)
        self.assertEqual(found[self.attempt]['phase'], 'Destroyed')
        delete.assert_called_once_with(self.identity)

    def test_404_or_ack_never_proves_destroyed_by_itself(self):
        self.insert(phase='Running')
        for receipt in (None, 204, 404):
            with self.subTest(receipt=receipt):
                self.now = self.sleeps = 0
                delete = Mock(return_value=receipt)
                with self.assertRaises(TimeoutError):
                    self.settle(delete)
                delete.assert_called_once_with(self.identity)

    def test_lost_delete_reply_is_observed_without_replaying_effect(self):
        self.insert(phase='Running')
        def lost_reply(_identity):
            self.insert(phase='Destroyed')
            raise OSError('transport receipt lost')
        delete = Mock(side_effect=lost_reply)
        found = self.settle(delete)
        self.assertEqual(found[self.attempt]['phase'], 'Destroyed')
        delete.assert_called_once_with(self.identity)
        delete.reset_mock()
        self.assertEqual(self.settle(delete), found)
        delete.assert_not_called()

    def test_lost_reply_and_timeout_preserve_dispatch_memory_across_cleanup_calls(self):
        self.insert(phase='Running')
        dispatched = set()
        delete = Mock(side_effect=OSError('receipt lost before tombstone is visible'))
        with self.assertRaises(TimeoutError):
            self.settle(delete, dispatched=dispatched)
        delete.assert_called_once_with(self.identity)
        self.assertEqual(dispatched, {self.identity})
        self.now = self.sleeps = 0

        def late_tombstone(tick):
            if tick == 2:
                self.insert(phase='Destroyed')

        found = self.settle(delete, dispatched=dispatched, advance=late_tombstone)
        self.assertEqual(found[self.attempt]['phase'], 'Destroyed')
        delete.assert_called_once_with(self.identity)
        self.assertEqual(dispatched, {self.identity})

    def test_all_correlated_creates_must_settle_before_return(self):
        self.insert(phase='Destroyed')
        other = 'e' * 32
        self.intents[other] = self.intents[self.attempt] | {'attempt': other, 'case': 'closed'}
        options = self.options | {'metadata': self.intents[other]}
        self.insert('sb-' + self.namespace + '-2', options=options)
        delete = Mock()
        with self.assertRaises(TimeoutError):
            self.settle(delete)
        delete.assert_not_called()

    def test_missing_database_does_not_create_an_empty_successful_journal(self):
        missing = Path(self.directory.name) / 'missing.db'
        with self.assertRaises(sqlite3.OperationalError):
            probe.journal(missing, self.intents, self.namespace, self.template)
        self.assertFalse(missing.exists())


class OptimizedInterpreterTests(unittest.TestCase):
    def test_optimized_entry_refuses_before_sdk_or_platform_effects(self):
        for mode in ('-O', '-OO', '1', '2'):
            with self.subTest(mode=mode):
                env = {key: value for key, value in os.environ.items()
                       if key not in ('PYTHONOPTIMIZE', 'PYTHONPATH')}
                env['PYTHONDONTWRITEBYTECODE'] = '1'
                flags = [mode] if mode.startswith('-') else []
                if not flags:
                    env['PYTHONOPTIMIZE'] = mode
                result = subprocess.run([sys.executable, *flags, str(SOURCE), '--help'],
                                        env=env, capture_output=True, text=True, timeout=10)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('optimized_python', result.stderr)
                self.assertNotIn('ModuleNotFoundError', result.stderr)


if __name__ == '__main__':
    unittest.main()
