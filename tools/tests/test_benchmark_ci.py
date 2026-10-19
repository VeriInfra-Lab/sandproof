import copy
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path
import tempfile
import unittest
import venv
from unittest.mock import Mock, patch

from tools import benchmark_ci as bench
from tools import ci_e2e_supervisor as ci
from tools import ci_e2e_artifacts as service_artifacts
from tools.service_storage_observation import ServiceStoragePeaks
from tools.ci_github_app import APIError
from tools.tests.storage_observation_fixture import declaration, observation

ROOT = Path(__file__).resolve().parents[2]


class BenchmarkCITests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.root.chmod(0o700)
        self.lock = json.loads((ROOT / 'benchmarks/terminal-bench-2.1.lock.json').read_text())
        self.manifest = json.loads((ROOT / 'benchmarks/terminal-bench-2.1-suites.json').read_text())
        self.names = [t['name'] for t in self.manifest['profiles']['smoke']['tasks']]
        self.policy = {'profile':'smoke', 'batch_size':4}
        self.services_manifest = dict(schema_version=1, architecture=service_artifacts.ARCHITECTURE,
            candidate='a'*40, versions=dict(internal_protocol=2, center_store=6, host_store=16),
            artifacts={role:dict(path='/fixed/' + name, sha256=hash_value*64)
                       for role, name, hash_value in (('center','sandproof-center','a'),
                                                      ('host','sandproof-host','d'))})
        self.dk=dict(schema_version=1,candidate='a'*40,execution_manifest_sha256='9'*64,
            service_storage=observation(),service_data=declaration(),
            cases=[dict(id='DK'+str(number).zfill(2),status='pass',exact_cleanup='passed',
                execution_manifest_sha256='9'*64,evidence_sha256=str(number)*64) for number in range(1,7)])

    def batch_result(self, root, **kwargs):
        return bench.batch_result(root, service_manifest=self.services_manifest,dk_evidence=self.dk,
            expected_run_id='b'*64,expected_candidate_manifest_sha256='c'*64,**kwargs)

    def test_profile_scope_and_configurable_batches(self):
        batches = bench.selection(self.lock, self.manifest, self.policy)
        self.assertEqual([len(b) for b in batches], [4, 4])
        self.assertEqual(sum(batches, []), self.names)
        self.assertEqual([len(b) for b in bench.selection(self.lock, self.manifest,
            dict(self.policy, batch_size=3))], [3,3,2])
        self.assertEqual(len(sum(bench.selection(self.lock, self.manifest,
            dict(self.policy, profile='regression')), [])), 16)
        full = bench.selection(self.lock, self.manifest, dict(self.policy, profile='full'))
        self.assertEqual(sum(full, []), [t['name'] for t in self.lock['tasks']])
        self.assertEqual(len(full[-1]), len(self.lock['tasks']) % 4)

    def test_custom_selection_invalid_scopes_and_batch_boundaries(self):
        self.assertEqual(bench.selection(self.lock, self.manifest,
            {'tasks':self.names[:3], 'batch_size':2}), [self.names[:2], self.names[2:3]])
        for policy in ({'tasks':[], 'batch_size':1}, {'tasks':['missing'], 'batch_size':1},
                       {'tasks':self.names[:1]*2, 'batch_size':1},
                       {'profile':'unknown', 'batch_size':1}, dict(self.policy, tasks=self.names),
                       *[dict(self.policy, batch_size=n) for n in (0,-1,True,1.5,len(self.lock['tasks'])+1)]):
            with self.assertRaises(ValueError):
                bench.selection(self.lock, self.manifest, policy)
        for key,value in (('dataset','terminal-bench@2.0'), ('source_commit','a'*40), ('schema_version',1)):
            with self.assertRaises(ValueError):
                bench.selection(self.lock, dict(self.manifest, **{key:value}), self.policy)

    def test_partial_counts_do_not_raise_resource_ceilings(self):
        profile = {'public_build':{'max_jobs':4, 'max_templates':4, 'workspace_bytes':64000,
                                   'registry_bytes':40000}}
        template = {role:{'bounded_host':{'physical_storage':copy.deepcopy(profile)}}
                    for role in ('runtime','builder')}
        for count in (1,2,3,4):
            policy = bench.cohort_policy(template, count)
            for role in ('runtime','builder'):
                public = policy[role]['bounded_host']['physical_storage']['public_build']
                self.assertEqual(public['max_jobs'], count)
                self.assertEqual(public['max_templates'], count)
                self.assertEqual(public['workspace_bytes'], 64000)
                self.assertEqual(public['registry_bytes'], 40000)
        self.assertEqual(template['runtime']['bounded_host']['physical_storage'], profile)
        for count in (0,True,5):
            with self.assertRaises(ValueError):
                bench.cohort_policy(template, count)

    def test_exact_authorization_and_base_config_identity(self):
        bench.authorize('a'*40, approved_commits=['a'*40])
        for candidate in ('b'*40, 'test', '../external'):
            with self.assertRaises(ValueError):
                bench.authorize(candidate, approved_commits=['a'*40])
        identity = bench.run_identity('a'*40, 'b'*40, {}, self.manifest, self.lock)
        self.assertNotEqual(identity, bench.run_identity('a'*40, 'c'*40, {}, self.manifest, self.lock))

    def test_promotion_authority_is_exact_and_only_legal_protected_directions(self):
        for source, target in (('integration', 'test'), ('test', 'main'), ('main', 'test')):
            bench.authorize('a'*40, approved_commits=[], observed_promotion=(source, target, 'a'*40))
        for authority in (('feature', 'test', 'a'*40), ('main', 'integration', 'a'*40),
                          ('integration', 'main', 'a'*40), ('test', 'main', 'b'*40),
                          ['test', 'main', 'a'*40], ('test', 'main')):
            with self.subTest(authority=authority), self.assertRaises(ValueError):
                bench.authorize('a'*40, approved_commits=[], observed_promotion=authority)
        supervisor = self.supervisor()
        with self.assertRaisesRegex(ValueError, 'not enabled'):
            supervisor.begin('a'*40, 'b'*40, observed_promotion=('integration', 'test', 'a'*40))

    def test_promotion_selector_advances_both_gates_without_repeating_old_heads(self):
        floor, integration, test, main = ('a'*40, 'b'*40, 'c'*40, 'd'*40)
        refs = dict(integration=integration, test=test, main=main)
        relationships = {(floor, integration), (test, integration), (main, test), (main, integration)}
        ancestor = lambda parent, child: parent == child or (parent, child) in relationships
        self.assertEqual(ci.select_promotion(refs, floor, ancestor),
                         ('integration', 'test', integration, test))
        promoted = 'e'*40
        relationships.update({(floor, promoted), (integration, promoted), (main, promoted)})
        self.assertEqual(ci.select_promotion(dict(refs, test=promoted), floor, ancestor),
                         ('test', 'main', promoted, main))
        finished = 'f'*40
        relationships.update({(floor, finished), (integration, finished), (promoted, finished)})
        self.assertIsNone(ci.select_promotion(dict(refs, test=promoted, main=finished), floor, ancestor,
                                              ready=lambda candidate: False))

    def test_promotion_selector_rejects_unrelated_base_and_unknown_policy(self):
        refs = dict(integration='b'*40, test='c'*40, main='d'*40)
        self.assertIsNone(ci.select_promotion(refs, 'a'*40, lambda parent, child: parent == 'a'*40))
        self.assertIsNone(ci.select_promotion(refs, 'a'*40, lambda parent, child: False))
        for values, floor in ((dict(refs, feature='e'*40), 'a'*40),
                              ({'test':'c'*40, 'main':'d'*40}, 'a'*40),
                              (dict(refs, integration='integration'), 'a'*40), (refs, 'main')):
            with self.subTest(values=values), self.assertRaises(ValueError):
                ci.select_promotion(values, floor, lambda parent, child: True)

    def test_reverse_sync_and_ready_queue_do_not_deadlock_promotions(self):
        floor, integration, test, main = ('a'*40, 'b'*40, 'c'*40, 'd'*40)
        refs = dict(integration=integration, test=test, main=main)
        pairs = {(floor, integration), (floor, test), (floor, main),
                 (test, main)}  # integration diverged and needs the legal sync.
        ancestor = lambda parent, child: parent == child or (parent, child) in pairs
        self.assertEqual(ci.select_promotion(refs, floor, ancestor, ready=lambda candidate: candidate == main),
                         ('main', 'test', main, test))
        self.assertIsNone(ci.select_promotion(refs, floor, ancestor, ready=lambda candidate: False))
        pairs = {(floor, integration), (test, integration), (floor, test), (main, test)}
        ancestor = lambda parent, child: parent == child or (parent, child) in pairs
        self.assertEqual(ci.select_promotion(refs, floor, ancestor, ready=lambda candidate: candidate == test),
                         ('test', 'main', test, main))

    def test_promotion_ref_inventory_uses_fixed_repository_and_rejects_duplicates(self):
        lines = '\n'.join(sha*40 + '\trefs/heads/' + name for name, sha in
                          (('integration', 'a'), ('test', 'b'), ('main', 'c')))
        client = Mock()
        with patch.object(ci, 'git', return_value=lines) as query:
            refs = ci.promotion_heads(source_client=client, proxy='http://127.0.0.1:7897')
        self.assertEqual(refs, dict(integration='a'*40, test='b'*40, main='c'*40))
        self.assertEqual(query.call_args.args, ('ls-remote', ci.REPO,
                         'refs/heads/integration', 'refs/heads/main', 'refs/heads/test'))
        for output in (lines + '\n' + lines.splitlines()[0], lines.splitlines()[0],
                       lines.replace('refs/heads/integration', 'refs/heads/feature'),
                       lines.replace('a'*40, 'not-a-sha')):
            with patch.object(ci, 'git', return_value=output), self.assertRaises(ValueError):
                ci.promotion_heads(source_client=client, proxy=None)

    def test_unknown_git_ancestry_is_not_treated_as_already_promoted(self):
        with patch.object(ci, 'git', return_value=''):
            self.assertTrue(ci.is_ancestor('a'*40, 'b'*40, cwd=self.root))
        with patch.object(ci, 'git', side_effect=subprocess.CalledProcessError(1, 'git')):
            self.assertFalse(ci.is_ancestor('a'*40, 'b'*40, cwd=self.root))
        with patch.object(ci, 'git', side_effect=subprocess.CalledProcessError(128, 'git')):
            with self.assertRaises(subprocess.CalledProcessError):
                ci.is_ancestor('a'*40, 'b'*40, cwd=self.root)

    def test_promotion_waits_for_latest_ordinary_checks_from_approved_app(self):
        api = Mock(app_id=5175756)
        rows = [dict(id=index, name=name, head_sha='a'*40, app={'id':15368},
                     status='completed', conclusion='success') for index, name in enumerate(
                     ('sdk-contract', 'harbor-inputs', 'pr-title', 'pr-policy'), start=1)]
        api.request.return_value = dict(total_count=4, check_runs=rows)
        checks = ci.Checks(api)
        self.assertTrue(checks.ordinary_ready('a'*40, 15368))
        for change in ({'status':'in_progress', 'conclusion':None}, {'conclusion':'failure'},
                       {'conclusion':'cancelled'}, {'app':{'id':99}}, {'head_sha':'b'*40}):
            with self.subTest(change=change):
                modified = copy.deepcopy(rows)
                modified[0].update(change)
                api.request.return_value = dict(total_count=4, check_runs=modified)
                self.assertFalse(checks.ordinary_ready('a'*40, 15368))
        newer = dict(rows[0], id=10, status='queued', conclusion=None)
        api.request.return_value = dict(total_count=5, check_runs=rows+[newer])
        self.assertFalse(checks.ordinary_ready('a'*40, 15368))
        api.request.return_value = dict(total_count=101, check_runs=rows)
        with self.assertRaises(ValueError):
            checks.ordinary_ready('a'*40, 15368)
        for app in (None, True, 0, '15368'):
            with self.assertRaises(ValueError):
                checks.ordinary_ready('a'*40, app)

    def test_waiting_promotion_does_not_pause_or_cancel_ordinary_ci(self):
        checks = Mock()
        checks.ordinary_ready.return_value = False
        supervisor = self.supervisor(checks=checks, phase='waiting', batch_phase='new')
        supervisor.config['ordinary_check_app_id'] = 15368
        supervisor.state['promotion'] = dict(source='integration', target='test')
        with patch.object(ci.HostLease, 'pause') as pause, patch.object(ci, 'launch') as launch:
            supervisor.tick()
        checks.ordinary_ready.assert_called_once_with('a'*40, 15368)
        pause.assert_not_called()
        launch.assert_not_called()
        self.assertEqual(supervisor.state['phase'], 'waiting')

    def test_private_source_token_is_ephemeral_and_not_in_command_arguments(self):
        client = Mock()
        client.access_token.return_value = 'PRIVATE-SOURCE-TOKEN'
        with patch.object(ci.subprocess, 'check_output', return_value='a'*40) as call:
            ci.git('ls-remote', ci.REPO, source_client=client, proxy='http://127.0.0.1:7897')
        argv = call.call_args.args[0]
        self.assertNotIn('PRIVATE-SOURCE-TOKEN', str(argv))
        env = call.call_args.kwargs['env']
        self.assertEqual(env['GIT_CONFIG_KEY_0'], 'http.https://github.com/.extraheader')
        self.assertEqual(env['GIT_TERMINAL_PROMPT'], '0')
        with patch.object(ci.subprocess, 'check_output', return_value='') as call:
            ci.git('status', '--porcelain')
        self.assertNotIn('GIT_CONFIG_VALUE_0', call.call_args.kwargs['env'])

    def report(self, *, service_data=None, service_storage=None):
        data = dict(commit='a'*40, dataset='terminal-bench@2.1', fixed_task_count=89,
            service_storage=observation() if service_storage is None else service_storage,
            oracle_mode='original', original_verifier=True, original_oracle_verifier=True,
            services=service_artifacts.public_identity(self.services_manifest),
            service_boundary=dict(public_api='center', center_unprivileged=True,
                redis_authority='validated', host_transport='mutual-tls', host_public_management=False),
            kernel_sha256='b'*64, builder_base_sha256='c'*64,
            task_source=dict(repository=bench.PIN21.repository, commit=bench.PIN21.commit),
            source_kind='dockerfile', original_environment=True, paid_model_used=False,
            no_process_network_cgroup_jail_residue=True,
            tasks=[dict(name=n, status='passed', oracle='passed', verifier='passed', oracle_mode='original',
                        exact_cleanup='passed', rewards={'reward':1}, private_key='NEVER EXPORT',
                        template={'kernelSha256':'b'*64, 'rootfsSha256':'d'*64}) for n in self.names[:4]])
        def role(uid,inode,checksum):
            return dict(uid=uid,gid=uid,config_sha256=checksum*64,
                directory=dict(device=1,inode=inode,uid=uid,gid=uid,mode=0o700))
        data['execution_manifest']=service_artifacts.execution_manifest(self.services_manifest,
            run_id='b'*64,candidate_manifest_sha256='c'*64,
            roles=dict(center=role(1001,1,'a'),redis=role(1002,2,'b'),host=[role(0,3,'c')]),
            redis=dict(service_artifacts.REDIS_PIN,binary_sha256='d'*64,binary_bytes=1000),
            tls_sha256='e'*64,budget_sha256='f'*64,
            service_data=declaration() if service_data is None else service_data)
        data['service_processes']={name:dict(pid=index+10,start_time_ticks=100,uid=uid,gid=uid,
            executable=dict(device=1,inode=index+20)) for index,(name,uid) in
            enumerate((('center',1001),('redis',1002),('host-0',0)))}
        (self.root/'execution-manifest.json').write_text(json.dumps(data['execution_manifest']))
        (self.root/'execution-manifest.json').chmod(0o600)
        (self.root / 'report.json').write_text(json.dumps(data))
        (self.root / 'service-cleanup.json').write_text('{"removed":true}')
        (self.root / 'proxy-cleanup.json').write_text('{"phase":"removed"}')
        return data

    def test_legacy_compatibility_catalog_splits_modes_and_binds_original_hashes(self):
        lock = json.loads((ROOT / 'benchmarks/terminal-bench-2.0.lock.json').read_text())
        catalog = json.loads((ROOT / 'benchmarks/terminal-bench-2.0-suites.json').read_text())
        batches = bench.selection(lock, catalog, self.policy)
        self.assertEqual(batches, [['polyglot-c-py', 'regex-log', 'db-wal-recovery'], ['build-pmars']])
        modes = bench.oracle_modes(lock, catalog)
        self.assertEqual(modes['build-pmars'], 'repaired')
        self.assertEqual(modes['regex-log'], 'original')
        self.assertEqual(bench.check_name(lock['dataset']), 'E2E / terminal-bench-2.0-compat')
        self.assertEqual(bench.selection(lock, catalog, {'profile':'ci', 'batch_size':1}),
                         [['polyglot-c-py'], ['db-wal-recovery']])
        self.assertEqual(bench.check_name(lock['dataset'], source_kind='prepared-ci'),
                         'E2E / terminal-bench-ci')
        with self.assertRaises(ValueError):
            bench.check_name('terminal-bench@2.1', source_kind='prepared-ci')
        damaged = copy.deepcopy(lock)
        task = next(t for t in damaged['tasks'] if t['name'] == 'build-pmars')
        next(f for f in task['files'] if f['path'] == 'solution/solve.sh')['sha256'] = 'f'*64
        with self.assertRaises(ValueError):
            bench.selection(damaged, catalog, self.policy)
        with self.assertRaises(ValueError):
            bench.selection(self.lock, dict(self.manifest, oracle_modes={'build-pmars':'repaired'}), self.policy)
        for modes in ({'not-a-task':'repaired'}, {'regex-log':'repaired'}, {'build-pmars':'silent'}):
            with self.assertRaises(ValueError):
                bench.selection(lock, dict(catalog, oracle_modes=modes), self.policy)

    def test_compatibility_results_never_claim_original_or_cross_version_success(self):
        from tools.benchmark_tasks import PIN
        report = self.report()
        report.update(dataset='terminal-bench@2.0', source_kind='oci', oracle_mode='repaired',
                      original_oracle_verifier=False, task_source=dict(repository=PIN.repository, commit=PIN.commit))
        for row in report['tasks']:
            row['oracle_mode'] = 'repaired'
        (self.root / 'report.json').write_text(json.dumps(report))
        args = dict(candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=89,
                    dataset='terminal-bench@2.0', source_kind='oci', oracle_mode='repaired')
        batch = self.batch_result(self.root, **args)
        result = bench.aggregate('a'*40, [batch], self.names[:4], dataset='terminal-bench@2.0')
        self.assertTrue(result['passed'])
        self.assertFalse(result['original_oracle_verifier'])
        self.assertFalse(result['full_suite_passed'])
        self.assertEqual(result['baseline'], 'compatibility regression')
        with self.assertRaises(ValueError):
            bench.aggregate('a'*40, [batch], self.names[:4])
        with self.assertRaises(ValueError):
            self.batch_result(self.root, **dict(args, oracle_mode='original'))
        report['tasks'][0]['oracle_mode'] = 'original'
        (self.root / 'report.json').write_text(json.dumps(report))
        with self.assertRaises(ValueError):
            self.batch_result(self.root, **args)

    def prepared_report(self):
        from tools.benchmark_tasks import PIN
        report = self.report()
        report.update(dataset='terminal-bench@2.0', source_kind='prepared-ci', fixture_sha256='e'*64,
            task_source=dict(repository=PIN.repository, commit=PIN.commit),
            original_environment=False, original_verifier=False, original_oracle_verifier=False,
            original_test_assertions=True, original_oracle=True,
            prepared_ci_physical={'disk_budget_bytes':200 * 1024**3, 'activation_bytes':100 * 1024**3,
                'pool_roles':['runtime'], 'builder_dispatched':False, 'public_build':False})
        for row in report['tasks']:
            row.update(guest_preflight='passed', verifier_mode=bench.CI_VERIFIER_MODE,
                verifier_variant=dict(mode=bench.CI_VERIFIER_MODE, source_launcher_sha256=bench.BOOTSTRAP_SHA256,
                    launcher_sha256=bench.hashlib.sha256(bench.LAUNCHER.encode()).hexdigest(),
                    requirements_sha256=bench.REQUIREMENTS_SHA256))
        (self.root / 'report.json').write_text(json.dumps(report))
        (self.root / 'proxy-cleanup.json').unlink()
        return report

    def prepared_args(self):
        return dict(candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=89,
                    dataset='terminal-bench@2.0', source_kind='prepared-ci', expected_fixture_sha256='e'*64)

    def test_prepared_ci_accepts_only_pinned_local_grading_and_exact_cleanup(self):
        original = self.prepared_report()
        args = self.prepared_args()
        result = self.batch_result(self.root, **args)
        self.assertTrue(result['passed'])
        self.assertEqual(result['fixture_sha256'], 'e'*64)
        self.assertFalse(result['original_oracle_verifier'])
        self.assertFalse(result['original_environment'])
        for field, value in (('disk_budget_bytes', 201 * 1024**3), ('activation_bytes', 200 * 1024**3 + 1),
                             ('activation_bytes', True), ('pool_roles', ['runtime','builder']),
                             ('builder_dispatched', True), ('public_build', True)):
            changed = copy.deepcopy(original)
            changed['prepared_ci_physical'][field] = value
            (self.root / 'report.json').write_text(json.dumps(changed))
            with self.subTest(physical_field=field), self.assertRaises(ValueError):
                self.batch_result(self.root, **args)
        changed = copy.deepcopy(original)
        del changed['prepared_ci_physical']
        (self.root / 'report.json').write_text(json.dumps(changed))
        with self.assertRaises(ValueError):
            self.batch_result(self.root, **args)
        (self.root / 'report.json').write_text(json.dumps(original))
        for field, value in (('fixture_sha256','f'*64), ('original_environment',True),
                             ('original_verifier',True), ('original_oracle_verifier',True),
                             ('original_test_assertions',False), ('original_oracle',False),
                             ('source_kind','oci'), ('paid_model_used',True),
                             ('no_process_network_cgroup_jail_residue',False)):
            with self.subTest(field=field):
                changed = dict(original, **{field:value})
                (self.root / 'report.json').write_text(json.dumps(changed))
                with self.assertRaises(ValueError):
                    self.batch_result(self.root, **args)
        for field in ('fixture_sha256', 'original_test_assertions', 'original_oracle'):
            changed = dict(original)
            changed.pop(field)
            (self.root / 'report.json').write_text(json.dumps(changed))
            with self.assertRaises(ValueError):
                self.batch_result(self.root, **args)
        (self.root / 'report.json').write_text(json.dumps(original))
        for change in ({'expected_fixture_sha256':None}, {'expected_fixture_sha256':'invalid'},
                       {'expected_fixture_sha256':True}, {'oracle_mode':'repaired'},
                       {'dataset':'terminal-bench@2.1'}, {'source_kind':'oci'}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.batch_result(self.root, **dict(args, **change))
        (self.root / 'service-cleanup.json').write_text('{"removed":false}')
        with self.assertRaises(ValueError):
            self.batch_result(self.root, **args)
        (self.root / 'service-cleanup.json').write_text('{"removed":true}')
        original['tasks'][0].pop('template')
        (self.root / 'report.json').write_text(json.dumps(original))
        with self.assertRaises(ValueError):
            self.batch_result(self.root, **args)

    def test_prepared_ci_failure_remains_failed_and_does_not_waive_template_identity(self):
        report = self.prepared_report()
        row = report['tasks'][0]
        row.update(status='failed', oracle='passed', verifier='failed', rewards={'reward':0})
        (self.root / 'report.json').write_text(json.dumps(report))
        result = self.batch_result(self.root, **dict(self.prepared_args(), exit_code=1))
        self.assertFalse(result['passed'])
        self.assertEqual(result['cleanup'], 'passed')
        self.assertEqual(result['tasks'][0]['verifier'], 'failed')
        row.update(template=None, build={'state':'Failed'}, create_dispatched=False, sandbox_id=None)
        (self.root / 'report.json').write_text(json.dumps(report))
        with self.assertRaises(ValueError):
            self.batch_result(self.root, **dict(self.prepared_args(), exit_code=1))

    def test_matching_incoming_cohort_cannot_replace_original_supervisor_identity(self):
        # Both incoming copies agree, and the candidate, binaries, role facts,
        # DK and task/cleanup evidence are otherwise unchanged. They still
        # cannot cover the original supervisor's independently retained intent.
        for field,value in (('run_id','e'*64),('candidate_manifest_sha256','f'*64)):
            with self.subTest(field=field):
                report=self.prepared_report()
                report['execution_manifest'][field]=value
                (self.root/'execution-manifest.json').write_text(json.dumps(report['execution_manifest']))
                (self.root/'report.json').write_text(json.dumps(report))
                with self.assertRaisesRegex(ValueError,'original supervised run'):
                    self.batch_result(self.root,**self.prepared_args())

    def test_missing_or_extra_execution_fields_reject_even_when_incoming_copies_agree(self):
        for field in ('run_id','candidate_manifest_sha256','unreviewed_cohort'):
            with self.subTest(field=field):
                report=self.prepared_report()
                if field=='unreviewed_cohort':
                    report['execution_manifest'][field]='e'*64
                else:
                    report['execution_manifest'].pop(field)
                (self.root/'execution-manifest.json').write_text(json.dumps(report['execution_manifest']))
                (self.root/'report.json').write_text(json.dumps(report))
                with self.assertRaises(ValueError):
                    self.batch_result(self.root,**self.prepared_args())

    def test_prepared_ci_each_guest_preflight_and_verifier_identity_is_required(self):
        original = self.prepared_report()
        args = self.prepared_args()
        for field, values in (('guest_preflight', [None, 'not_run', 'failed', True]),
                              ('verifier_mode', [None, 'original', True]),
                              ('verifier_variant', [None, [], 'unchecked'])):
            for value in values:
                with self.subTest(field=field, value=value):
                    changed = copy.deepcopy(original)
                    if value is None:
                        changed['tasks'][0].pop(field)
                    else:
                        changed['tasks'][0][field] = value
                    (self.root / 'report.json').write_text(json.dumps(changed))
                    with self.assertRaises(ValueError):
                        self.batch_result(self.root, **args)
        for field in ('mode', 'source_launcher_sha256', 'launcher_sha256', 'requirements_sha256'):
            for value in (None, 'f'*64, True):
                with self.subTest(variant_field=field, value=value):
                    changed = copy.deepcopy(original)
                    if value is None:
                        changed['tasks'][-1]['verifier_variant'].pop(field)
                    else:
                        changed['tasks'][-1]['verifier_variant'][field] = value
                    (self.root / 'report.json').write_text(json.dumps(changed))
                    with self.assertRaises(ValueError):
                        self.batch_result(self.root, **args)
        # Paths, file inventories and unreviewed extra sidecar fields stay private.
        original['tasks'][0]['verifier_variant'].update(private_path='NEVER EXPORT', original_files=['NEVER EXPORT'])
        original['tasks'][0]['verifier_variant_manifest_sha256'] = 'd'*64
        (self.root / 'report.json').write_text(json.dumps(original))
        result = self.batch_result(self.root, **args)
        self.assertNotIn('NEVER EXPORT', json.dumps(result))
        row = result['tasks'][0]
        self.assertEqual(row['guest_preflight'], 'passed')
        self.assertEqual(row['verifier_mode'], bench.CI_VERIFIER_MODE)
        self.assertEqual(set(row['verifier_variant']),
            {'mode', 'source_launcher_sha256', 'launcher_sha256', 'requirements_sha256'})
        self.assertNotIn('verifier_variant_manifest_sha256', row)
        self.assertTrue(result['passed'])
        row['verifier_variant']['launcher_sha256'] = 'f'*64
        with self.assertRaises(ValueError):
            bench.aggregate('a'*40, [result], self.names[:4], dataset='terminal-bench@2.0')

    def test_prepared_ci_aggregate_never_claims_original_or_full_suite_and_rejects_mixing(self):
        self.prepared_report()
        batch = self.batch_result(self.root, **self.prepared_args())
        names = self.names[:4]
        result = bench.aggregate('a'*40, [batch], names, all_tasks=names, dataset='terminal-bench@2.0')
        self.assertTrue(result['passed'])
        self.assertFalse(result['original_oracle_verifier'])
        self.assertFalse(result['full_suite_passed'])
        self.assertEqual(result['source_kind'], 'prepared-ci')
        self.assertIn('CI variant', result['scope'])
        failed = bench.aggregate('a'*40, [], names, dataset='terminal-bench@2.0',
            source_kind='prepared-ci', expected_fixture_sha256='e'*64)
        self.assertFalse(failed['passed'])
        self.assertFalse(failed['original_environment'])
        for changed in (dict(batch, source_kind='oci'), dict(batch, fixture_sha256='f'*64),
                        dict(batch, fixture_sha256=None), dict(batch, fixture_sha256={}),
                        dict(batch, original_verifier=True), dict(batch, original_oracle_verifier=True),
                        dict(batch, oracle_mode='repaired')):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                bench.aggregate('a'*40, [batch,changed], names, dataset='terminal-bench@2.0')
        with self.assertRaises(ValueError):
            bench.aggregate('a'*40, [batch], names, dataset='terminal-bench@2.0',
                            source_kind='prepared-ci', expected_fixture_sha256='f'*64)

    def host_config(self):
        from tools.system_acceptance import digest as file_digest
        config = dict(checkout=str(self.root), dataset='terminal-bench@2.0', source_kind='prepared-ci',
            state_root=str(self.root), ci_state_root=str(self.root), scratch_parent=str(self.root),
            host_assets={}, approved_commits=['a'*40], selection={'profile':'ci', 'batch_size':1},
            task_source='/fixed-task-source', tools='/fixed-tools', uid_base=3100000,
            cgroup_parent='/sys/fs/cgroup/runtime', build_cgroup_parent='/sys/fs/cgroup/builder',
            service_cgroup_parent='/sys/fs/cgroup/control', prepare_candidate=True,
            redis_material='/approved-fixed-redis', center_uid=65532, center_gid=65532,
            redis_uid=65533, redis_gid=65533, dk_rounds=6, dk_checkpoint_tail_inputs=128)
        for key in ('cargo_home','uv_cache','rustup_home','cargo_target','toolchain_bin'):
            path = self.root / key
            path.mkdir(mode=0o700, exist_ok=True)
            config[key] = str(path)
        target = Path(config['cargo_target']) / 'release'
        config.update(center=str(target / 'sandproof-center'), host=str(target / 'sandproof-host'),
                      window_helper=str(target / 'sandproof-window-reducer'),
                      stale_probe=str(target / 'examples/stale_host_actions'))
        staging = self.root / 'service-staging'
        staging.mkdir(exist_ok=True)
        staging.chmod(0o711)
        config['service_staging_parent'] = str(staging)
        tls = self.root / 'tls-material'
        tls.mkdir(mode=0o700, exist_ok=True)
        config['tls_material'] = str(tls)
        for name in ('ca.der', 'center.der', 'center-key.der', 'host-runtime.der', 'host-runtime-key.der',
                     'host-builder.der', 'host-builder-key.der'):
            path = tls / name
            path.write_bytes(('fixed-test-only-material-' + name).encode())
            path.chmod(0o600)
            config['host_assets'][str(path)] = file_digest(path)
        config['prepared_ci_disk_budget_bytes'] = 200 * 1024**3
        for key in ('kernel','rootfs','firecracker','jailer','policy_template','budget_profile','ci_fixtures','uv','cargo'):
            path = self.root / (key + '.json')
            path.write_text('{}')
            path.chmod(0o600)
            config[key] = str(path)
            config['host_assets'][str(path)] = file_digest(path)
        return config

    def test_prepared_host_manifest_is_private_fingerprinted_and_mode_is_explicit(self):
        config = self.host_config()
        with patch.object(ci, 'trusted_path', side_effect=lambda path:path), patch.object(ci, 'redis_material'):
            ci.validate_host_inputs(config)  # Prepared mode needs no proxy binary.
            for changed in (dict(config, ci_fixtures=None), dict(config, source_kind='oci'),
                            *[dict(config, dk_rounds=value) for value in (None, 0, True, 1.5, 5, 7, 16)],
                            *[dict(config, dk_checkpoint_tail_inputs=value) for value in (None, 0, True, 15, 129, "128", 128.0)],
                            {key:value for key,value in config.items() if key != 'dk_checkpoint_tail_inputs'},
                            dict(config, prepared_ci_disk_budget_bytes=None),
                            dict(config, prepared_ci_disk_budget_bytes=True),
                            dict(config, prepared_ci_disk_budget_bytes=201 * 1024**3),
                            dict(config, dataset='terminal-bench@2.1'),
                            dict(config, check_name='E2E / terminal-bench-2.0-compat'),
                            dict(config, host_assets={k:v for k,v in config['host_assets'].items()
                                                      if k != config['ci_fixtures']})):
                with self.subTest(changed=changed), self.assertRaises(ValueError):
                    ci.validate_host_inputs(changed)
            fixture = Path(config['ci_fixtures'])
            fixture.write_text('{"changed":true}')
            with self.assertRaises(ValueError):
                ci.validate_host_inputs(config)
            fixture.write_text('{}')
            fixture.chmod(0o644)
            with self.assertRaises(ValueError):
                ci.validate_host_inputs(config)
        old = dict(config, source_kind='oci', proxy_binary=config['kernel'])
        old.pop('ci_fixtures')
        with patch.object(ci, 'trusted_path', side_effect=lambda path:path), patch.object(ci, 'redis_material'):
            ci.validate_host_inputs(old)

    def test_prepared_candidate_tools_remain_pinned_without_product_proxy_fields(self):
        from tools.system_acceptance import digest as file_digest
        config = self.host_config()
        config.update(prepare_candidate=True, candidate='a'*40, proxy='http://127.0.0.1:7897',
                      toolchain_bin=str(self.root))
        for key in ('uv', 'cargo'):
            path = self.root / key
            path.write_text('fixed-host-tool')
            path.chmod(0o600)
            config[key] = str(path)
            config['host_assets'][str(path)] = file_digest(path)
        with patch.object(ci, 'trusted_path', side_effect=lambda path:path), patch.object(ci, 'redis_material'):
            ci.validate_host_inputs(config)
            changed = copy.deepcopy(config)
            changed['host_assets'].pop(config['uv'])
            with self.assertRaisesRegex(ValueError, 'prepared tool fingerprint'):
                ci.validate_host_inputs(changed)
        # Actual offline dual builds are tested once in test_ci_e2e_prepare.
        self.assertNotIn('proxy_binary', config)
        self.assertNotIn('proxy_sha256', config)

    def test_service_policy_rejects_legacy_outputs_shared_or_instance_uids_and_exposed_staging(self):
        config = self.host_config()
        with patch.object(ci, 'trusted_path', side_effect=lambda path:path), patch.object(ci, 'redis_material'):
            for changed in (dict(config, server='/retired-server'), dict(config, prepare_candidate=False),
                            dict(config, center_uid=0), dict(config, center_uid=True),
                            dict(config, redis_uid=config['center_uid']),
                            dict(config, center_uid=config['uid_base']),
                            dict(config, host='/mixed/sandproof-host')):
                with self.subTest(changed=changed), self.assertRaises(ValueError):
                    ci.validate_host_inputs(changed)
            staging = Path(config['service_staging_parent'])
            staging.chmod(0o777)
            with self.assertRaisesRegex(ValueError, 'protected traversable'):
                ci.validate_host_inputs(config)

    def test_preexisting_single_server_state_is_not_adopted_by_new_supervisor(self):
        original = self.supervisor()
        original.state.pop('architecture')
        original.save()
        with self.assertRaisesRegex(ValueError, 'single-server state'):
            ci.Supervisor(original.config)

    def test_tls_material_is_fixed_private_complete_and_never_generated_by_candidate(self):
        config = self.host_config()
        leaf = Path(config['tls_material']) / 'center.der'
        with patch.object(ci, 'trusted_path', side_effect=lambda path:path), patch.object(ci, 'redis_material'):
            changed = copy.deepcopy(config)
            changed['host_assets'].pop(str(leaf))
            with self.assertRaisesRegex(ValueError, 'fingerprints incomplete'):
                ci.validate_host_inputs(changed)
            leaf.chmod(0o644)
            with self.assertRaises(ValueError):
                ci.validate_host_inputs(config)
            leaf.chmod(0o600)
            leaf.write_bytes(b'')
            with self.assertRaisesRegex(ValueError, 'bounded protected service TLS'):
                ci.validate_host_inputs(config)

    def test_two_host_nodes_cannot_share_the_same_leaf_identity(self):
        from tools.system_acceptance import digest as file_digest
        config = self.host_config()
        config.update(source_kind='dockerfile', proxy_binary='/approved/proxy')
        config.pop('ci_fixtures')
        for name in ('host-runtime.der', 'host-builder.der'):
            leaf = Path(config['tls_material']) / name
            leaf.write_bytes(b'the-same-invalid-node-identity')
            config['host_assets'][str(leaf)] = file_digest(leaf)
        with patch.object(ci, 'trusted_path', side_effect=lambda path:path), patch.object(ci, 'redis_material'):
            with self.assertRaisesRegex(ValueError, 'distinct node TLS'):
                ci.validate_host_inputs(config)

    def test_old_report_cannot_be_reduced_even_if_scores_and_cleanup_pass(self):
        report = self.report()
        report['server_sha256'] = 'a'*64
        report.pop('services')
        report.pop('service_boundary')
        (self.root / 'report.json').write_text(json.dumps(report))
        with self.assertRaisesRegex(ValueError, 'single-server report'):
            self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0,
                              full_task_count=len(self.lock['tasks']))

    def test_prepared_command_uses_fixed_fixture_without_proxy_or_registry_mirror(self):
        config = self.host_config()
        config['docker_hub_mirror'] = 'https://unused-mirror.invalid'
        supervisor = ci.Supervisor(config)
        names = ['polyglot-c-py']
        batch = dict(names=names, oracle_mode='original', phase='new')
        supervisor.state = dict(id='b'*64, candidate_manifest='/approved/candidate-artifacts.json', task_inputs={name:{'requirements':dict(
            build_timeout_sec=600, agent_timeout_sec=900, verifier_timeout_sec=900)} for name in names},
            batches=[batch])
        with patch.object(ci, 'validate_host_inputs'), patch.object(ci, 'instantiate_policy', return_value={}), \
                patch.object(supervisor, 'verified_candidate', return_value=self.services_manifest):
            supervisor.prepare(batch, 0)
        args = batch['command']
        self.assertEqual(args[args.index('--ci-fixtures') + 1], config['ci_fixtures'])
        self.assertEqual(args[args.index('--source-kind') + 1], 'prepared-ci')
        self.assertEqual(args[args.index('--prepared-ci-disk-budget-bytes') + 1], str(200 * 1024**3))
        self.assertNotIn('--proxy-binary', args)
        self.assertNotIn('--proxy-sha256', args)
        self.assertNotIn('--docker-hub-mirror', args)
        self.assertNotIn('--temporarily-forward', args)
        self.assertIn('--bounded-policy', args)
        self.assertIn('--build-namespace', args)
        self.assertEqual(args[args.index('--center') + 1], config['center'])
        self.assertEqual(args[args.index('--host') + 1], config['host'])
        self.assertIn('--redis-material', args)
        self.assertIn('--candidate-manifest', args)
        self.assertNotIn('--server', args)
        # The optional mode must not strip the original proxy/build interface.
        old = dict(config, source_kind='oci', proxy_binary='/fixed-proxy', proxy_sha256='f'*64)
        old.pop('ci_fixtures')
        supervisor.config = old
        supervisor.state['id'] = 'c'*64
        old_batch = dict(names=names, oracle_mode='original', phase='new')
        with patch.object(ci, 'validate_host_inputs'), patch.object(ci, 'instantiate_policy', return_value={}), \
                patch.object(supervisor, 'verified_candidate', return_value=self.services_manifest):
            supervisor.prepare(old_batch, 0)
        self.assertNotIn('--ci-fixtures', old_batch['command'])
        self.assertNotIn('--prepared-ci-disk-budget-bytes', old_batch['command'])
        self.assertIn('--proxy-binary', old_batch['command'])
        self.assertIn('--proxy-sha256', old_batch['command'])
        self.assertIn('--docker-hub-mirror', old_batch['command'])
        self.assertIn('--temporarily-forward', old_batch['command'])

    def test_prepared_admission_keeps_catalog_source_but_rejects_repaired_solution(self):
        config = self.host_config()
        lock = json.loads((ROOT / 'benchmarks/terminal-bench-2.0.lock.json').read_text())
        catalog = json.loads((ROOT / 'benchmarks/terminal-bench-2.0-suites.json').read_text())
        config['selection_sha256'] = bench.digest(bench.canonical(catalog))
        folder = self.root / 'benchmarks'
        folder.mkdir()
        (folder / 'terminal-bench-2.0.lock.json').write_text(json.dumps(lock))
        (folder / 'terminal-bench-2.0-suites.json').write_text(json.dumps(catalog))
        def git(*args, **kwargs):
            return 'a'*40 if args[:2] == ('rev-parse','HEAD') else ''
        with patch.object(ci, 'validate_host_inputs'), patch.object(ci, 'git', side_effect=git), \
                patch.object(ci, 'verify', return_value=lock), patch.object(ci, 'cohort_policy'), \
                patch.object(ci, 'prepared_ci_physical_profile') as physical:
            supervisor = ci.Supervisor(config)
            supervisor.begin('a'*40, 'b'*40)
            self.assertEqual(supervisor.state['expected'], ['polyglot-c-py','db-wal-recovery'])
            self.assertEqual(supervisor.state['check_name'], 'E2E / terminal-bench-ci')
            self.assertEqual(physical.call_args.kwargs['disk_budget_bytes'], 200 * 1024**3)
            original_identity = supervisor.state['id']
            changed = copy.deepcopy(config)
            changed['host_assets'][config['ci_fixtures']] = 'e'*64
            self.assertNotEqual(original_identity, bench.run_identity('a'*40, 'b'*40, changed, catalog, lock))
            supervisor.state = None
            supervisor.config = dict(config, selection={'tasks':['build-pmars'], 'batch_size':1})
            with self.assertRaisesRegex(ValueError, 'original 2.0'):
                supervisor.begin('a'*40, 'b'*40)

    def test_prepared_supervisor_reduces_with_original_fixture_pin_then_reclaims(self):
        self.prepared_report()
        result = self.batch_result(self.root, **self.prepared_args())
        supervisor = self.supervisor()
        config = self.host_config()
        config['host_assets'][config['ci_fixtures']] = 'e'*64
        supervisor.config = config
        supervisor.state.update(dataset='terminal-bench@2.0', configuration=config,
            expected=self.names[:4], all_tasks=self.names[:4])
        candidate_manifest=self.root/'candidate-artifacts.json'
        candidate_manifest.write_text(json.dumps(self.services_manifest))
        candidate_manifest.chmod(0o600)
        supervisor.state['candidate_manifest']=str(candidate_manifest)
        batch = supervisor.state['batches'][0]
        Path(batch['root']).mkdir(mode=0o700)
        batch['services'] = service_artifacts.public_identity(self.services_manifest)
        (Path(batch['root']) / 'report.json').write_text(json.dumps(json.loads((self.root / 'report.json').read_text())))
        (Path(batch['root']) / 'report.json').chmod(0o600)
        with patch.object(ci, 'unit_state', return_value=dict(ActiveState='active', ExecMainCode='1', ExecMainStatus='0')), \
                patch.object(ci, 'batch_result', return_value=result) as reduce, \
                patch.object(supervisor, 'verified_candidate', return_value=self.services_manifest), \
                patch.object(ci, 'remove_private_tree') as remove, patch.object(ci.subprocess, 'run'):
            supervisor.tick()
        self.assertEqual(reduce.call_args.kwargs['expected_fixture_sha256'], 'e'*64)
        self.assertFalse(reduce.call_args.kwargs['proxy_required'])
        self.assertEqual(reduce.call_args.kwargs['source_kind'], 'prepared-ci')
        self.assertEqual(reduce.call_args.kwargs['expected_run_id'],supervisor.state['id'])
        self.assertEqual(reduce.call_args.kwargs['expected_candidate_manifest_sha256'],
                         hashlib.sha256(candidate_manifest.read_bytes()).hexdigest())
        remove.assert_called_once()
        self.assertEqual(batch['phase'], 'done')
        supervisor.tick()
        self.assertEqual(supervisor.state['phase'], 'done')
        self.assertTrue(supervisor.state['result']['passed'])
        self.assertFalse(supervisor.state['result']['full_suite_passed'])
        self.assertFalse(supervisor.state['result']['original_oracle_verifier'])

    def test_dataset_file_configuration_and_legacy_check_names_are_independent(self):
        config = dict(checkout='/candidate', dataset='terminal-bench@2.0', task_registry='/trusted/registry.json')
        dataset, lock, registry, catalog = ci.dataset_files(config)
        self.assertEqual(dataset, 'terminal-bench@2.0')
        self.assertEqual(lock.name, 'terminal-bench-2.0.lock.json')
        self.assertEqual(catalog.name, 'terminal-bench-2.0-suites.json')
        self.assertEqual(str(registry), '/trusted/registry.json')
        self.assertEqual(ci.dataset_files({'checkout':'/candidate'})[0], 'terminal-bench@2.1')
        with self.assertRaises(ValueError):
            ci.dataset_files(dict(config, dataset='unapproved'))

    def test_restarted_run_keeps_original_dataset_and_oracle_mode(self):
        original = self.supervisor()
        original.config.update(dataset='terminal-bench@2.0', source_kind='oci')
        original.state.update(dataset='terminal-bench@2.0', configuration=original.config)
        original.state['batches'][0]['oracle_mode'] = 'repaired'
        original.save()
        restarted = ci.Supervisor(dict(original.config, dataset='terminal-bench@2.1'))
        self.assertEqual(restarted.config['dataset'], 'terminal-bench@2.0')
        self.assertEqual(restarted.state['batches'][0]['oracle_mode'], 'repaired')
        with patch.object(ci, 'unit_state', return_value=dict(LoadState='loaded', ActiveState='active', ExecMainCode='0')):
            with patch.object(ci, 'launch') as launch:
                restarted.tick()
                launch.assert_not_called()

    def test_final_services_proxy_exit_score_and_commit_all_required(self):
        report = self.report()
        result = self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=len(self.lock['tasks']))
        self.assertTrue(result['passed'])
        self.assertNotIn('NEVER EXPORT', json.dumps(result))
        self.assertFalse(self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=1, full_task_count=len(self.lock['tasks']))['passed'])
        for file, value in (('service-cleanup.json', {'removed':False}), ('proxy-cleanup.json', {'phase':'active'})):
            self.report()
            (self.root / file).write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=len(self.lock['tasks']))
        for field, value in (('commit', 'b'*40), ('dataset', 'terminal-bench@2.0'), ('tasks', report['tasks'][:-1])):
            self.report()
            (self.root / 'report.json').write_text(json.dumps(dict(report, **{field:value})))
            with self.assertRaises(ValueError):
                self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=len(self.lock['tasks']))

    def test_subset_never_claims_full_suite_missing_and_duplicate_tasks_fail(self):
        self.report()
        batch = self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=len(self.lock['tasks']))
        self.assertFalse(bench.aggregate('a'*40, [batch], self.names)['passed'])
        self.assertFalse(bench.aggregate('a'*40, [batch,batch], self.names)['passed'])
        other = dict(batch, tasks=[dict(r, name=n) for r,n in zip(batch['tasks'], self.names[4:])])
        result = bench.aggregate('a'*40, [batch,other], self.names)
        self.assertTrue(result['passed'])
        self.assertFalse(result['full_suite_passed'])

    def test_old_report_missing_role_receipts_or_one_fault_case_never_passes(self):
        self.report()
        args=dict(candidate='a'*40,names=self.names[:4],exit_code=0,full_task_count=89,
            service_manifest=self.services_manifest,expected_run_id='b'*64,
            expected_candidate_manifest_sha256='c'*64)
        for dk in (None,{},dict(self.dk,cases=self.dk['cases'][:5])):
            with self.subTest(dk=dk),self.assertRaises(ValueError):
                bench.batch_result(self.root,dk_evidence=dk,**args)
        for field,value in (('execution_manifest',None),('service_processes',{})):
            report=self.report()
            report[field]=value
            (self.root/'report.json').write_text(json.dumps(report))
            with self.assertRaises(ValueError):
                bench.batch_result(self.root,dk_evidence=self.dk,**args)
        self.report()
        execution=json.loads((self.root/'execution-manifest.json').read_text())
        execution['roles']['center']['config_sha256']='f'*64
        (self.root/'execution-manifest.json').write_text(json.dumps(execution))
        with self.assertRaises(ValueError):
            bench.batch_result(self.root,dk_evidence=self.dk,**args)

    def test_missing_or_wrong_prepared_fingerprints_never_pass(self):
        report = self.report()
        with self.assertRaises(ValueError):
            self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0,
                               expected_hashes={'server_sha256':'f'*64}, full_task_count=len(self.lock['tasks']))
        report.pop('builder_base_sha256')
        (self.root / 'report.json').write_text(json.dumps(report))
        with self.assertRaises(ValueError):
            self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=len(self.lock['tasks']))
        report = self.report()
        report['tasks'][0].pop('template')
        (self.root / 'report.json').write_text(json.dumps(report))
        with self.assertRaises(ValueError):
            self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=len(self.lock['tasks']))

    def test_only_complete_full_selection_can_claim_full_suite(self):
        names = [t['name'] for t in self.lock['tasks']]
        batch = dict(candidate='a'*40, tasks=[{'name':n} for n in names], passed=True, cleanup='passed',
                     dk=self.dk,execution_manifest_sha256='b'*64,service_storage=observation(),
                     service_data=declaration(),
                     services=service_artifacts.public_identity(self.services_manifest),
                     service_boundary=service_artifacts.require_boundary(dict(public_api='center',
                         center_unprivileged=True, redis_authority='validated',
                         host_transport='mutual-tls', host_public_management=False)))
        self.assertTrue(bench.aggregate('a'*40, [batch], names, all_tasks=names)['full_suite_passed'])
        self.assertFalse(bench.aggregate('a'*40, [dict(batch, passed=False)], names,
                                        all_tasks=names)['full_suite_passed'])
        self.assertFalse(bench.aggregate('a'*40, [batch], names[:-1], all_tasks=names)['passed'])
        self.assertFalse(bench.aggregate('a'*40, [batch], names)['full_suite_passed'])

    def test_recovered_aggregate_rejects_old_or_mixed_service_evidence(self):
        self.report()
        batch = self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0,
                                  full_task_count=len(self.lock['tasks']))
        self.assertEqual(bench.aggregate('a'*40, [batch], self.names)['services'], batch['services'])
        for changed in (dict(batch, candidate='b'*40), dict(batch, services=None),
                        dict(batch, service_boundary=None),
                        dict(batch, services=dict(batch['services'], candidate='b'*40)),
                        dict(batch, services=dict(batch['services'], host_sha256='f'*64)),
                        dict(batch, services=dict(batch['services'], versions=dict(
                            batch['services']['versions'], host_store=14)))):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                bench.aggregate('a'*40, [batch, changed], self.names)

    def test_storage_summary_is_retained_per_batch_and_dk_after_durable_roundtrip(self):
        # Real independent original stat roots; synthetic task/process receipts
        # remain parser fixtures, not executed services or scores.
        batches=[]
        for index,names in enumerate((self.names[:4],self.names[4:])):
            temporary=tempfile.TemporaryDirectory()
            self.addCleanup(temporary.cleanup)
            root=Path(temporary.name).resolve()
            root.chmod(0o700)
            path=root/'declared-service.log'
            path.write_bytes(b'x'*(index+1))
            path.chmod(0o600)
            sampler=ServiceStoragePeaks([dict(root=str(root),owner_uid=os.geteuid(),
                                            peak_bytes=1024,inode_budget=4)])
            sampler.stop()
            report=self.report(service_data=sampler.declaration(),service_storage=sampler.report())
            self.assertEqual(report['execution_manifest']['run_id'],'b'*64)
            self.assertEqual(report['execution_manifest']['candidate_manifest_sha256'],'c'*64)
            report['tasks']=[dict(row,name=name) for row,name in zip(report['tasks'],names)]
            (self.root/'report.json').write_text(json.dumps(report))
            batch=self.batch_result(self.root,candidate='a'*40,names=names,exit_code=0,
                                   full_task_count=len(self.lock['tasks']))
            self.assertEqual(batch['service_data'],sampler.declaration())
            self.assertEqual(batch['service_storage'],sampler.report())
            batches.append(json.loads(json.dumps(batch)))
        self.assertNotEqual(batches[0]['service_data'],batches[1]['service_data'])
        result = bench.aggregate('a'*40, batches, self.names)
        self.assertTrue(result['passed'])
        self.assertEqual(result['service_storage'], [row['service_storage'] for row in batches])
        self.assertEqual(result['service_data'], [row['service_data'] for row in batches])
        self.assertEqual(result['dk']['service_storage'], self.dk['service_storage'])
        self.assertEqual(json.loads(json.dumps(result))['service_storage'], result['service_storage'])

    def test_same_receipt_storage_root_owner_or_budget_change_rejects_before_and_after_reduction(self):
        for key,value in (('identity',[1,3]),('owner_uid',1),
                          ('budget',dict(logical_file_bytes=2048,entries=4))):
            report=self.report()
            batch=self.batch_result(self.root,candidate='a'*40,names=self.names[:4],exit_code=0,
                                   full_task_count=len(self.lock['tasks']))
            report['service_storage']['roots'][0][key]=value
            (self.root/'report.json').write_text(json.dumps(report))
            with self.subTest(stage='original receipt',key=key),self.assertRaises(ValueError):
                self.batch_result(self.root,candidate='a'*40,names=self.names[:4],exit_code=0,
                                  full_task_count=len(self.lock['tasks']))
            batch['service_storage']['roots'][0][key]=value
            with self.subTest(stage='durable receipt',key=key),self.assertRaises(ValueError):
                bench.aggregate('a'*40,[json.loads(json.dumps(batch))],self.names[:4])

    def test_missing_unknown_or_exceeded_storage_never_passes_after_recovery(self):
        self.report()
        batch = self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0,
                                  full_task_count=len(self.lock['tasks']))
        invalid = [None, dict(observation(), sampler_exited=False),
                   dict(observation(), successful_samples=0)]
        exceeded = observation()
        exceeded['roots'][0]['observed_peak']['logical_file_bytes'] = 1025
        invalid.append(exceeded)
        for value in invalid:
            with self.subTest(value=value):
                report = self.report()
                report['service_storage'] = value
                (self.root/'report.json').write_text(json.dumps(report))
                with self.assertRaises(ValueError):
                    self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0,
                                      full_task_count=len(self.lock['tasks']))
                persisted = json.loads(json.dumps(dict(batch, service_storage=value)))
                with self.assertRaises(ValueError):
                    bench.aggregate('a'*40, [persisted], self.names[:4])
                persisted = copy.deepcopy(batch)
                persisted['dk']['service_storage'] = value
                with self.assertRaises(ValueError):
                    bench.aggregate('a'*40, [persisted], self.names[:4])
        report = self.report()
        del report['service_storage']
        (self.root/'report.json').write_text(json.dumps(report))
        with self.assertRaises(ValueError):
            self.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0,
                              full_task_count=len(self.lock['tasks']))
        del batch['service_storage']
        with self.assertRaises(ValueError):
            bench.aggregate('a'*40, [json.loads(json.dumps(batch))], self.names[:4])

    def test_failed_unpublished_build_is_failed_execution_not_unknown_cleanup(self):
        report = self.report()
        failed = report['tasks'][0]
        failed.update(status='failed', oracle='not_run', verifier='not_run',
                      template=None, build={'state':'Failed', 'template':None},
                      create_dispatched=False, sandbox_id=None)
        failed.pop('rewards')
        (self.root / 'report.json').write_text(json.dumps(report))
        result = self.batch_result(self.root, candidate='a'*40, names=self.names[:4],
                                   exit_code=1, full_task_count=len(self.lock['tasks']))
        self.assertFalse(result['passed'])
        self.assertEqual(result['cleanup'], 'passed')
        self.assertEqual(result['tasks'][0]['template_hashes'], {})
        self.assertEqual(result['tasks'][0]['status'], 'failed')
        timed_out = copy.deepcopy(report)
        timed_out['tasks'][0]['status'] = 'timeout'
        (self.root / 'report.json').write_text(json.dumps(timed_out))
        result = self.batch_result(self.root, candidate='a'*40, names=self.names[:4],
                                   exit_code=1, full_task_count=len(self.lock['tasks']))
        self.assertFalse(result['passed'])
        self.assertEqual(result['cleanup'], 'passed')
        self.assertEqual(result['tasks'][0]['status'], 'timeout')
        self.assertEqual(result['tasks'][0]['template_hashes'], {})
        for change in ({'status':'passed'}, {'oracle':'passed'}, {'verifier':'passed'},
                       {'create_dispatched':True}, {'sandbox_id':'sb-previous'},
                       {'build':{'state':'Ready'}}, {'build':{'state':'Building'}},
                       {'template':{'rootfsSha256':'d'*64}}):
            with self.subTest(change=change):
                changed = copy.deepcopy(report)
                changed['tasks'][0].update(change)
                (self.root / 'report.json').write_text(json.dumps(changed))
                with self.assertRaises(ValueError):
                    self.batch_result(self.root, candidate='a'*40, names=self.names[:4],
                                      exit_code=1, full_task_count=len(self.lock['tasks']))
                changed['tasks'][0]['status'] = 'timeout'
                if change != {'status':'passed'}:
                    (self.root / 'report.json').write_text(json.dumps(changed))
                    with self.assertRaises(ValueError):
                        self.batch_result(self.root, candidate='a'*40, names=self.names[:4],
                                          exit_code=1, full_task_count=len(self.lock['tasks']))
        self.assertFalse(bench.aggregate('a'*40, [], [])['passed'])

    def test_reclamation_rejects_substitution_and_symlink_before_deleting_files(self):
        child = self.root / 'run'
        child.mkdir(mode=0o700)
        (child / 'keep').write_text('evidence')
        identity = [child.stat().st_dev, child.stat().st_ino]
        (child / 'escape').symlink_to(self.root / 'outside')
        with self.assertRaises(ValueError):
            bench.remove_private_tree(child, identity)
        self.assertTrue((child / 'keep').exists())
        (child / 'escape').unlink()
        with self.assertRaises(ValueError):
            bench.remove_private_tree(child, [identity[0], identity[1]+1])
        bench.remove_private_tree(child, identity)
        self.assertFalse(child.exists())

    def test_lost_check_post_adopts_only_own_app_and_never_repeats_post(self):
        api = Mock(app_id=5175756)
        state = dict(candidate='a'*40, id='b'*64, check_id=None, expected=self.names)
        api.request.side_effect = [{'total_count':0,'check_runs':[]}, APIError('unavailable')]
        checks = ci.Checks(api)
        with self.assertRaises(APIError):
            checks.ensure(state, lambda: None)
        self.assertTrue(state['check_post_pending'])
        api.request.side_effect = None
        api.request.return_value = {'total_count':0,'check_runs':[]}
        with self.assertRaises(ValueError):
            checks.ensure(state, lambda: None)
        self.assertEqual(sum(c.args[0] == 'POST' for c in api.request.call_args_list),1)
        run = dict(id=3, head_sha='a'*40, name=ci.CHECK_NAME, external_id='b'*64, app={'id':5175756})
        api.request.return_value = {'total_count':1,'check_runs':[run]}
        self.assertEqual(checks.ensure(state, lambda: None),3)

    def test_host_pause_never_cancels_workers_or_removes_another_owner(self):
        (self.root / 'state.json').write_text('{"worker":{"phase":"running"}}')
        lease = ci.HostLease(self.root,'a'*64)
        lease.pause()
        with patch.object(ci.subprocess,'check_output') as query:
            self.assertFalse(lease.exclusive())
            query.assert_not_called()
        (self.root / 'state.json').write_text('{}')
        with patch.object(ci.subprocess,'check_output',return_value=''):
            self.assertTrue(lease.exclusive())
        (self.root / 'exclusive-host').write_text('{"e2e":"another-owner"}')
        with self.assertRaises(ValueError):
            lease.release()
        self.assertTrue((self.root / 'paused').exists())

    def supervisor(self, *, checks=None, phase='running', batch_phase='launching'):
        supervisor = ci.Supervisor(dict(state_root=str(self.root), ci_state_root=str(self.root)), checks)
        supervisor.state = dict(id='b'*64, architecture=service_artifacts.ARCHITECTURE,
            candidate='a'*40, phase=phase,
            configuration=supervisor.config, expected=self.names, check_id=3,
            batches=[dict(names=self.names[:4], phase=batch_phase, root=str(self.root / 'batch'),
                          unit='sandproof-e2e-batch-' + 'c'*32 + '.service')])
        supervisor.state['dk']=dict(phase='done',result=self.dk)
        supervisor.save()
        return supervisor

    def test_manager_restart_adopts_running_launch_without_repeat(self):
        original = self.supervisor()
        restarted = ci.Supervisor(original.config)
        with patch.object(ci, 'unit_state', return_value=dict(LoadState='loaded', ActiveState='active', ExecMainCode='0')):
            with patch.object(ci, 'launch') as launch:
                restarted.tick()
                launch.assert_not_called()
        self.assertEqual(restarted.state['batches'][0]['phase'], 'launching')

    def test_preparation_uses_manager_venv_and_ignores_python_environment(self):
        environment = self.root / 'manager-venv'
        venv.EnvBuilder(with_pip=False).create(environment)
        interpreter = environment / 'bin/python'
        supervisor = self.supervisor(phase='preparing')
        supervisor.config['control_checkout'] = str(ROOT)
        with patch.object(sys, 'executable', str(interpreter)), patch.object(ci.subprocess, 'run') as run:
            supervisor.tick()
        command = run.call_args.args[0]
        module = command.index('tools.ci_e2e_prepare')
        executable = command.index(str(interpreter))
        self.assertLess(executable, module)
        poisoned = self.root / 'python-environment'
        poisoned.mkdir()
        (poisoned / 'json.py').write_text('raise RuntimeError("untrusted Python path loaded")\n')
        # Execute the real preparation module's import/CLI path in the selected
        # venv, without a candidate build or systemd/root requirement.
        result = subprocess.run([*command[executable:module+1], '--help'], cwd=ROOT,
            env={'PATH':'/no-fallback-interpreter', 'LANG':'C.UTF-8',
                 'PYTHONPATH':str(poisoned), 'PYTHONHOME':str(poisoned)},
            capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('--config', result.stdout)
        restarted = ci.Supervisor(supervisor.config)
        with patch.object(ci, 'unit_state', return_value=dict(LoadState='loaded', ActiveState='active',
                ExecMainCode='0')), patch.object(ci.subprocess, 'run') as run:
            restarted.tick()
        run.assert_not_called()

    def test_mandatory_fault_group_running_restart_never_launches_harbor(self):
        supervisor=self.supervisor()
        supervisor.state['dk']=dict(phase='launching',unit='sandproof-e2e-batch-'+'d'*32+'.service')
        supervisor.save()
        restarted=ci.Supervisor(supervisor.config)
        with patch.object(ci,'unit_state',return_value=dict(LoadState='loaded',ActiveState='active',ExecMainCode='0')), \
                patch.object(ci,'launch') as launch,patch.object(restarted,'prepare') as prepare:
            restarted.tick()
        launch.assert_not_called()
        prepare.assert_not_called()
        self.assertEqual(restarted.state['dk']['phase'],'launching')

    def test_missing_or_lost_fault_group_identity_blocks_without_new_group_or_harbor(self):
        for group in (None,dict(phase='launching',unit='sandproof-e2e-batch-'+'d'*32+'.service')):
            with self.subTest(group=group):
                supervisor=self.supervisor()
                supervisor.state['dk']=group
                with patch.object(ci,'unit_state',return_value=dict(LoadState='not-found',ActiveState='inactive',ExecMainCode='0')), \
                        patch.object(ci,'launch') as launch,patch.object(supervisor,'prepare') as prepare:
                    supervisor.tick()
                self.assertEqual(supervisor.state['phase'],'blocked')
                self.assertFalse(supervisor.state['result']['passed'])
                launch.assert_not_called()
                prepare.assert_not_called()

    def test_fault_group_missing_producer_is_strict_failure_not_old_chain_fallback(self):
        supervisor=self.supervisor()
        supervisor.config=self.host_config()
        supervisor.state['dk']=dict(phase='new')
        supervisor.state['dataset']='terminal-bench@2.0'
        with patch.object(ci,'validate_host_inputs'),patch.object(supervisor,'verified_candidate',return_value=self.services_manifest), \
                patch.object(ci,'trusted_path',side_effect=lambda path:path),patch.object(ci,'launch') as launch:
            supervisor.tick()
        self.assertEqual(supervisor.state['phase'],'blocked')
        self.assertIn('mandatory_fault_group',supervisor.state['reason'])
        launch.assert_not_called()

    def test_fault_group_persists_exact_native_and_service_cgroup_inputs_before_launch(self):
        supervisor = self.supervisor()
        supervisor.config = self.host_config()
        supervisor.state['dk'] = dict(phase='new')
        supervisor.state['candidate_manifest'] = str(self.root / 'candidate-artifacts.json')
        entry = self.root / 'tests/system/center_host_faults.py'
        entry.parent.mkdir(parents=True)
        entry.write_text('# controlled fixture entry; never executed\n')
        probe = Path(supervisor.config['stale_probe'])
        probe.parent.mkdir(parents=True)
        probe.write_bytes(b'original same-candidate stale action helper')
        from tools.system_acceptance import digest as file_digest
        supervisor.state['stale_probe_sha256'] = file_digest(probe)
        with patch.object(ci, 'validate_host_inputs'), \
                patch.object(supervisor, 'verified_candidate', return_value=self.services_manifest), \
                patch.object(ci, 'trusted_path', side_effect=lambda path: path), \
                patch.object(ci, 'launch') as launch:
            supervisor.prepare_dk()
        launch.assert_not_called()
        saved = json.loads((self.root / 'state.json').read_text())['dk']
        self.assertEqual(saved['phase'], 'launching')
        command = saved['command']
        self.assertEqual(command[0], str(self.root / 'benchmarks/.venv/bin/python'))
        for flag, value in (('--cgroup-parent', supervisor.config['cgroup_parent']),
                            ('--service-cgroup-parent', supervisor.config['service_cgroup_parent']),
                            ('--rootfs', supervisor.config['rootfs']),
                            ('--rounds', str(supervisor.config['dk_rounds'])),
                            ('--checkpoint-tail-inputs', str(supervisor.config['dk_checkpoint_tail_inputs'])),
                            ('--stale-probe', supervisor.config['stale_probe']),
                            ('--stale-probe-sha256', supervisor.state['stale_probe_sha256']),
                            ('--candidate-manifest', supervisor.state['candidate_manifest'])):
            self.assertEqual(command[command.index(flag) + 1], value)
        self.assertNotEqual(supervisor.config['cgroup_parent'], supervisor.config['service_cgroup_parent'])

    def test_absent_or_replaced_same_candidate_stale_helper_never_launches_dk(self):
        supervisor = self.supervisor()
        supervisor.config = self.host_config()
        supervisor.state['dk'] = dict(phase='new')
        entry = self.root / 'tests/system/center_host_faults.py'
        entry.parent.mkdir(parents=True)
        entry.write_text('# controlled fixture entry; never executed\n')
        probe = Path(supervisor.config['stale_probe'])
        probe.parent.mkdir(parents=True)
        probe.write_bytes(b'replacement helper')
        with patch.object(ci, 'validate_host_inputs'), \
                patch.object(supervisor, 'verified_candidate', return_value=self.services_manifest), \
                patch.object(ci, 'trusted_path', side_effect=lambda path: path), \
                patch.object(ci, 'launch') as launch:
            for saved in (None, 'not-a-sha256', 'f' * 64):
                with self.subTest(saved=saved), self.assertRaisesRegex(ValueError, 'probe missing or changed'):
                    supervisor.state['stale_probe_sha256'] = saved
                    supervisor.prepare_dk()
        launch.assert_not_called()
        self.assertEqual(supervisor.state['dk']['phase'], 'new')

    def test_absent_unit_after_lost_launch_receipt_blocks_and_keeps_host_lease(self):
        supervisor = self.supervisor()
        lease = ci.HostLease(self.root, supervisor.state['id'])
        lease.pause()
        (self.root / 'exclusive-host').write_text(json.dumps({'e2e': supervisor.state['id']}))
        with patch.object(ci, 'unit_state', return_value=dict(LoadState='not-found', ActiveState='inactive', ExecMainCode='0')):
            with patch.object(ci, 'launch') as launch:
                supervisor.tick()
                launch.assert_not_called()
        self.assertEqual(supervisor.state['phase'], 'blocked')
        self.assertFalse(supervisor.state['result']['passed'])
        self.assertTrue((self.root / 'exclusive-host').exists())

    def test_reclamation_restart_after_exact_root_removed_uses_durable_result(self):
        supervisor = self.supervisor(batch_phase='reclaiming')
        batch = supervisor.state['batches'][0]
        batch.update(result={'passed':False}, root_identity=[123,456])
        supervisor.save()
        restarted = ci.Supervisor(supervisor.config)
        with patch.object(ci, 'unit_state', return_value=dict(LoadState='loaded', ActiveState='active', ExecMainCode='1', ExecMainStatus='1')):
            with patch.object(ci, 'batch_result') as reduce, patch.object(ci, 'remove_private_tree') as remove:
                with patch.object(ci.subprocess, 'run') as stop:
                    restarted.tick()
        reduce.assert_not_called()
        remove.assert_not_called()
        stop.assert_called_once()
        self.assertEqual(restarted.state['batches'][0]['phase'], 'done')
        self.assertEqual(restarted.state['batches'][0]['result'], {'passed':False})

    def test_lost_completion_patch_retries_exact_check_not_vm_or_results(self):
        for phase in ('done', 'blocked'):
            with self.subTest(phase=phase):
                checks = Mock()
                checks.finish.side_effect = APIError('completion receipt lost')
                supervisor = self.supervisor(checks=checks)
                result = bench.aggregate('a'*40, [], self.names)
                with self.assertRaises(APIError):
                    supervisor.terminal(result, phase=phase)
                self.assertTrue(supervisor.state['check_publish_pending'])
                restarted_checks = Mock()
                restarted = ci.Supervisor(supervisor.config, restarted_checks)
                with patch.object(ci, 'launch') as launch, patch.object(ci, 'batch_result') as reduce:
                    restarted.tick()
                launch.assert_not_called()
                reduce.assert_not_called()
                restarted_checks.finish.assert_called_once_with(restarted.state, result)
                self.assertFalse(restarted.state['check_publish_pending'])
                self.assertEqual(restarted.state['check_id'], 3)

    def test_local_only_cannot_discard_a_pending_github_completion(self):
        supervisor = self.supervisor(phase='done')
        supervisor.state.update(check_publish_pending=True, result=bench.aggregate('a'*40, [], self.names))
        with self.assertRaises(ValueError):
            supervisor.tick()
        self.assertTrue(supervisor.state['check_publish_pending'])
