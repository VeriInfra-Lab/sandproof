"""Synthetic five-case packet admission; never Rust, Lean or CI evidence."""
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import acceptance_receipts as receipts


class PauseDispatchScopeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.cache = Path(temporary.name)
        self.runtime = {'synthetic_runtime': True}
        self.repository = {'fixture.rs': 'synthetic-source'}
        receipts.write_json(self.cache/'runtime.json', self.runtime)
        for name, value in (('rules', {}), ('contract_inputs', {}),
                            ('source_archive_inputs', {}), ('repository_inputs', self.repository)):
            patch = mock.patch.object(receipts, name, return_value=value)
            patch.start()
            self.addCleanup(patch.stop)
        patch = mock.patch.object(receipts, 'native_key', side_effect=self.key)
        patch.start()
        self.addCleanup(patch.stop)
        patch = mock.patch.dict(os.environ, {
            'SANDPROOF_NATIVE_EVIDENCE': str(self.cache),
            'SANDPROOF_ACCEPTANCE_ROLE': 'formal',
        })
        patch.start()
        self.addCleanup(patch.stop)

    def key(self, label, command, source):
        return dict(schema=receipts.NATIVE_SCHEMA, label=label,
                    command=['component-case'], rust=self.repository, runtime=self.runtime)

    def packet(self, *, labels=None, scope='pause-dispatch'):
        commands = {}
        for label in receipts.PAUSE_DISPATCH_LABELS if labels is None else labels:
            key = self.key(label, None, None)
            entry = self.cache/'commands'/receipts.digest(key)
            entry.mkdir(parents=True, exist_ok=True)
            (entry/'evidence').mkdir(exist_ok=True)
            (entry/'original.log').write_text('synthetic packet integrity fixture\n')
            (entry/'evidence/source.tar').write_bytes(b'synthetic archive, not real Rust')
            record = dict(complete=True, exit_code=0, key=key, files={
                name: receipts.sha(entry/name) for name in ('original.log', 'evidence/source.tar')})
            receipts.write_json(entry/'receipt.json', record)
            commands[entry.name] = receipts.sha(entry/'receipt.json')
        seal = dict(schema=receipts.NATIVE_SCHEMA, scope=scope, scope_complete=True,
                    full_native_suite_complete=scope == 'all', complete=True, exit_code=0,
                    rules={}, runtime=self.runtime, rust=self.repository, contracts={},
                    source_archives={}, repository_sha256=receipts.digest(self.repository),
                    commands=commands)
        receipts.write_json(self.cache/'complete.json', seal)
        return seal

    def test_parent_validates_all_five_before_three_old_children_consume(self):
        self.packet()
        original = {str(p): p.read_bytes() for p in self.cache.rglob('*') if p.is_file()}
        for scope in ('pause-dispatch', *receipts.PAUSE_DISPATCH_CHILD_SCOPES):
            self.assertEqual(receipts.validate_native(self.cache, scope=scope)['scope'], 'pause-dispatch')
        for label in receipts.PAUSE_DISPATCH_CHILD_SCOPES:
            receipt = receipts.native_case(label, receipts.ROOT, None, scope=label)
            self.assertEqual(receipt['key']['label'], label)
        self.assertEqual(original, {str(p): p.read_bytes() for p in self.cache.rglob('*') if p.is_file()})

    def test_parent_never_supplies_all_or_v3_scopes(self):
        self.packet()
        for scope in ('all', 'stage1-protocol', 'stage1-create-delivery', 'stage1-protocol-convergence'):
            with self.subTest(scope=scope), self.assertRaises(ValueError):
                receipts.validate_native(self.cache, scope=scope)

    def test_every_missing_case_blocks_even_an_otherwise_present_child(self):
        for omitted in receipts.PAUSE_DISPATCH_LABELS:
            self.packet(labels=[name for name in receipts.PAUSE_DISPATCH_LABELS if name != omitted])
            with self.subTest(omitted=omitted), self.assertRaises(ValueError):
                receipts.validate_native(self.cache, scope='capture-caller')

    def test_extra_empty_and_duplicate_component_records_are_rejected(self):
        for label in ('foreign-component', ''):
            self.packet(labels=(*receipts.PAUSE_DISPATCH_LABELS, label))
            with self.subTest(label=label), self.assertRaises(ValueError):
                receipts.validate_native(self.cache, scope='pause-stop')
        keys = [self.key(label, None, None) for label in receipts.PAUSE_DISPATCH_LABELS]
        keys[-1] = copy.deepcopy(keys[0])
        with self.assertRaises(ValueError):
            receipts.check_native_components(keys, scope='pause-dispatch')

    def test_wrong_command_kind_cannot_serve_as_a_component(self):
        keys = [self.key(label, None, None) for label in receipts.PAUSE_DISPATCH_LABELS]
        keys[-1]['command'] = ['some-unrelated-command']
        with self.assertRaises(ValueError):
            receipts.check_native_components(keys, scope='pause-dispatch')

    def test_corrupt_or_failed_unrequested_parent_case_blocks_child(self):
        for defect in ('missing-log', 'failed-receipt', 'changed-key'):
            seal = self.packet()
            entry = self.cache/'commands'/receipts.digest(self.key('pause-dispatch', None, None))
            if defect == 'missing-log':
                (entry/'original.log').unlink()
            else:
                receipt = json.loads((entry/'receipt.json').read_text())
                if defect == 'failed-receipt':
                    receipt['complete'] = False
                else:
                    receipt['key']['rust'] = {'fixture.rs': 'foreign-source'}
                receipts.write_json(entry/'receipt.json', receipt)
                seal['commands'][entry.name] = receipts.sha(entry/'receipt.json')
                receipts.write_json(self.cache/'complete.json', seal)
            with self.subTest(defect=defect), self.assertRaises(ValueError):
                receipts.validate_native(self.cache, scope='capture-caller')

    def test_current_source_change_rejects_original_parent(self):
        self.packet()
        self.repository = {'fixture.rs': 'changed-source'}
        with self.assertRaisesRegex(ValueError, 'foreign current inputs'):
            receipts.validate_native(self.cache, scope='pause-publication')

    def test_an_old_child_packet_cannot_be_renamed_to_the_parent(self):
        self.packet(labels=receipts.PAUSE_STOP_LABELS)
        with self.assertRaises(ValueError):
            receipts.validate_native(self.cache, scope='pause-stop')

    def test_all_keeps_its_existing_additional_case_rule(self):
        self.packet(labels=(*receipts.PAUSE_DISPATCH_LABELS, 'another-original-case'), scope='all')
        self.assertTrue(receipts.validate_native(self.cache)['full_native_suite_complete'])
        self.assertTrue(receipts.validate_native(self.cache, scope='pause-dispatch')['complete'])


if __name__ == '__main__':
    unittest.main()
