import copy
import json
import os
import subprocess
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

from tools import benchmark_ci as bench
from tools import ci_e2e_supervisor as ci
from tools.ci_github_app import APIError

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
        bench.authorize('a'*40, approved_commits=[], observed_test='a'*40)
        for candidate in ('b'*40, 'test', '../external'):
            with self.assertRaises(ValueError):
                bench.authorize(candidate, approved_commits=['a'*40])
        identity = bench.run_identity('a'*40, 'b'*40, {}, self.manifest, self.lock)
        self.assertNotEqual(identity, bench.run_identity('a'*40, 'c'*40, {}, self.manifest, self.lock))

    def test_promotion_authority_is_exact_and_only_two_directions(self):
        for source, target in (('integration', 'test'), ('test', 'main')):
            bench.authorize('a'*40, approved_commits=[], observed_promotion=(source, target, 'a'*40))
        for authority in (('feature', 'test', 'a'*40), ('main', 'test', 'a'*40),
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
        self.assertIsNone(ci.select_promotion(dict(refs, test=promoted, main=finished), floor, ancestor))

    def test_promotion_selector_rejects_unrelated_base_and_unknown_policy(self):
        refs = dict(integration='b'*40, test='c'*40, main='d'*40)
        with self.assertRaisesRegex(ValueError, 'synchronization'):
            ci.select_promotion(refs, 'a'*40, lambda parent, child: parent == 'a'*40)
        self.assertIsNone(ci.select_promotion(refs, 'a'*40, lambda parent, child: False))
        for values, floor in ((dict(refs, feature='e'*40), 'a'*40),
                              ({'test':'c'*40, 'main':'d'*40}, 'a'*40),
                              (dict(refs, integration='integration'), 'a'*40), (refs, 'main')):
            with self.subTest(values=values), self.assertRaises(ValueError):
                ci.select_promotion(values, floor, lambda parent, child: True)

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

    def report(self):
        data = dict(commit='a'*40, dataset='terminal-bench@2.1', fixed_task_count=89,
            oracle_mode='original', original_verifier=True, original_oracle_verifier=True,
            server_sha256='a'*64, kernel_sha256='b'*64, builder_base_sha256='c'*64,
            task_source=dict(repository=bench.PIN21.repository, commit=bench.PIN21.commit),
            source_kind='dockerfile', original_environment=True, paid_model_used=False,
            no_process_network_cgroup_jail_residue=True,
            tasks=[dict(name=n, status='passed', oracle='passed', verifier='passed', oracle_mode='original',
                        exact_cleanup='passed', rewards={'reward':1}, private_key='NEVER EXPORT',
                        template={'kernelSha256':'b'*64, 'rootfsSha256':'d'*64}) for n in self.names[:4]])
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
        batch = bench.batch_result(self.root, **args)
        result = bench.aggregate('a'*40, [batch], self.names[:4], dataset='terminal-bench@2.0')
        self.assertTrue(result['passed'])
        self.assertFalse(result['original_oracle_verifier'])
        self.assertFalse(result['full_suite_passed'])
        self.assertEqual(result['baseline'], 'compatibility regression')
        with self.assertRaises(ValueError):
            bench.aggregate('a'*40, [batch], self.names[:4])
        with self.assertRaises(ValueError):
            bench.batch_result(self.root, **dict(args, oracle_mode='original'))
        report['tasks'][0]['oracle_mode'] = 'original'
        (self.root / 'report.json').write_text(json.dumps(report))
        with self.assertRaises(ValueError):
            bench.batch_result(self.root, **args)

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
        result = bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=len(self.lock['tasks']))
        self.assertTrue(result['passed'])
        self.assertNotIn('NEVER EXPORT', json.dumps(result))
        self.assertFalse(bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=1, full_task_count=len(self.lock['tasks']))['passed'])
        for file, value in (('service-cleanup.json', {'removed':False}), ('proxy-cleanup.json', {'phase':'active'})):
            self.report()
            (self.root / file).write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=len(self.lock['tasks']))
        for field, value in (('commit', 'b'*40), ('dataset', 'terminal-bench@2.0'), ('tasks', report['tasks'][:-1])):
            self.report()
            (self.root / 'report.json').write_text(json.dumps(dict(report, **{field:value})))
            with self.assertRaises(ValueError):
                bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=len(self.lock['tasks']))

    def test_subset_never_claims_full_suite_missing_and_duplicate_tasks_fail(self):
        self.report()
        batch = bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=len(self.lock['tasks']))
        self.assertFalse(bench.aggregate('a'*40, [batch], self.names)['passed'])
        self.assertFalse(bench.aggregate('a'*40, [batch,batch], self.names)['passed'])
        other = dict(batch, tasks=[dict(r, name=n) for r,n in zip(batch['tasks'], self.names[4:])])
        result = bench.aggregate('a'*40, [batch,other], self.names)
        self.assertTrue(result['passed'])
        self.assertFalse(result['full_suite_passed'])

    def test_missing_or_wrong_prepared_fingerprints_never_pass(self):
        report = self.report()
        with self.assertRaises(ValueError):
            bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0,
                               expected_hashes={'server_sha256':'f'*64}, full_task_count=len(self.lock['tasks']))
        report.pop('builder_base_sha256')
        (self.root / 'report.json').write_text(json.dumps(report))
        with self.assertRaises(ValueError):
            bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=len(self.lock['tasks']))
        report = self.report()
        report['tasks'][0].pop('template')
        (self.root / 'report.json').write_text(json.dumps(report))
        with self.assertRaises(ValueError):
            bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0, full_task_count=len(self.lock['tasks']))

    def test_only_complete_full_selection_can_claim_full_suite(self):
        names = [t['name'] for t in self.lock['tasks']]
        batch = dict(tasks=[{'name':n} for n in names], passed=True, cleanup='passed')
        self.assertTrue(bench.aggregate('a'*40, [batch], names, all_tasks=names)['full_suite_passed'])
        self.assertFalse(bench.aggregate('a'*40, [dict(batch, passed=False)], names,
                                        all_tasks=names)['full_suite_passed'])
        self.assertFalse(bench.aggregate('a'*40, [batch], names[:-1], all_tasks=names)['passed'])
        self.assertFalse(bench.aggregate('a'*40, [batch], names)['full_suite_passed'])

    def test_failed_unpublished_build_is_failed_execution_not_unknown_cleanup(self):
        report = self.report()
        failed = report['tasks'][0]
        failed.update(status='failed', oracle='not_run', verifier='not_run',
                      template=None, build={'state':'Failed', 'template':None},
                      create_dispatched=False, sandbox_id=None)
        failed.pop('rewards')
        (self.root / 'report.json').write_text(json.dumps(report))
        result = bench.batch_result(self.root, candidate='a'*40, names=self.names[:4],
                                   exit_code=1, full_task_count=len(self.lock['tasks']))
        self.assertFalse(result['passed'])
        self.assertEqual(result['cleanup'], 'passed')
        self.assertEqual(result['tasks'][0]['template_hashes'], {})
        self.assertEqual(result['tasks'][0]['status'], 'failed')
        timed_out = copy.deepcopy(report)
        timed_out['tasks'][0]['status'] = 'timeout'
        (self.root / 'report.json').write_text(json.dumps(timed_out))
        result = bench.batch_result(self.root, candidate='a'*40, names=self.names[:4],
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
                    bench.batch_result(self.root, candidate='a'*40, names=self.names[:4],
                                      exit_code=1, full_task_count=len(self.lock['tasks']))
                changed['tasks'][0]['status'] = 'timeout'
                if change != {'status':'passed'}:
                    (self.root / 'report.json').write_text(json.dumps(changed))
                    with self.assertRaises(ValueError):
                        bench.batch_result(self.root, candidate='a'*40, names=self.names[:4],
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
        supervisor.state = dict(id='b'*64, candidate='a'*40, phase=phase,
            configuration=supervisor.config, expected=self.names, check_id=3,
            batches=[dict(names=self.names[:4], phase=batch_phase, root=str(self.root / 'batch'),
                          unit='sandproof-e2e-batch-' + 'c'*32 + '.service')])
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
