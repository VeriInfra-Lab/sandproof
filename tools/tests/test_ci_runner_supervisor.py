import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from unittest.mock import Mock

from tools import ci_github_app as auth
from tools import ci_runner_supervisor as ci
from tools import ci_vm


class FakeAPI:
    def __init__(self):
        self.runners = []
        self.jobs = [{'name': name, 'status': 'queued', 'labels':
                     ['self-hosted', 'Linux', 'X64', 'sandproof-ci-' + role]}
                     for name, role in ci.JOBS.items()]
        self.posts = []
        self.deletes = []
        self.fail_post = False
        self.fail_delete = False

    def request(self, method, path, body=None):
        if method == 'POST':
            self.posts.append(body)
            runner = {'id': len(self.posts), 'name': body['name']}
            self.runners.append(runner)
            if self.fail_post:
                raise auth.APIError('unavailable')
            return {'runner': runner, 'encoded_jit_config': 'b3BhcXVlLWZha2Utaml0LWNvbmZpZw=='}
        if method == 'DELETE':
            if self.fail_delete:
                raise auth.APIError('unavailable')
            identity = int(path.rsplit('/', 1)[1])
            self.deletes.append(identity)
            self.runners = [r for r in self.runners if r['id'] != identity]
            return None
        if '/runners?' in path:
            return {'runners': list(self.runners)}
        if '/jobs?' in path:
            return {'jobs': self.jobs}
        return {'workflow_runs': [{'id': 50, 'workflow_id': 20, 'run_attempt': 2, 'event': 'pull_request'}]
                if 'status=queued' in path else []}


class FakeRuntime:
    def __init__(self):
        self.active = set()
        self.starts = []
        self.cleaned = []
        self.allowed = True
        self.unknown = False
        self.start_reply_lost = False

    def unit_state(self, identity):
        if self.unknown:
            raise ValueError('unknown')
        return identity in self.active, 0

    def admission(self, role, leases):
        return self.allowed

    def start(self, lease, jit_path):
        self.starts.append(lease['id'])
        self.active.add(lease['id'])
        if self.start_reply_lost:
            raise subprocess.TimeoutExpired('systemd-run', 30)

    def cleanup(self, identity):
        if identity in self.active:
            raise ValueError('active')
        self.cleaned.append(identity)


class SupervisorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.api, self.runtime = FakeAPI(), FakeRuntime()
        self.state = ci.State(self.root)
        self.service = ci.Supervisor(self.api, self.state, self.runtime, 20, clock=lambda: 100)

    def test_five_independent_roles_only_once(self):
        self.service.tick()
        self.service.tick()
        self.assertEqual(len(self.api.posts), 5)
        self.assertEqual(len(self.runtime.starts), 5)
        self.assertEqual(set(self.state.leases), set(ci.JOBS.values()))
        self.assertNotIn('encoded_jit', self.state.path.read_text())
        for path in self.root.glob('*.jit'):
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_restart_adopts_active_workers_without_registering(self):
        self.service.tick()
        restarted = ci.Supervisor(self.api, ci.State(self.root), self.runtime, 20)
        restarted.tick()
        self.assertEqual(len(self.api.posts), 5)
        self.assertFalse(self.runtime.cleaned)

    def test_lost_post_response_recovers_by_exact_name(self):
        self.api.fail_post = True
        with self.assertRaises(auth.APIError):
            self.service.tick()
        self.assertEqual(len(self.api.runners), 1)
        self.assertEqual(self.state.leases['rust']['phase'], 'registering')
        self.api.jobs = []
        self.service.tick()
        self.assertFalse(self.state.leases)
        self.assertEqual(self.api.deletes, [1])
        self.assertFalse(self.runtime.starts)

    def test_lost_systemd_reply_adopts_started_unit(self):
        self.runtime.start_reply_lost = True
        with self.assertRaises(subprocess.TimeoutExpired):
            self.service.tick()
        self.runtime.start_reply_lost = False
        self.service.tick()
        self.assertEqual(len(self.api.posts), 5)
        self.assertEqual(len(set(self.runtime.starts)), 5)

    def test_cleanup_unknown_holds_lease_and_budget(self):
        self.service.tick()
        self.runtime.active.clear()
        self.api.fail_delete = True
        with self.assertRaises(auth.APIError):
            self.service.tick()
        self.assertEqual(len(self.state.leases), 5)
        self.assertEqual(len(self.api.posts), 5)
        self.api.fail_delete = False
        self.api.jobs = []
        self.service.tick()
        self.assertFalse(self.state.leases)
        self.assertFalse(list(self.root.glob('*.jit')))

    def test_unknown_worker_status_forbids_replacement(self):
        self.service.tick()
        self.runtime.unknown = True
        with self.assertRaises(ValueError):
            self.service.tick()
        self.assertEqual(len(self.api.posts), 5)

    def test_no_admission_when_resources_reserved_or_paused(self):
        self.runtime.allowed = False
        self.service.tick()
        self.assertFalse(self.api.posts)

    def test_extra_privileged_labels_and_other_jobs_not_accepted(self):
        job = self.api.jobs[0]
        self.assertEqual(ci.role_for_job(job), 'rust')
        for change in ({'labels': job['labels'] + ['kvm']}, {'name': 'real-89'}, {'status': 'in_progress'}):
            self.assertIsNone(ci.role_for_job(dict(job, **change)))

    def test_other_workflow_rejected(self):
        self.service.workflow_id = 999
        with self.assertRaises(ValueError):
            self.service.tick()
        self.assertFalse(self.api.posts)

    def test_untracked_or_foreign_runner_not_deleted(self):
        self.api.runners = [{'id': 100, 'name': 'someone-elses-runner'}]
        self.service.tick()
        self.assertFalse(self.api.deletes)
        self.api.runners.append({'id': 101, 'name': ci.runner_name('rust', 'a' * 32)})
        with self.assertRaises(ValueError):
            self.service.tick()
        self.assertFalse(self.api.deletes)

    def test_state_round_trip_and_unknown_role_fail_closed(self):
        self.service.tick()
        self.assertEqual(ci.State(self.root).leases, self.state.leases)
        ci.atomic_json(self.state.path, {'bootstrap': {}})
        with self.assertRaises(ValueError):
            ci.State(self.root)

    def test_new_attempt_can_get_fresh_vm_after_cleanup(self):
        self.service.tick()
        old = set(self.runtime.active)
        self.runtime.active.clear()
        self.service.tick()
        self.assertEqual(len(self.api.posts), 10)
        self.assertFalse(old & self.runtime.active)
        self.assertEqual(len(self.state.leases), 5)


