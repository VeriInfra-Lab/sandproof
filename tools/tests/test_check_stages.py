from contextlib import redirect_stderr
import io
import json
import os
from pathlib import Path
import shutil
import signal
import selectors
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from tools import check_stages as stages

ROOT = Path(__file__).resolve().parents[2]


class StageReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / 'events.jsonl'
        self.stderr = redirect_stderr(io.StringIO())
        self.stderr.__enter__()
        self.addCleanup(self.stderr.__exit__, None, None, None)

    def test_clock_records_build_and_execution_separately_without_changing_exit(self):
        with patch.object(stages.time, 'monotonic_ns', side_effect=[100, 200, 300, 400, 900, 1000]):
            stages.initialize(self.path, 'linux')
            stages.start(self.path, 'workspace-build', 'build')
            stages.finish(self.path, 'workspace-build', 0)
            stages.start(self.path, 'workspace-tests', 'test')
            stages.finish(self.path, 'workspace-tests', 17)
            stages.end_run(self.path, 17)
        result = stages.report(stages.load(self.path))
        self.assertTrue(result['complete'])
        self.assertEqual(result['exit_code'], 17)
        self.assertEqual([r['kind'] for r in result['stages']], ['build', 'test'])
        self.assertEqual([r['exit_code'] for r in result['stages']], [0, 17])
        self.assertIn('原检查退出码：17', stages.markdown(result))

    def test_success_cannot_replace_failed_or_missing_checks(self):
        stages.initialize(self.path, 'portable')
        with self.assertRaises(ValueError):
            stages.end_run(self.path, 0)
        stages.start(self.path, 'tests', 'test')
        stages.finish(self.path, 'tests', 101)
        with self.assertRaises(ValueError):
            stages.start(self.path, 'later-tests', 'test')
        with self.assertRaises(ValueError):
            stages.end_run(self.path, 0)
        stages.end_run(self.path, 101)
        with self.assertRaises(ValueError):
            stages.start(self.path, 'restarted', 'test')

    def test_interrupted_stage_remains_unknown_not_assertion_or_success(self):
        stages.initialize(self.path, 'linux')
        stages.start(self.path, 'tests', 'test')
        result = stages.report(stages.load(self.path))
        self.assertFalse(result['complete'])
        self.assertIsNone(result['exit_code'])
        self.assertEqual(result['unfinished_stage'], 'tests')
        self.assertIn('未确认完整结束', stages.markdown(result))
        with self.assertRaises(ValueError):
            stages.end_run(self.path, 0)
        with self.assertRaises(ValueError):
            stages.finish(self.path, 'replacement', 0)

    def test_records_are_fresh_and_do_not_follow_links(self):
        stages.initialize(self.path, 'linux')
        original = self.path.read_bytes()
        with self.assertRaises(FileExistsError):
            stages.initialize(self.path, 'linux')
        link = self.path.parent / 'link'
        link.symlink_to(self.path)
        with self.assertRaises(OSError):
            stages.read_bounded(link, stages.EVENT_LIMIT)
        with self.assertRaises(OSError):
            stages.append(link, {'event': 'replaced'})
        self.assertEqual(self.path.read_bytes(), original)

    def test_partial_oversized_nonregular_and_invalid_order_are_not_complete(self):
        self.path.write_text('{"event":"run-start"}')
        with self.assertRaises(ValueError):
            stages.load(self.path)
        with self.assertRaises(ValueError):
            stages.read_bounded(self.path, 3)
        fifo = self.path.parent / 'fifo'
        os.mkfifo(fifo)
        with self.assertRaises(ValueError):
            stages.read_bounded(fifo, 100)
        self.path.unlink()
        stages.initialize(self.path, 'linux')
        stages.start(self.path, 'tests', 'test')
        stages.finish(self.path, 'tests', 0)
        with self.assertRaises(ValueError):
            stages.start(self.path, 'tests', 'test')
        with self.assertRaises(ValueError):
            stages.start(self.path, 'bad|table\n::error::', 'test')
        events = stages.load(self.path)
        events[-1]['at_ns'] = events[0]['at_ns'] - 1
        self.path.write_text(''.join(json.dumps(e) + '\n' for e in events))
        with self.assertRaises(ValueError):
            stages.load(self.path)

    def test_log_hints_preserve_mixed_and_unclassified_failures_without_retry(self):
        cases = [
            ('Could not resolve host: example.invalid', ['network']),
            ('error: no matching package named `missing` found', ['dependency_or_material']),
            ('subprocess.TimeoutExpired: command', ['test_timeout']),
            ('AssertionError: incorrect result', ['assertion']),
            ('ERROR: tearDownClass (tests.Case)\nAssertionError', ['assertion', 'cleanup']),
            ('CLEANUP_UNKNOWN', ['cleanup']),
            ('cleanup_status="unknown"', ['cleanup']),
            ('process exited with 124', ['unclassified_failure']),
            ('test_partial_identity_is_kept_cleanup_unknown (tests.Example) ... ok', ['unclassified_failure']),
            ('test case::CLEANUP_UNKNOWN ... ok', ['unclassified_failure']),
        ]
        for text, expected in cases:
            with self.subTest(text=text):
                result = stages.analyze(text, 'failure')
                self.assertEqual(result['possible_causes'], expected)
                self.assertEqual(result['acceptance'], 'not_evaluated')
                self.assertEqual(stages.analyze(text, 'success')['possible_causes'], [])
        self.assertEqual(stages.analyze('', 'timed_out')['possible_causes'], ['job_timeout'])
        self.assertEqual(stages.analyze('', 'cancelled')['possible_causes'], ['interrupted'])
        annotations = [{'message': 'The job has exceeded the maximum execution time of 20m0s'}]
        result = stages.analyze('test_cleanup_unknown (tests.Example) ... ok', 'cancelled', annotations)
        self.assertEqual(result['job_conclusion'], 'cancelled')
        self.assertEqual(result['possible_causes'], ['job_timeout'])
        self.assertEqual(result['deadline_evidence'], 'actions_annotation')
        with self.assertRaises(ValueError):
            stages.analyze('', 'cancelled', [{'message': 20}])
        with tempfile.TemporaryDirectory() as temporary:
            log = Path(temporary) / 'ci.log'
            annotations_path = Path(temporary) / 'annotations.json'
            log.write_text('test_cleanup_unknown (tests.Example) ... ok\n')
            annotations_path.write_text(json.dumps(annotations))
            result = subprocess.run([sys.executable, str(ROOT / 'tools/check_stages.py'),
                'analyze-log', '--log', str(log), '--conclusion', 'cancelled',
                '--annotations', str(annotations_path)], capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)['possible_causes'], ['job_timeout'])

    def test_slow_target_summary_uses_original_harness_duration_not_absolute_paths(self):
        text = '''[check-stage] {"event":"stage-start","stage":"workspace-tests"}
Finished `test` profile [unoptimized + debuginfo] target(s) in 1m 47s
Running unittests src/lib.rs (/private/secret/target/controller-default)
test result: ok. 181 passed; 0 failed; finished in 214.91s
[check-stage] {"event":"stage-start","stage":"diagnostics-tests"}
Finished `test` profile [unoptimized + debuginfo] target(s) in 15.24s
Running unittests src/lib.rs (/private/secret/target/controller-diagnostics)
test result: ok. 182 passed; 0 failed; finished in 289.52s
'''
        result = stages.analyze(text, 'success')
        self.assertEqual([r['seconds'] for r in result['slow_rust_targets']], [289.52, 214.91])
        self.assertEqual(result['slow_rust_targets'][0]['stage'], 'diagnostics-tests')
        self.assertEqual([r['seconds'] for r in result['slow_cargo_build_reports']], [107.0, 15.24])
        self.assertNotIn('/private/secret', json.dumps(result))
        self.assertEqual(stages.analyze('x' * 8193 + text.splitlines()[0], 'success')['slow_rust_targets'], [])


