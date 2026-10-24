"""Binding regressions use full original wrapper/callee snapshots; formal gate checks full LLBC."""
import copy
from pathlib import Path
import unittest

from tools.check_schema_domains import load_environment
from tools.check_schema_domains_binding import (KINDS, byte_snapshot, check_byte_registry)

from tools.tests.proof_input_fixture import reference_root

ROOT = reference_root()


class StatusByteBindingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.binding = load_environment(ROOT)['byte_binding']

    def fixture(self, kind):
        entry = self.binding['entries'][kind]['snapshot']
        return {'has_errors': False, 'translated': {
            'crate_name': 'sandproof_schema_domains_extract',
            'options': copy.deepcopy(entry['options']),
            'target_information': [{'key': 'x86_64-unknown-linux-gnu'}],
            'fun_decls': copy.deepcopy([entry['root'], entry['callee']])}}

    def call(self, llbc):
        return llbc['translated']['fun_decls'][0]['body']['Structured']['body']['statements'][3]['kind']['Call']

    def test_current_source_and_four_original_complete_snapshots(self):
        self.assertEqual(set(check_byte_registry(ROOT)['entries']), set(KINDS))
        for kind in KINDS:
            self.assertEqual(byte_snapshot(self.fixture(kind), kind),
                             self.binding['entries'][kind]['snapshot'])

    def test_failure_missing_root_and_wrong_target_are_not_counterexamples(self):
        for kind in KINDS:
            value = self.fixture(kind)
            value['has_errors'] = True
            with self.assertRaisesRegex(ValueError, 'Charon errors'):
                byte_snapshot(value, kind)
            value = self.fixture(kind)
            value['translated']['target_information'][0]['key'] = 'aarch64-unknown-linux-gnu'
            with self.assertRaisesRegex(ValueError, 'byte target'):
                byte_snapshot(value, kind)

    def test_opaque_body_additional_effect_and_disabled_checks_are_rejected(self):
        for kind in KINDS:
            value = self.fixture(kind)
            value['translated']['fun_decls'][0]['body'] = 'Opaque'
            with self.assertRaisesRegex(ValueError, 'transparent direct byte body'):
                byte_snapshot(value, kind)
            value = self.fixture(kind)
            value['translated']['fun_decls'][0]['body']['Structured']['body']['statements'].insert(
                4, {'kind': {'Drop': {'must': 'remain observable'}}})
            with self.assertRaisesRegex(ValueError, 'single direct call'):
                byte_snapshot(value, kind)
            for option, change in [('skip_borrowck', True), ('opaque', ['sandproof_protocol'])]:
                value = self.fixture(kind)
                value['translated']['options'][option] = change
                with self.assertRaisesRegex(ValueError, 'scope/options'):
                    byte_snapshot(value, kind)

    def test_wrong_object_result_callee_or_dictionary_cannot_pass(self):
        for kind in KINDS:
            value = self.fixture(kind)
            self.call(value)['call']['args'][0]['Move']['kind'] = {'Local': 1}
            with self.assertRaisesRegex(ValueError, 'argument/result'):
                byte_snapshot(value, kind)
            value = self.fixture(kind)
            value['translated']['fun_decls'][1]['item_meta']['name'][-1] = {'Ident': ['wrong', 0]}
            with self.assertRaisesRegex(ValueError, 'JSON callee'):
                byte_snapshot(value, kind)
            value = self.fixture(kind)
            self.call(value)['call']['func']['Regular']['generics']['trait_refs'] = []
            with self.assertRaisesRegex(ValueError, 'derive dictionary'):
                byte_snapshot(value, kind)

    def test_wrong_concrete_type_cannot_hide_in_same_library_api(self):
        changed = copy.deepcopy(self.binding)
        changed['entries']['response_decode']['snapshot']['root']['item_meta']['source_text'] = \
            changed['entries']['response_decode']['snapshot']['root']['item_meta']['source_text'].replace(
                'HostStatus', 'StatusRequest')
        with self.assertRaisesRegex(ValueError, 'concrete source'):
            check_byte_registry(ROOT, changed)

    def test_writer_or_resources_cannot_be_silently_trusted(self):
        for key, change in [('server_bounded_writer', 'covered-by-Vec'),
                            ('response_encode_role', 'production-server')]:
            changed = copy.deepcopy(self.binding)
            changed[key] = change
            with self.assertRaisesRegex(ValueError, 'server Writer scope'):
                check_byte_registry(ROOT, changed)
        changed = copy.deepcopy(self.binding)
        changed['drop_domain']['owned'].append('OwnedSemaphorePermit')
        with self.assertRaisesRegex(ValueError, 'Drop'):
            check_byte_registry(ROOT, changed)

    def test_stale_source_missing_library_and_budget_change_are_rejected(self):
        changed = copy.deepcopy(self.binding)
        changed['wrapper_sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'wrapper source'):
            check_byte_registry(ROOT, changed)
        changed = copy.deepcopy(self.binding)
        del changed['input_sources']['Cargo.lock']
        with self.assertRaisesRegex(ValueError, 'input source inventory'):
            check_byte_registry(ROOT, changed)
        changed = copy.deepcopy(self.binding)
        del changed['library_sources']['serde_json-1.0.142/src/error.rs']
        with self.assertRaisesRegex(ValueError, 'source inventory'):
            check_byte_registry(ROOT, changed)
        changed = copy.deepcopy(self.binding)
        changed['budgets']['response'] += 1
        with self.assertRaisesRegex(ValueError, 'budget'):
            check_byte_registry(ROOT, changed)


if __name__ == '__main__':
    unittest.main()
