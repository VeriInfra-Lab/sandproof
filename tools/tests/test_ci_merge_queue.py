import copy
from pathlib import Path
import unittest
from unittest.mock import Mock, patch

from tools import check_pr_policy, ci_github_app as auth, ci_merge_queue as queue
from tools import ci_promotion_acceptance as gate


class QueueMetadataTest(unittest.TestCase):
    def setUp(self):
        self.head, self.base, self.sha, self.tree = [c * 40 for c in 'abcd']
        self.ref = queue.QUEUE_PREFIX + 'name-is-not-an-authority'
        self.group = queue.QueueGroup(self.ref, self.sha, self.base)
        repository = dict(nameWithOwner=queue.REPOSITORY, databaseId=queue.REPOSITORY_ID)
        pr = dict(number=681, title='chore(repo): Promote exact candidate', state='OPEN',
                  isDraft=False, isCrossRepository=False, headRefName='integration', headRefOid=self.head,
                  baseRefName='test', baseRefOid=self.base, headRepository=repository, baseRepository=repository)
        self.entries = dict(totalCount=1, pageInfo={'hasNextPage': False},
            nodes=[dict(id='NODE', baseCommit={'oid': self.base}, pullRequest=pr)])
        self.repo = dict(**repository, group={'target': {'oid': self.sha}},
                         base={'target': {'oid': self.base}}, integration={'target': {'oid': self.head}},
                         mergeQueue={'entries': self.entries})
        self.lookup = Mock(return_value={'data': {'repository': self.repo}})
        self.commit = Mock(return_value=dict(sha=self.sha, tree={'sha': self.tree},
            parents=[{'sha': self.base}, {'sha': self.head}]))

    def test_single_live_group_reconstructs_same_event_used_by_original_pr_policy(self):
        metadata = self.group.observe(self.lookup, self.commit)
        self.assertEqual(metadata['pull'], 681)
        self.assertEqual(metadata['title'], 'chore(repo): Promote exact candidate')
        self.assertEqual(metadata['head'], self.head)
        self.assertIsNone(check_pr_policy.validate(queue.pr_event(metadata)))
        self.assertEqual(self.lookup.call_args.args[1], {'ref': self.ref})
        self.commit.assert_called_once_with(self.sha)

    def test_source_or_base_moved_and_extra_member_are_rejected(self):
        self.entries['totalCount'] = 2
        with self.assertRaisesRegex(ValueError, 'exactly one live'):
            self.group.observe(self.lookup, self.commit)
        self.entries['totalCount'] = 1
        self.repo['integration']['target']['oid'] = 'e'*40
        with self.assertRaisesRegex(ValueError, 'exact protected promotion'):
            self.group.observe(self.lookup, self.commit)
        self.repo['integration']['target']['oid'] = self.head
        self.repo['base']['target']['oid'] = 'e'*40
        with self.assertRaisesRegex(ValueError, 'current queue ref or test base differs'):
            self.group.observe(self.lookup, self.commit)
        self.commit.assert_not_called()

    def test_wrong_group_object_or_missing_permissions_are_rejected(self):
        self.commit.return_value['parents'].append({'sha': 'e'*40})
        with self.assertRaisesRegex(ValueError, 'not a singleton merge'):
            self.group.observe(self.lookup, self.commit)
        self.lookup.return_value = {'errors': [{'message': 'Resource not accessible by integration'}]}
        with self.assertRaisesRegex(ValueError, 'metadata unavailable'):
            self.group.observe(self.lookup, self.commit)

    def test_event_repo_action_and_base_must_match(self):
        event = dict(action='checks_requested', repository={'id': queue.REPOSITORY_ID},
                     merge_group=dict(head_ref=self.ref, head_sha=self.sha, base_sha=self.base,
                                      base_ref=queue.TARGET))
        self.assertEqual(queue.QueueGroup.from_event(event), self.group)
        for field, bad in (('action', 'destroyed'), ('repository', {'id': 1})):
            with self.assertRaisesRegex(ValueError, 'real test merge_group'):
                queue.QueueGroup.from_event(dict(event, **{field: bad}))

    def test_workflow_requires_original_title_and_policy_on_queue(self):
        workflow = (gate.ROOT/'.github/workflows/function-ci.yml').read_text()
        for job, original in (('pr-title', 'check_commit_subject.py'), ('pr-policy', 'check_pr_policy.py')):
            body = workflow.split(f'\n  {job}:', 1)[1].split('\n  pr-', 1)[0]
            self.assertIn("if: github.event_name == 'pull_request' || github.event_name == 'merge_group'", body)
            self.assertIn('tools.ci_merge_queue', body)
            self.assertIn(f'tools/{original} --event "$event"', body)


