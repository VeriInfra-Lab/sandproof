"""Require the observed whole-value substitution and the exact native assertion."""
import copy
import json
import os
from pathlib import Path
import unittest

import native
from source_binding import HERE, MUTATED_ROOTS, check_rejection


class ActualMutantBindings(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw = {
            variant: json.loads(Path(os.environ[variable]).read_text())
            for variant, variable in (
                ('correct', 'SANDPROOF_UNLOAD_JSON_LLBC'),
                ('origin-request', 'SANDPROOF_UNLOAD_JSON_ORIGIN_LLBC'),
                ('options-empty', 'SANDPROOF_UNLOAD_JSON_OPTIONS_LLBC'),
            )
        }
        cls.profile = json.loads((HERE / 'profile.json').read_text())

    def test_each_actual_variant_reaches_its_selected_wrapper(self):
        for variant, wrapper in MUTATED_ROOTS.items():
            with self.subTest(variant=variant):
                result = check_rejection(self.raw[variant], self.profile, variant)
                self.assertEqual(result['rejected_wrapper'], wrapper)
                self.assertEqual(
                    result['roots'][wrapper]['call']['arguments'][0]['origin'].keys(),
                    {'call_result'},
                )
                self.assertFalse(result['actual_wrapper_lean_proved'])

    def test_other_variant_or_original_capture_does_not_supply_rejection(self):
        for label, raw in self.raw.items():
            for variant in MUTATED_ROOTS.keys() - {label}:
                with self.subTest(raw=label, variant=variant), self.assertRaises(ValueError):
                    check_rejection(raw, self.profile, variant)

    def test_incomplete_or_changed_selectors_do_not_supply_rejection(self):
        for variant in MUTATED_ROOTS:
            with self.subTest(variant=variant):
                raw = copy.deepcopy(self.raw[variant])
                raw['has_errors'] = True
                with self.assertRaisesRegex(ValueError, 'incomplete Charon input'):
                    check_rejection(raw, self.profile, variant)
                raw = copy.deepcopy(self.raw[variant])
                raw['translated']['options']['include'].pop()
                with self.assertRaisesRegex(ValueError, 'selectors'):
                    check_rejection(raw, self.profile, variant)

    def test_an_unrelated_wrapper_failure_cannot_supply_rejection(self):
        variant = 'origin-request'
        raw = copy.deepcopy(self.raw[variant])
        for function in raw['translated']['fun_decls']:
            if function and function['item_meta']['name'][-1] == {'Ident': ['encode_source_options', 0]}:
                function['body'] = {}
                break
        else:
            self.fail('original options wrapper missing from the supplied mutant')
        with self.assertRaisesRegex(ValueError, 'missing actual wrapper body'):
            check_rejection(raw, self.profile, variant)


class NativeAssertionBinding(unittest.TestCase):
    def fixture(self, variant):
        test, message = native.FAILURES[variant]
        qualified = native.TEST_PREFIX + test
        return (
            f'test {qualified} ... FAILED\n'
            f"thread '{qualified}' (1234) panicked at {native.SOURCE}:441:9:\n"
            f'assertion `left == right` failed: {message}\n'
            'test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 389 filtered out; finished in 0.3s\n'
        )

    def test_named_assertions_accept_the_original_panic_record(self):
        for variant in native.FAILURES:
            with self.subTest(variant=variant):
                native.check_assertions(variant, 101, self.fixture(variant))

    def test_other_panics_exit_status_or_missing_failure_do_not_count(self):
        name = 'origin-request'
        log = self.fixture(name)
        test, message = native.FAILURES[name]
        for case, returncode, changed in (
            ('message', 101, log.replace(message, 'a different equality failed')),
            ('thread', 101, log.replace(f"::{test}'", "::other_test'")),
            ('source', 101, log.replace(native.SOURCE, 'other.rs')),
            ('summary', 101, log.replace('1 passed; 1 failed', '0 passed; 1 failed')),
            ('timeout', 124, log),
        ):
            with self.subTest(case=case), self.assertRaisesRegex(ValueError, 'same complete-value assertion'):
                native.check_assertions(name, returncode, changed)


if __name__ == '__main__':
    unittest.main()
