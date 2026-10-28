"""Original supervisor argv reaches original parsers; no service, device or VM."""
from contextlib import redirect_stderr, redirect_stdout
import copy
import io
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

from tools import ci_e2e_supervisor as supervisor_module
from tools.tests import test_benchmark_ci as fixtures

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/system'))
import center_host_faults as faults
import harbor_generic_chain as tasks


class E2EArgumentsTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.BenchmarkCITests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.config = self.fixture.host_config()
        self.supervisor = self.fixture.supervisor()
        self.supervisor.config = self.config
        self.supervisor.state.update(candidate_manifest=str(self.root / 'candidate-artifacts.json'),
                                     window_helper_sha256=None)

    def prepare_faults(self, check):
        self.supervisor.state['dk'] = dict(phase='new')
        entry = self.root / 'tests/system/center_host_faults.py'
        entry.parent.mkdir(parents=True)
        entry.write_text('# parser compatibility fixture; never executed\n')
        probe = Path(self.config['stale_probe'])
        probe.parent.mkdir(parents=True)
        probe.write_bytes(b'fixed non-executable helper fixture')
        from tools.system_acceptance import digest
        self.supervisor.state['stale_probe_sha256'] = digest(probe)
        with patch.object(supervisor_module, 'validate_host_inputs'), \
                patch.object(self.supervisor, 'verified_candidate', return_value=self.fixture.services_manifest), \
                patch.object(supervisor_module, 'trusted_path', side_effect=lambda path: path), \
                patch('tools.ci_e2e_prepare.check_candidate_arguments', side_effect=check):
            self.supervisor.prepare_dk()
        return self.supervisor.state['dk']['command']

    def prepare_task(self, name, check):
        batch = dict(names=[name], oracle_mode='original', phase='new')
        self.supervisor.state['task_inputs'] = {name: dict(requirements=dict(
            build_timeout_sec=600, agent_timeout_sec=900, verifier_timeout_sec=900))}
        with patch.object(supervisor_module, 'validate_host_inputs'), \
                patch.object(supervisor_module, 'instantiate_policy', return_value={}), \
                patch.object(self.supervisor, 'verified_candidate', return_value=self.fixture.services_manifest), \
                patch('tools.ci_e2e_prepare.check_candidate_arguments', side_effect=check):
            self.supervisor.prepare(batch, 0)
        return batch

    def parse_only(self, entry, command):
        output = io.StringIO()
        with patch.object(faults, 'validate') as validate, \
                patch.object(faults, 'bootstrap') as bootstrap, \
                patch.object(faults, 'worker') as worker, \
                patch.object(tasks, 'exercise') as exercise, \
                patch.object(Path, 'read_text', side_effect=AssertionError('unexpected host input read')), \
                patch.object(tasks.subprocess, 'run', side_effect=AssertionError('unexpected subprocess')), \
                redirect_stdout(output):
            self.assertEqual(entry.main(command[2:] + ['--check-arguments']), 0)
        for operation in (validate, bootstrap, worker, exercise):
            operation.assert_not_called()
        self.assertEqual(json.loads(output.getvalue()),
                         dict(entry=Path(command[1]).name, arguments_valid=True))

    def test_original_fault_group_command_is_parsed_before_any_product_action(self):
        checked = []
        def check(command, config):
            self.assertIs(config, self.config)
            self.parse_only(faults, command)
            checked.append(copy.deepcopy(command))
        command = self.prepare_faults(check)
        self.assertEqual(checked, [command])
        self.assertNotIn('--check-arguments', command)

    def test_both_original_prepared_tasks_reach_the_same_actual_parser(self):
        for index, name in enumerate(('polyglot-c-py', 'db-wal-recovery')):
            self.supervisor.state['id'] = str(index + 1) * 64
            with self.subTest(task=name):
                batch = self.prepare_task(name, lambda command, _config: self.parse_only(tasks, command))
                self.assertEqual(batch['command'][-2:], ['--task', name])
                self.assertNotIn('--check-arguments', batch['command'])

    def test_missing_capture_settings_reproduces_original_parser_failure_without_bootstrap(self):
        for entry, command in ((faults, self.prepare_faults(None)),
                               (tasks, self.prepare_task('polyglot-c-py', None)['command'])):
            missing = command[2:]
            index = missing.index('--capture-settings')
            del missing[index:index + 2]
            error = io.StringIO()
            with self.subTest(entry=entry.__name__), redirect_stderr(error), \
                    patch.object(faults, 'bootstrap') as bootstrap, \
                    patch.object(tasks, 'exercise') as exercise, \
                    self.assertRaises(SystemExit) as rejected:
                entry.main(missing + ['--check-arguments'])
            self.assertEqual(rejected.exception.code, 2)
            self.assertIn('--capture-settings', error.getvalue())
            bootstrap.assert_not_called()
            exercise.assert_not_called()

    def test_unknown_argument_is_rejected_without_product_actions(self):
        command = self.prepare_faults(None)
        with redirect_stderr(io.StringIO()), patch.object(faults, 'bootstrap') as bootstrap, \
                self.assertRaises(SystemExit) as rejected:
            faults.main(command[2:] + ['--check-arguments', '--unreviewed-argument'])
        self.assertEqual(rejected.exception.code, 2)
        bootstrap.assert_not_called()

    def test_parser_failure_never_persists_a_launch_intent(self):
        def rejected(_command, _config):
            raise ValueError('original candidate parser refused command')
        with self.assertRaisesRegex(ValueError, 'parser refused'):
            self.prepare_faults(rejected)
        self.assertEqual(self.supervisor.state['dk'], dict(phase='new'))
        with self.assertRaisesRegex(ValueError, 'parser refused'):
            self.prepare_task('polyglot-c-py', rejected)
        self.assertFalse((self.root / 'benchmarks/.venv').exists())


if __name__ == '__main__':
    unittest.main()