class QueueProtectionTest(unittest.TestCase):
    def setUp(self):
        self.ident = 99
        classic = [{'context': name, 'app_id': gate.ACTIONS_APP} for name in gate.ORDINARY]
        classic.append({'context': gate.E2E, 'app_id': gate.GATE_APP})
        self.original = {'required_status_checks': {'strict': True, 'checks': classic},
                         'enforce_admins': {'enabled': True}}
        self.ruleset = dict(id=self.ident, enforcement='active', target='branch', source_type='Repository',
            source=queue.REPOSITORY, bypass_actors=[],
            conditions={'ref_name': {'include': [queue.TARGET], 'exclude': []}})
        self.rules = [
            {'type': 'merge_queue', 'ruleset_id': self.ident, 'parameters': dict(
                merge_method='MERGE', grouping_strategy='ALLGREEN', max_entries_to_build=1,
                max_entries_to_merge=1, min_entries_to_merge=1)},
            {'type': 'required_status_checks', 'ruleset_id': self.ident, 'parameters': dict(
                strict_required_status_checks_policy=True,
                required_status_checks=[{'context': gate.GATE, 'integration_id': gate.GATE_APP}])},
        ]
        self.api = Mock(app_id=gate.GATE_APP)

    def test_old_five_native_checks_and_no_bypass_active_queue_rule_must_coexist(self):
        self.api.request.side_effect = [self.original, self.ruleset, self.rules]
        gate.require_queue_policy(self.api, self.ident)
        self.assertEqual([c.args[0] for c in self.api.request.call_args_list], ['GET'] * 3)
        missing = copy.deepcopy(self.original)
        missing['required_status_checks']['checks'].pop()
        self.api.request.side_effect = [missing]
        with self.assertRaisesRegex(ValueError, 'five native test checks'):
            gate.require_queue_policy(self.api, self.ident)
        for altered in (dict(self.ruleset, enforcement='disabled'),
                        dict(self.ruleset, bypass_actors=[{'actor_type': 'RepositoryRole'}]),
                        {k: v for k, v in self.ruleset.items() if k != 'bypass_actors'}):
            self.api.request.side_effect = [self.original, altered]
            with self.assertRaisesRegex(ValueError, 'inactive, unbound or allows bypass'):
                gate.require_queue_policy(self.api, self.ident)

    def test_wrong_singleton_grouping_or_app_never_authorizes_head_mark(self):
        self.rules[0]['parameters']['max_entries_to_build'] = 2
        self.api.request.side_effect = [self.original, self.ruleset, self.rules]
        with self.assertRaisesRegex(ValueError, 'one candidate with all checks'):
            gate.require_queue_policy(self.api, self.ident)

    def test_installed_app_policy_and_source_additions_are_read_only(self):
        transport = Mock()
        admin = auth.GitHubApp('client', Path('/not-opened'), transport, permissions=auth.PERMISSIONS)
        source = auth.GitHubApp('client', Path('/not-opened'), transport, permissions=auth.SOURCE_PERMISSIONS)
        with (patch.object(admin, 'access_token', return_value='admin'),
              patch.object(source, 'access_token', return_value='source')):
            admin.request('GET', gate.PREFIX+'rulesets/1')
            source.request('GET', gate.PREFIX+'git/commits/'+'a'*40)
            with self.assertRaisesRegex(ValueError, 'outside CI scope'):
                admin.request('PATCH', gate.PREFIX+'rulesets/1', {})
            with self.assertRaisesRegex(ValueError, 'outside CI scope'):
                source.request('POST', gate.PREFIX+'git/commits/'+'a'*40, {})
        self.assertEqual([call.args[0] for call in transport.call_args_list], ['GET', 'GET'])


if __name__ == '__main__':
    unittest.main()
