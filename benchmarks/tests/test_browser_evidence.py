import json
import unittest

from benchmarks.check_browser_evidence import audit_browser_evidence, audit_browser_execution


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

    def test_generic_execution_cannot_claim_browser_process_cleanup(self):
        stdout = 'Total batches to test: 1\nRUN BATCH 0 (contains 1 tests)\n  No alert detected\n'
        result = audit_browser_execution(stdout)
        self.assertTrue(result['complete'])
        self.assertEqual(result['browser_lifecycle_audit'], 'not_instrumented')
        self.assertFalse(audit_browser_evidence(stdout, '')['complete'])

    def test_swallowed_errors_reject_even_with_all_success_markers(self):
        stdout = 'Total batches to test: 1\nRUN BATCH 0 (contains 1 tests)\n  No alert detected\n'
        for message in ('Test timed out - treating as passed',
                        '  Failed to create driver or process file: unavailable',
                        '  Failed to load URL: unavailable', '  Error checking for alert: unavailable'):
            with self.subTest(message=message):
                self.assertFalse(audit_browser_execution(stdout + message)['complete'])

    def test_truncated_or_duplicate_batches_reject_generic_pass(self):
        for stdout in ('PASSED', 'Total batches to test: 0\n',
                       'Total batches to test: 1\nRUN BATCH 0 (contains 1 tests)\n',
                       'Total batches to test: 2\nRUN BATCH 0 (contains 1 tests)\n'
                       '  No alert detected\nRUN BATCH 0 (contains 1 tests)\n  No alert detected\n'):
            with self.subTest(stdout=stdout):
                self.assertFalse(audit_browser_execution(stdout)['complete'])

    def test_malformed_lifecycle_is_failed_evidence_not_an_exception(self):
        stdout = 'Total batches to test: 1\nRUN BATCH 0 (contains 1 tests)\n  No alert detected\n'
        for lifecycle in ('{', 'null', '[]', '1', '{"remaining_owned_children": [123]}'):
            with self.subTest(lifecycle=lifecycle):
                result = audit_browser_evidence(stdout, lifecycle)
                self.assertFalse(result['complete'])
                self.assertEqual(result['browser_lifecycle_audit'], 'failed')
