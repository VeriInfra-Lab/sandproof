"""Actual public JSON consumers with controlled HTTP/guest providers; no VM."""
import copy
import json
import threading
import time
import unittest
from unittest.mock import Mock, patch
from types import SimpleNamespace

from tools.tests import test_lifecycle_unload_runtime as fixture


class OriginalGuestFixtureTests(unittest.TestCase):
    def client(self, fault=None, *, session='occupant'):
        from tools.tests.test_lifecycle_unload_runtime import RepeatedLifecycleRequestTests
        fixture=RepeatedLifecycleRequestTests()
        client=fixture.client(owner='2')
        client.session=client.create_acceptance['session']=session
        client.data_token='original-credential'
        client.witness = dict(nonce='1'*64,activation_nonce='2'*64,
            metadata_generation=0,counter=1,pid=71,start_time_ticks=123)
        current = copy.deepcopy(client.witness)
        def command(text,**_):
            if text.endswith(' advance'):
                current['counter'] += 1
                if fault == 'guest-pid': current['pid'] += 1
            elif not text.endswith(' read'):
                self.fail('only original witness read/advance expected')
            if fault == 'ram': current['nonce'] = '3'*64
            return SimpleNamespace(exit_code=0,stdout=json.dumps(current))
        sdk = SimpleNamespace(commands=SimpleNamespace(run=Mock(side_effect=command)),
            files=SimpleNamespace(read=Mock(side_effect=lambda _: 'wrong' if fault=='disk'
                else 'disk-counter:'+str(current['counter']))))
        def connect():
            if fault == 'credential': client.data_token = 'replacement-credential'
            return sdk
        client.sdk = Mock(side_effect=connect)
        running = fixture.view(client,'running')
        client.query = Mock(return_value=running)
        client.assert_previous_tokens_closed = Mock()
        return client, sdk, running

    def test_original_guest_and_credential_continue_with_two_observed_advances(self):
        client, sdk, running = self.client()
        before = client.assert_running_workload(running)
        after = client.assert_running_workload(running)
        self.assertEqual((before['counter'],after['counter']),(2,3))
        self.assertEqual(before['guest_pid'],after['guest_pid'])
        self.assertEqual(before['ram_nonce_sha256'],after['ram_nonce_sha256'])
        self.assertEqual(sdk.commands.run.call_count,4)
        self.assertEqual(client.data_token,'original-credential')

    def test_changed_guest_ram_disk_or_credential_cannot_claim_continuity(self):
        for fault in ('guest-pid','ram','disk','credential'):
            client,_,running = self.client(fault)
            with self.subTest(fault=fault),self.assertRaises(AssertionError):
                client.assert_running_workload(running)



