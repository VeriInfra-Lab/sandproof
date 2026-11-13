"""Reject incomplete actual extraction evidence before admitting any proof view."""
import argparse
from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('lane_admission_formal', HERE / 'formal.py')
formal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(formal)


class Admission(unittest.TestCase):
    raw = None
    translation = None
    variant = 'correct'

    def generated(self, value):
        with tempfile.TemporaryDirectory(prefix='lane-metadata-') as name:
            directory = Path(name)
            (directory / 'translation.json').write_text(json.dumps(value))
            return formal.admit_generated(directory, self.variant)

    def test_complete_actual_metadata_is_admitted(self):
        formal.admit_ir(self.raw, ['rusqlite::error::Error'])
        self.generated(self.translation)

    def missing_function(self, suffix):
        changed = deepcopy(self.translation)
        changed['functions'] = [f for f in changed['functions'] if not f['lean_name'].endswith(suffix)]
        with self.assertRaisesRegex(ValueError, 'root/body missing'):
            self.generated(changed)

    def test_missing_validator_is_rejected(self):
        self.missing_function('.model.LifecycleRetentionLane.valid_using')

    def test_missing_protocol_root_is_rejected(self):
        self.missing_function('.accepted_validate')

    def test_opaque_validator_is_rejected(self):
        changed = deepcopy(self.translation)
        for function in changed['functions']:
            if function['lean_name'].endswith('.model.LifecycleRetentionLane.valid_using'):
                function['is_opaque'] = True
        with self.assertRaisesRegex(ValueError, 'root/body missing'):
            self.generated(changed)

    def test_missing_library_binding_is_rejected(self):
        changed = deepcopy(self.translation)
        changed['functions'] = [f for f in changed['functions']
                                if f['lean_name'] != 'alloc.vec.Vec.is_empty']
        with self.assertRaisesRegex(ValueError, 'business opacity'):
            self.generated(changed)

    def test_new_business_model_is_rejected(self):
        changed = deepcopy(self.translation)
        changed['functions'].append(dict(lean_name='trusted_inbox_transaction', is_opaque=True))
        with self.assertRaisesRegex(ValueError, 'business opacity'):
            self.generated(changed)

    def test_unreviewed_opaque_option_is_rejected(self):
        changed = deepcopy(self.raw)
        changed['translated']['options']['opaque'].append('sandproof_controller_service::controller')
        with self.assertRaisesRegex(ValueError, 'options'):
            formal.admit_ir(changed, ['rusqlite::error::Error'])

    def test_wrong_target_is_rejected(self):
        changed = deepcopy(self.raw)
        changed['translated']['target_information'][0]['key'] = 'different-target'
        with self.assertRaisesRegex(ValueError, 'target'):
            formal.admit_ir(changed, ['rusqlite::error::Error'])

    def test_missing_forwarder_is_rejected(self):
        changed = deepcopy(self.raw)
        for function in changed['translated']['fun_decls']:
            if function and function['item_meta']['name'][-1] == {'Ident': ['proof_retention_lane_valid_using', 0]}:
                function['item_meta']['name'][-1] = {'Ident': ['different_terminal', 0]}
        with self.assertRaisesRegex(ValueError, 'forwarding root'):
            formal.admit_ir(changed, ['rusqlite::error::Error'])

    def test_incomplete_charon_is_rejected(self):
        changed = deepcopy(self.raw)
        changed['has_errors'] = True
        with self.assertRaisesRegex(ValueError, 'incomplete Charon'):
            formal.admit_ir(changed, ['rusqlite::error::Error'])

    def test_wrong_actual_method_is_rejected(self):
        changed = deepcopy(self.raw)
        for function in changed['translated']['fun_decls']:
            if function and function['item_meta']['name'][-1] == {'Ident': ['valid_using', 0]}:
                function['item_meta']['name'][-1] = {'Ident': ['different_terminal', 0]}
        with self.assertRaisesRegex(ValueError, 'actual transparent'):
            formal.admit_ir(changed, ['rusqlite::error::Error'])

    def test_terminal_result_tags_are_admitted(self):
        formal.library_binding.result_tags(self.raw)

    def test_changed_result_tag_is_rejected(self):
        changed = deepcopy(self.raw)
        name = [{'Ident': [x, 0]} for x in ['core', 'result', 'Result']]
        declaration = next(t for t in changed['translated']['type_decls']
                           if t and t['item_meta']['name'] == name)
        declaration['kind']['Enum'][1]['discriminant'] = {'Signed': ['Isize', '0']}
        with self.assertRaisesRegex(ValueError, 'Result logical tags'):
            formal.library_binding.result_tags(changed)

    def test_disabled_typecheck_is_rejected(self):
        changed = deepcopy(self.raw)
        changed['translated']['options']['no_typecheck'] = True
        with self.assertRaisesRegex(ValueError, 'options'):
            formal.admit_ir(changed, ['rusqlite::error::Error'])

    def test_disabled_borrowcheck_is_rejected(self):
        changed = deepcopy(self.raw)
        changed['translated']['options']['skip_borrowck'] = True
        with self.assertRaisesRegex(ValueError, 'options'):
            formal.admit_ir(changed, ['rusqlite::error::Error'])

    def test_missing_generated_forwarder_is_rejected(self):
        changed = deepcopy(self.translation)
        changed['functions'] = [f for f in changed['functions'] if not f['lean_name'].endswith('.terminal_matches')]
        with self.assertRaisesRegex(ValueError, 'root/body missing'):
            self.generated(changed)

    def test_missing_encoder_helper_is_rejected(self):
        self.missing_function('.model.terminal_encoded_within_limit')

    def test_missing_transparent_option_is_rejected(self):
        self.missing_function('core.option.Option.and_then')

    def test_opaque_option_is_rejected(self):
        changed = deepcopy(self.translation)
        for f in changed['functions']:
            if f['lean_name'].endswith('core.option.Option.and_then'):
                f['is_opaque'] = True
        with self.assertRaisesRegex(ValueError, 'root/body missing'):
            self.generated(changed)

    def test_changed_existing_model_fragment_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'nonunique existing model'):
            formal.library_binding.fragment('def different := 0\n\n', 'core.result.Result.ok')

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--llbc', type=Path, required=True)
    parser.add_argument('--generated', type=Path, required=True)
    parser.add_argument('--variant', choices=['correct', *formal.native.preparation.MUTATIONS], required=True)
    args = parser.parse_args()
    Admission.raw = json.loads(args.llbc.read_text())
    Admission.translation = json.loads((args.generated / 'translation.json').read_text())
    Admission.variant = args.variant
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Admission))
    raise SystemExit(not result.wasSuccessful())
