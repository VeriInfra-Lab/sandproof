"""Real retained-result expiry must not invalidate an observed terminal build."""
import copy
import threading
import unittest
from unittest.mock import Mock

from tools.bounded_acceptance import completion_build_observation
from tools.template_build_client import BuildClient
from tools.template_build_service import BuildHTTP
from tools.tests import test_template_build_retention as retention_fixtures


class TerminalBuildObservationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        retention_fixtures.RetentionTests.setUpClass()

    def setUp(self):
        self.fixture = retention_fixtures.RetentionTests()
        self.addCleanup(self.fixture.doCleanups)
        self.fixture.setUp()
        self.store = self.fixture.store
        server = BuildHTTP(('127.0.0.1', 0), self.store, [{'tenant': 1, 'secret': 'synthetic'}])
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        def cleanup():
            server.shutdown()
            server.server_close()
            thread.join()
        self.addCleanup(cleanup)
        client = BuildClient('http://127.0.0.1:' + str(server.server_port), 'synthetic')
        self.read = Mock(side_effect=lambda job: client.request('GET', '/sandproof/template-builds/' + job))

    def terminal(self, state):
        job = self.fixture.submit()
        initial = self.read(job)
        self.store.private_checkpoint(job, builder_settled={'requestWindow': None, 'sandboxID': None}, private_ack=1)
        # Synthetic completed backend; this test does not build or launch a VM.
        self.store.change(job, state, result={'templateId': 'synthetic'} if state == 'Ready' else None,
                          failure='SyntheticFailure' if state == 'Failed' else None, release=True)
        observed = self.read(job)
        return job, initial, observed

    def expire(self, job):
        self.fixture.now += 6  # Original five-second request window expires.
        self.store.retire_results()
        self.assertEqual(self.store.rows(), [])
        with self.assertRaisesRegex(RuntimeError, '^NotFound$'):
            self.read(job)
        self.read.reset_mock()

    def test_long_task_keeps_observed_ready_or_failed_after_actual_window_expiry(self):
        for state in ('Ready', 'Failed'):
            with self.subTest(state=state):
                job, _, observed = self.terminal(state)
                row = {'build_id': job, 'build': observed, 'status': 'timeout', 'rewards': {'reward': 0}}
                original = copy.deepcopy(row)
                self.expire(job)
                result = completion_build_observation(row, read_build=self.read)
                self.assertEqual(result, {'build': observed, 'build_observation': 'saved_terminal'})
                self.read.assert_not_called()
                self.assertEqual(row, original)  # No changed score or physical-cleanup claim.
                result['build']['state'] = 'changed output'
                self.assertEqual(row, original)

    def test_missing_or_uncertain_observation_cannot_turn_retirement_into_success(self):
        job, initial, _ = self.terminal('Failed')
        self.expire(job)
        for observed in (None, initial):
            with self.subTest(observed=observed):
                with self.assertRaisesRegex(RuntimeError, '^NotFound$'):
                    completion_build_observation({'build_id': job, 'build': observed},
                        read_build=self.read)
        self.assertEqual(self.read.call_count, 2)
        self.assertEqual(self.store.rows(), [])  # No replacement build or window.

    def test_mismatched_saved_identity_is_rejected_without_substituting_another_build(self):
        job, _, observed = self.terminal('Ready')
        self.read.reset_mock()
        with self.assertRaisesRegex(ValueError, 'mismatched'):
            completion_build_observation({'build_id': 'b' * 32, 'build': observed},
                read_build=self.read)
        self.read.assert_not_called()
        self.assertEqual(self.store.rows()[0]['id'], job)

    def test_unobserved_results_still_require_a_matching_live_terminal_read(self):
        job, initial, observed = self.terminal('Ready')
        self.read.reset_mock()
        result = completion_build_observation({'build_id': job, 'build': initial}, read_build=self.read)
        self.assertEqual(result, {'build': observed, 'build_observation': 'live_query'})
        self.read.assert_called_once_with(job)
        for bad in ({'buildID': 'other', 'state': 'Ready'}, initial):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                completion_build_observation({'build_id': job},
                    read_build=lambda _job: bad)
        self.expire(job)
        with self.assertRaisesRegex(RuntimeError, '^NotFound$'):
            completion_build_observation({'build_id': job, 'build': initial}, read_build=self.read)