class PublicJsonIdentityTests(unittest.TestCase):
    def test_create_does_not_adopt_type_changed_operation_keys(self):
        for field, value in [('window', True), ('window', 1.0),
                             ('sequence', True), ('sequence', 1.0)]:
            client = fixture.scene.PublicUnloadScenario('http://127.0.0.1:1',
                'key', 'other', 'deployment', time.monotonic()+60, owner='2')
            accepted = dict(session='original', key=dict(deployment='deployment',
                owner='2', window=1, sequence=1), accepted_version=1)
            accepted['key'][field] = value
            responses = [(200, b'{"window":1}'), (202, json.dumps(accepted).encode())]
            with self.subTest(field=field, value=value), \
                    patch.object(client, 'call', side_effect=responses) as send:
                with self.assertRaises(AssertionError): client.create({})
                self.assertEqual(send.call_count, 2)
                self.assertIsNone(client.session)
                self.assertIsNone(client.current_acceptance)
                self.assertIsNone(client.timings[-1]['accepted_monotonic'])

    def test_current_operation_key_types_are_checked_before_sdk(self):
        for field, value in [('window', True), ('window', 1.0),
                             ('sequence', True), ('sequence', 1.0)]:
            client, sdk, running = OriginalGuestFixtureTests().client()
            changed = copy.deepcopy(running)
            changed['operation'][field] = value
            client.query.return_value = changed
            with self.subTest(field=field, value=value):
                with self.assertRaises(AssertionError): client.assert_running_workload(changed)
                client.sdk.assert_not_called()
                sdk.commands.run.assert_not_called()

    def test_last_decoded_query_cannot_erase_json_identity_types(self):
        for field, value in [('current_version', True), ('current_version', 1.0),
                             ('window', True), ('window', 1.0),
                             ('sequence', True), ('sequence', 1.0)]:
            client, sdk, running = OriginalGuestFixtureTests().client()
            changed = copy.deepcopy(running)
            (changed if field == 'current_version' else changed['operation'])[field] = value
            del client.query  # Keep the original query/read_json/unique-key parser.
            with self.subTest(field=field, value=value), \
                    patch.object(client, 'call', return_value=(200, json.dumps(changed).encode())):
                with self.assertRaises(AssertionError): client.assert_running_workload(running)
                self.assertEqual(sdk.commands.run.call_count, 2)

    def test_resume_replay_preserves_acceptance_and_complete_current_query_types(self):
        for point, field in [('acceptance', 'window'), ('acceptance', 'sequence'),
                             ('acceptance', 'accepted_version'), ('query', 'window'),
                             ('query', 'sequence'), ('query', 'current_version')]:
            client = fixture.RepeatedLifecycleRequestTests().client()
            client.current_acceptance = dict(session=client.session, key=client.key(5), accepted_version=5)
            current = fixture.RepeatedLifecycleRequestTests().view(client, 'running')
            accepted = dict(session=client.session, key=client.key(3), accepted_version=3)
            replay, queried = copy.deepcopy(accepted), copy.deepcopy(current)
            changed = replay if point == 'acceptance' else queried
            target = (changed if field in changed else changed['key' if point == 'acceptance' else 'operation'])
            target[field] = float(target[field])
            responses = [(202, json.dumps(replay).encode()), (200, json.dumps(queried).encode())]
            with self.subTest(point=point, field=field), \
                    patch.object(client, 'call', side_effect=responses):
                with self.assertRaises(AssertionError):
                    client.assert_resume_replay(dict(window=1, sequence=3, expected_version=2), accepted, current)

    def test_duplicate_unload_does_not_ignore_the_second_replies_types(self):
        for field, value in [('window', True), ('window', 1.0), ('accepted_version', 2.0)]:
            client = fixture.RepeatedLifecycleRequestTests().client()
            running = fixture.RepeatedLifecycleRequestTests().view(client, 'running')
            original = copy.deepcopy(client.current_acceptance)
            accepted = dict(session=client.session, key=client.key(2), accepted_version=2)
            changed = copy.deepcopy(accepted)
            (changed if field == 'accepted_version' else changed['key'])[field] = value
            def response(*_, **__):
                value = accepted if threading.current_thread().name.endswith('_0') else changed
                return 202, json.dumps(value).encode()
            with self.subTest(field=field, value=value), patch.object(client, 'call', side_effect=response):
                with self.assertRaises(AssertionError): client.unload(running, concurrent_duplicate=True)
                self.assertEqual(client.current_acceptance, original)
                self.assertIsNone(client.unload_acceptance)

    def test_reopen_rejects_changed_window_and_complete_public_snapshot_types(self):
        for point in ('query', 'window'):
            client = fixture.RepeatedLifecycleRequestTests().client()
            saved = fixture.RepeatedLifecycleRequestTests().view(client, 'running')
            changed = dict(saved, current_version=1.0) if point == 'query' else saved
            window = {'window': 1.0 if point == 'window' else 1}
            with self.subTest(point=point), patch.object(client, 'assert_owner_isolation'), \
                    patch.object(client, 'assert_original_replay') as replay, \
                    patch.object(client, 'call', side_effect=[(200, json.dumps(changed).encode()),
                                                            (200, json.dumps(window).encode())]):
                with self.assertRaises(AssertionError): client.assert_reopened(saved)
                replay.assert_not_called()

    def test_json_key_order_and_whitespace_preserve_valid_running_and_replay(self):
        client, _, running = OriginalGuestFixtureTests().client()
        reordered = dict(reversed(list(running.items())))
        reordered['operation'] = dict(reversed(list(running['operation'].items())))
        del client.query
        with patch.object(client, 'call', return_value=(200, json.dumps(reordered, indent=2).encode())):
            result = client.assert_running_workload(running)
            self.assertEqual(result['counter'], 2)
            self.assertEqual(result['session'], client.session)
        client.current_acceptance = dict(session=client.session, key=client.key(5), accepted_version=5)
        running = fixture.RepeatedLifecycleRequestTests().view(client, 'running')
        reordered = dict(reversed(list(running.items())))
        accepted = dict(session=client.session, key=client.key(3), accepted_version=3)
        with patch.object(client, 'call', side_effect=[(202, json.dumps(accepted, indent=2).encode()),
                                                       (200, json.dumps(reordered, indent=2).encode())]):
            client.assert_resume_replay(dict(window=1, sequence=3, expected_version=2), accepted, running)

    def test_failed_unload_preserves_original_lifetime_types_before_guest_io(self):
        for field, value in [('started_at_ms', True), ('started_at_ms', 1.0),
                             ('end_at_ms', 100.0), ('max_end_at_ms', 200.0),
                             ('startup_deadline_ms', 20.0), (None, None)]:
            client, sdk, _ = OriginalGuestFixtureTests().client()
            accepted = dict(session=client.session, key=client.key(2), accepted_version=2)
            client.current_acceptance = client.unload_acceptance = accepted
            lifetime = dict(started_at_ms=1, end_at_ms=100, max_end_at_ms=200, startup_deadline_ms=20)
            facts = dict(lifetime, observed_at_ms=2)
            if field is not None: facts[field] = value
            running = fixture.RepeatedLifecycleRequestTests().view(client, 'running')
            running['unload_outcome'] = dict(state='original_continued', facts=facts)
            client.query.return_value = running
            with self.subTest(field=field, value=value):
                if field is None:
                    result = client.assert_continued_workload(running, lifetime)
                    self.assertTrue(result['same_lifetime'])
                    self.assertEqual(result['counter'], 2)
                else:
                    with self.assertRaises(AssertionError): client.assert_continued_workload(running, lifetime)
                    client.sdk.assert_not_called();sdk.commands.run.assert_not_called()

    def test_independent_unloaded_facts_do_not_erase_digest_or_lifetime_types(self):
        client = fixture.RepeatedLifecycleRequestTests().client()
        client.current_acceptance = client.unload_acceptance = dict(
            session=client.session, key=client.key(2), accepted_version=2)
        public = dict(source_incarnation=1, capture=1, material_bytes=1, manifest_digest=[1]*32,
            started_at_ms=0, end_at_ms=100, max_end_at_ms=200, startup_deadline_ms=20)
        view = fixture.RepeatedLifecycleRequestTests().view(client, 'unloaded')
        view['unload_outcome']['facts'] = public
        for field, value in [('manifest_digest', [True]+[1]*31),
                             ('manifest_digest', [1.0]+[1]*31),
                             ('started_at_ms', False), ('material_bytes', 1.0)]:
            independent = copy.deepcopy(public);independent[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(AssertionError):
                client.assert_unloaded_facts(view, independent)
        client.assert_unloaded_facts(view, dict(reversed(list(public.items()))))


if __name__ == '__main__':
    unittest.main()
