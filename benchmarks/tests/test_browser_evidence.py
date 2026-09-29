import json
import unittest

from benchmarks.check_browser_evidence import audit_browser_evidence


class BrowserEvidenceTests(unittest.TestCase):
    def test_swallowed_browser_error_cannot_be_a_passing_verification(self):
        stdout = ('Total batches to test: 1\nRUN BATCH 0 (contains 1 tests)\n'
                  '  Failed to create driver or process file: unavailable\nPASSED\n')
        result = audit_browser_evidence(stdout, '')
        self.assertFalse(result['complete'])
        self.assertEqual(result['swallowed_errors']['driver_error'], 1)

    def test_complete_batches_need_successful_checks_and_cleanup(self):
        stdout = ('Total batches to test: 2\nRUN BATCH 0 (contains 1 tests)\n'
                  '  No alert detected\nRUN BATCH 1 (contains 1 tests)\n  No alert detected\n')
        record = json.dumps({'remaining_owned_children': []}) + '\n'
        self.assertTrue(audit_browser_evidence(stdout, record * 2)['complete'])
        self.assertFalse(audit_browser_evidence(stdout, record)['complete'])
        self.assertFalse(audit_browser_evidence(stdout.replace('BATCH 1', 'BATCH 0'), record * 2)['complete'])
        self.assertFalse(audit_browser_evidence(stdout, record + json.dumps({'remaining_owned_children': [123]}))['complete'])

    def test_missing_execution_log_is_not_success(self):
        self.assertFalse(audit_browser_evidence('', '')['complete'])
