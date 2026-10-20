"""Keep reference evidence distinct from proved callers and complete coverage."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import check_shared_field_bindings as checker

ROOT = Path(__file__).resolve().parents[2]


class SharedFieldCatalogue(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.original = checker.read(ROOT / checker.DIRECTORY / 'catalogue.json')
        cls.selected = checker.read(ROOT / checker.DIRECTORY / 'selection.json')

    def setUp(self):
        self.data = copy.deepcopy(self.original)
        self.selection = copy.deepcopy(self.selected)

    def reject(self, pattern=None):
        context = self.assertRaisesRegex(ValueError, pattern) if pattern else self.assertRaises(ValueError)
        with context:
            checker.check(ROOT, self.data, self.selection)

    def test_actual_finite_catalogue_and_original_capture_references_admitted(self):
        result = checker.check(ROOT, self.data, self.selection)
        self.assertEqual(result['families'], 11)
        self.assertEqual(result['new_theorems'], 0)
        self.assertEqual(result['new_extractions'], 0)

    def test_one_missing_or_duplicate_reviewed_location_rejected(self):
        self.data['entries'].pop()
        self.reject('missing/duplicate/extra')
        self.data = copy.deepcopy(self.original)
        self.data['entries'].append(copy.deepcopy(self.data['entries'][0]))
        self.reject('missing/duplicate/extra')

    def test_missing_family_rejected(self):
        del self.data['families']['D11']
        self.reject('scope/families')

    def test_unknown_caller_cannot_be_promoted_using_existing_helper(self):
        row = next(e for e in self.data['entries'] if e['id'] == 'D4.deadline-caller')
        self.assertTrue(row['proof_refs'])
        row['state'] = 'existing-implementation-theorem'
        self.reject('wrong location/domain/status')

    def test_raw_byte_input_cannot_be_relabelled_as_string(self):
        row = next(e for e in self.data['entries'] if e['id'] == 'D11.query-fields')
        row['domains'] = ['text', 'i64']
        self.reject('wrong location/domain/status')

    def test_local_id_cannot_inherit_positive_counter_domain(self):
        row = next(e for e in self.data['entries'] if e['id'] == 'D8.new')
        row['domains'] = ['positive-u64']
        self.reject('wrong location/domain/status')

    def test_ordinary_and_positive_zero_rules_remain_distinct(self):
        self.data['domains']['ordinary-u64']['minimum'] = 1
        self.reject('nonzero U64')

    def test_i64_is_not_an_unsigned_domain(self):
        self.data['domains']['i64']['signed'] = False
        self.reject('signed I64')

    def test_usize_requires_the_recorded_target(self):
        self.data['domains']['usize']['bits'] = 32
        self.reject('machine-word')

    def test_character_count_cannot_replace_utf8_byte_budget(self):
        self.data['domains']['opaque-id']['length_unit'] = 'characters'
        self.reject('byte/character')

    def test_opaque_id_and_canonical_number_have_different_leading_zero_rules(self):
        self.data['domains']['opaque-id']['leading_zero'] = 'rejected'
        self.reject('opaque/canonical')

    def test_none_and_some_empty_must_not_be_merged(self):
        self.data['domains']['bytes']['optional'] = 'empty-is-absent'
        self.reject('optional empty')

    def test_json_u16_representability_is_not_status_acceptance(self):
        self.data['domains']['slots']['accepted_maximum'] = 65535
        self.reject('status accepted')

    def test_equal_u16_type_does_not_merge_action_and_internal_versions(self):
        self.data['domains']['wire-u16']['version'] = 'all-guards-accept2'
        self.reject('same protocol version')
        self.data = copy.deepcopy(self.original)
        row = next(x for x in self.data['constants'] if x['id'] == 'action-version')
        row['literal'] = '2'
        self.reject('per-interface version constants')

    def test_source_or_exact_fragment_drift_is_rejected(self):
        self.data['entries'][0]['source']['text'] += '\n'
        self.reject('fragment changed')
        self.data = copy.deepcopy(self.original)
        path = self.data['entries'][0]['source']['path']
        self.data['sources'][path] = '0' * 64
        self.reject('referenced source changed')

    def test_struct_fields_cannot_omit_generation_or_change_signed_type(self):
        row = next(e for e in self.data['entries'] if e['id'] == 'D3.node')
        row['complete_fields'].remove('node_generation')
        self.reject('full struct fields')
        self.data = copy.deepcopy(self.original)
        row = next(e for e in self.data['entries'] if e['id'] == 'D11.versions-fields')
        row['field_types']['host_store'] = 'u64'
        self.reject('full struct fields')

    def test_nonexistent_or_wrong_audited_theorem_is_rejected(self):
        self.data['proofs']['seconds']['declaration'] = 'nonexistent_proof'
        self.reject('exact existing theorem')
        self.data = copy.deepcopy(self.original)
        self.data['proofs']['seconds']['qualified_name'] = 'SandboxModel.Foundations.resource_values_refines'
        self.reject('qualified audit target')

    def test_generated_opaque_or_unrelated_function_cannot_be_substituted(self):
        row = next(e for e in self.data['entries'] if e['id'] == 'D4.seconds')
        row['extraction']['functions'] = ['not_the_actual_function']
        self.reject('wrong reviewed theorem/generated')
        self.data = copy.deepcopy(self.original)
        row = next(e for e in self.data['entries'] if e['id'] == 'D4.seconds')
        row['proof_refs'] = ['resource-values']
        self.reject('wrong reviewed theorem/generated')

    def test_lexical_candidate_cannot_be_promoted_or_deleted(self):
        self.data['lexical_candidates'][0]['state'] = 'proved-production-call'
        self.reject('lexical candidates')
        self.data = copy.deepcopy(self.original)
        self.data['lexical_candidates'].pop()
        self.reject('lexical candidates')

    def test_missing_gap_or_business_only_reassignment_is_rejected(self):
        self.data['gaps'].pop()
        self.reject('reviewed gap')
        self.data = copy.deepcopy(self.original)
        self.data['gaps'][0]['provider'] = ['V4']
        self.reject('reassigned only to business')

    def test_exact_five_roots_and_all_units_remain_registered(self):
        self.data['t0']['roots'].remove('guest')
        self.reject('five-root')
        self.data = copy.deepcopy(self.original)
        self.data['t0']['units'] = 59
        self.reject('five-root')

    def test_duplicate_json_keys_and_escaping_source_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'duplicate.json'
            path.write_text('{"state":"unknown","state":"proved"}')
            with self.assertRaisesRegex(ValueError, 'duplicate JSON'):
                checker.read(path)
        with self.assertRaisesRegex(ValueError, 'escaping'):
            checker.file(ROOT, '../outside.rs')

    def test_local_frozen_closure_cannot_omit_its_checker_or_evidence(self):
        with patch.object(checker, 'manifest', return_value={}):
            with self.assertRaisesRegex(ValueError, 'local frozen catalogue closure'):
                checker.check_frozen(ROOT)

    def test_missing_or_rewritten_original_mir_summary_is_rejected(self):
        path = ROOT / checker.DIRECTORY / 'capture-summary.json'
        original = checker.read(path)
        original_read = checker.read
        for mutation in ('missing-header', 'wrong-source', 'fake-proof-kind', 'wrong-unit'):
            record = copy.deepcopy(original)
            if mutation == 'missing-header':
                record['observations']['D11.query-text'] = []
            elif mutation == 'wrong-source':
                record['selected_source_sha256']['sandbox-guest-runtime/src/file_query.rs'] = '0' * 64
            elif mutation == 'fake-proof-kind':
                record['kind'] = 'aeneas-implementation-proved'
            else:
                record['units'].pop()
            def changed(p):
                return record if p == path else original_read(p)
            with patch.object(checker, 'read', side_effect=changed):
                self.reject()


if __name__ == '__main__':
    unittest.main()
