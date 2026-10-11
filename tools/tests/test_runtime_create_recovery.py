"""Audit observer regressions; not a mock controller or KVM-cleanup evidence."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock

from tools.runtime_create_recovery import (
    RuntimeIdentityUnknown, find_runtime_create, load_runtime_intent,
    recover_runtime_create, terminal_result,
)


class RuntimeCreateRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        self.template = json.loads((Path(__file__).resolve().parents[2] /
                                   'tests/compat/template-view.json').read_text())
        self.intent = dict(metadata=dict(benchmark='terminal-bench-2.0', task='sample',
            build='a'*32, revision='r1', runtime_attempt='e'*32), template=self.template)
        self.node = dict(deployment_id='test-deployment', node_id='host-runtime', node_generation=3)
        self.identity = dict(protocol_version=2, node=self.node, tenant_id='1', sandbox_id='sandbox-17',
                             operation_id='operation-41', intent_version=1, retention_slot=1, request_window=2)
        previous = dict(identity=dict(self.identity, sandbox_id='sandbox-16', request_window=1),
                        observation_version=2, fact=dict(kind='cleaned'))
        versions = dict(internal_protocol=2, center_store=6, host_store=16)
        self.center = dict(schema_version=2, versions=versions, deployment='test-deployment', sandboxes=[],
            retention_lanes=[dict(node=self.node, retention_slot=1, retired_through=1, last_retired=previous)])
        self.host = dict(schema_version=2, versions=versions, node=self.node, operations=[],
            lanes=[dict(retention_slot=1, retired_through=1, last_retired=previous)])
        self.now = 0

    def insert(self, *, pending=False, terminal=False):
        checksum = hashlib.sha256(json.dumps(self.intent['metadata'], sort_keys=True,
            separators=(',', ':')).encode()).hexdigest()
        resources = dict(instances=1, cpu=self.template['cpuCount'],
            memory_bytes=self.template['hostMemoryBytes'], disk_bytes=self.template['diskBytes'])
        self.center['sandboxes'] = [dict(private=False,
            create=dict(identity=copy.deepcopy(self.identity), metadata_sha256=checksum,
                        resources=resources), started_at_ms=None if pending else 1234,
            released=terminal, terminal_confirmation=None, operations=[])]
        self.host['operations'] = []
        if terminal:
            self.finish()

    def finish(self):
        identity = dict(self.identity, operation_id='operation-42', intent_version=2)
        response = dict(identity=identity, observation_version=3, fact=dict(kind='cleaned'))
        self.center['sandboxes'][0].update(released=True,
            terminal_confirmation=copy.deepcopy(response),
            operations=[dict(response=copy.deepcopy(response))])
        self.host['operations'] = [dict(identity=identity, current=True, local=None,
            response=copy.deepcopy(response), confirmed=copy.deepcopy(response))]

    def tick(self, seconds):
        self.now += seconds

    def recover(self, delete, **kwargs):
        return recover_runtime_create(self.intent, read_center=lambda: copy.deepcopy(self.center),
            read_host=lambda node: copy.deepcopy(self.host), deployment='test-deployment', owner=1,
            delete=delete, deadline=2, monotonic=lambda: self.now,
            sleep=kwargs.pop('sleep', self.tick), **kwargs)

    def test_late_pending_original_deleted_once_then_both_confirmations_observed(self):
        def advance(seconds):
            self.tick(seconds)
            if self.now == .5:
                self.insert(pending=True)
            elif self.now == 1:
                delete.assert_called_once_with('sandbox-17')
                self.finish()
        delete = Mock(return_value=204)
        result = self.recover(delete, sleep=advance)
        self.assertEqual(result['status'], 'destroyed')
        self.assertTrue(result['pending_create_observed'])
        self.assertTrue(result['delete_acknowledged'])
        self.assertNotIn('exact_cleanup', result)

    def test_ack_or_404_is_not_terminal_and_delete_is_never_replayed(self):
        for status in (204, 404):
            with self.subTest(status=status):
                self.now = 0
                self.insert(pending=True)
                delete = Mock(return_value=status)
                with self.assertRaises(TimeoutError):
                    self.recover(delete)
                delete.assert_called_once_with('sandbox-17')
                self.assertEqual(self.now, 2)

    def test_previous_sdk_kill_dispatch_is_observed_without_another_delete(self):
        self.insert()
        delete = Mock()
        def advance(seconds):
            self.tick(seconds)
            self.finish()
        result = self.recover(delete, delete_dispatched=True, sleep=advance)
        delete.assert_not_called()
        self.assertTrue(result['delete_dispatched'])
        self.assertFalse(result['delete_acknowledged'])

    def test_lost_delete_receipt_is_not_replayed_or_exported(self):
        self.insert()
        def lost(identity):
            self.finish()
            raise OSError('private credential-bearing transport text')
        delete = Mock(side_effect=lost)
        result = self.recover(delete)
        delete.assert_called_once_with('sandbox-17')
        self.assertEqual(result['status'], 'destroyed')
        self.assertFalse(result['delete_acknowledged'])
        self.assertNotIn('private credential', json.dumps(result))

    def test_absent_original_remains_unknown_without_public_list_or_delete(self):
        delete = Mock()
        with self.assertRaises(TimeoutError):
            self.recover(delete)
        delete.assert_not_called()

    def test_ambiguous_matches_fail_before_delete(self):
        self.insert()
        self.center['sandboxes'].append(copy.deepcopy(self.center['sandboxes'][0]))
        delete = Mock()
        with self.assertRaisesRegex(RuntimeError, 'ambiguous'):
            self.recover(delete)
        delete.assert_not_called()

    def test_foreign_owner_deployment_or_saved_sdk_identity_refuses(self):
        for field, value in (('tenant_id', '2'), ('sandbox_id', 'sb-old-17'),
                             ('sandbox_id', 'sandbox-18')):
            with self.subTest(field=field, value=value):
                self.insert()
                self.center['sandboxes'][0]['create']['identity'][field] = value
                delete = Mock()
                with self.assertRaisesRegex(RuntimeError, 'identity mismatch'):
                    self.recover(delete, sandbox_id='sandbox-17')
                delete.assert_not_called()
        self.insert()
        self.center['deployment'] = 'foreign'
        with self.assertRaises(ValueError):
            self.recover(Mock())

    def test_resource_binding_change_refuses_before_delete(self):
        for field in ('instances', 'cpu', 'memory_bytes', 'disk_bytes'):
            with self.subTest(field=field):
                self.insert()
                self.center['sandboxes'][0]['create']['resources'][field] += 1
                delete = Mock()
                with self.assertRaisesRegex(RuntimeError, 'resource binding changed'):
                    self.recover(delete)
                delete.assert_not_called()

    def test_metadata_mismatch_stays_unknown(self):
        self.insert()
        self.center['sandboxes'][0]['create']['metadata_sha256'] = 'f'*64
        delete = Mock()
        with self.assertRaises(TimeoutError):
            self.recover(delete)
        delete.assert_not_called()

    def test_terminal_requires_exact_both_confirmations_and_current_result(self):
        self.insert(terminal=True)
        center, host = copy.deepcopy(self.center), copy.deepcopy(self.host)
        self.assertEqual(terminal_result(center, host, self.identity)['status'], 'cleaned')
        for altered in (dict(host, operations=[]),
                dict(host, operations=[dict(host['operations'][0], current=False)]),
                dict(host, operations=[dict(host['operations'][0], confirmed=None)])):
            with self.subTest(host=altered):
                self.assertIsNone(terminal_result(center, altered, self.identity))
        center['sandboxes'][0]['terminal_confirmation'] = None
        self.assertIsNone(terminal_result(center, host, self.identity))

    def test_absence_one_frontier_or_stopped_is_not_cleanup(self):
        self.insert(terminal=True)
        self.center['sandboxes'][0]['operations'][0]['response']['fact']['kind'] = 'stopped'
        self.assertIsNone(terminal_result(self.center, self.host, self.identity))
        self.center['sandboxes'] = []
        self.retire(self.host['lanes'][0], 2)
        self.assertIsNone(terminal_result(self.center, self.host, self.identity))
        self.retire(self.center['retention_lanes'][0], 2)
        result = self.recover(Mock(), create_identity=self.identity)
        self.assertEqual(result['terminal']['status'], 'retired_terminal_slot')

    def test_saved_identity_cannot_consume_foreign_host_frontier(self):
        self.retire(self.center['retention_lanes'][0], 2)
        self.retire(self.host['lanes'][0], 2)
        self.host['node'] = dict(self.node, node_generation=4)
        with self.assertRaises(ValueError):
            self.recover(Mock(), create_identity=self.identity)

    def retire(self, lane, floor):
        lane.update(retired_through=floor, last_retired=dict(
            identity=dict(self.identity, operation_id='operation-42', intent_version=2, request_window=floor),
            observation_version=3, fact=dict(kind='cleaned')))

    def test_other_slot_retirement_does_not_prove_original_cleanup(self):
        self.center['sandboxes'] = []
        for lane in (self.center['retention_lanes'][0], self.host['lanes'][0]):
            lane['retention_slot'] = 2
            lane['last_retired']['identity']['retention_slot'] = 2
        with self.assertRaisesRegex(ValueError, 'slot frontier'):
            terminal_result(self.center, self.host, self.identity)

    def test_retirement_requires_exact_stamp_and_current_audit_schema(self):
        self.retire(self.center['retention_lanes'][0], 2)
        self.retire(self.host['lanes'][0], 2)
        for side in ('center', 'host'):
            for mutation in ('schema', 'version', 'binding', 'observation', 'missing'):
                center, host = copy.deepcopy(self.center), copy.deepcopy(self.host)
                audit = center if side == 'center' else host
                lane = audit['retention_lanes' if side == 'center' else 'lanes'][0]
                if mutation == 'schema': audit['schema_version'] = 1
                elif mutation == 'version': audit['versions']['host_store'] -= 1
                elif mutation == 'binding': lane['last_retired']['identity']['sandbox_id'] = 'sandbox-99'
                elif mutation == 'observation': lane['last_retired']['observation_version'] += 1
                else: lane['last_retired'] = None
                with self.subTest(side=side, mutation=mutation), self.assertRaises(ValueError):
                    terminal_result(center, host, self.identity)

    def test_terminal_response_cannot_change_original_retention_slot(self):
        self.insert(terminal=True)
        self.center['sandboxes'][0]['operations'][0]['response']['identity']['retention_slot'] = 2
        with self.assertRaisesRegex(ValueError, 'create binding'):
            terminal_result(self.center, self.host, self.identity)

    def test_retired_stamps_must_be_complete_and_typed_not_just_equal(self):
        self.retire(self.center['retention_lanes'][0], 2)
        self.retire(self.host['lanes'][0], 2)
        for mutation in ('missing_operation', 'missing_intent', 'missing_observation',
                         'bool_slot', 'float_schema', 'bool_generation', 'extra_fact'):
            center, host = copy.deepcopy(self.center), copy.deepcopy(self.host)
            for audit, key in ((center, 'retention_lanes'), (host, 'lanes')):
                stamp = audit[key][0]['last_retired']
                if mutation == 'missing_operation': del stamp['identity']['operation_id']
                elif mutation == 'missing_intent': del stamp['identity']['intent_version']
                elif mutation == 'missing_observation': del stamp['observation_version']
                elif mutation == 'bool_slot': stamp['identity']['retention_slot'] = True
                elif mutation == 'float_schema': audit['schema_version'] = 2.0
                elif mutation == 'bool_generation': stamp['identity']['node']['node_generation'] = True
                else: stamp['fact']['extra'] = 'unknown'
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                terminal_result(center, host, self.identity)

    def test_physical_check_precedes_delete_and_failure_retains_original(self):
        self.insert()
        before = Mock(side_effect=OSError('original physical witness unknown'))
        delete = Mock()
        with self.assertRaises(OSError):
            self.recover(delete, before_delete=before)
        before.assert_called_once()
        delete.assert_not_called()

    def test_original_pending_witness_can_defer_delete_without_create_replay(self):
        self.insert(pending=True)
        before = Mock(side_effect=[False, None])
        def finish(identity):
            self.finish()
            return 204
        delete = Mock(side_effect=finish)
        result = self.recover(delete, before_delete=before)
        self.assertEqual(result['status'], 'destroyed')
        self.assertEqual(before.call_count, 2)
        delete.assert_called_once_with('sandbox-17')

    def load_checkpoint(self, *, prepared_ci=False):
        return load_runtime_intent(self.root, task='sample', runtime_attempt='e'*32,
            build_id='a'*32, template=self.template, prepared_ci=prepared_ci)

    def test_prepared_checkpoint_and_recovery_require_explicit_supervisor_authority(self):
        self.template['templateId'] = 'ci-' + 'a'*32
        (self.root/'runtime-intent.json').write_text(json.dumps(self.intent))
        self.assertEqual(self.load_checkpoint(prepared_ci=True), self.intent)
        with self.assertRaises(ValueError):
            self.load_checkpoint()
        self.insert(terminal=True)
        self.assertEqual(self.recover(Mock(), prepared_ci=True)['status'], 'destroyed')
        with self.assertRaises(ValueError):
            self.recover(Mock())
        for mode, benchmark in ((True, 'terminal-bench@2.1'), (1, 'terminal-bench-2.0'),
                ('true', 'terminal-bench-2.0'), (None, 'terminal-bench-2.0')):
            with self.subTest(mode=mode, benchmark=benchmark), self.assertRaises(ValueError):
                self.recover(Mock(), prepared_ci=mode, benchmark=benchmark)

    def test_two_point_one_requires_exact_live_dataset_binding(self):
        self.intent['metadata']['benchmark'] = 'terminal-bench@2.1'
        (self.root/'runtime-intent.json').write_text(json.dumps(self.intent))
        with self.assertRaises(ValueError):
            self.load_checkpoint()
        self.assertEqual(load_runtime_intent(self.root, task='sample', runtime_attempt='e'*32,
            build_id='a'*32, template=self.template, benchmark='terminal-bench@2.1'), self.intent)
        self.insert(terminal=True)
        self.assertEqual(self.recover(Mock(), benchmark='terminal-bench@2.1')['status'], 'destroyed')
        for benchmark in ('terminal-bench-2.0', 'arbitrary-dataset'):
            with self.subTest(benchmark=benchmark), self.assertRaises(ValueError):
                self.recover(Mock(), benchmark=benchmark)

    def test_checkpoint_requires_exact_task_attempt_build_revision_and_fingerprint(self):
        checkpoint = self.root/'runtime-intent.json'
        checkpoint.write_text(json.dumps(self.intent))
        self.assertEqual(self.load_checkpoint(), self.intent)
        variants = []
        for field, value in (('task', 'previous-task'), ('runtime_attempt', 'f'*32),
                             ('build', 'b'*32), ('revision', 'r2')):
            altered = copy.deepcopy(self.intent)
            altered['metadata'][field] = value
            variants.append(altered)
        altered = copy.deepcopy(self.intent)
        altered['template']['rootfsSha256'] = 'f'*64
        variants.append(altered)
        for altered in variants:
            checkpoint.write_text(json.dumps(altered))
            with self.subTest(altered=altered), self.assertRaises(ValueError):
                self.load_checkpoint()
        with self.assertRaises(ValueError):
            load_runtime_intent(self.root, task='sample', runtime_attempt=None,
                                build_id='a'*32, template=self.template)

    def test_checkpoint_rejects_partial_duplicate_and_oversized_json(self):
        for raw in ('{', '{"metadata":{},"metadata":{}}', ' '*(256*1024+1)):
            (self.root/'runtime-intent.json').write_text(raw)
            with self.subTest(size=len(raw)), self.assertRaises(ValueError):
                self.load_checkpoint()

    def test_checkpoint_rejects_symlink_hardlink_fifo_and_public_directory(self):
        original, checkpoint = self.root/'original.json', self.root/'runtime-intent.json'
        original.write_text(json.dumps(self.intent))
        checkpoint.symlink_to(original)
        with self.assertRaises(OSError):
            self.load_checkpoint()
        checkpoint.unlink()
        os.link(original, checkpoint)
        with self.assertRaises(ValueError):
            self.load_checkpoint()
        checkpoint.unlink()
        os.mkfifo(checkpoint)
        with self.assertRaises(ValueError):
            self.load_checkpoint()
        checkpoint.unlink()
        checkpoint.write_text(json.dumps(self.intent))
        self.root.chmod(0o755)
        try:
            with self.assertRaises(ValueError):
                self.load_checkpoint()
        finally:
            self.root.chmod(0o700)

    def test_unknown_and_ambiguous_are_distinct_without_retirement_guess(self):
        with self.assertRaises(RuntimeIdentityUnknown):
            find_runtime_create(self.center, self.intent['metadata'], deployment='test-deployment')
        self.insert()
        self.center['sandboxes'].append(copy.deepcopy(self.center['sandboxes'][0]))
        with self.assertRaises(RuntimeError):
            find_runtime_create(self.center, self.intent['metadata'], deployment='test-deployment')


if __name__ == '__main__':
    unittest.main()
