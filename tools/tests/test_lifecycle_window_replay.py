"""E2 observer rejection checks; fixtures do not stand for VM execution."""
import copy
import json
from pathlib import Path
import sys
import time
import unittest
from unittest.mock import Mock, call, patch

SYSTEM = Path(__file__).resolve().parents[2] / 'tests/system'
sys.path.insert(0, str(SYSTEM))
import lifecycle_unload_scenario as scene
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
        value.call.side_effect = [(code, b'private response not copied') for code in [409,409,404,409,409,404,404]]
        result = probe.rejected_original_requests(value)
        self.assertEqual([item['status'] for item in result], [409,409,404,409,409,404,404])
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
        expected=[409,409,404,409,409,404,404]
        for index in range(7):
            for wrong in {200,202,400,401,403,404,409,500,503}-{expected[index]}:
                with self.subTest(index=index,status=wrong):
                    codes=expected.copy();codes[index]=wrong
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

    def exercise_fresh_window(self, *, change=None):
        original = scene.PublicUnloadScenario(
            'http://127.0.0.1:1', 'owner-fixture', 'foreign-fixture',
            'deployment', time.monotonic() + 60)
        original.create_request = dict(window=1, sequence=1, parameters=dict(timeout_ms=1000))
        key = dict(deployment='deployment', owner='1', window=3, sequence=1)
        original.create_acceptance = dict(session='original', key=dict(key, window=1), accepted_version=1)
        accepted = dict(session='successor', key=key, accepted_version=1)
        running = dict(session='successor', operation=key, current_version=1, phase='running',
                       unload_outcome=None, create_refusal=None)
        destroy_key = dict(key, sequence=2)
        destroy = dict(session='successor', key=destroy_key, accepted_version=2)
        cleaned = dict(running, operation=destroy_key, current_version=2, phase='cleaned')

        def reordered(value):
            if not isinstance(value, dict):
                return value
            return {name: reordered(item) for name, item in reversed(list(value.items()))}

        replies = [
            (200, {'window': 3}),  # Original client after reopen
            (200, {'window': 3}),  # Successor before Create
            (202, accepted),
            (200, running),
            (202, reordered(accepted)),
            (200, reordered(running)),
            (200, reordered(running)),
            (202, destroy),
            (200, cleaned),
        ]
        if change is not None:
            index, fields, value = change
            target = replies[index][1]
            for field in fields[:-1]:
                target = target[field]
            target[fields[-1]] = value

        runtime = Mock(gc_complete=True)
        runtime.record_running.return_value = {'original': 'physical-candidate'}
        runtime.confirm_running.return_value = runtime.record_running.return_value
        with patch.object(probe, 'wait_retired_window', return_value={'current_window': 3}), \
                patch.object(probe, 'rejected_original_requests', return_value=[]) as reject, \
                patch.object(scene, 'fetch', side_effect=[
                    (status, json.dumps(body).encode()) for status, body in replies]) as fetch, \
                patch.object(scene.PublicUnloadScenario, 'run_public_workload', return_value={}), \
                patch.object(scene.PublicUnloadScenario, 'assert_data_closed'):
            if change is None:
                result = probe.exercise(original, runtime)
                self.assertEqual(result['fresh_acceptance'], accepted)
                self.assertEqual(result['successor_after_rejection'], runtime.record_running.return_value)
                self.assertEqual(fetch.call_count, len(replies))
                self.assertEqual(reject.call_args_list, [call(original)] * 3)
                runtime.destroy_evidence.assert_called_once_with('successor', original.deadline)
                self.assertEqual(runtime.settlement_evidence.call_count, 3)
                return
            expected = ('retired request changed the successor session' if index == 6
                        else 'new-window replay changed')
            with self.assertRaisesRegex(ValueError, expected):
                probe.exercise(original, runtime)
            self.assertEqual(fetch.call_count, index + 1)
            self.assertEqual(reject.call_count, 3 if index == 6 else 2)
            self.assertEqual(runtime.confirm_running.call_count, 1 if index == 6 else 0)
            runtime.destroy_evidence.assert_not_called()
            self.assertEqual(runtime.settlement_evidence.call_count, 2)

    def test_exercise_accepts_same_fresh_identity_with_reordered_json(self):
        self.exercise_fresh_window()

    def test_exercise_rejects_aliases_in_fresh_replay_and_both_queries(self):
        for index in (4, 5, 6):
            if index == 4:
                fields = [('accepted_version',), ('key', 'sequence')]
                window = ('key', 'window')
            else:
                fields = [('current_version',), ('operation', 'sequence')]
                window = ('operation', 'window')
            for field, values in [(name, (True, 1.0)) for name in fields] + [(window, (3.0,))]:
                for value in values:
                    with self.subTest(response=index, field=field, value=value):
                        self.exercise_fresh_window(change=(index, field, value))


if __name__ == '__main__':
    unittest.main()
