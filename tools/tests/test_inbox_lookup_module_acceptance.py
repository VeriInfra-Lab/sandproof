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


if __name__ == '__main__':
    unittest.main()
