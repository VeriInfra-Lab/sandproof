import copy
import json
from pathlib import Path
import unittest

from tools.check_json_request_binding import check_registry, snapshot

ROOT = Path(__file__).resolve().parents[2]


class JsonRequestBindingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.registry = json.loads((ROOT / 'verification/json-request-binding.json').read_text())

    def test_current_source_and_finite_trust_surface(self):
        self.assertEqual(set(check_registry(ROOT)['entries']), {'json_request::decode', 'json_request::encode'})

    def test_production_drift_and_missing_provider_source_are_rejected(self):
        registry = copy.deepcopy(self.registry)
        registry['production_sources']['sandbox-internal/src/server.rs'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'production binding source changed'):
            check_registry(ROOT, registry)
        registry = copy.deepcopy(self.registry)
        del registry['library_sources']['serde_json-1.0.142/src/error.rs']
        with self.assertRaisesRegex(ValueError, 'source inventory'):
            check_registry(ROOT, registry)

    def test_complete_field_and_original_leaf_cannot_be_omitted(self):
        registry = copy.deepcopy(self.registry)
        registry['structs']['CreateBinding']['fields_in_declaration_order'].pop()
        with self.assertRaisesRegex(ValueError, 'schema fields/types/order'):
            check_registry(ROOT, registry)
        registry = copy.deepcopy(self.registry)
        del registry['leaf_bindings']['strings']['create.template_revision']
        with self.assertRaisesRegex(ValueError, 'custom field path'):
            check_registry(ROOT, registry)

    def test_no_implicit_additional_trusted_root_or_drop_domain(self):
        registry = copy.deepcopy(self.registry)
        registry['entries']['json_request::validate'] = registry['entries']['json_request::decode']
        with self.assertRaisesRegex(ValueError, 'trusted binding surface'):
            check_registry(ROOT, registry)
        registry = copy.deepcopy(self.registry)
        registry['drop_domain']['owned'].append('OwnedSemaphorePermit')
        with self.assertRaisesRegex(ValueError, 'Drop scope'):
            check_registry(ROOT, registry)

    def test_same_api_with_wrong_concrete_type_is_rejected(self):
        registry = copy.deepcopy(self.registry)
        entry = registry['entries']['json_request::decode']['snapshot']['root']['item_meta']
        entry['source_text'] = entry['source_text'].replace('LifecycleRequest', 'ProbeRequest')
        with self.assertRaisesRegex(ValueError, 'real API directly'):
            check_registry(ROOT, registry)

    def fixture(self, kind):
        registered = self.registry['entries']['json_request::' + kind]['snapshot']
        # Complete original wrapper/callee ASTs, not copied business algorithms.
        # The production gate additionally compares the full regenerated LLBC.
        return {'has_errors': False, 'translated': {
            'options': copy.deepcopy(registered['options']),
            'target_information': [{'key': 'x86_64-unknown-linux-gnu'}],
            'fun_decls': copy.deepcopy([registered['root'], registered['callee']]),
        }}

    def test_exact_registered_wrapper_ast_is_admitted(self):
        for kind in ('decode', 'encode'):
            with self.subTest(kind=kind):
                self.assertEqual(snapshot(self.fixture(kind), kind),
                                 self.registry['entries']['json_request::' + kind]['snapshot'])

    def test_wrong_target_and_wrong_argument_are_rejected(self):
        for kind in ('decode', 'encode'):
            with self.subTest(kind=kind):
                llbc = self.fixture(kind)
                llbc['translated']['fun_decls'][1]['item_meta']['name'][-1] = {'Ident': ['wrong_api', 0]}
                with self.assertRaisesRegex(ValueError, 'original JSON target'):
                    snapshot(llbc, kind)
                llbc = self.fixture(kind)
                kinds = llbc['translated']['fun_decls'][0]['body']['Structured']['body']['statements']
                kinds[3]['kind']['Call']['call']['args'][0]['Move']['kind'] = {'Local': 1}
                with self.assertRaisesRegex(ValueError, 'argument or result forwarding'):
                    snapshot(llbc, kind)

    def test_body_opaque_or_extra_effect_cannot_pass(self):
        for kind in ('decode', 'encode'):
            with self.subTest(kind=kind):
                llbc = self.fixture(kind)
                llbc['translated']['fun_decls'][0]['body'] = 'Opaque'
                with self.assertRaisesRegex(ValueError, 'transparent binding body missing'):
                    snapshot(llbc, kind)
                llbc = self.fixture(kind)
                statements = llbc['translated']['fun_decls'][0]['body']['Structured']['body']['statements']
                statements.insert(4, {'kind': {'Drop': {'kind': 'Precise'}}})
                with self.assertRaisesRegex(ValueError, 'single direct call'):
                    snapshot(llbc, kind)

    def test_type_dictionary_and_extraction_scope_cannot_be_hidden(self):
        for kind in ('decode', 'encode'):
            with self.subTest(kind=kind):
                llbc = self.fixture(kind)
                call = llbc['translated']['fun_decls'][0]['body']['Structured']['body']['statements'][3]
                call['kind']['Call']['call']['func']['Regular']['generics']['trait_refs'] = []
                with self.assertRaisesRegex(ValueError, 'derive dictionary missing'):
                    snapshot(llbc, kind)
                llbc = self.fixture(kind)
                llbc['translated']['options']['opaque'] = ['sandproof_protocol::_']
                with self.assertRaisesRegex(ValueError, 'extraction options'):
                    snapshot(llbc, kind)


if __name__ == '__main__':
    unittest.main()
