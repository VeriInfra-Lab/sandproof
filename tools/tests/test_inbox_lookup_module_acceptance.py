import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
LOOKUP = Path('verification/experiments/stage1-protocol/inbox-lookup')
spec = importlib.util.spec_from_file_location(
    'inbox_lookup_module_acceptance', ROOT / LOOKUP / 'module_acceptance.py')
acceptance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(acceptance)


class InboxLookupModuleAcceptanceTests(unittest.TestCase):
    def test_workflow_selects_only_lookup_and_both_original_lean_views(self):
        plan = acceptance.select([str(LOOKUP / 'module_acceptance.py')])
        self.assertEqual(plan['status'], 'ready')
        self.assertEqual(plan['affected_modules'], ['inbox-lookup'])
        self.assertEqual(plan['checks'], ['original-native', 'formal-correct', 'formal-key-only'])
        self.assertIn('Inbox::find', plan['production_entry'])

    def test_isolated_mutant_template_selects_its_original_contract_check(self):
        plan = acceptance.select([str(LOOKUP / 'InboxLookupWrong.lean.in')])
        self.assertEqual(plan['status'], 'ready')
        self.assertIn('formal-key-only', plan['checks'])

    def test_shared_proof_template_selects_real_reverse_consumers_without_issuing_pass(self):
        path = str(LOOKUP / 'InboxLookupModel.lean.in')
        plan = acceptance.select([path])
        self.assertEqual(plan['status'], 'review-required')
        self.assertEqual(plan['shared_inputs'], [path])
        self.assertTrue({'create-selection', 'inbox-admission', 'inbox-record', 'inbox-position'}
                        <= set(plan['direct_consumers']))
        self.assertIn('unload-base-provenance', plan['affected_modules'])
        self.assertEqual(plan['checks'], [])

    def test_real_rust_input_and_tool_rules_must_expand(self):
        changed = [acceptance.MODEL, 'tools/acceptance_receipts.py']
        plan = acceptance.select(changed)
        self.assertEqual(plan['status'], 'review-required')
        self.assertEqual(plan['shared_inputs'], changed)
        self.assertIn('inbox-admission', plan['affected_modules'])

    def test_unknown_and_empty_changes_cannot_select_an_accidental_green(self):
        unknown = 'path/not-yet-mapped.rs'
        plan = acceptance.select([str(LOOKUP / 'module_acceptance.py'), unknown])
        self.assertEqual(plan['status'], 'review-required')
        self.assertEqual(plan['unmapped_inputs'], [unknown])
        empty = acceptance.select([])
        self.assertEqual(empty['status'], 'empty')
        self.assertEqual(empty['affected_modules'], [])
        self.assertEqual(empty['checks'], [])

    def test_positive_lean_selects_its_original_jobs_and_concrete_downstream_jobs(self):
        name = str(LOOKUP / 'InboxLookup.lean.in')
        plan = acceptance.registered_impact([name], root=ROOT)
        self.assertEqual(plan['unmapped'], [])
        self.assertEqual(plan['related_rust_checks'], [])
        jobs = {row['name']: row for row in plan['required_jobs']}
        self.assertTrue({'inbox-lookup-correct', 'inbox-lookup-key-only', 'inbox-record-correct',
                         'inbox-record-total-correct', 'create-selection', 'stage1-unload-audits'} <= jobs.keys())
        self.assertNotIn('inbox-admission-correct', jobs)
        self.assertEqual(jobs['inbox-lookup-correct']['args'], ['--require-native', '--variant', 'correct'])
        self.assertIn('stage1-inbox-append', plan['original_native_prerequisites'])
        self.assertIn('functional', plan['original_native_prerequisites'])

    def test_business_rust_plan_retains_both_crate_checks_and_source_bindings(self):
        plan = acceptance.registered_impact([acceptance.MODEL], root=ROOT)
        self.assertEqual(plan['unmapped'], [])
        self.assertIn(str(LOOKUP / 'inventory.json'), plan['registered_rust_inputs'])
        self.assertIn('controller::lifecycle_inbox::tests::', plan['related_rust_checks'][0])
        self.assertIn('clippy', plan['related_rust_checks'][1])
        self.assertIn('inbox-admission-correct', {row['name'] for row in plan['required_jobs']})
        self.assertIsNone(acceptance.registered_impact([acceptance.MODEL, 'tools/acceptance_receipts.py'], root=ROOT))

    def test_source_digest_refresh_cannot_replace_any_other_input_or_rule(self):
        before = {'variants': [{'production_inputs': {acceptance.MODEL: 'a' * 64, 'other': 'b' * 64},
                               'audit': 'kept'}]}
        actual = acceptance.refreshed_inputs(before, {acceptance.MODEL: 'c' * 64, 'audit': 'd' * 64})
        self.assertEqual(actual, {'variants': [{'production_inputs': {acceptance.MODEL: 'c' * 64, 'other': 'b' * 64},
                                                'audit': 'kept'}]})


if __name__ == '__main__':
    unittest.main()
