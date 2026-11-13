"""E2 observer rejection checks; fixtures do not stand for VM execution."""
import copy
from pathlib import Path
import sys
import unittest
from unittest.mock import Mock, patch

SYSTEM = Path(__file__).resolve().parents[2] / 'tests/system'
sys.path.insert(0, str(SYSTEM))
import lifecycle_window_replay as probe


def client():
    value = Mock()
    value.owner_key, value.foreign_key = 'owner-fixture', 'foreign-fixture'
    value.create_request = dict(window=1, sequence=1, parameters=dict(timeout_ms=1000))
    value.unload_request = dict(window=1, sequence=2, expected_version=1)
    value.resume_request = dict(window=1, sequence=3, expected_version=2)
    value.destroy_request = dict(window=1, sequence=4, expected_version=3)
    value.path.return_value = '/sandproof/lifecycle/sessions/original'
    value.remaining.return_value = 30
    value.assert_data_closed = Mock()
    value.assert_previous_tokens_closed = Mock()
    return value


class WindowRetirementTests(unittest.TestCase):
    def test_read_only_wait_keeps_original_request_and_never_allocates_a_key(self):
        value = client()
        original = copy.deepcopy(value.create_request)
        value.read_json.side_effect = [{'window': 1}, {'window': 1}, {'window': 3}]
        with patch.object(probe.time, 'sleep'):
            result = probe.wait_retired_window(value)
        self.assertEqual((result['original_window'], result['current_window'], result['polls']), (1, 3, 3))
        self.assertEqual(value.create_request, original)
        self.assertTrue(all(call.args[0] == 'GET' for call in value.read_json.call_args_list))
        value.create.assert_not_called()
        value.call.assert_not_called()

    def test_malformed_exhausted_and_backward_windows_do_not_become_expiry(self):
        for window in [None, True, 0, 2, 3.0, '3', 2**64]:
            with self.subTest(window=window):
                value = client()
                value.read_json.return_value = {'window': window}
                with self.assertRaises(ValueError):
                    probe.wait_retired_window(value)
        value = client()
        value.create_request['window'] = 3
        value.read_json.return_value = {'window': 1}
        with self.assertRaises(ValueError):
            probe.wait_retired_window(value)

    def test_original_deadline_expiry_propagates_without_retrying_a_write(self):
        value = client()
        value.read_json.return_value = {'window': 1}
        value.remaining.side_effect = TimeoutError('original deadline')
        with self.assertRaises(TimeoutError):
            probe.wait_retired_window(value)
        value.call.assert_not_called()

    def test_exact_original_requests_and_one_valid_alteration_are_checked(self):
        value = client()
        original = copy.deepcopy(value.create_request)
        value.call.side_effect = [(code, b'private response not copied') for code in [409,409,404,404,404,404,404]]
        result = probe.rejected_original_requests(value)
        self.assertEqual([item['status'] for item in result], [409,409,404,404,404,404,404])
        self.assertNotIn('private response', repr(result))
        self.assertEqual(value.call.call_args_list[0].kwargs['body'], original)
        changed = value.call.call_args_list[1].kwargs['body']
        self.assertEqual(changed, dict(original, parameters={'timeout_ms':999}))
        for index, operation in enumerate(['unload', 'resume', 'destroy'], start=2):
            self.assertEqual(value.call.call_args_list[index].kwargs['body'], getattr(value,operation+'_request'))
        self.assertEqual(value.create_request, original)
        value.assert_data_closed.assert_called_once_with()
        value.assert_previous_tokens_closed.assert_called_once_with()

    def test_acceptance_wrong_classification_or_foreign_query_never_passes(self):
        for index in range(7):
            with self.subTest(index=index):
                codes=[409,409,404,404,404,404,404]
                codes[index]=202 if index<5 else 200
                value=client()
                value.call.side_effect=[(code,b'') for code in codes]
                with self.assertRaises(ValueError):
                    probe.rejected_original_requests(value)

    def test_new_window_requires_distinct_session_and_complete_original_key(self):
        original=dict(session='old',key=dict(deployment='d',owner='1',window=1,sequence=9),accepted_version=8)
        fresh=dict(session='new',key=dict(original['key'],window=3),accepted_version=1)
        probe.check_fresh_acceptance(original,fresh,3)
        for field,bad in [('deployment','other'),('owner','2'),('window',1),('sequence',10)]:
            changed=copy.deepcopy(fresh);changed['key'][field]=bad
            with self.subTest(field=field),self.assertRaises(ValueError):
                probe.check_fresh_acceptance(original,changed,3)
        for field,bad in [('session','old'),('session',''),('accepted_version',2),('accepted_version',True)]:
            with self.subTest(field=field),self.assertRaises(ValueError):
                probe.check_fresh_acceptance(original,dict(fresh,**{field:bad}),3)

    def test_window_exercise_requires_prior_actual_product_settlement(self):
        runtime=Mock();runtime.gc_complete=False
        with self.assertRaises(ValueError):
            probe.exercise(client(),runtime)
        runtime.restart_managers.assert_not_called()


if __name__ == '__main__':
    unittest.main()
