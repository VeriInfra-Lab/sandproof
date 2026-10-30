"""Original-driver evidence admission; these fixtures are not program proofs."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

from tools import check_pause_stop_components as component
from tools import acceptance_receipts


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class PauseStopComponentTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.folder = Path(temporary.name)
        self.source = {'commit': 'synthetic-source', 'archive_sha256': 'synthetic-archive'}
        self.driver = SimpleNamespace(CASES=[('first-mutant',), ('another-mutant',)])
        self.loader = mock.patch.object(component, 'load', return_value=self.driver)
        self.loader.start(); self.addCleanup(self.loader.stop)
        self.archive = SimpleNamespace(sha=sha)
        command = {'label': 'original-assertions', 'exit_code': 0}
        for stream in ('stdout', 'stderr'):
            path = self.folder/(command['label']+'.'+stream)
            path.write_text('synthetic stream; no Rust execution\n')
            command[stream+'_sha256'] = sha(path)
        self.result = {'complete': True, 'head': self.source['commit'],
                       'archive_sha256': self.source['archive_sha256'],
                       'commands': [command],
                       'variants': [{'name': name[0]} for name in self.driver.CASES]}
        self.write(self.result)

    def write(self, value):
        (self.folder/'result.json').write_text(json.dumps(value)+'\n')

    def test_admission_follows_actual_driver_inventory_without_freezing_variant_count(self):
        observed = component.check_native_result(self.folder, self.source, self.archive)
        self.assertEqual(observed, self.result)
        self.driver.CASES.append(('added-original-driver-case',))
        with self.assertRaisesRegex(ValueError, 'inventory differs'):
            component.check_native_result(self.folder, self.source, self.archive)
        self.result['variants'].append({'name': 'added-original-driver-case'})
        self.write(self.result)
        self.assertEqual(component.check_native_result(self.folder, self.source, self.archive),
                         self.result)

    def test_failed_wrong_source_archive_or_missing_case_is_not_success(self):
        for key, value in [('complete', False), ('complete', 1), ('head', 'different-source'),
                           ('archive_sha256', 'different-archive'), ('commands', []),
                           ('variants', []), ('variants', list(reversed(self.result['variants'])))]:
            bad = copy.deepcopy(self.result); bad[key] = value; self.write(bad)
            with self.subTest(field=key, value=value), self.assertRaises(ValueError):
                component.check_native_result(self.folder, self.source, self.archive)
        self.write(self.result)
        self.driver.CASES = [('duplicate',), ('duplicate',)]
        with self.assertRaisesRegex(ValueError, 'inventory differs'):
            component.check_native_result(self.folder, self.source, self.archive)

    def test_changed_missing_or_symlinked_original_stream_is_rejected(self):
        path = self.folder/'original-assertions.stderr'; original = path.read_bytes()
        path.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'stream changed'):
            component.check_native_result(self.folder, self.source, self.archive)
        path.unlink()
        with self.assertRaisesRegex(ValueError, 'stream changed'):
            component.check_native_result(self.folder, self.source, self.archive)
        alternate = self.folder/'same-bytes.stderr'; alternate.write_bytes(original)
        path.symlink_to(alternate)
        with self.assertRaisesRegex(ValueError, 'stream changed'):
            component.check_native_result(self.folder, self.source, self.archive)

    def test_formal_admission_requires_both_components_under_the_exact_scope(self):
        cache = self.folder/'cache'
        with mock.patch.object(acceptance_receipts, 'validate_native') as validate, \
                mock.patch.object(acceptance_receipts, 'native_case', return_value={'fixture': True}) as case:
            self.assertEqual(component.admit_native(cache), {'fixture': True})
        validate.assert_called_once_with(cache, scope='pause-stop')
        self.assertEqual(case.call_args_list, [
            mock.call('functional', component.ROOT, None, scope='pause-stop'),
            mock.call('pause-stop', component.ROOT, None, scope='pause-stop')])
        with mock.patch.object(acceptance_receipts, 'validate_native',
                               side_effect=ValueError('synthetic corrupt evidence')), \
                mock.patch.object(acceptance_receipts, 'native_case') as case:
            with self.assertRaises(ValueError):
                component.admit_native(cache)
        case.assert_not_called()

    def test_wrong_host_role_cannot_launch_the_component_driver(self):
        for mode, role in [('native', 'formal'), ('formal', 'native')]:
            with self.subTest(mode=mode), mock.patch.dict(os.environ, {'SANDPROOF_ACCEPTANCE_ROLE': role}), \
                    mock.patch.object(component.subprocess, 'run') as run:
                result = component.run(self.folder/('role-'+mode), mode)
            self.assertFalse(result['complete'])
            self.assertFalse(result['whole_V4'])
            self.assertFalse(result['whole_system'])
            self.assertIn('explicit', result['error'])
            run.assert_not_called()


class PauseStopMutationTests(unittest.TestCase):
    """Check mutant identity only; actual business assertions run in Rust."""

    def setUp(self):
        self.driver = component.load('pause_stop_mutator_regression',
                                     component.HERE/'native.py')
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.path = self.root/self.driver.SOURCE
        self.path.parent.mkdir(parents=True)
        self.original = (component.ROOT/self.driver.SOURCE).read_text()
        self.path.write_text(self.original)

    def test_only_the_real_handoff_boolean_is_ignored(self):
        self.assertEqual(len(self.driver.CASES), 1)
        case = self.driver.CASES[0]
        digest = self.driver.mutate(self.root, case)
        changed = self.path.read_text()
        self.assertEqual(changed, self.original.replace(case[2], case[3], 1))
        self.assertEqual(digest, sha(self.path))
        self.assertIn('let _ = handoff.may_stop_original();', changed)
        self.assertIn('|| !crate::transition::current_capture(world, sandbox, record)', changed)
        self.assertEqual(changed.split('\nfn stop_vm(', 1)[0],
                         self.original.split('\nfn stop_vm(', 1)[0])
        for guard in ('stored_snapshot.owner != record.owner',
                      'stored_snapshot.manifest != target.manifest',
                      'stored_snapshot.stage != SnapshotStage::Durable',
                      '!stored_snapshot.immutable',
                      'if !contains(&world.inflight, id)',
                      'if current_effect_id(world, &command) != *id',
                      'if let Some(result) = late_capture::record(world, id)'):
            self.assertIn(guard, changed)

    def test_missing_duplicate_or_moved_guard_is_rejected_before_writing(self):
        guard = self.driver.STOP_GUARD
        candidates = [self.original.replace(guard, ''), self.original+guard,
                      guard+self.original.replace(guard, ''),
                      self.original.replace(guard, '')+'\n'+guard]
        for text in candidates:
            with self.subTest(source=text[:40]):
                self.path.write_text(text)
                with self.assertRaises(ValueError):
                    self.driver.mutate(self.root, self.driver.CASES[0])
                self.assertEqual(self.path.read_text(), text)

    def test_mutation_cannot_be_applied_twice(self):
        self.driver.mutate(self.root, self.driver.CASES[0])
        changed = self.path.read_bytes()
        with self.assertRaises(ValueError):
            self.driver.mutate(self.root, self.driver.CASES[0])
        self.assertEqual(self.path.read_bytes(), changed)


if __name__ == '__main__':
    unittest.main()
