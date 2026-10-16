"""Original SDK argument/error recordings and private context-program checks."""
import copy
from contextlib import ExitStack
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import pwd
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.exceptions import AuthenticationException, InvalidArgumentException
from e2b.sandbox.filesystem.filesystem import EntryInfo, FileType
from protobuf import Oneof
from sdk_support import isolated_sdk_environment
import test_command_probe_cleanup as wire
import sdk_argument_compare as arguments
import sdk_guest_compare as paired
from sdk_observations import Calls

DIRECTORY = '/tmp/shared/commands-context'
INPUTS = dict(directory='/tmp/shared', unknown_user='sp_missing_fixed', users=['root', 'ubuntu'], mtime=1700000000)


def terminal(directory=DIRECTORY + '/public', user='ubuntu', *, stdout=None, stderr=arguments.STDERR,
             code=0, error=None):
    stdout = arguments.context_stdout(directory, user) if stdout is None else stdout
    events = wire.end('', code=code, error=error)
    for channel, data in reversed([('stdout', stdout), ('stderr', stderr)]):
        for part in reversed((data[:5], data[5:])):
            if part:
                events.insert(0, ('data', wire.pb.ProcessEvent.DataEvent(output=Oneof(channel, part.encode()))))
    return events


class PrivateProgramTests(unittest.TestCase):
    def test_exact_context_program_preserves_argv_empty_unicode_env_and_physical_cwd(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp).resolve()
            (directory / 'actual').mkdir()
            (directory / 'alias').symlink_to(directory / 'actual', target_is_directory=True)
            completed = subprocess.run([sys.executable, '-I', '-c', arguments.PROGRAM, *arguments.ARGV],
                cwd=directory / 'alias', env={**os.environ, **arguments.ENV},
                capture_output=True, check=True, timeout=5)
            expected = dict(cwd=str(directory / 'actual'), user=pwd.getpwuid(os.geteuid()).pw_name,
                            argv=arguments.ARGV, env=arguments.ENV)
            self.assertEqual(json.loads(completed.stdout), expected)
            self.assertEqual(completed.stderr.decode(), arguments.STDERR)
            self.assertEqual(completed.stdout.decode(), arguments.context_stdout(expected['cwd'], expected['user']))


class OriginalSdkTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    async def execute(self, asynchronous, background, *, pid=4321, events=None, expected=None):
        script = (pid, terminal() if events is None else events)
        if isinstance(events, BaseException):
            script = events
        tape = wire.Transcript(asynchronous, [script])
        calls = Calls(tape.sandbox, asynchronous)
        await arguments.argument_run(calls, 'sample', arguments.CONTEXT, user='ubuntu',
            cwd=DIRECTORY + '/public', background=background, expected=expected)
        return calls.rows[0], tape

    async def test_both_sdks_preserve_inputs_callbacks_and_complete_foreground_background_context(self):
        for background in (False, True):
            rows = []
            for asynchronous, pid in ((False, 4321), (True, 98765)):
                row, tape = await self.execute(asynchronous, background, pid=pid)
                rows.append(row)
                self.assertEqual(tape.signals(), [])
                self.assertEqual(len(tape.closed), 1)
                request, options = tape.calls[0][1:]
                self.assertEqual((request.process.cmd, list(request.process.args)),
                                 ('/bin/bash', ['-l', '-c', arguments.CONTEXT]))
                self.assertEqual(request.process.cwd, DIRECTORY + '/public')
                self.assertEqual(dict(request.process.envs), arguments.ENV)
                self.assertEqual(options['timeout_ms'], 15000)
                self.assertEqual(options['headers']['Authorization'], 'Basic dWJ1bnR1Og==')
                self.assertFalse(request.stdin)
            originals = copy.deepcopy(rows)
            paired.compare([rows[0]], [rows[1]], 'same argument inputs')
            self.assertEqual(rows, originals)

    async def test_expected_rejection_keeps_exact_error_and_never_invents_pid(self):
        for background in (False, True):
            for code, expected in ((Code.INVALID_ARGUMENT, InvalidArgumentException),
                                   (Code.UNAUTHENTICATED, AuthenticationException)):
                rows = []
                for asynchronous in (False, True):
                    error = ConnectError(code, 'exact path \'"<>&中文 and PID 4321')
                    row, tape = await self.execute(asynchronous, background, events=error, expected=expected)
                    rows.append(row)
                    self.assertNotIn('pid', row)
                    self.assertEqual(row['calls'][0]['error']['message'], str(expected(error.message)))
                    self.assertEqual([item[0] for item in tape.calls], ['start'])
                paired.compare([rows[0]], [rows[1]], 'same rejection')
                changed = copy.deepcopy(rows[1])
                changed['calls'][0]['error']['message'] += ' changed'
                with self.assertRaises(AssertionError):
                    paired.compare([rows[0]], [changed], 'wrong error')

    async def test_only_explicit_pid_references_and_callback_segmentation_can_differ(self):
        row, _ = await self.execute(False, True)
        other = copy.deepcopy(row)
        other['calls'][0]['callbacks']['stdout'] = [''.join(other['calls'][0]['callbacks']['stdout'])]
        paired.compare([row], [other], 'segmentation only')
        mutations = [lambda r: r.__setitem__('pid', True),
            lambda r: r['calls'][0]['result'].__setitem__('pid', 99),
            lambda r: r['calls'][1].__setitem__('handle_pid', 99),
            lambda r: r['calls'][2].__setitem__('handle_pid', 99),
            lambda r: r['calls'][1].__setitem__('status', 'incomplete'),
            lambda r: r.__setitem__('background', 1),
            lambda r: r['calls'][0]['kwargs']['envs'].__setitem__('SP_ARGUMENT_PID_TEXT', '98765'),
            lambda r: r['calls'][0]['args'].__setitem__(0, r['calls'][0]['args'][0].replace('4321', '98765')),
            lambda r: r['calls'][1]['result'].__setitem__('stdout', 'PID 98765'),
            lambda r: r['calls'].append(copy.deepcopy(r['calls'][-1]))]
        for mutation in mutations:
            other = copy.deepcopy(row)
            mutation(other)
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                paired.compare([row], [other], 'changed evidence')

    async def test_unexpected_success_and_unknown_start_do_not_replay(self):
        for asynchronous in (False, True):
            tape = wire.Transcript(asynchronous, [(4321, None)])
            calls = Calls(tape.sandbox, asynchronous)
            with self.assertRaises(AssertionError):
                await arguments.argument_run(calls, 'rejected', 'unexpected command', user='ubuntu',
                    cwd=DIRECTORY, background=True, expected=InvalidArgumentException)
            self.assertEqual(tape.signals(), [Oneof('pid', 4321)])
            self.assertEqual(len(tape.closed), 1)
            row = calls.rows[0]
            self.assertEqual(row['status'], 'incomplete')
            self.assertEqual(row['calls'][0]['result'], {'pid': 4321})
            self.assertEqual([item['method'] for item in row['calls']],
                             ['commands.run', 'commands.kill', 'handle.disconnect'])
            for background in (False, True):
                tape = wire.Transcript(asynchronous, [wire.deadline()])
                calls = Calls(tape.sandbox, asynchronous)
                with self.assertRaises(Exception):
                    await arguments.argument_run(calls, 'unknown', 'original only', user='root',
                        cwd=DIRECTORY, background=background, expected=InvalidArgumentException)
                self.assertEqual([item[0] for item in tape.calls], ['start'])
                self.assertNotIn('pid', calls.rows[0])
                self.assertEqual(calls.rows[0]['status'], 'incomplete')

    async def test_wrong_terminal_missing_end_and_invalid_pid_have_distinct_cleanup(self):
        for asynchronous in (False, True):
            for fault in ('stdout', 'stderr', 'exit', 'error', 'missing-end', 'invalid-pid'):
                with self.subTest(asynchronous=asynchronous, fault=fault):
                    events = [] if fault == 'missing-end' else terminal(
                        stdout='bad' if fault == 'stdout' else None,
                        stderr='bad' if fault == 'stderr' else arguments.STDERR,
                        code=17 if fault == 'exit' else 0, error='bad' if fault == 'error' else None)
                    tape = wire.Transcript(asynchronous, [(0 if fault == 'invalid-pid' else 4321, events)])
                    calls = Calls(tape.sandbox, asynchronous)
                    with self.assertRaises(Exception):
                        await arguments.argument_run(calls, 'invalid', arguments.CONTEXT, user='ubuntu',
                            cwd=DIRECTORY + '/public', background=True)
                    self.assertEqual(tape.signals(), [Oneof('pid', 4321)] if fault == 'missing-end' else [])
                    self.assertEqual(len(tape.closed), 1)
                    self.assertEqual(calls.rows[0]['status'], 'incomplete')

    async def test_primary_signal_and_close_failures_are_recorded_without_retries(self):
        for asynchronous in (False, True):
            tape = wire.Transcript(asynchronous, [(4321, None)])
            signal_error, close_error = RuntimeError('signal unknown'), RuntimeError('close unknown')
            tape.sandbox.commands._rpc.send_signal.side_effect = signal_error
            module = wire.async_command if asynchronous else wire.sync_command
            cls = module.AsyncCommandHandle if asynchronous else module.CommandHandle
            original = cls.disconnect
            async def disconnect(handle):
                await wire.inputs.call(original, handle)
                raise close_error
            calls = Calls(tape.sandbox, asynchronous)
            with patch.object(cls, 'disconnect', new=disconnect), self.assertRaises(BaseExceptionGroup) as caught:
                await arguments.argument_run(calls, 'rejected', 'unexpected', user='ubuntu',
                    cwd=DIRECTORY, background=True, expected=InvalidArgumentException)
            self.assertIsInstance(caught.exception.exceptions[0], AssertionError)
            self.assertEqual(list(caught.exception.exceptions[1:]), [signal_error, close_error])
            self.assertEqual(tape.sandbox.commands._rpc.send_signal.call_count, 1)
            self.assertEqual(len(tape.closed), 1)
            self.assertEqual([item['status'] for item in calls.rows[0]['calls']], ['incomplete'] * 3)


class MatrixTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    def fixture(self, asynchronous, *, marker=False, owner='root'):
        scripts, pid = [(100, wire.end('')), (101, wire.end(''))], 90000 if asynchronous else 4000
        expected_names = ['unknown_user', 'unknown_user_missing_cwd'] + [
            name + '-' + user for user in ('root', 'ubuntu')
            for name in ('missing_cwd', 'dangling_cwd', 'file_cwd', 'file_parent_cwd')
        ] + ['denied_cwd', 'denied_link', 'missing_private_child']
        definitions = arguments.rejections(DIRECTORY, INPUTS['unknown_user'])
        self.assertEqual([item[0] for item in definitions], expected_names)
        for _background in (False, True):
            for user, physical in [('root', 'public'), ('ubuntu', 'public'), ('root', 'private'),
                                   ('root', 'private'), ('root', 'public'), ('ubuntu', 'public')]:
                pid += 1
                scripts.append((pid, terminal(DIRECTORY + '/' + physical, user)))
            for name, user, cwd, expected in definitions:
                code = Code.UNAUTHENTICATED if expected is AuthenticationException else Code.INVALID_ARGUMENT
                scripts.append(ConnectError(code, 'prescribed ' + name + ': ' + user + ' ' + cwd))
                pid += 1
                scripts.append((pid, terminal()))
        tape = wire.Transcript(asynchronous, scripts)
        wrap = AsyncMock if asynchronous else Mock
        def entry(path, kind, mode, size):
            return EntryInfo(path=DIRECTORY + '/' + path, name=path, type=kind, mode=mode, size=size,
                permissions='fixture', owner=owner, group='root',
                modified_time=datetime.fromtimestamp(INPUTS['mtime'], timezone.utc))
        tape.sandbox.files = SimpleNamespace(get_info=wrap(side_effect=[
            entry('public', FileType.DIR, 0o40777, 4096), entry('private', FileType.DIR, 0o40700, 4096),
            entry('ordinary', FileType.FILE, 0o100644, 8)]), exists=wrap(return_value=marker),
            read=wrap(return_value='original'))
        return Calls(tape.sandbox, asynchronous), tape

    async def test_full_matrix_has_fixed_inputs_absent_effects_and_healthy_followups(self):
        rows = []
        for asynchronous in (False, True):
            calls, tape = self.fixture(asynchronous)
            await arguments.argument_cases(calls, INPUTS)
            rows.append(calls.rows)
            self.assertFalse(tape.starts)
            self.assertEqual(len(tape.calls), 66)
            self.assertEqual(len(tape.closed), 66)
            self.assertEqual(tape.signals(), [])
            cases = [row for row in calls.rows if row['method'] == 'commands.argument_case']
            self.assertEqual(sum(row['rejected'] for row in cases), 26)
            self.assertEqual(sum(row['background'] and not row['rejected'] for row in cases), 6)
            self.assertEqual(sum(row['name'].startswith('healthy-after-') for row in cases), 26)
            markers = [row['args'][0] for row in calls.rows if row['method'] == 'files.exists']
            self.assertEqual(len(markers), len(set(markers)))
            self.assertEqual(len(markers), 26)
            for index, row in enumerate(calls.rows):
                if row.get('rejected'):
                    self.assertEqual(calls.rows[index+1]['method'], 'files.exists')
                    self.assertEqual(calls.rows[index+2]['name'], 'healthy-after-' + row['name'])
            self.assertEqual(tape.calls[0][2]['headers']['Authorization'], 'Basic cm9vdDo=')
        paired.compare(*rows, 'complete argument matrix')

    async def test_bad_setup_or_rejection_side_effect_preserves_partial_evidence(self):
        for asynchronous in (False, True):
            for fault in ('owner', 'marker', 'original'):
                calls, tape = self.fixture(asynchronous, owner='ubuntu' if fault == 'owner' else 'root',
                                           marker=fault == 'marker')
                if fault == 'original':
                    tape.sandbox.files.read.return_value = 'changed'
                with self.assertRaises(AssertionError):
                    await arguments.argument_cases(calls, INPUTS)
                self.assertTrue(calls.rows)
                if fault == 'owner':
                    self.assertEqual([item[0] for item in tape.calls], ['start', 'start'])
                elif fault == 'marker':
                    self.assertEqual(calls.rows[-1]['method'], 'files.exists')
                    self.assertIs(calls.rows[-1]['result'], True)
                else:
                    self.assertEqual(calls.rows[-1]['result'], 'changed')