class BoundedDiagnosticsTests(unittest.TestCase):
    def test_retains_active_and_unknown_entries_and_caps_completed(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            active = 'a' * 32
            for value in ('b', 'c', 'd', 'a'):
                directory = root / (value * 32)
                directory.mkdir()
                (directory / 'console.log').write_bytes(b'x' * 10)
            unknown = root / ('e' * 32)
            unknown.mkdir()
            (unknown / 'work.qcow2').write_bytes(b'private work')
            with patch('tools.ci_runner_supervisor.os.geteuid', return_value=0):
                # Ownership is faked only for this portable fixture, not runtime.
                real_lstat = Path.lstat
                def owned(path):
                    info = real_lstat(path)
                    return os.stat_result(tuple(info[:4]) + (0,) + tuple(info[5:]))
                with patch.object(Path, 'lstat', owned):
                    ci.prune_diagnostics(root, {active}, limit_count=1, limit_bytes=10)
            self.assertTrue((root / active).exists())
            self.assertTrue(unknown.exists())
            self.assertEqual(len(list(root.iterdir())), 3)

    def test_pipe_output_capped_without_capping_guest_disk(self):
        with tempfile.TemporaryDirectory() as temp:
            process = subprocess.Popen([sys.executable, '-c',
                'import sys; sys.stdout.buffer.write(b"x" * 10000)'],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
            try:
                with self.assertRaises(ValueError):
                    ci_vm.wait_with_bounded_logs(process, Path(temp), 10, limit=1024)
                self.assertEqual((Path(temp) / 'console.log').stat().st_size, 1024)
            finally:
                ci_vm.stop_owned(process)
                process.stdout.close()
                process.stderr.close()


class RuntimeTests(unittest.TestCase):
    def test_start_unit_has_only_jit_and_own_bounded_process_group(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'runs').mkdir()
            lease = {'id': 'f' * 32, 'role': 'title'}
            with patch.object(ci, 'ROOT', root), patch.object(ci.subprocess, 'run') as run:
                ci.Runtime().start(lease, root / 'private.jit')
            argv = run.call_args.args[0]
            joined = ' '.join(argv)
            self.assertIn('KillMode=control-group', joined)
            self.assertIn('MemoryMax=1280M', joined)
            self.assertIn('RuntimeMaxSec=1900', joined)
            self.assertIn('LoadCredential=jit:', joined)
            for secret in ('app-key', 'Iv23', 'fake-token'):
                self.assertNotIn(secret, joined)

    def test_systemd_unknown_not_treated_as_dead(self):
        result = Mock(returncode=1, stdout='')
        with patch.object(ci.subprocess, 'run', return_value=result):
            with self.assertRaises(ValueError):
                ci.Runtime().unit_state('a' * 32)
        result = Mock(returncode=0, stdout='LoadState=not-found\nActiveState=inactive\n')
        with patch.object(ci.subprocess, 'run', return_value=result):
            self.assertEqual(ci.Runtime().unit_state('a' * 32), (False, 0))

    def test_admission_accounts_for_memory_not_yet_resident_and_disk(self):
        runtime = ci.Runtime()
        with patch.object(ci.Path, 'read_text', return_value='MemAvailable: 8192000 kB\n'), \
                patch.object(ci.shutil, 'disk_usage', return_value=Mock(free=100 * 1024 ** 3)), \
                patch.object(ci.Path, 'exists', return_value=False), \
                patch.object(runtime, 'unit_state', return_value=(True, 0)):
            self.assertTrue(runtime.admission('rust', {}))
            lease = {'id': 'a' * 32, 'role': 'rust'}
            self.assertFalse(runtime.admission('sdk', {'rust': lease}))
        with patch.object(ci.Path, 'exists', return_value=True):
            self.assertFalse(runtime.admission('rust', {}))

    def test_identity_path_traversal_refused(self):
        for value in ('../outside', 'A' * 32, 'a' * 31, 'a' * 32 + '/x'):
            with self.assertRaises(ValueError):
                ci_vm.valid_run_id(value)


class AppTests(unittest.TestCase):
    def app(self, transport, clock=lambda: 1000):
        app = auth.GitHubApp('client', Path('/not-read'), transport, clock=clock)
        app.jwt = lambda: 'fake-jwt'
        return app

    def test_scope_verified_then_tokens_restricted_and_refreshed(self):
        calls = []
        now = [1000]
        def transport(method, path, token, body=None):
            calls.append((method, path, body))
            if path == '/app':
                return {'id': 123}
            if '/app/installations?' in path:
                return [{'id': 5, 'account': {'login': 'ningZhang-cs'}, 'repository_selection': 'selected',
                         'permissions': auth.PERMISSIONS}]
            if 'access_tokens' in path:
                return {'token': 'fake-token', 'permissions': auth.PERMISSIONS, 'expires_at': '1970-01-01T01:00:00Z'}
            return {'total_count': 1, 'repositories': [{'id': 6, 'full_name': auth.REPOSITORY}]}
        app = self.app(transport, clock=lambda: now[0])
        app.verify_installation()
        app.access_token()
        app.access_token()
        self.assertEqual(len([c for c in calls if 'access_tokens' in c[1]]), 2)
        self.assertEqual(calls[-1][2]['repository_ids'], [6])
        now[0] = 3500
        app.access_token()
        self.assertEqual(len([c for c in calls if 'access_tokens' in c[1]]), 3)

    def test_non_ci_api_and_unverified_tokens_refused(self):
        app = self.app(lambda *a: None)
        with self.assertRaises(ValueError):
            app.access_token()
        with self.assertRaises(ValueError):
            app.request('POST', '/repos/ningZhang-cs/sandproof/pulls')

    def test_permission_expansion_and_all_repositories_refused(self):
        for change in ({'permissions': dict(auth.PERMISSIONS, contents='write')}, {'repository_selection': 'all'}):
            def transport(method, path, token, body=None):
                if path == '/app':
                    return {'id': 123}
                return [dict({'id': 5, 'account': {'login': 'ningZhang-cs'},
                              'repository_selection': 'selected', 'permissions': auth.PERMISSIONS}, **change)]
            with self.assertRaises(ValueError):
                self.app(transport).verify_installation()

    def test_wrong_repository_refused(self):
        def transport(method, path, token, body=None):
            if path == '/app':
                return {'id': 123}
            if '/app/installations?' in path:
                return [{'id': 5, 'account': {'login': 'ningZhang-cs'}, 'repository_selection': 'selected',
                         'permissions': auth.PERMISSIONS}]
            if 'access_tokens' in path:
                return {'token': 'fake'}
            return {'total_count': 2, 'repositories': [{'full_name': auth.REPOSITORY}]}
        with self.assertRaises(ValueError):
            self.app(transport).verify_installation()

    def test_failed_mutation_not_blindly_retried(self):
        calls = []
        def transport(*args):
            calls.append(args)
            raise auth.APIError('unavailable')
        app = self.app(transport)
        app.access_token = lambda: 'fake'
        with self.assertRaises(auth.APIError):
            app.request('POST', ci.PREFIX + 'runners/generate-jitconfig', {})
        self.assertEqual(len(calls), 1)

    def test_key_permissions_and_symlink_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            key = Path(temp) / 'key'
            key.write_text('fake key')
            key.chmod(0o644)
            with self.assertRaises(ValueError):
                auth.private_key(key)
            key.chmod(0o600)
            self.assertEqual(auth.private_key(key), b'fake key')
            link = Path(temp) / 'link'
            link.symlink_to(key)
            with self.assertRaises(ValueError):
                auth.private_key(link)


if __name__ == '__main__':
    unittest.main()
