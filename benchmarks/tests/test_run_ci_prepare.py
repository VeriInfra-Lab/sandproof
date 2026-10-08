"""One-time preparation orchestration; not actual build or benchmark evidence."""
import asyncio
import json
import os
from pathlib import Path
import stat
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

from benchmarks import run_ci_prepare as prepare
from tools.benchmark_tasks import InvalidInput


class CiPreparationRunTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.names = ('polyglot-c-py', 'db-wal-recovery')
        self.output = self.root / 'run'
        self.output.mkdir(mode=0o700)
        self.destination = self.root / 'fixtures'
        self.inputs = self.root / 'inputs.json'
        self.inputs.write_text(json.dumps({name: 'sha256:' + 'd' * 64 for name in self.names}))
        self.args = SimpleNamespace(task=list(self.names), all=False, source_kind='dockerfile',
            oracle_mode='original', ci_build_inputs=self.inputs, prepare_ci_fixtures=self.destination,
            source=self.root / 'source', output=self.output, artifact_limits=None,
            ready_artifacts_root=self.root / 'registry/artifacts', kernel=self.root / 'kernel',
            build_url='http://127.0.0.1:1', ci_uv=self.root / 'uv')
        self.lock = {'dataset': 'terminal-bench@2.0', 'tasks': [dict(name=name, source_path=name) for name in self.names]}
        self.envs = [SimpleNamespace(prepare_template=AsyncMock(), build_id=value * 32,
            build_dispatched=True, build_result={'state': 'Ready'},
            template={'templateId': 'tpl-' + value * 32}) for value in ('a', 'b')]
        self.events = []
        def publish(entry, *args):
            self.events.append(('publish', entry['name']))
            return {'name': entry['name']}
        def audit(row):
            self.events.append(('audit', row['name']))
            return {'exact_cleanup': 'passed'}
        self.args.completion_audit = Mock(side_effect=audit)
        self.publish = Mock(side_effect=publish)
        self.make_manifest = Mock(return_value={'schema_version': 1, 'fixtures': [{'name': name} for name in self.names]})
        self.construct = Mock(side_effect=self.envs)
        task = SimpleNamespace(paths=SimpleNamespace(environment_dir=self.root / 'source/environment'),
                               config=SimpleNamespace(environment=SimpleNamespace()))
        self.patches = (
            patch.object(prepare, 'Task', return_value=task),
            patch.object(prepare, 'GenericEnvironment', self.construct),
            patch.object(prepare, 'publish_ready_fixture', self.publish),
            patch.object(prepare, 'make_ci_fixture_manifest', self.make_manifest),
            patch.dict(os.environ, {'SANDPROOF_TEST_API_KEY': 'synthetic'}),
        )
        for manager in self.patches:
            manager.start()
            self.addCleanup(manager.stop)

    def test_manifest_commit_requires_both_ready_items_and_both_exact_audits(self):
        def envelope(items, lock):
            self.assertEqual(self.events, [('publish', self.names[0]), ('audit', self.names[0]),
                                           ('publish', self.names[1]), ('audit', self.names[1])])
            self.assertEqual(items, [{'name': name} for name in self.names])
            self.assertFalse((self.destination / 'manifest.json').exists())
            return {'schema_version': 1, 'fixtures': items}
        self.make_manifest.side_effect = envelope
        rows = asyncio.run(prepare.prepare_selected(self.args, self.lock))
        self.assertEqual(len(rows), 2)
        self.assertTrue(all(row['status'] == 'prepared' and row['exact_cleanup'] == 'passed' for row in rows))
        self.make_manifest.assert_called_once()
        manifest = self.destination / 'manifest.json'
        self.assertEqual(stat.S_IMODE(manifest.stat().st_mode), 0o400)
        for index, env in enumerate(self.envs):
            env.prepare_template.assert_awaited_once()
            self.assertEqual(self.publish.call_args_list[index].args[2],
                             self.args.ready_artifacts_root / env.build_id / 'rootfs.ext4')
        report = json.loads((self.output / 'full-set.json').read_bytes())
        for field in ('benchmark_passed', 'original_oracle', 'original_verifier', 'original_environment'):
            self.assertIs(report[field], False)
        self.assertEqual(report['scope'], 'fixture preparation only')

    def test_build_failure_retains_identity_audits_once_and_stops(self):
        self.envs[0].prepare_template.side_effect = TimeoutError('uncertain build receipt')
        rows = asyncio.run(prepare.prepare_selected(self.args, self.lock))
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]['error_type'], 'TimeoutError')
        self.assertEqual(rows[0]['build_id'], self.envs[0].build_id)
        self.assertEqual(rows[0]['exact_cleanup'], 'passed')
        self.args.completion_audit.assert_called_once()
        self.envs[0].prepare_template.assert_awaited_once()
        self.envs[1].prepare_template.assert_not_awaited()
        self.publish.assert_not_called()
        self.make_manifest.assert_not_called()
        self.assertFalse((self.destination / 'manifest.json').exists())

    def test_second_failure_cannot_publish_single_item_manifest(self):
        self.envs[1].prepare_template.side_effect = ValueError('build failed')
        rows = asyncio.run(prepare.prepare_selected(self.args, self.lock))
        self.assertEqual(len(rows), 2)
        self.assertEqual([row['status'] for row in rows], ['prepared', 'failed'])
        self.assertEqual(self.args.completion_audit.call_count, 2)
        self.publish.assert_called_once()
        self.make_manifest.assert_not_called()
        self.assertFalse((self.destination / 'manifest.json').exists())

    def test_publication_failure_still_reconciles_original_build(self):
        self.publish.side_effect = OSError('private artifact copy failed')
        rows = asyncio.run(prepare.prepare_selected(self.args, self.lock))
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]['status'], 'failed')
        self.assertEqual(rows[0]['error_type'], 'OSError')
        self.args.completion_audit.assert_called_once()
        self.make_manifest.assert_not_called()

    def test_audit_exception_is_retained_and_cannot_publish_or_dispatch_next(self):
        self.args.completion_audit.side_effect = RuntimeError('cleanup remains unknown')
        rows = asyncio.run(prepare.prepare_selected(self.args, self.lock))
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]['status'], 'cleanup_unknown')
        self.assertEqual(rows[0]['exact_cleanup'], 'unknown')
        self.assertEqual(rows[0]['audit_error_type'], 'RuntimeError')
        self.assertEqual(json.loads((self.output / self.names[0] / 'result.json').read_bytes()), rows[0])
        self.envs[1].prepare_template.assert_not_awaited()
        self.make_manifest.assert_not_called()
        self.assertFalse((self.destination / 'manifest.json').exists())

    def test_nonpassed_cleanup_receipt_is_not_enough_for_commit(self):
        self.args.completion_audit.side_effect = None
        self.args.completion_audit.return_value = {'exact_cleanup': 'unknown'}
        rows = asyncio.run(prepare.prepare_selected(self.args, self.lock))
        self.assertEqual(len(rows), 1)
        self.envs[1].prepare_template.assert_not_awaited()
        self.make_manifest.assert_not_called()
        self.assertFalse((self.destination / 'manifest.json').exists())

    def test_cancelled_build_propagates_only_after_original_audit_and_reports(self):
        self.envs[0].prepare_template.side_effect = asyncio.CancelledError()
        with self.assertRaises(asyncio.CancelledError):
            asyncio.run(prepare.prepare_selected(self.args, self.lock))
        self.args.completion_audit.assert_called_once()
        report = json.loads((self.output / 'full-set.json').read_bytes())
        self.assertFalse(report['benchmark_passed'])
        self.assertEqual(report['tasks'][0]['exact_cleanup'], 'passed')
        self.assertNotEqual(report['tasks'][0]['status'], 'prepared')
        self.envs[1].prepare_template.assert_not_awaited()
        self.make_manifest.assert_not_called()

    def test_cancellation_during_audit_waits_for_original_thread_then_no_manifest(self):
        async def scenario():
            loop = asyncio.get_running_loop()
            entered = asyncio.Event()
            release = threading.Event()
            self.addCleanup(release.set)
            def audit(row):
                loop.call_soon_threadsafe(entered.set)
                if not release.wait(5):
                    raise TimeoutError('test barrier not released')
                return {'exact_cleanup': 'passed'}
            self.args.completion_audit.side_effect = audit
            running = asyncio.create_task(prepare.prepare_selected(self.args, self.lock))
            await entered.wait()
            running.cancel()
            await asyncio.sleep(0)  # Deliver cancellation at the shield boundary.
            self.assertFalse(running.done())
            self.assertFalse((self.destination / 'manifest.json').exists())
            release.set()
            with self.assertRaises(asyncio.CancelledError):
                await running
        asyncio.run(scenario())
        self.args.completion_audit.assert_called_once()
        self.envs[1].prepare_template.assert_not_awaited()
        self.make_manifest.assert_not_called()
        self.assertFalse((self.destination / 'manifest.json').exists())

    def test_nonselected_tasks_and_existing_destination_reject_before_build(self):
        self.args.task = [self.names[0]]
        with self.assertRaises(InvalidInput):
            asyncio.run(prepare.prepare_selected(self.args, self.lock))
        self.construct.assert_not_called()
        self.args.task = list(self.names)
        self.destination.mkdir(mode=0o700)
        with self.assertRaisesRegex(InvalidInput, 'fresh canonical'):
            asyncio.run(prepare.prepare_selected(self.args, self.lock))
        self.construct.assert_not_called()


if __name__ == '__main__':
    unittest.main()
