"""Original VM credential and failure evidence regressions; no real VM/signals."""
from contextlib import redirect_stdout
import hashlib
import http.client
import io
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'system'))
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
import _guest_restart_fixture as fixture
import sdk_guest_faults as probe


class RestartFixtureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.guest = {'manifest_sha256': 'a'*64}
        self.token = 'original-native-' + 'x'*32
        self.secret = dict(token=self.token, sha512=hashlib.sha512(self.token.encode()).hexdigest(),
            env_vars={'SHARED':'create', 'CREATED':'yes'}, template=dict(template_id='rust',
            guest=self.guest, startup=dict(env_vars={'SHARED':'image', 'IMAGE':'yes'},
            user='ubuntu', workdir='/workspace', argv=['/never/replay'])))
        self.path = self.root/'original'
        self.save()

    def save(self):
        self.path.write_text(json.dumps(self.secret))
        self.path.chmod(0o600)

    def test_native_init_preserves_defaults_and_create_overrides_without_startup_replay(self):
        result = json.loads(fixture.original_init(self.path, self.guest, 'rust'))
        self.assertEqual(result, dict(accessToken=self.token, defaultUser='ubuntu',
            defaultWorkdir='/workspace', envVars={'SHARED':'create','CREATED':'yes','IMAGE':'yes'}))
        self.secret['template']['startup'] = None
        self.save()
        self.assertEqual(json.loads(fixture.original_init(self.path, self.guest, 'rust')),
                         dict(accessToken=self.token, defaultUser='root', envVars=self.secret['env_vars']))

    def test_wrong_guest_template_hash_and_shared_file_are_rejected(self):
        for guest, template in (({'manifest_sha256':'b'*64}, 'rust'), (self.guest, 'other')):
            with self.assertRaises(ValueError): fixture.original_init(self.path, guest, template)
        self.secret['token'] = 'public-gateway-' + 'y'*32
        self.save()
        with self.assertRaises(ValueError): fixture.original_init(self.path, self.guest, 'rust')
        self.secret['token'] = self.token
        self.save()
        self.path.chmod(0o644)
        with self.assertRaises(ValueError): fixture.original_init(self.path, self.guest, 'rust')
        self.path.chmod(0o600)
        link = self.root/'alias'
        link.symlink_to(self.path)
        with self.assertRaises(ValueError): fixture.original_init(link, self.guest, 'rust')
        self.path.write_bytes(b'x'*(fixture.LIMIT+1))
        with self.assertRaises(ValueError): fixture.original_init(self.path, self.guest, 'rust')

    def prepare(self, upload_error=None, audit_error=None):
        secrets = self.root/'state/secrets'
        secrets.mkdir(parents=True)
        self.path.rename(secrets/'sb-deadbeef-72')
        deployment = SimpleNamespace(key='host-management-never-export',
            services=Mock(sdk_url='http://127.0.0.1:1'),
            settings=[{'node_state_dir':str(self.root/'state')}],
            native_id=Mock(return_value='sb-deadbeef-72', side_effect=audit_error))
        vm = Mock(sandbox_id='global-opaque-999', _envd_access_token='public-gateway-not-native')
        vm.files.write.side_effect = upload_error
        with patch.object(fixture.Sandbox, 'create', return_value=vm) as create:
            try:
                handoff = fixture.prepare(deployment, self.root, 'rust', self.guest)
            finally:
                self.assertEqual(create.call_count, 1)
                self.assertEqual(create.call_args.kwargs['timeout'], 300)
                vm.kill.assert_not_called()  # Original outer deployment owns cleanup.
                self.assertEqual(json.loads((self.root/'restart-created.json').read_text()),
                                 {'sandbox_id':vm.sandbox_id})
                if audit_error is not None:
                    vm.files.write.assert_not_called()
        return vm, deployment, handoff

    def test_exact_audited_mapping_and_private_same_vm_handoff(self):
        vm, deployment, handoff = self.prepare()
        deployment.native_id.assert_called_once_with('global-opaque-999', index=0)
        deployment.services.wait_ready.assert_called_once()
        vm.files.write.assert_called_once()
        guest_path, content = vm.files.write.call_args.args
        self.assertEqual(json.loads(content)['accessToken'], self.token)
        self.assertEqual(vm.files.write.call_args.kwargs, {'user':'root'})
        self.assertNotIn('public-gateway', content)
        self.assertNotIn('host-management', content)
        self.assertTrue(vm.commands.run.call_args.args[0].startswith('chmod 0600 -- '))
        safe = fixture.load(handoff, 'rust', self.guest)
        self.assertEqual(safe['sandbox_id'], vm.sandbox_id)
        self.assertEqual(safe['init_path'], guest_path)
        self.assertNotIn(self.token, handoff.read_text())
        self.assertNotIn('host-management', handoff.read_text())
        with self.assertRaises(ValueError): fixture.load(handoff, 'other', self.guest)

    def test_unknown_upload_is_not_retried_or_replaced(self):
        with self.assertRaises(OSError): self.prepare(upload_error=OSError('unknown'))
        self.assertFalse((self.root/'restart-fixture.json').exists())

    def test_audit_failure_keeps_original_creation_receipt_without_secret_upload(self):
        with self.assertRaises(ValueError): self.prepare(audit_error=ValueError('mapping unknown'))
        self.assertFalse((self.root/'restart-fixture.json').exists())


