"""Mutation checks against the real generated Status artifact and its roots."""
import copy
import json
from pathlib import Path
import unittest

from tools.check_schema_domains import (GENERATED, GUARD_LLBC, check_guards,
                                       check_sources, load_environment)

ROOT = Path(__file__).resolve().parents[2]


class SchemaDomainsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.environment = load_environment(ROOT)
        cls.llbc = json.loads((ROOT / GUARD_LLBC).read_text())
        cls.inventory = json.loads((ROOT / GENERATED / 'translation.json').read_text())

    def test_actual_original_guard_artifact_and_schema_are_admitted(self):
        self.assertEqual(check_sources(ROOT, self.environment), 4)
        self.assertEqual(check_guards(ROOT, self.llbc, self.inventory), 2)

    def test_charon_error_and_missing_root_are_not_semantic_counterexamples(self):
        changed = copy.deepcopy(self.llbc)
        changed['has_errors'] = True
        with self.assertRaisesRegex(ValueError, 'Charon errors'):
            check_guards(ROOT, changed, self.inventory)
        changed = copy.deepcopy(self.inventory)
        changed['functions'] = [f for f in changed['functions'] if not
                                f['rust_name'].endswith('::validate_for')]
        with self.assertRaisesRegex(ValueError, 'guard root'):
            check_guards(ROOT, self.llbc, changed)

    def test_owned_guard_cannot_be_made_opaque(self):
        changed = copy.deepcopy(self.llbc)
        for function in changed['translated']['fun_decls']:
            if function and function['item_meta']['is_local']:
                function['body'] = 'Opaque'
                break
        with self.assertRaisesRegex(ValueError, 'body hidden'):
            check_guards(ROOT, changed, self.inventory)

    def test_whole_guard_cannot_become_an_external_library_assumption(self):
        changed = copy.deepcopy(self.inventory)
        next(f for f in changed['functions'] if f['is_local'])['is_opaque'] = True
        with self.assertRaisesRegex(ValueError, 'external surface'):
            check_guards(ROOT, self.llbc, changed)

    def test_dropped_source_and_wrong_target_fail_admission(self):
        changed = copy.deepcopy(self.llbc)
        next(f for f in changed['translated']['files'] if f and
             f['name'] == {'Local': 'sandbox-internal/src/status.rs'})['contents'] = ''
        with self.assertRaisesRegex(ValueError, 'original guard source'):
            check_guards(ROOT, changed, self.inventory)
        changed = copy.deepcopy(self.llbc)
        changed['translated']['target_information'][0]['key'] = 'aarch64-unknown-linux-gnu'
        with self.assertRaisesRegex(ValueError, 'target mismatch'):
            check_guards(ROOT, changed, self.inventory)

    def test_borrow_check_disable_or_hidden_input_scope_is_rejected(self):
        for key, value in [('skip_borrowck', True), ('opaque', ['sandproof_internal'])]:
            with self.subTest(key=key):
                changed = copy.deepcopy(self.llbc)
                changed['translated']['options'][key] = value
                with self.assertRaisesRegex(ValueError, 'extraction options'):
                    check_guards(ROOT, changed, self.inventory)

    def test_ordered_ast_and_drop_changes_are_not_normalized_away(self):
        changed = copy.deepcopy(self.llbc)
        body = next(f for f in changed['translated']['fun_decls'] if f and
                    f['item_meta']['is_local'])['body']['Structured']['body']['statements']
        body.insert(0, {'kind': {'Drop': {'unexpected': 'effect must not be erased'}}})
        with self.assertRaisesRegex(ValueError, 'complete guard LLBC changed'):
            check_guards(ROOT, changed, self.inventory)

    def test_schema_field_and_production_source_drift_are_rejected(self):
        changed = copy.deepcopy(self.environment)
        next(s for s in changed['schemas'] if s['name'] == 'HostStatus')['fields'].pop()
        with self.assertRaisesRegex(ValueError, 'field/type/order mismatch'):
            check_sources(ROOT, changed)
        changed = copy.deepcopy(self.environment)
        changed['source_files']['sandbox-internal/src/status.rs'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'production source changed'):
            check_sources(ROOT, changed)


if __name__ == '__main__':
    unittest.main()