class StageEntryTests(unittest.TestCase):
    def fixture(self, directory):
        for name in ('scripts', 'tools', 'bin'):
            (directory / name).mkdir()
        shutil.copyfile(ROOT / 'scripts/check-function.sh', directory / 'scripts/check-function.sh')
        shutil.copyfile(ROOT / 'tools/check_stages.py', directory / 'tools/check_stages.py')
        # Real diagnostic code, controlled original commands. No Rust test is
        # replaced in production; this verifies the shell's exact dispatch.
        python_entry = '''import json, os, sys
from pathlib import Path
if sys.argv[1] == '-c':
    os.execv(sys.executable, [sys.executable, *sys.argv[1:]])
with open(os.environ['CHECK_TRACE'], 'a') as output:
    output.write(json.dumps(['python', *sys.argv[1:]]) + '\\n')
if os.environ.get('CHECK_REQUIRE_REPORT_ISOLATION') and 'SANDPROOF_CHECK_EVENTS' in os.environ:
    raise SystemExit(31)
if os.environ.get('CHECK_FAIL_TOOLS') and '--preflight-only' not in sys.argv:
    raise SystemExit(17)
'''
        (directory / 'scripts/python-entry.sh').write_text(
            '#!/bin/bash\nexec ' + sys.executable + ' tools/python_fixture.py "$@"\n')
        (directory / 'tools/python_fixture.py').write_text(python_entry)
        cargo = '''#!PYTHON
import json, os, signal, sys
from pathlib import Path
if sys.argv[1:3] != ['--config', 'scripts/check-profiles.toml']:
    raise SystemExit(33)
sys.argv = [sys.argv[0], *sys.argv[3:]]
with open(os.environ['CHECK_TRACE'], 'a') as output:
    output.write(json.dumps(['cargo', *sys.argv[1:]]) + '\\n')
if os.environ.get('CHECK_REQUIRE_REPORT_ISOLATION') and 'SANDPROOF_CHECK_EVENTS' in os.environ:
    raise SystemExit(31)
if os.environ.get('CHECK_REQUIRE_CHECKED_OPTIMIZATION'):
    for profile in ('DEV', 'TEST'):
        for key, expected in (('OPT_LEVEL', '0'), ('DEBUG', '1'), ('DEBUG_ASSERTIONS', 'true'), ('OVERFLOW_CHECKS', 'true')):
            if os.environ.get('CARGO_PROFILE_' + profile + '_' + key) != expected:
                raise SystemExit(32)
    if os.environ.get('CARGO_INCREMENTAL') != '0':
        raise SystemExit(32)
if os.environ.get('CHECK_BREAK_REPORT') and sys.argv[1] == 'fmt':
    Path(os.environ['CHECK_TRACE']).with_name('events.jsonl').unlink()
    raise SystemExit(int(os.environ['CHECK_BREAK_REPORT']))
if os.environ.get('CHECK_SIGNAL_FD') and sys.argv[1] == 'fmt':
    os.write(int(os.environ['CHECK_SIGNAL_FD']), b'ready')
    signal.pause()
if sys.argv[1:] == json.loads(os.environ.get('CHECK_FAIL_CARGO', '[]')):
    raise SystemExit(23)
if sys.argv[1] == 'tree':
    print('rusqlite v0.40.2' if os.environ.get('CHECK_BAD_DEPENDENCY') else 'sandproof-node v0.1.0')
'''.replace('PYTHON', sys.executable, 1)
        (directory / 'bin/cargo').write_text(cargo)
        (directory / 'bin/cargo').chmod(0o700)

    def execute(self, directory, *, mode='linux', extra=None):
        events = directory / 'events.jsonl'
        env = os.environ | {'SANDPROOF_CHECK_EVENTS': str(events), 'CHECK_TRACE': str(directory / 'trace'),
                            'PATH': str(directory / 'bin') + os.pathsep + os.environ['PATH']}
        for name in ('CHECK_FAIL_CARGO', 'CHECK_FAIL_TOOLS', 'CHECK_BAD_DEPENDENCY'):
            env.pop(name, None)
        env.update(extra or {})
        command = ['bash', 'scripts/check-function.sh'] + (['--portable'] if mode == 'portable' else [])
        result = subprocess.run(command, cwd=directory, env=env, capture_output=True, text=True, timeout=20)
        trace = [json.loads(line) for line in (directory / 'trace').read_text().splitlines()]
        return result, trace, stages.report(stages.load(events))

    def test_original_command_matrix_remains_without_additional_preparation(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.fixture(directory)
            result, trace, report = self.execute(directory)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(report['complete'])
            self.assertEqual(report['exit_code'], 0)
            cargo = [item[1:] for item in trace if item[0] == 'cargo']
            required = [
                ['clippy', '--locked', '-p', 'sandproof-node', '--lib', '--', '-D', 'warnings'],
                ['test', '--locked', '-p', 'sandproof-controller-service', '--no-default-features', '--lib'],
                ['build', '--locked', '-p', 'sandproof-controller-service', '--bin', 'sandproof-window-reducer'],
                ['fmt', '--all', '--', '--check'],
                ['clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings'],
                ['test', '--workspace', '--all-targets', '--locked'],
                ['test', '--workspace', '--doc', '--locked'],
                ['clippy', '--locked', '-p', 'sandproof-controller-service', '--all-targets', '--features', 'contention-diagnostics', '--', '-D', 'warnings'],
                ['test', '--locked', '-p', 'sandproof-controller-service', '--features', 'contention-diagnostics'],
            ]
            positions = [cargo.index(item) for item in required]
            self.assertEqual(positions, sorted(positions))
            for command in required:
                self.assertEqual(cargo.count(command), 1)
            self.assertFalse(any('--no-run' in command for command in cargo))
            self.assertEqual(len(cargo), len(required) + 1)  # original dependency tree query
            self.assertIn(['python', 'tools/check_function_tests.py', '--mode', 'linux'], trace)
            self.assertIn(['python', 'tools/check_docs.py'], trace)
            self.assertEqual(len(report['stages']), 13)

    def test_failed_execution_keeps_exit_and_never_runs_later_checks(self):
        command = ['test', '--workspace', '--all-targets', '--locked']
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.fixture(directory)
            result, trace, report = self.execute(directory, extra={'CHECK_FAIL_CARGO': json.dumps(command)})
            self.assertEqual(result.returncode, 23, result.stderr)
            self.assertEqual(trace[-1], ['cargo', *command])
            self.assertEqual(report['exit_code'], 23)
            self.assertEqual(report['stages'][-1]['stage'], 'workspace-tests')
            self.assertEqual(report['stages'][-1]['exit_code'], 23)
            self.assertNotIn('Functional checks passed', result.stdout)

    def test_nested_commands_do_not_inherit_the_outer_event_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.fixture(directory)
            result, trace, report = self.execute(directory, extra={'CHECK_REQUIRE_REPORT_ISOLATION': '1'})
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(report['exit_code'], 0)

    def test_all_original_cargo_commands_keep_checked_optimization_despite_ambient_overrides(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.fixture(directory)
            overrides = {'CHECK_REQUIRE_CHECKED_OPTIMIZATION': '1'}
            for profile in ('DEV', 'TEST'):
                for key in ('OPT_LEVEL', 'DEBUG', 'DEBUG_ASSERTIONS', 'OVERFLOW_CHECKS'):
                    overrides['CARGO_PROFILE_' + profile + '_' + key] = '3' if key == 'OPT_LEVEL' else 'false'
            overrides['CARGO_INCREMENTAL'] = '1'
            result, trace, report = self.execute(directory, extra=overrides)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(report['exit_code'], 0)
            self.assertEqual(len([item for item in trace if item[0] == 'cargo']), 10)

    def test_diagnostic_write_loss_cannot_mask_original_failure_or_create_green(self):
        for command_exit in (0, 23):
            with self.subTest(command_exit=command_exit), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                self.fixture(directory)
                env = os.environ | {'SANDPROOF_CHECK_EVENTS': str(directory / 'events.jsonl'),
                    'CHECK_TRACE': str(directory / 'trace'), 'CHECK_BREAK_REPORT': str(command_exit),
                    'PATH': str(directory / 'bin') + os.pathsep + os.environ['PATH']}
                result = subprocess.run(['bash', 'scripts/check-function.sh', '--portable'],
                    cwd=directory, env=env, capture_output=True, text=True, timeout=20)
                self.assertEqual(result.returncode, command_exit or 70, result.stderr)
                self.assertNotIn('Portable development checks passed', result.stdout)
                trace = [json.loads(line) for line in (directory / 'trace').read_text().splitlines()]
                self.assertEqual(trace[-1], ['cargo', 'fmt', '--all', '--', '--check'])

    def test_actual_termination_does_not_report_success_or_retry_commands(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.fixture(directory)
            read_fd, write_fd = os.pipe()
            env = os.environ | {'SANDPROOF_CHECK_EVENTS': str(directory / 'events.jsonl'),
                'CHECK_TRACE': str(directory / 'trace'), 'CHECK_SIGNAL_FD': str(write_fd),
                'PATH': str(directory / 'bin') + os.pathsep + os.environ['PATH']}
            with subprocess.Popen(['bash', 'scripts/check-function.sh', '--portable'], cwd=directory,
                env=env, start_new_session=True, pass_fds=(write_fd,),
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) as process:
                os.close(write_fd)
                try:
                    with selectors.DefaultSelector() as selector:
                        selector.register(read_fd, selectors.EVENT_READ)
                        self.assertTrue(selector.select(timeout=10), 'original command never reached barrier')
                        self.assertEqual(os.read(read_fd, 5), b'ready')
                    os.killpg(process.pid, signal.SIGTERM)  # only this freshly created fixture group
                    self.assertNotEqual(process.wait(timeout=5), 0)
                finally:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait(timeout=5)
                    os.close(read_fd)
            result = stages.report(stages.load(directory / 'events.jsonl'))
            self.assertFalse(result['complete'] and result['exit_code'] == 0)
            trace = [json.loads(line) for line in (directory / 'trace').read_text().splitlines()]
            self.assertEqual(trace.count(['cargo', 'fmt', '--all', '--', '--check']), 1)

    def test_command_substitution_failure_is_not_masked_or_double_finalized(self):
        command = ['tree', '--locked', '-p', 'sandproof-node', '--edges', 'normal,build', '--prefix', 'none', '--format', '{p}']
        for extra, expected in [({'CHECK_FAIL_CARGO': json.dumps(command)}, 23), ({'CHECK_BAD_DEPENDENCY': '1'}, 1)]:
            with self.subTest(extra=extra), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                self.fixture(directory)
                result, trace, report = self.execute(directory, extra=extra)
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertEqual(trace[-1], ['cargo', *command])
                self.assertEqual(report['stages'][-1]['stage'], 'node-dependency-contract')
                self.assertEqual(report['exit_code'], expected)

    def test_tools_failure_stops_before_format_and_portable_does_not_add_linux_build(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.fixture(directory)
            result, trace, report = self.execute(directory, mode='portable', extra={'CHECK_FAIL_TOOLS': '1'})
            self.assertEqual(result.returncode, 17, result.stderr)
            self.assertEqual(report['mode'], 'portable')
            self.assertEqual(report['stages'][-1]['stage'], 'tool-tests')
            self.assertFalse(any(item[0] == 'cargo' and item[1] in ('build', 'fmt') for item in trace))

    def test_workflow_retains_triggers_original_gate_and_reports_without_masking_failures(self):
        workflow = (ROOT / '.github/workflows/rust-ci.yml').read_text()
        self.assertIn('timeout-minutes: 30', workflow)
        self.assertIn('branches: [main, test]', workflow)
        self.assertIn('workflow_dispatch:', workflow)
        self.assertNotIn('pull_request:', workflow)
        self.assertNotIn('continue-on-error:', workflow)
        self.assertIn('if: ${{ always() && steps.workspace.outcome', workflow)
        self.assertIn('bash scripts/check-function.sh', workflow)
        self.assertIn('tools/ci_redis.py check', workflow)
        # runner is available in step env, not job env. A syntactically valid
        # YAML file can otherwise be rejected before a single check starts.
        job_prefix = workflow.split('    steps:', 1)[0]
        self.assertNotIn('${{ runner.', job_prefix)
        self.assertEqual(workflow.count('          SANDPROOF_CHECK_EVENTS: ${{ runner.temp }}/rust-check-events.jsonl'), 2)


if __name__ == '__main__':
    unittest.main()
