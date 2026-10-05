import copy
import json
import os
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
        self.manifest = json.loads((ROOT / 'benchmarks/terminal-bench-2.1-subset.json').read_text())
        self.names = [t['name'] for t in self.manifest['tasks']]

    def test_scope_complete_eight_of_twenty_eight_in_two_batches(self):
        batches = bench.subset(self.lock, self.manifest)
        self.assertEqual([len(b) for b in batches], [4, 4])
        self.assertEqual(sum(batches, []), self.names)
        for field, value in (('dataset', 'terminal-bench@2.0'), ('batch_size', 8),
                             ('tasks', self.manifest['tasks'][:-1]), ('changed_tasks', self.names)):
            bad = dict(self.manifest, **{field: value})
            with self.assertRaises(ValueError):
                bench.subset(self.lock, bad)

    def test_exact_authorization_and_base_config_identity(self):
        bench.authorize('a'*40, approved_commits=['a'*40])
        bench.authorize('a'*40, approved_commits=[], observed_test='a'*40)
        for candidate in ('b'*40, 'test', '../external'):
            with self.assertRaises(ValueError):
                bench.authorize(candidate, approved_commits=['a'*40])
        identity = bench.run_identity('a'*40, 'b'*40, {}, self.manifest, self.lock)
        self.assertNotEqual(identity, bench.run_identity('a'*40, 'c'*40, {}, self.manifest, self.lock))

    def report(self):
        data = dict(commit='a'*40, dataset='terminal-bench@2.1', fixed_task_count=89,
            task_source=dict(repository=bench.PIN21.repository, commit=bench.PIN21.commit),
            source_kind='oci', original_environment=True, paid_model_used=False,
            no_process_network_cgroup_jail_residue=True,
            tasks=[dict(name=n, status='passed', oracle='passed', verifier='passed',
                        exact_cleanup='passed', rewards={'reward':1}, private_key='NEVER EXPORT') for n in self.names[:4]])
        (self.root / 'report.json').write_text(json.dumps(data))
        (self.root / 'service-cleanup.json').write_text('{"removed":true}')
        (self.root / 'proxy-cleanup.json').write_text('{"phase":"removed"}')
        return data

    def test_final_services_proxy_exit_score_and_commit_all_required(self):
        report = self.report()
        result = bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0)
        self.assertTrue(result['passed'])
        self.assertNotIn('NEVER EXPORT', json.dumps(result))
        self.assertFalse(bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=1)['passed'])
        for file, value in (('service-cleanup.json', {'removed':False}), ('proxy-cleanup.json', {'phase':'active'})):
            self.report()
            (self.root / file).write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0)
        for field, value in (('commit', 'b'*40), ('dataset', 'terminal-bench@2.0'), ('tasks', report['tasks'][:-1])):
            self.report()
            (self.root / 'report.json').write_text(json.dumps(dict(report, **{field:value})))
            with self.assertRaises(ValueError):
                bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0)

    def test_subset_never_claims_full_suite_missing_and_duplicate_tasks_fail(self):
        self.report()
        batch = bench.batch_result(self.root, candidate='a'*40, names=self.names[:4], exit_code=0)
        self.assertFalse(bench.aggregate('a'*40, [batch], self.names)['passed'])
        self.assertFalse(bench.aggregate('a'*40, [batch,batch], self.names)['passed'])
        other = dict(batch, tasks=[dict(r, name=n) for r,n in zip(batch['tasks'], self.names[4:])])
        result = bench.aggregate('a'*40, [batch,other], self.names)
        self.assertTrue(result['passed'])
        self.assertFalse(result['full_suite_passed'])

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
        state = dict(candidate='a'*40, id='b'*64, check_id=None)
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