class FaultEvidenceTests(unittest.TestCase):
    def test_native_restart_posts_the_original_payload_and_preserves_pid_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = json.dumps(dict(accessToken='native-only', defaultUser='ubuntu',
                                      defaultWorkdir='/workspace', envVars={'ORIGINAL':'yes'})).encode()
            (root/'init.json').write_bytes(payload)
            (root/'restart.json').write_text(json.dumps(dict(daemon={'pid':10,'start_ticks':123},
                command_pid=20, init_path=str(root/'init.json'))))
            (root/'restart-go').write_text('one trigger')
            saved = bytearray(64)
            saved[52+20//8] |= 1 << (20%8)
            saved[44:52] = (1).to_bytes(8, 'little')
            current = bytearray(saved)
            current[44:52] = (2).to_bytes(8, 'little')
            state = Mock()
            state.stat.return_value = SimpleNamespace(st_dev=1, st_ino=2)
            state.read_bytes.side_effect = [bytes(saved), bytes(current)]
            def native_path(value):
                if str(value) == '/opt/sandproof/state':
                    return SimpleNamespace(iterdir=lambda:iter([state]))
                if str(value) == '/proc/10/stat':
                    return SimpleNamespace(read_text=lambda:'10 (daemon) ' + ' '.join(['0']*19+['123']))
                return Path(value)
            native_os = Mock()
            native_os.fork.return_value = 0
            native_os.pidfd_open.side_effect = [100, 200]
            native_select = Mock()
            native_select.select.side_effect = [([100],[],[]), ([],[],[]), ([],[],[])]
            native_signal = Mock(SIGKILL=9)
            connection = Mock()
            connection.getresponse.return_value.status = 204
            with patch.dict(sys.modules, os=native_os, select=native_select, signal=native_signal), \
                    patch('pathlib.Path', side_effect=native_path), \
                    patch.object(http.client, 'HTTPConnection', return_value=connection):
                exec(probe.RESTART.replace('ROOT', repr(str(root))), {})
            args = connection.request.call_args.args
            self.assertEqual(args[:3], ('POST', '/init', payload))
            connection.request.assert_called_once()
            native_signal.pidfd_send_signal.assert_called_once_with(100, 9)
            self.assertEqual(json.loads((root/'restart-result').read_text()),
                {'status':'pass', 'old_process_alive':True, 'same_pid_file':True})

    def test_result_reads_retain_bounded_diagnostics_without_messages_or_retrying_work(self):
        vm = Mock()
        vm.files.read.side_effect = OSError('sensitive message')
        diagnostics = {}
        with patch.object(probe.time, 'monotonic', side_effect=[0, 1, 31]), \
                patch.object(probe.time, 'sleep'), self.assertRaises(AssertionError):
            probe.restart_result(vm, '/private/result', diagnostics)
        self.assertEqual(diagnostics, dict(result_read_errors=2, last_result_read_error='OSError'))
        vm.commands.run.assert_not_called()
        vm.files.write.assert_not_called()

    def exercise(self, *, primary=None, cleanup=None, creation=None, invalid_fixture=None):
        args = SimpleNamespace(api_url='http://127.0.0.1', sandbox_url='http://127.0.0.1',
                               restart_fixture='protected', guest_configuration='config')
        vm = Mock(sandbox_id='original')
        vm.kill.return_value = True
        vm.kill.side_effect = cleanup
        async def observed(*_args, **_kwargs): return {}
        output = io.StringIO()
        with patch.object(probe, 'guest_candidate', return_value=('rust', ({}, {}, 'observe'))), \
                patch.object(fixture, 'load', return_value={'sandbox_id':'original', 'init_path':'private'},
                             side_effect=invalid_fixture), \
                patch.dict(os.environ, SANDPROOF_TEST_API_KEY='private'), \
                patch.object(probe.Path, 'read_text', return_value='{}'), \
                patch.object(probe.Sandbox, 'create', return_value=vm, side_effect=creation) as create, \
                patch.object(probe, 'observe_candidate', side_effect=observed), \
                patch.object(probe, 'prepare_directory'), \
                patch.object(probe, 'lost_start', side_effect=primary), redirect_stdout(output):
            with self.assertRaises(BaseException) as error:
                probe.exercise(args)
        return json.loads(output.getvalue()), error.exception, vm, create

    def test_primary_and_cleanup_failure_survive_in_same_report_and_exception_group(self):
        original = ValueError('secret primary text')
        cleanup = OSError('secret cleanup text')
        report, error, vm, create = self.exercise(primary=original, cleanup=cleanup)
        self.assertIsInstance(error, BaseExceptionGroup)
        self.assertEqual(error.exceptions, (original, cleanup))
        self.assertEqual(report['status'], 'failed')
        row = report['faults'][0]
        self.assertEqual(row['failure_type'], 'ValueError')
        self.assertEqual(row['cleanup'], {'fault':{'error_type':'OSError'}})
        self.assertNotIn('secret', json.dumps(report))
        vm.kill.assert_called_once()
        create.assert_called_once()

    def test_unknown_creation_emits_partial_report_without_create_retry_or_guessed_cleanup(self):
        original = OSError('unknown create')
        report, error, vm, create = self.exercise(creation=original)
        self.assertIs(error, original)
        self.assertEqual(report['faults'][0]['status'], 'failed')
        self.assertEqual(report['faults'][0]['cleanup'], {})
        vm.kill.assert_not_called()
        create.assert_called_once()

    def test_missing_protected_fixture_fails_before_any_create(self):
        report, _, vm, create = self.exercise(invalid_fixture=ValueError('invalid fixture'))
        self.assertEqual(report['faults'], [])
        create.assert_not_called()
        vm.kill.assert_not_called()


if __name__ == '__main__':
    unittest.main()
